// P2 Task 9：多设备同步、转交与离线补投。
// SyncEngine 在对端上线时互发 SyncOffer（会话版本向量），按差量分批 SyncPush
// 补齐；离线期间 Sending 的消息由重连后的同步补投（收到 Ack 置 Delivered），
// 不另建 outbox 表。回发 offer 以 (对端, 会话) 去重防乒乓，连接断开即清除。
// 注释中文，标识符英文。

use std::collections::HashSet;
use std::sync::{Arc, Mutex, RwLock};

use lanmsg_proto::consts::SYNC_BATCH;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::types::{ConvKindWire, ConvMeta, DeviceCert, SyncOffer, SyncPush};

use crate::error::CoreError;
use crate::identity::Identity;
use crate::net::{ConnManager, PeerInfo};
use crate::service::now_millis;
use crate::store::{ContactState, ConvKindRow, ConvRow, Gate, Store};

/// 多设备同步引擎。
#[derive(Clone)]
pub struct SyncEngine {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
    conns: ConnManager,
    /// 本连接上已回发过 offer 的 (对端设备, 会话) 集合；连接断开时清除对应条目。
    replied: Arc<Mutex<HashSet<(DeviceId, ConvId)>>>,
}

/// `prepare_offer` 的产物：交给后台任务执行的发送计划。
pub struct OfferPlan {
    offer: SyncOffer,
    local_vv: lanmsg_proto::types::VersionVector,
    push: bool,
    reply: bool,
}

/// 会话成员规则：SelfDevices 仅接受自己用户；Dm 接受自己与对端用户；
/// 其他 kind（Guest/LocalSend/Station）不参与同步，一律拒绝。
pub fn conv_member_ok(conv: &ConvRow, author_user: &UserId, self_user: &UserId) -> bool {
    match conv.kind {
        ConvKindRow::SelfDevices => author_user == self_user,
        ConvKindRow::Dm => author_user == self_user || conv.peer_user.as_ref() == Some(author_user),
        _ => false,
    }
}

impl SyncEngine {
    /// 构造同步引擎。
    pub fn new(store: Arc<Store>, identity: Arc<RwLock<Identity>>, conns: ConnManager) -> Self {
        Self {
            store,
            identity,
            conns,
            replied: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 列出与对端共享的会话（计划规则）：
    /// 对端是自己设备 → SelfDevices（排第一）+ 全部 gate = Open 的 Dm；
    /// 对端是其他用户 U → 仅 `dm(self, U)`（gate = Open 时）。
    pub fn shared_convs(
        &self,
        peer: &PeerInfo,
    ) -> Result<Vec<(ConvId, Option<ConvMeta>)>, CoreError> {
        let id = self.identity.read().unwrap();
        let self_user = id.user_id().ok_or(CoreError::NotInitialized)?;
        let Some(cert) = &peer.cert else {
            return Ok(Vec::new()); // 未初始化对端无可同步
        };
        let peer_user = cert.user;
        if peer_user == self_user {
            let mut out = Vec::new();
            // SelfDevices 排第一（先同步联系人/资料等隐藏系统消息）。
            let self_conv = ConvId::self_devices(&self_user);
            if self.store.get_conversation(&self_conv)?.is_some() {
                out.push((
                    self_conv,
                    Some(ConvMeta {
                        kind: ConvKindWire::SelfDevices,
                        peer_user: None,
                    }),
                ));
            }
            // list_conversations 只返回 gate = 'open' 的会话。
            for row in self.store.list_conversations()? {
                if row.kind == ConvKindRow::Dm {
                    out.push((
                        row.conv,
                        Some(ConvMeta {
                            kind: ConvKindWire::Dm,
                            peer_user: row.peer_user,
                        }),
                    ));
                }
            }
            Ok(out)
        } else {
            let conv = ConvId::dm(&self_user, &peer_user);
            let open = matches!(self.store.get_conversation(&conv)?, Some(r) if r.kind == ConvKindRow::Dm)
                && self.store.get_gate(&conv)?.0 == Gate::Open;
            if open {
                Ok(vec![(conv, None)])
            } else {
                Ok(Vec::new())
            }
        }
    }

    /// 对端上线：自己设备先发证书（自己全部未吊销设备 + 全部 Accepted 联系人
    /// 设备），再对每个共享会话发 SyncOffer。陌生人同样调用（以投递离线消息）。
    pub async fn on_peer_up(&self, peer: &PeerInfo) -> Result<(), CoreError> {
        let Some(cert) = &peer.cert else {
            return Ok(());
        };
        let self_user = {
            let id = self.identity.read().unwrap();
            id.user_id().ok_or(CoreError::NotInitialized)?
        };

        if cert.user == self_user {
            let certs = self.sync_certs(&self_user)?;
            if !certs.is_empty() {
                let _ = self
                    .conns
                    .send(&peer.device, Frame::CertUpdate(certs))
                    .await;
            }
        }

        for (conv, meta) in self.shared_convs(peer)? {
            let vv = self.store.version_vector(&conv)?;
            let _ = self
                .conns
                .send(&peer.device, Frame::SyncOffer(SyncOffer { conv, vv, meta }))
                .await;
        }
        Ok(())
    }

    /// 对端下线：清除该连接上的 offer 回发去重记录。
    pub fn on_peer_down(&self, device: &DeviceId) {
        self.replied.lock().unwrap().retain(|(d, _)| d != device);
    }

    /// 处理对端 SyncOffer 的同步部分（在事件循环内顺序执行，不 await）：
    /// 1. 会话不存在 → 按创建规则建会话（自己设备按 meta；其他用户仅 dm(self, U)
    ///    先建 Pending），不满足则丢弃（返回 None）；
    /// 2. 计算是否推送（gate = Open）与是否回发本端 offer（对方向量里有我缺的
    ///    作者，且同一连接同一会话最多一次，去重记录在此登记）。
    ///
    /// 建会话必须在循环内完成：同一对端紧随其后的 SyncPush 要求会话已存在。
    /// 向外发送（可能等待对端写队列）交给 [`SyncEngine::run_offer`] 在后台执行。
    pub fn prepare_offer(
        &self,
        from: &PeerInfo,
        offer: SyncOffer,
    ) -> Result<Option<OfferPlan>, CoreError> {
        let Some(cert) = &from.cert else {
            return Ok(None);
        };
        let self_user = {
            let id = self.identity.read().unwrap();
            id.user_id().ok_or(CoreError::NotInitialized)?
        };
        let now = now_millis();
        let peer_user = cert.user;

        if self.store.get_conversation(&offer.conv)?.is_none()
            && !self.create_missing_conv(&self_user, peer_user, &offer, now)?
        {
            return Ok(None); // 不满足创建规则，丢弃
        }

        let gate_open = self.store.get_gate(&offer.conv)?.0 == Gate::Open;
        let local_vv = self.store.version_vector(&offer.conv)?;

        // 对方有我缺的 → 回发本端 offer（去重防乒乓）。
        let need = offer
            .vv
            .0
            .iter()
            .any(|(a, v)| local_vv.0.get(a).is_none_or(|m| v > m));
        let reply = need
            && self
                .replied
                .lock()
                .unwrap()
                .insert((from.device, offer.conv));
        Ok(Some(OfferPlan {
            offer,
            local_vv,
            push: gate_open,
            reply,
        }))
    }

    /// 执行 [`OfferPlan`] 的发送部分：对本地向量中的每个作者按 SYNC_BATCH 分批
    /// 推送差量，再按需回发本端 offer。可能长时间等待对端写队列，须在后台任务中调用。
    pub async fn run_offer(&self, from: &PeerInfo, plan: OfferPlan) -> Result<(), CoreError> {
        let OfferPlan {
            offer,
            local_vv,
            push,
            reply,
        } = plan;
        if push {
            for (author, mine) in &local_vv.0 {
                let mut cursor = offer.vv.0.get(author).copied().unwrap_or(0);
                if cursor >= *mine {
                    continue;
                }
                loop {
                    let envs = self.store.messages_after(
                        &offer.conv,
                        author,
                        cursor,
                        SYNC_BATCH as u32,
                    )?;
                    if envs.is_empty() {
                        break;
                    }
                    cursor = envs.last().map(|e| e.conv_seq).unwrap_or(cursor);
                    self.conns
                        .send(&from.device, Frame::SyncPush(SyncPush { envelopes: envs }))
                        .await?;
                    if cursor >= *mine {
                        break;
                    }
                }
            }
        }
        if reply {
            let _ = self
                .conns
                .send(
                    &from.device,
                    Frame::SyncOffer(SyncOffer {
                        conv: offer.conv,
                        vv: local_vv,
                        meta: None,
                    }),
                )
                .await;
        }
        Ok(())
    }

    /// 同步用证书：自己全部未吊销设备 + 全部 Accepted 联系人的设备证书。
    fn sync_certs(&self, self_user: &UserId) -> Result<Vec<DeviceCert>, CoreError> {
        let mut out = Vec::new();
        for d in self.store.devices_of_user(self_user)? {
            if !d.revoked {
                out.push(d.cert);
            }
        }
        for c in self.store.list_contacts(Some(ContactState::Accepted))? {
            for d in self.store.devices_of_user(&c.user)? {
                if !d.revoked {
                    out.push(d.cert);
                }
            }
        }
        Ok(out)
    }

    /// 会话不存在时按创建规则建会话；返回 false 表示不满足规则应丢弃。
    fn create_missing_conv(
        &self,
        self_user: &UserId,
        peer_user: UserId,
        offer: &SyncOffer,
        now: i64,
    ) -> Result<bool, CoreError> {
        let row = if peer_user == *self_user {
            // 自己设备：按 meta 创建，conv 必须与 meta 一致，否则丢弃。
            let Some(meta) = &offer.meta else {
                return Ok(false);
            };
            match meta.kind {
                ConvKindWire::SelfDevices => {
                    if offer.conv != ConvId::self_devices(self_user) {
                        return Ok(false);
                    }
                    ConvRow {
                        conv: offer.conv,
                        kind: ConvKindRow::SelfDevices,
                        peer_user: None,
                        title: None,
                        created_at: now,
                    }
                }
                ConvKindWire::Dm => {
                    let Some(pu) = meta.peer_user else {
                        return Ok(false);
                    };
                    if offer.conv != ConvId::dm(self_user, &pu) {
                        return Ok(false);
                    }
                    ConvRow {
                        conv: offer.conv,
                        kind: ConvKindRow::Dm,
                        peer_user: Some(pu),
                        title: None,
                        created_at: now,
                    }
                }
            }
        } else {
            // 其他用户：仅 dm(self, U)；先建 Pending，第一条被 Accept 的消息置 Open。
            if offer.conv != ConvId::dm(self_user, &peer_user) {
                return Ok(false);
            }
            ConvRow {
                conv: offer.conv,
                kind: ConvKindRow::Dm,
                peer_user: Some(peer_user),
                title: None,
                created_at: now,
            }
        };
        self.store.ensure_conversation(&row)?;
        // 陌生人的 Dm 会话先 Pending；自己设备按 meta 建的保持默认 Open。
        if row.kind == ConvKindRow::Dm && peer_user != *self_user {
            let is_contact = self
                .store
                .get_contact(&peer_user)?
                .is_some_and(|r| r.state == ContactState::Accepted);
            if !is_contact {
                self.store.set_gate(&offer.conv, Gate::Pending, None)?;
            }
        }
        Ok(true)
    }
}
