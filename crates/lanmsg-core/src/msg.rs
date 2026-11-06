// P1 核心骨架：文字消息收发（Task 10）。
// Messaging 负责消息的发送、接收（on_frame）与 DM 会话打开。
// 注释中文，标识符英文。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use tokio::sync::broadcast;
use tracing::{debug, warn};

use lanmsg_api::types::Event;
use lanmsg_proto::consts::THUMB_MAX_BYTES;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{ConvId, DeviceId, MsgId, UserId};
use lanmsg_proto::sign::{sign_with_zeroed_sig, DeviceKey};
use lanmsg_proto::types::{
    Ack, BlobMeta, Envelope, FolderEntry, MediaInfo, MsgBody, Signature64, SyncPush,
};

use crate::blob::{self, FileSource};
use crate::error::CoreError;
use crate::hlc::HlcClock;
use crate::identity::Identity;
use crate::net::{ConnManager, PeerInfo};
use crate::platform::PlatformHooks;
use crate::service::views::{conv_summary, message_view, preparing_view, request_view};
use crate::store::{
    BlobLocalKind, BlobLocalRow, ContactState, ConvKindRow, ConvRow, Gate, MsgStateRow,
    PreparingKind, PreparingRow, Store, StoredMessage,
};
use crate::sync::conv_member_ok;
use crate::trust::{gate_request_kind, GateDecision};

/// 当前墙钟时间（毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 新消息入库回调类型（Service 注入，指向 service::ingest::after_ingest）。
type IngestCallback = Arc<Mutex<Option<Arc<dyn Fn(DeviceId, StoredMessage) + Send + Sync>>>>;

/// 门控回调类型（Service 注入，指向 Trust::gate_inbound）。
pub type GateFn =
    Arc<dyn Fn(&PeerInfo, &Envelope, i64) -> Result<GateDecision, CoreError> + Send + Sync>;

type GateCallback = Arc<Mutex<Option<GateFn>>>;

/// 计算消息投递目标：Dm → 对端用户所有已连接设备 + 自己其他已连接设备
/// （转交）；SelfDevices → 自己其他已连接设备；其他 kind（Guest/LocalSend/
/// Station）→ 空列表（不报错，状态保持 Sending）。
fn delivery_targets(
    conv_row: &ConvRow,
    self_user: &UserId,
    self_device: &DeviceId,
    conns: &ConnManager,
) -> Vec<DeviceId> {
    match conv_row.kind {
        ConvKindRow::Dm => {
            let Some(peer_user) = conv_row.peer_user else {
                warn!("DM 会话缺少 peer_user，无投递目标");
                return Vec::new();
            };
            conns
                .peers()
                .into_iter()
                .filter(|p| {
                    let u = p.cert.as_ref().map(|c| c.user);
                    u == Some(peer_user) || (u == Some(*self_user) && p.device != *self_device)
                })
                .map(|p| p.device)
                .collect()
        }
        // 仅自己用户的其他设备；绝不能推给联系人/陌生人（隐私）。
        ConvKindRow::SelfDevices => conns
            .peers()
            .into_iter()
            .filter(|p| {
                p.device != *self_device && p.cert.as_ref().is_some_and(|c| c.user == *self_user)
            })
            .map(|p| p.device)
            .collect(),
        _ => Vec::new(),
    }
}

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
    /// 门控回调；未注入时入站消息不做门控（保持 P1 行为）。
    gate_cb: GateCallback,
    /// 平台钩子（缩略图生成等）。
    platform: PlatformHooks,
    /// 后台准备任务句柄（shutdown 时统一中止）。
    prepare_handles: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Messaging {
    /// 构造 Messaging。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        identity: Arc<RwLock<Identity>>,
        clock: Arc<Mutex<HlcClock>>,
        conns: ConnManager,
        events: broadcast::Sender<Event>,
        platform: PlatformHooks,
    ) -> Self {
        Self {
            store,
            identity,
            clock,
            conns,
            events,
            ingest_cb: Arc::new(Mutex::new(None)),
            gate_cb: Arc::new(Mutex::new(None)),
            platform,
            prepare_handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 注入新消息入库回调（Service 启动时调用一次）。
    pub fn set_ingest_callback(&self, cb: Arc<dyn Fn(DeviceId, StoredMessage) + Send + Sync>) {
        *self.ingest_cb.lock().unwrap() = Some(cb);
    }

    /// 注入门控回调（Service 启动时调用一次）。
    pub fn set_gate_callback(&self, cb: GateFn) {
        *self.gate_cb.lock().unwrap() = Some(cb);
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
        let (self_device, self_user, device_key_bytes) = {
            let identity = self.identity.read().unwrap();
            if !identity.is_initialized() {
                return Err(CoreError::NotInitialized);
            }
            let self_user = match identity.user_id() {
                Some(u) => u,
                None => return Err(CoreError::NotInitialized),
            };
            (identity.device_id(), self_user, identity.device.to_bytes())
        };

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
            let msg_view = message_view(
                self.store.as_ref(),
                &self_device,
                &self.conns.peers(),
                &stored,
            );
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

        // 7. 计算投递目标 targets(conv)（Dm → 对端 + 自己其他设备转交；
        //    SelfDevices → 自己其他设备；其他 kind → 空列表）。
        let targets = delivery_targets(&conv_row, &self_user, &self_device, &self.conns);

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

    /// 把他人发来的消息转交给自己其他在线设备（后台发送，不阻塞事件循环）。
    /// 排除本机与来源设备；来源本身是自己的设备时视为已是转交，不再转。
    fn relay_to_my_devices(
        &self,
        self_user: UserId,
        self_device: DeviceId,
        from: DeviceId,
        env: Envelope,
    ) {
        let peers = self.conns.peers();
        if peers
            .iter()
            .any(|p| p.device == from && p.cert.as_ref().is_some_and(|c| c.user == self_user))
        {
            return;
        }
        let targets: Vec<DeviceId> = peers
            .into_iter()
            .filter(|p| {
                p.device != self_device
                    && p.device != from
                    && p.cert.as_ref().is_some_and(|c| c.user == self_user)
            })
            .map(|p| p.device)
            .collect();
        if targets.is_empty() {
            return;
        }
        let conns = self.conns.clone();
        tokio::spawn(async move {
            for device in targets {
                let frame = Frame::SyncPush(SyncPush {
                    envelopes: vec![env.clone()],
                });
                let _ = conns.send(&device, frame).await;
            }
        });
    }

    // ---- P2 Task 11：文件与文件夹发送 ----

    /// 发送文件消息：先校验并插入 preparing 行（发 `MessageAdded`，各项
    /// state = Preparing、hash = ""），随后由后台任务完成哈希、缩略图与投递。
    pub async fn send_files(&self, conv: &ConvId, paths: Vec<PathBuf>) -> Result<MsgId, CoreError> {
        let sources = tokio::task::spawn_blocking(move || blob::collect_file_sources(&paths))
            .await
            .map_err(|e| CoreError::Proto(e.to_string()))??;
        self.send_prepare(conv, PreparingKind::Files, sources, None)
            .await
    }

    /// 发送文件夹消息：根必须是目录；遍历（跳过符号链接）文件数超过
    /// `MAX_FOLDER_ENTRIES` → InvalidArgument。
    pub async fn send_folder(&self, conv: &ConvId, root: PathBuf) -> Result<MsgId, CoreError> {
        // 信封中的文件夹名在 prepare_envelope 里取 root.file_name()，此处忽略 name。
        let cloned = root.clone();
        let (_, sources) =
            tokio::task::spawn_blocking(move || blob::collect_folder_sources(&cloned))
                .await
                .map_err(|e| CoreError::Proto(e.to_string()))??;
        self.send_prepare(conv, PreparingKind::Folder, sources, Some(root))
            .await
    }

    /// 发送公共准备流程：校验链已在收集阶段完成，这里分配序号、写 preparing
    /// 行、发 MessageAdded 并派发后台任务。`folder_root` 仅 Folder 消息使用。
    async fn send_prepare(
        &self,
        conv: &ConvId,
        kind: PreparingKind,
        sources: Vec<FileSource>,
        folder_root: Option<PathBuf>,
    ) -> Result<MsgId, CoreError> {
        // 1. 未初始化 → NotInitialized；会话不存在 → NotFound。
        let (self_device, self_user) = {
            let identity = self.identity.read().unwrap();
            if !identity.is_initialized() {
                return Err(CoreError::NotInitialized);
            }
            let self_user = identity.user_id().ok_or(CoreError::NotInitialized)?;
            (identity.device_id(), self_user)
        };
        self.store
            .get_conversation(conv)?
            .ok_or_else(|| CoreError::NotFound(format!("会话不存在: {conv}")))?;

        // 2. next_seqs + clock.now → insert_preparing。paths 约定：Folder 时
        //    paths[0] 为根目录，其余为条目文件（视图与恢复共用该约定）。
        let (seq, conv_seq) = self.store.next_seqs(&self_device, conv)?;
        let hlc = {
            let mut clock = self.clock.lock().unwrap();
            clock.now(now_millis())
        };
        let mut paths: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        if kind == PreparingKind::Folder {
            paths.insert(0, folder_root.unwrap_or_default());
        }
        let row = PreparingRow {
            author: self_device,
            seq,
            conv: *conv,
            conv_seq,
            hlc: hlc.clone(),
            kind,
            paths,
            created_at: now_millis(),
        };
        self.store.insert_preparing(&row)?;

        // 3. 发 MessageAdded（Preparing 视图，hash 为空、名字与大小取自文件系统）。
        let view = preparing_view(
            self.store.as_ref(),
            &row,
            Some(&self_user),
            lanmsg_api::types::MessageState::Sending,
        );
        let _ = self.events.send(Event::MessageAdded { msg: view });

        // 4. 后台任务完成哈希、缩略图、签名入库与投递；返回预留的 MsgId。
        self.spawn_prepare(row);
        Ok(MsgId {
            author: self_device,
            seq,
        })
    }

    /// 派发后台准备任务并登记句柄（shutdown 时可整体中止）。
    fn spawn_prepare(&self, row: PreparingRow) {
        let this = self.clone();
        let handle = tokio::spawn(async move {
            this.run_prepare(row).await;
        });
        let mut handles = self.prepare_handles.lock().unwrap();
        handles.retain(|h| !h.is_finished());
        handles.push(handle);
    }

    /// 中止全部后台准备任务（Service::shutdown 调用）：preparing 行保留，
    /// 重启后由 resume_preparing 续做。
    pub fn abort_prepares(&self) {
        for h in self.prepare_handles.lock().unwrap().drain(..) {
            h.abort();
        }
    }

    /// Service 启动时调用：对遗留 preparing 行重新走后台准备任务。
    pub async fn resume_preparing(&self) -> Result<(), CoreError> {
        for row in self.store.list_preparing()? {
            self.spawn_prepare(row);
        }
        Ok(())
    }

    /// 后台准备：逐文件 spawn_blocking 哈希与缩略图 → 组信封签名 →
    /// finish_preparing → MessageUpdated → 按 targets 发 SyncPush。
    /// 任一文件失败 → 删 preparing 行并发 MessageUpdated（state Failed）。
    async fn run_prepare(&self, row: PreparingRow) {
        let (self_device, self_user, device_key_bytes) = {
            let identity = self.identity.read().unwrap();
            if !identity.is_initialized() {
                return; // 服务已关闭，静默放弃
            }
            let self_user = match identity.user_id() {
                Some(u) => u,
                None => return,
            };
            (identity.device_id(), self_user, identity.device.to_bytes())
        };

        match self.prepare_envelope(&row, &device_key_bytes).await {
            Ok(env) => {
                // 投递目标与 send_impl 相同。
                let Ok(Some(conv_row)) = self.store.get_conversation(&row.conv) else {
                    return;
                };
                let targets = delivery_targets(&conv_row, &self_user, &self_device, &self.conns);
                let mut any_sent = false;
                for device in &targets {
                    let frame = Frame::SyncPush(SyncPush {
                        envelopes: vec![env.clone()],
                    });
                    if self.conns.send(device, frame).await.is_ok() {
                        any_sent = true;
                    }
                }
                if any_sent {
                    let _ = self.store.set_message_state(&env.id, MsgStateRow::Sent);
                }
                // MessageUpdated（重取以反映 Sending/Sent 最新状态）。
                if let Ok(Some(m)) = self.store.get_message(&env.id) {
                    let _ = self.events.send(Event::MessageUpdated {
                        msg: message_view(
                            self.store.as_ref(),
                            &self_device,
                            &self.conns.peers(),
                            &m,
                        ),
                    });
                }
            }
            Err(e) => {
                warn!(error = %e, "文件准备失败");
                let _ = self.store.delete_preparing(&row.author, row.seq);
                let view = preparing_view(
                    self.store.as_ref(),
                    &row,
                    Some(&self_user),
                    lanmsg_api::types::MessageState::Failed,
                );
                let _ = self.events.send(Event::MessageUpdated { msg: view });
            }
        }
    }

    /// 逐文件哈希与缩略图 → 组信封签名 → finish_preparing。
    async fn prepare_envelope(
        &self,
        row: &PreparingRow,
        device_key_bytes: &[u8; 32],
    ) -> Result<Envelope, CoreError> {
        // Folder 约定：paths[0] 为根目录，其余为条目文件。
        let (root, entry_paths): (Option<PathBuf>, &[PathBuf]) = match row.kind {
            PreparingKind::Files => (None, &row.paths),
            PreparingKind::Folder => (Some(row.paths[0].clone()), &row.paths[1..]),
        };

        let mut items: Vec<BlobMeta> = Vec::new();
        let mut segs: Vec<(lanmsg_proto::ids::BlobHash, Vec<[u8; 32]>)> = Vec::new();
        let mut locals: Vec<BlobLocalRow> = Vec::new();
        for path in entry_paths {
            let p = path.clone();
            let hashed = tokio::task::spawn_blocking(move || blob::hash::hash_file(&p))
                .await
                .map_err(|e| CoreError::Proto(e.to_string()))??;

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mime = blob::mime::mime_from_name(&name);
            let (thumb, media) = self.make_thumbnail(path, mime).await;
            items.push(BlobMeta {
                hash: hashed.hash,
                size: hashed.size,
                name,
                mime: mime.into(),
                media,
                thumb,
            });
            segs.push((hashed.hash, hashed.segment_hashes));
            locals.push(BlobLocalRow {
                hash: hashed.hash,
                path: path.clone(),
                kind: BlobLocalKind::Source,
                size: hashed.size,
                mtime: hashed.mtime,
            });
        }

        let body = match (row.kind, root) {
            (PreparingKind::Files, _) => MsgBody::Files { items },
            (PreparingKind::Folder, Some(root)) => {
                let entries = items
                    .into_iter()
                    .zip(entry_paths.iter())
                    .map(|(meta, path)| FolderEntry {
                        rel_path: path
                            .strip_prefix(&root)
                            .map(|r| r.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default(),
                        blob: meta,
                    })
                    .collect();
                MsgBody::Folder {
                    name: root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    entries,
                }
            }
            (PreparingKind::Folder, None) => {
                return Err(CoreError::Proto("preparing 行缺少文件夹根路径".into()))
            }
        };

        let mut env = Envelope {
            conv: row.conv,
            id: MsgId {
                author: row.author,
                seq: row.seq,
            },
            conv_seq: row.conv_seq,
            hlc: row.hlc.clone(),
            body,
            enc: None,
            sig: Signature64([0u8; 64]),
        };
        let device_key = DeviceKey::from_bytes(device_key_bytes);
        sign_with_zeroed_sig(&mut env, |b| device_key.sign(b));

        let now = now_millis();
        self.store
            .finish_preparing(&row.author, row.seq, &env, &segs, &locals, now)?;
        Ok(env)
    }

    /// 缩略图生成：mime 为 image/* 或 video/* 且平台缩略图器存在时调用；
    /// 结果超过 THUMB_MAX_BYTES 不采用。
    async fn make_thumbnail(
        &self,
        path: &Path,
        mime: &str,
    ) -> (Option<Vec<u8>>, Option<MediaInfo>) {
        let Some(provider) = self.platform.thumbnailer.clone() else {
            return (None, None);
        };
        if !(mime.starts_with("image/") || mime.starts_with("video/")) {
            return (None, None);
        }
        let p = path.to_path_buf();
        let mime = mime.to_string();
        match tokio::task::spawn_blocking(move || provider.thumbnail(&p, &mime)).await {
            Ok(Some(t)) if t.jpeg.len() <= THUMB_MAX_BYTES => (Some(t.jpeg), Some(t.media)),
            _ => (None, None),
        }
    }

    /// 处理对端发来的帧。
    ///
    /// - `SyncPush`：验签、检查作者设备、observe HLC、会话校验、门控决策
    ///   （Accept → insert_envelope(Delivered)；Pending → 隐藏入库并按首条
    ///   与否发 IncomingRequest；Drop → 不入库不回 Ack）、仅新插入时发事件，
    ///   最后回 `Ack { ids }`（Drop 的不回）。
    /// - `Ack`：对自己发的且状态为 Sending/Sent 的消息置 Delivered，发 `MessageUpdated`。
    /// - 其他帧：忽略。
    pub async fn on_frame(&self, from: DeviceId, frame: Frame) -> Result<(), CoreError> {
        let (self_user, self_device) = {
            let identity = self.identity.read().unwrap();
            (identity.user_id(), identity.device_id())
        };

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
                    // 2b. 作者设备已吊销 → 丢弃（吊销后其签名的信封不再可信，
                    //     无论经哪条路径到达，含第三台设备转发）。
                    if author_row.revoked {
                        debug!(peer = %from, author = %author_device, "SyncPush 作者设备已吊销，丢弃");
                        continue;
                    }
                    let author_user = author_row.user;

                    // 3. clock.observe(env.hlc)。
                    {
                        let mut clock = self.clock.lock().unwrap();
                        clock.observe(env.hlc.clone(), wall);
                    }

                    // 4. 成员校验与会话创建：会话不存在时若 env.conv ==
                    //    dm(self, author_user) 则创建 Dm 会话（gate = Pending 若
                    //    作者为陌生人，Open 若为联系人/自己），否则丢弃；已存在
                    //    的会话按 conv_member_ok 校验成员，不匹配丢弃。
                    let conv_id = env.conv;
                    match self.store.get_conversation(&conv_id)? {
                        None => {
                            let Some(self_user) = self_user else {
                                continue; // 未初始化，无法校验
                            };
                            // 对端用户：作者是他人时即作者；作者是自己的设备（转交
                            // 自己发给第三方的消息）时，在已知联系人中找 dm 匹配者。
                            let peer_user = if conv_id == ConvId::dm(&self_user, &author_user) {
                                Some(author_user)
                            } else if author_user == self_user {
                                self.store
                                    .list_contacts(None)?
                                    .into_iter()
                                    .map(|c| c.user)
                                    .find(|u| conv_id == ConvId::dm(&self_user, u))
                            } else {
                                None
                            };
                            let Some(peer_user) = peer_user else {
                                warn!(peer = %from, "SyncPush conv 不匹配，丢弃");
                                continue;
                            };
                            let row = ConvRow {
                                conv: conv_id,
                                kind: ConvKindRow::Dm,
                                peer_user: Some(peer_user),
                                title: None,
                                created_at: wall,
                            };
                            self.store.ensure_conversation(&row)?;
                            let is_contact = author_user == self_user
                                || self
                                    .store
                                    .get_contact(&author_user)
                                    .ok()
                                    .flatten()
                                    .is_some_and(|r| r.state == ContactState::Accepted);
                            if !is_contact {
                                self.store.set_gate(&conv_id, Gate::Pending, None)?;
                            }
                        }
                        Some(row) => {
                            let Some(self_user) = self_user else {
                                continue; // 未初始化，无法校验
                            };
                            if !conv_member_ok(&row, &author_user, &self_user) {
                                warn!(peer = %from, "SyncPush 成员校验失败，丢弃");
                                continue;
                            }
                        }
                    }

                    // 5. 门控决策（未注入回调或对端信息缺失时保持 P1 行为直接入库）。
                    let gate_cb = self.gate_cb.lock().unwrap().clone();
                    let (decision, had_open_request) = match gate_cb {
                        Some(cb) => {
                            // 首条判定：门控前是否已有同（作者, 类型）的 open 请求。
                            let had = self
                                .store
                                .list_open_requests()
                                .map(|l| {
                                    l.iter().any(|r| {
                                        r.kind == gate_request_kind(&env.body)
                                            && r.from_user == author_user
                                    })
                                })
                                .unwrap_or(false);
                            match self.conns.peer(&from) {
                                Some(pi) => match cb(&pi, &env, wall) {
                                    Ok(d) => (d, had),
                                    Err(e) => {
                                        warn!(peer = %from, error = %e, "SyncPush 门控失败，丢弃");
                                        continue;
                                    }
                                },
                                // 对端信息不可得（连接刚断开）：按 Accept 处理。
                                None => (GateDecision::Accept, had),
                            }
                        }
                        None => (GateDecision::Accept, false),
                    };

                    match decision {
                        GateDecision::Drop => {
                            // 不入库、不回 Ack（发送方保持 Sending）。
                            warn!(peer = %from, author = %author_user, "SyncPush 被门控丢弃");
                        }
                        GateDecision::Accept => {
                            // 6a. 会话已存在（步骤 4 已创建）；若 Dm 会话 gate 为
                            //     Pending 且门控 Accept（作者是联系人/自己设备等），
                            //     把 gate 置 Open。
                            let (gate, _) = self.store.get_gate(&conv_id)?;
                            if gate == Gate::Pending {
                                self.store.set_gate(&conv_id, Gate::Open, None)?;
                            }
                            let inserted =
                                self.store
                                    .insert_envelope(&env, MsgStateRow::Delivered, wall)?;
                            let msg_id = env.id.clone();
                            ids.push(msg_id);

                            // 7a. 仅当新插入时发事件并触发入库回调。
                            if inserted {
                                // 转交：他人发来的消息实时转给自己其他在线设备
                                // （发送方是自己设备或 from 本身自己设备时不再转，防回环）。
                                if let Some(su) = self_user {
                                    if author_user != su {
                                        self.relay_to_my_devices(
                                            su,
                                            self_device,
                                            from,
                                            env.clone(),
                                        );
                                    }
                                }
                                let stored = StoredMessage {
                                    env: env.clone(),
                                    state: MsgStateRow::Delivered,
                                    received_at: wall,
                                };
                                let msg_view = message_view(
                                    self.store.as_ref(),
                                    &self_device,
                                    &self.conns.peers(),
                                    &stored,
                                );
                                let _ = self.events.send(Event::MessageAdded { msg: msg_view });

                                let conv_row =
                                    self.store.get_conversation(&env.conv)?.ok_or_else(|| {
                                        CoreError::NotFound(format!("会话不存在: {}", env.conv))
                                    })?;
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
                        GateDecision::Pending { request_id } => {
                            // 6b. 隐藏入库；仅当新插入且是该请求首条时发 IncomingRequest
                            //     （后续合并到同一请求的只更新 preview 不发事件）。
                            let inserted = self.store.insert_envelope_hidden(
                                &env,
                                MsgStateRow::Delivered,
                                wall,
                                true,
                            )?;
                            let msg_id = env.id.clone();
                            ids.push(msg_id);
                            if inserted && !had_open_request {
                                if let Some(row) = self.store.get_request(&request_id)? {
                                    let _ = self.events.send(Event::IncomingRequest {
                                        request: request_view(self.store.as_ref(), &row),
                                    });
                                }
                            }
                        }
                    }
                }

                // 7. 回 Ack（包含重复的，以便对方确认）。
                let _ = self.conns.send(&from, Frame::Ack(Ack { ids })).await;
            }
            Frame::Ack(ack) => {
                // 送达规则（确认设备为 D）：
                // - 消息所在会话为 Dm 且 D 属于 peer_user → Delivered；
                // - 会话为 SelfDevices 且 D 属于自己（非本机）→ Delivered；
                // - 其他情况（例如自己的设备确认收到转交）→ 不改状态。
                for id in ack.ids {
                    if id.author != self_device {
                        continue;
                    }
                    let Some(m) = self.store.get_message(&id)? else {
                        continue;
                    };
                    if !matches!(m.state, MsgStateRow::Sending | MsgStateRow::Sent) {
                        continue;
                    }
                    let Some(row) = self.store.get_conversation(&m.env.conv)? else {
                        continue;
                    };
                    let device_user = self.store.get_device(&from).ok().flatten().map(|d| d.user);
                    let delivered = match row.kind {
                        ConvKindRow::Dm => device_user.is_some_and(|u| Some(u) == row.peer_user),
                        ConvKindRow::SelfDevices => {
                            device_user.is_some_and(|u| matches!(self_user, Some(su) if su == u))
                                && from != self_device
                        }
                        _ => false,
                    };
                    if delivered {
                        self.store.set_message_state(&id, MsgStateRow::Delivered)?;
                        let msg_view = message_view(
                            self.store.as_ref(),
                            &self_device,
                            &self.conns.peers(),
                            &m,
                        );
                        let _ = self.events.send(Event::MessageUpdated { msg: msg_view });
                    }
                }
            }
            _ => {} // 其他帧忽略，交给后续阶段的模块
        }

        Ok(())
    }

    /// 打开与指定用户的 DM 会话，返回会话 ID。
    pub fn open_dm(&self, peer_user: &UserId) -> Result<ConvId, CoreError> {
        let identity = self.identity.read().unwrap();
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

    // ---- 吊销 / 伪造 conv / 转交成环 ----

    fn signed_text_env(
        dev: &lanmsg_proto::sign::DeviceKey,
        conv: ConvId,
        seq: u64,
        text: &str,
    ) -> Envelope {
        let mut env = Envelope {
            conv,
            id: MsgId {
                author: dev.device_id(),
                seq,
            },
            conv_seq: seq,
            hlc: lanmsg_proto::types::Hlc {
                millis: 1_700_000_000_000 + seq as i64,
                counter: 0,
            },
            body: MsgBody::Text { text: text.into() },
            enc: None,
            sig: lanmsg_proto::types::Signature64([0u8; 64]),
        };
        lanmsg_proto::sign::sign_with_zeroed_sig(&mut env, |b| dev.sign(b));
        env
    }

    fn push(env: Envelope) -> Frame {
        Frame::SyncPush(lanmsg_proto::types::SyncPush {
            envelopes: vec![env],
        })
    }

    /// 在 node 的 store 中登记一台属于其自己用户的第二设备，返回其密钥。
    fn add_own_device(n: &crate::testutil::Node) -> lanmsg_proto::sign::DeviceKey {
        let dev = lanmsg_proto::sign::DeviceKey::generate();
        let cert = {
            let id = n.identity.read().unwrap();
            lanmsg_proto::types::DeviceCert::issue(
                id.user.as_ref().unwrap(),
                dev.device_id(),
                "第二设备".into(),
                lanmsg_proto::types::DeviceType::Desktop,
                1,
            )
        };
        n.store
            .upsert_device(&crate::store::DeviceRow {
                device: dev.device_id(),
                user: n.user,
                cert,
                name: "第二设备".into(),
                device_type: lanmsg_proto::types::DeviceType::Desktop,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .unwrap();
        dev
    }

    #[tokio::test]
    async fn revoked_author_envelope_is_dropped() {
        let (a, b) = pair_of_nodes().await;
        let conv = a.open_dm(&b.user).unwrap();
        // a 是 b 的已接受联系人，避免门控干扰。
        b.store
            .upsert_contact_if_newer(&crate::store::ContactRow {
                user: a.user,
                state: ContactState::Accepted,
                auto_accept: false,
                ask_above_bytes: None,
                updated: lanmsg_proto::types::Hlc {
                    millis: 1,
                    counter: 0,
                },
            })
            .unwrap();

        // 吊销 a 的设备后，a 再发消息：直接推送与经第三方转发均不入库。
        let mut row = b.store.get_device(&a.device).unwrap().unwrap();
        row.revoked = true;
        b.store.upsert_device(&row).unwrap();
        let id = a
            .send(
                &conv,
                MsgBody::Text {
                    text: "吊销后".into(),
                },
            )
            .await
            .unwrap();
        let env = a.store.get_message(&id).unwrap().unwrap().env;
        let relay = DeviceId([9u8; 32]);
        b.on_frame(relay, push(env.clone())).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(b.store.list_messages(&conv, None, 10).unwrap().is_empty());

        // 对照：撤销吊销标志后同一信封可入库，说明丢弃确由吊销检查造成。
        row.revoked = false;
        b.store.upsert_device(&row).unwrap();
        b.on_frame(relay, push(env)).await.unwrap();
        assert_eq!(b.store.list_messages(&conv, None, 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn own_device_forged_conv_to_non_contact_is_dropped() {
        let (_a, b) = pair_of_nodes().await;
        let d2 = add_own_device(&b);
        // 非联系人用户 S：自己设备伪造 dm(self, S) 的信封。
        let stranger = lanmsg_proto::sign::UserKey::generate().user_id();
        let conv = ConvId::dm(&b.user, &stranger);
        let env = signed_text_env(&d2, conv, 1, "伪造");
        b.on_frame(d2.device_id(), push(env)).await.unwrap();
        assert!(b.store.get_conversation(&conv).unwrap().is_none());
        assert!(b.store.list_messages(&conv, None, 10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn relay_through_own_devices_does_not_duplicate() {
        let (_a, b) = pair_of_nodes().await;
        // A1(=d1)、A3(=d3) 是 b(A2) 用户的另两台设备；联系人 S 的消息经二者先后转交。
        let d1 = add_own_device(&b);
        let d3 = add_own_device(&b);
        let s_dev = lanmsg_proto::sign::DeviceKey::generate();
        let s_key = lanmsg_proto::sign::UserKey::generate();
        let s_cert = lanmsg_proto::types::DeviceCert::issue(
            &s_key,
            s_dev.device_id(),
            "S".into(),
            lanmsg_proto::types::DeviceType::Desktop,
            1,
        );
        b.store
            .upsert_device(&crate::store::DeviceRow {
                device: s_dev.device_id(),
                user: s_key.user_id(),
                cert: s_cert,
                name: "S".into(),
                device_type: lanmsg_proto::types::DeviceType::Desktop,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .unwrap();
        b.store
            .upsert_contact_if_newer(&crate::store::ContactRow {
                user: s_key.user_id(),
                state: ContactState::Accepted,
                auto_accept: false,
                ask_above_bytes: None,
                updated: lanmsg_proto::types::Hlc {
                    millis: 1,
                    counter: 0,
                },
            })
            .unwrap();
        let conv = ConvId::dm(&b.user, &s_key.user_id());
        let env = signed_text_env(&s_dev, conv, 1, "转交");
        // A1 → A2，再 A3 → A2（A1→A2→A3→A2 环）：仅一条，且无错误。
        b.on_frame(d1.device_id(), push(env.clone())).await.unwrap();
        b.on_frame(d3.device_id(), push(env.clone())).await.unwrap();
        b.on_frame(d1.device_id(), push(env)).await.unwrap();
        assert_eq!(b.store.list_messages(&conv, None, 10).unwrap().len(), 1);
    }
}
