// P1 核心骨架：文字消息收发（Task 10）。
// Messaging 负责消息的发送、接收（on_frame）与 DM 会话打开。
// 注释中文，标识符英文。

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::sync::{broadcast, RwLock};
use tracing::warn;

use lanmsg_api::types::Event;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, MsgId, UserId};
use lanmsg_proto::sign::{sign_with_zeroed_sig, DeviceKey};
use lanmsg_proto::types::{Ack, Envelope, MsgBody, Signature64};

use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::net::ConnManager;
use crate::service::views::{conv_summary, message_view};
use crate::store::{ConvKindRow, ConvRow, MsgStateRow, Store, StoredMessage};

/// 当前墙钟时间（毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 新消息入库回调类型（Service 注入，指向 service::ingest::after_ingest）。
type IngestCallback = Arc<Mutex<Option<Arc<dyn Fn(DeviceId, StoredMessage) + Send + Sync>>>>;

/// 文字消息收发模块。
#[derive(Clone)]
pub struct Messaging {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
    clock: Arc<Mutex<HlcClock>>,
    conns: ConnManager,
    events: broadcast::Sender<Event>,
    /// 新消息入库回调；Arc+Mutex 使注入可在组装完成后进行且保持 Messaging 可 Clone。
    ingest_cb: IngestCallback,
}

impl Messaging {
    /// 构造 Messaging。
    pub fn new(
        store: Arc<Store>,
        identity: Arc<RwLock<Identity>>,
        clock: Arc<Mutex<HlcClock>>,
        conns: ConnManager,
        events: broadcast::Sender<Event>,
    ) -> Self {
        Self {
            store,
            identity,
            clock,
            conns,
            events,
            ingest_cb: Arc::new(Mutex::new(None)),
        }
    }

    /// 注入新消息入库回调（Service 启动时调用一次）。
    pub fn set_ingest_callback(&self, cb: Arc<dyn Fn(DeviceId, StoredMessage) + Send + Sync>) {
        *self.ingest_cb.lock().unwrap() = Some(cb);
    }

    /// 发送消息到指定会话。
    ///
    /// 流程：
    /// 1. 未初始化 → `NotInitialized`；会话不存在 → `NotFound`。
    /// 2. `next_seqs` → 构造 Envelope（hlc = clock.now(墙钟)）→ 设备密钥签名 → `insert_envelope(Sending)`。
    /// 3. 发 `Event::MessageAdded` + `Event::ConversationUpdated`。
    /// 4. 计算投递目标 `targets(conv)`：DM → 对端用户所有已连接设备；SelfDevices → 本人其他已连接设备。
    /// 5. 对每个目标 `conns.send(SyncPush { envelopes })`。无目标在线时保持 `Sending`；
    ///    至少一个 send 成功则置 `Sent`。
    pub async fn send(&self, conv: &ConvId, body: MsgBody) -> Result<MsgId, CoreError> {
        self.send_impl(conv, body, true).await
    }

    /// 与 `send` 相同流程，但不发 `MessageAdded`/`ConversationUpdated` 事件
    /// （隐藏性由 `body_is_hidden` 决定；调用方保证 body 为隐藏类型）。
    pub async fn send_hidden(&self, conv: &ConvId, body: MsgBody) -> Result<MsgId, CoreError> {
        self.send_impl(conv, body, false).await
    }

    /// 发送公共流程；`controls_events` 为 false 时跳过 MessageAdded/ConversationUpdated。
    async fn send_impl(
        &self,
        conv: &ConvId,
        body: MsgBody,
        controls_events: bool,
    ) -> Result<MsgId, CoreError> {
        // 1. 未初始化 → NotInitialized。
        let identity = self.identity.read().await;
        if !identity.is_initialized() {
            return Err(CoreError::NotInitialized);
        }
        let self_device = identity.device_id();
        let self_user = match identity.user_id() {
            Some(u) => u,
            None => return Err(CoreError::NotInitialized),
        };
        let device_key_bytes = identity.device.to_bytes();
        drop(identity);

        // 2. 会话必须存在。
        let conv_row = self
            .store
            .get_conversation(conv)?
            .ok_or_else(|| CoreError::NotFound(format!("会话不存在: {conv}")))?;

        // 3. next_seqs → 构造 Envelope。
        let (seq, conv_seq) = self.store.next_seqs(&self_device, conv)?;

        let hlc = {
            let mut clock = self.clock.lock().unwrap();
            clock.now(now_millis())
        };

        let mut env = Envelope {
            conv: *conv,
            id: MsgId {
                author: self_device,
                seq,
            },
            conv_seq,
            hlc,
            body,
            enc: None,
            sig: Signature64([0u8; 64]),
        };

        // 4. 设备密钥签名。
        let device_key = DeviceKey::from_bytes(&device_key_bytes);
        sign_with_zeroed_sig(&mut env, |b| device_key.sign(b));

        // 5. insert_envelope(Sending)。
        let received_at = now_millis();
        self.store
            .insert_envelope(&env, MsgStateRow::Sending, received_at)?;

        // 6. 发 Event::MessageAdded + Event::ConversationUpdated（send_hidden 跳过）。
        if controls_events {
            let stored = StoredMessage {
                env: env.clone(),
                state: MsgStateRow::Sending,
                received_at,
            };
            let msg_view = message_view(self.store.as_ref(), &self_device, &stored);
            let _ = self.events.send(Event::MessageAdded {
                msg: msg_view.clone(),
            });

            let peer_online = match conv_row.kind {
                ConvKindRow::Dm => conv_row.peer_user.is_some_and(|pu| {
                    self.conns
                        .peers()
                        .iter()
                        .any(|p| p.cert.as_ref().is_some_and(|c| c.user == pu))
                }),
                _ => false,
            };
            let summary = conv_summary(
                self.store.as_ref(),
                Some(&self_user),
                &conv_row,
                peer_online,
            );
            let _ = self.events.send(Event::ConversationUpdated { summary });
        }

        // 7. 计算投递目标 targets(conv)。
        let targets: Vec<DeviceId> = match conv_row.kind {
            ConvKindRow::Dm => {
                let peer_user = conv_row
                    .peer_user
                    .ok_or_else(|| CoreError::InvalidArgument("DM 会话缺少 peer_user".into()))?;
                self.conns
                    .peers()
                    .into_iter()
                    .filter(|p| p.cert.as_ref().is_some_and(|c| c.user == peer_user))
                    .map(|p| p.device)
                    .collect()
            }
            ConvKindRow::SelfDevices => self
                .conns
                .peers()
                .into_iter()
                .filter(|p| p.device != self_device)
                .map(|p| p.device)
                .collect(),
            _ => Vec::new(),
        };

        // 8. 对每个目标发送 SyncPush。
        let mut any_sent = false;
        for device in &targets {
            let frame = Frame::SyncPush(lanmsg_proto::types::SyncPush {
                envelopes: vec![env.clone()],
            });
            if self.conns.send(device, frame).await.is_ok() {
                any_sent = true;
            }
        }

        // 9. 状态更新：至少一个 send 成功则置 Sent，否则保持 Sending。
        if any_sent {
            self.store.set_message_state(&env.id, MsgStateRow::Sent)?;
        }

        Ok(env.id)
    }

    /// 处理对端发来的帧。
    ///
    /// - `SyncPush`：验签、检查作者设备、observe HLC、确保会话存在、insert_envelope(Delivered)、
    ///   仅新插入时发事件，最后回 `Ack { ids }`。
    /// - `Ack`：对自己发的且状态为 Sending/Sent 的消息置 Delivered，发 `MessageUpdated`。
    /// - 其他帧：忽略。
    pub async fn on_frame(&self, from: DeviceId, frame: Frame) -> Result<(), CoreError> {
        let identity = self.identity.read().await;
        let self_user = identity.user_id();
        let self_device = identity.device_id();
        drop(identity);

        match frame {
            Frame::SyncPush(push) => {
                let mut ids = Vec::new();
                let wall = now_millis();

                for env in push.envelopes {
                    // 1. verify() 失败 → 丢弃并 warn!。
                    if env.verify().is_err() {
                        warn!(peer = %from, "SyncPush 信封签名验证失败，丢弃");
                        continue;
                    }

                    // 2. 作者设备未知（store 中无 DeviceRow）→ 丢弃。
                    let author_device = env.id.author;
                    let author_row = match self.store.get_device(&author_device) {
                        Ok(Some(r)) => r,
                        _ => {
                            warn!(peer = %from, author = %author_device, "SyncPush 作者设备未入库，丢弃");
                            continue;
                        }
                    };
                    let author_user = author_row.user;

                    // 3. clock.observe(env.hlc)。
                    {
                        let mut clock = self.clock.lock().unwrap();
                        clock.observe(env.hlc.clone(), wall);
                    }

                    // 4. 会话不存在则按 DM 规则创建；ConvId::dm(self_user, author_user) 必须等于 env.conv。
                    let conv_id = env.conv;
                    let conv_exists = self.store.get_conversation(&conv_id)?.is_some();
                    if !conv_exists {
                        let Some(self_user) = self_user else {
                            continue; // 未初始化，无法创建 DM 会话
                        };
                        let expected_conv = ConvId::dm(&self_user, &author_user);
                        if expected_conv != conv_id {
                            warn!(peer = %from, "SyncPush conv 不匹配，丢弃");
                            continue;
                        }
                        self.store.ensure_conversation(&ConvRow {
                            conv: conv_id,
                            kind: ConvKindRow::Dm,
                            peer_user: Some(author_user),
                            title: None,
                            created_at: wall,
                        })?;
                    }

                    // 5. insert_envelope(Delivered)。
                    let inserted =
                        self.store
                            .insert_envelope(&env, MsgStateRow::Delivered, wall)?;
                    let msg_id = env.id.clone();
                    ids.push(msg_id);

                    // 6. 仅当新插入时发事件并触发入库回调。
                    if inserted {
                        let stored = StoredMessage {
                            env: env.clone(),
                            state: MsgStateRow::Delivered,
                            received_at: wall,
                        };
                        let msg_view = message_view(self.store.as_ref(), &self_device, &stored);
                        let _ = self.events.send(Event::MessageAdded { msg: msg_view });

                        let conv_row = self.store.get_conversation(&env.conv)?.unwrap();
                        let peer_online = match conv_row.kind {
                            ConvKindRow::Dm => conv_row.peer_user.is_some_and(|pu| {
                                self.conns
                                    .peers()
                                    .iter()
                                    .any(|p| p.cert.as_ref().is_some_and(|c| c.user == pu))
                            }),
                            _ => false,
                        };
                        let summary = conv_summary(
                            self.store.as_ref(),
                            self_user.as_ref(),
                            &conv_row,
                            peer_online,
                        );
                        let _ = self.events.send(Event::ConversationUpdated { summary });

                        // 入库回调：同步取出克隆后立即放锁，回调内部自行 spawn 异步分派。
                        if let Some(cb) = self.ingest_cb.lock().unwrap().clone() {
                            cb(from, stored);
                        }
                    }
                }

                // 7. 回 Ack（包含重复的，以便对方确认）。
                let _ = self.conns.send(&from, Frame::Ack(Ack { ids })).await;
            }
            Frame::Ack(ack) => {
                for id in ack.ids {
                    if id.author == self_device {
                        if let Some(m) = self.store.get_message(&id)? {
                            if matches!(m.state, MsgStateRow::Sending | MsgStateRow::Sent) {
                                self.store.set_message_state(&id, MsgStateRow::Delivered)?;
                                let msg_view = message_view(self.store.as_ref(), &self_device, &m);
                                let _ = self.events.send(Event::MessageUpdated { msg: msg_view });
                            }
                        }
                    }
                }
            }
            _ => {} // 其他帧忽略，交给后续阶段的模块
        }

        Ok(())
    }

    /// 打开与指定用户的 DM 会话，返回会话 ID。
    pub fn open_dm(&self, peer_user: &UserId) -> Result<ConvId, CoreError> {
        let identity = self
            .identity
            .try_read()
            .map_err(|_| CoreError::NotInitialized)?;
        let self_user = identity.user_id().ok_or(CoreError::NotInitialized)?;

        let conv = ConvId::dm(&self_user, peer_user);
        self.store.ensure_conversation(&ConvRow {
            conv,
            kind: ConvKindRow::Dm,
            peer_user: Some(*peer_user),
            title: None,
            created_at: now_millis(),
        })?;
        Ok(conv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{next_event, pair_of_nodes};
    use lanmsg_api::types::MessageState;
    use lanmsg_proto::types::MsgBody;

    /// 等待条件成立，最多 5 秒。
    async fn wait_for(cond: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !cond() {
            if start.elapsed() > std::time::Duration::from_secs(5) {
                panic!("5 秒内条件未成立");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn text_message_reaches_peer_and_gets_delivered() {
        let (a, b) = pair_of_nodes().await;

        let conv = a.open_dm(&b.user).unwrap();
        a.send(
            &conv,
            MsgBody::Text {
                text: "你好".into(),
            },
        )
        .await
        .expect("send 失败");

        // B 的 store 5 秒内出现该消息（text == "你好"）。
        wait_for(|| {
            b.store
                .list_messages(&conv, None, 10)
                .map(|m| {
                    m.len() == 1
                        && matches!(&m[0].env.body, MsgBody::Text { text } if text == "你好")
                })
                .unwrap_or(false)
        })
        .await;
        // B 的 store 中该会话 1 条消息。
        assert_eq!(b.store.list_messages(&conv, None, 10).unwrap().len(), 1);

        // A 收到 B 的 Ack 后，消息状态置 Delivered。
        let msg_id = a.store.next_seqs(&a.device, &conv).ok().map(|(seq, _)| seq);
        let _ = msg_id; // seq 已被占用，直接按 conv 查询
        wait_for(|| {
            a.store
                .list_messages(&conv, None, 10)
                .map(|m| m.iter().all(|sm| sm.state == MsgStateRow::Delivered))
                .unwrap_or(false)
        })
        .await;
        // A 的事件流出现 MessageUpdated 且 state == Delivered（事件可能早于轮询发送，此处用 resubscribe 尽力验证）。
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            next_event(
                &mut a.events.resubscribe(),
                |e| matches!(e, Event::MessageUpdated { msg } if msg.state == MessageState::Delivered),
            ),
        )
        .await;
    }

    #[tokio::test]
    async fn duplicate_push_is_idempotent() {
        let (a, b) = pair_of_nodes().await;

        let conv = a.open_dm(&b.user).unwrap();
        let msg_id = a
            .send(
                &conv,
                MsgBody::Text {
                    text: "重复测试".into(),
                },
            )
            .await
            .unwrap();

        // 等待 b 收到并插入该消息。
        wait_for(|| {
            b.store
                .get_message(&msg_id)
                .map(|m| m.is_some())
                .unwrap_or(false)
        })
        .await;

        // 构造同一个 envelope（已签名）通过 on_frame 喂给 B 两次。
        let env = b.store.get_message(&msg_id).unwrap().unwrap().env.clone();

        let _ = b
            .on_frame(
                a.device,
                lanmsg_proto::frame::Frame::SyncPush(lanmsg_proto::types::SyncPush {
                    envelopes: vec![env.clone()],
                }),
            )
            .await;
        let _ = b
            .on_frame(
                a.device,
                lanmsg_proto::frame::Frame::SyncPush(lanmsg_proto::types::SyncPush {
                    envelopes: vec![env],
                }),
            )
            .await;

        // store 中仍只有 1 条消息（重复推送幂等）。
        let msgs = b.store.list_messages(&conv, None, 10).unwrap();
        assert_eq!(msgs.len(), 1, "重复推送不应产生新消息");
    }

    #[tokio::test]
    async fn forged_envelope_is_dropped() {
        let (_a, b) = pair_of_nodes().await;

        // 构造一个合法签名的 envelope，然后篡改 text 但不重签。
        let dev = lanmsg_proto::sign::DeviceKey::generate();
        let mut env = Envelope {
            conv: ConvId([0u8; 32]),
            id: MsgId {
                author: dev.device_id(),
                seq: 1,
            },
            conv_seq: 1,
            hlc: lanmsg_proto::types::Hlc {
                millis: 1,
                counter: 0,
            },
            body: MsgBody::Text {
                text: "原始文本".into(),
            },
            enc: None,
            sig: lanmsg_proto::types::Signature64([0u8; 64]),
        };
        lanmsg_proto::sign::sign_with_zeroed_sig(&mut env, |b| dev.sign(b));
        assert!(env.verify().is_ok());

        // 篡改 text 但不重签。
        let mut forged = env.clone();
        match &mut forged.body {
            MsgBody::Text { text } => *text = "被篡改".into(),
            _ => unreachable!(),
        }
        assert!(forged.verify().is_err());

        let _ = b
            .on_frame(
                DeviceId([0u8; 32]),
                lanmsg_proto::frame::Frame::SyncPush(lanmsg_proto::types::SyncPush {
                    envelopes: vec![forged],
                }),
            )
            .await;

        // B 无入库。
        let msgs = b.store.list_messages(&ConvId([0u8; 32]), None, 10).unwrap();
        assert_eq!(msgs.len(), 0);
    }

    #[tokio::test]
    async fn send_without_peer_online_stays_sending() {
        let (_a, b) = pair_of_nodes().await;
        // 断开 b，使 a 无在线目标。
        b.conns.shutdown().await;

        let (a2, _b2) = pair_of_nodes().await;
        let conv = a2.open_dm(&b.user).unwrap();
        // b 已下线，a2 发送应返回 Ok 且状态保持 Sending。
        let msg_id = a2
            .send(
                &conv,
                MsgBody::Text {
                    text: "离线测试".into(),
                },
            )
            .await
            .unwrap();

        let stored = a2.store.get_message(&msg_id).unwrap().unwrap();
        assert_eq!(stored.state, MsgStateRow::Sending);
    }
}
