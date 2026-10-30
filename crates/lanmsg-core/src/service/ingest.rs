// P2 Task 4：消息入库后分派。
// after_ingest 在 Messaging::on_frame 每条新插入消息后异步执行（经 Service 注入的回调）。
// 本任务处理 System{ProfileUpdated}；后续任务在此函数追加分支。
// 注释中文，标识符英文。

use lanmsg_proto::consts::RECALL_WINDOW;
use lanmsg_proto::ids::{DeviceId, MsgId, UserId};
use lanmsg_proto::types::{ContactRecord, MsgBody, SystemEvent};

use crate::identity::{hex_decode, hex_encode};
use crate::service::{build_announce, now_millis, Inner};
use crate::store::{StoredMessage, UserRow};
use lanmsg_api::types::Event;
use lanmsg_proto::types::Hlc;

/// 消息入库后的分派入口：按消息体类型触发本地状态更新。
pub(crate) async fn after_ingest(svc: &Inner, _from: DeviceId, m: &StoredMessage) {
    match &m.env.body {
        MsgBody::System {
            event: SystemEvent::ProfileUpdated { display_name },
        } => on_profile_updated(svc, m, display_name).await,
        MsgBody::System {
            event: SystemEvent::ContactUpdated(rec),
        } => on_contact_updated(svc, m, rec).await,
        MsgBody::System {
            event: SystemEvent::LocalDeleted { msgs },
        } => on_local_deleted(svc, m, msgs).await,
        MsgBody::Read { upto } => on_read(svc, m, upto).await,
        MsgBody::Recall { target } => on_recall(svc, m, target).await,
        _ => {}
    }
}

/// 查设备所属用户（devices 表）。
fn device_user(svc: &Inner, device: &DeviceId) -> Option<UserId> {
    svc.store.get_device(device).ok().flatten().map(|r| r.user)
}

/// Read{upto}：作者用户 == 自己 → 同步已读水位（变化则发 ConversationUpdated）；
/// 否则为对端已读回执 → 自己发出的、hlc <= upto 的消息置 Read，逐条发
/// MessageUpdated。
async fn on_read(svc: &Inner, m: &StoredMessage, upto: &Hlc) {
    let Some(self_user) = svc.state.read().unwrap().self_user else {
        return;
    };
    if device_user(svc, &m.env.id.author).as_ref() == Some(&self_user) {
        if svc
            .store
            .set_last_read(&m.env.conv, upto.clone())
            .unwrap_or(false)
        {
            crate::service::notify_conversation(svc, &m.env.conv);
        }
        return;
    }
    let ids = match svc
        .store
        .mark_read_upto(&m.env.conv, &self_user, upto.clone())
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "已读回执应用失败");
            return;
        }
    };
    if ids.is_empty() {
        return;
    }
    let self_device = svc.identity.read().await.device_id();
    for id in ids {
        if let Ok(Some(sm)) = svc.store.get_message(&id) {
            let _ = svc.events.send(Event::MessageUpdated {
                msg: crate::service::views::message_view(svc.store.as_ref(), &self_device, &sm),
            });
        }
    }
}

/// Recall{target}：存储层已在入库时处理召回状态。目标存在、目标作者用户与
/// 撤回作者用户一致、且时限内 → 发 MessageUpdated（state Recalled）；
/// 否则撤回无效，undo_recall 删除 recalls 行并恢复目标状态。
async fn on_recall(svc: &Inner, m: &StoredMessage, target: &MsgId) {
    let recall_user = device_user(svc, &m.env.id.author);
    let target_msg = svc.store.get_message(target).ok().flatten();
    let window_millis = RECALL_WINDOW.as_millis() as i64;
    let valid = matches!(&target_msg, Some(t)
        if device_user(svc, &t.env.id.author) == recall_user
            && m.env.hlc.millis.saturating_sub(t.env.hlc.millis) <= window_millis);
    match (valid, target_msg) {
        (true, Some(t)) => {
            let self_device = svc.identity.read().await.device_id();
            let _ = svc.events.send(Event::MessageUpdated {
                msg: crate::service::views::message_view(svc.store.as_ref(), &self_device, &t),
            });
        }
        _ => {
            if let Err(e) = svc.store.undo_recall(target) {
                tracing::warn!(error = %e, "undo_recall 失败");
            }
        }
    }
}

/// System{LocalDeleted}：作者用户 == 自己（我的其他设备的删除广播）→
/// 本地标记删除并发 ConversationUpdated。
async fn on_local_deleted(svc: &Inner, m: &StoredMessage, msgs: &[MsgId]) {
    let Some(self_user) = svc.state.read().unwrap().self_user else {
        return;
    };
    if device_user(svc, &m.env.id.author).as_ref() != Some(&self_user) {
        return;
    }
    if let Err(e) = svc.store.mark_deleted(msgs) {
        tracing::warn!(error = %e, "本地删除应用失败");
        return;
    }
    crate::service::notify_conversation(svc, &m.env.conv);
}

/// ContactUpdated：仅当作者用户 == 自己时按 HLC 严格更新应用；变化则发
/// ContactsChanged。
async fn on_contact_updated(svc: &Inner, m: &StoredMessage, rec: &ContactRecord) {
    let Some(self_user) = svc.state.read().unwrap().self_user else {
        return;
    };
    let author_user = svc
        .store
        .get_device(&m.env.id.author)
        .ok()
        .flatten()
        .map(|r| r.user);
    if author_user.as_ref() != Some(&self_user) {
        return;
    }
    match svc.trust.apply_contact_record(rec) {
        Ok(true) => {
            let _ = svc.events.send(lanmsg_api::types::Event::ContactsChanged);
        }
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "ContactUpdated 应用失败"),
    }
}

/// ProfileUpdated：仅当作者用户 == 自己且 hlc 更新时，更新自己的显示名。
async fn on_profile_updated(svc: &Inner, m: &StoredMessage, display_name: &str) {
    // 作者用户 == 自己。
    let author_user = svc
        .store
        .get_device(&m.env.id.author)
        .ok()
        .flatten()
        .map(|r| r.user);
    let Some(self_user) = svc.state.read().unwrap().self_user else {
        return;
    };
    if author_user.as_ref() != Some(&self_user) {
        return;
    }

    // hlc 新于 setting "profile_hlc" 才应用（缺失视为最小值）。
    let applied = read_profile_hlc(svc);
    if m.env.hlc <= applied {
        return;
    }

    let now = now_millis();
    let display = display_name.to_string();

    // users 表自己行（is_self 保持）。
    let is_self = svc
        .store
        .get_user(&self_user)
        .ok()
        .flatten()
        .map(|r| r.is_self)
        .unwrap_or(true);
    let _ = svc.store.upsert_user(&UserRow {
        user: self_user,
        display_name: display.clone(),
        is_self,
        updated_at: now,
    });
    let _ = svc.store.set_setting("display_name", &display);

    // HelloInfo 换新显示名（证书与端口保持）。
    let mut hello = svc.hello_tx.borrow().clone();
    hello.display_name = display.clone();
    svc.hello_tx.send_replace(hello);

    // announce 换新显示名（状态取当前值）。
    let status = svc.state.read().unwrap().status.clone();
    if let Some(ann) = build_announce(&svc.identity, &display, status, svc.local_addr.port()).await
    {
        svc.announce_tx.send_replace(Some(ann));
    }

    // 记录已应用 hlc（CBOR hex）。
    let hlc_bytes = lanmsg_proto::to_cbor(&m.env.hlc);
    let _ = svc.store.set_setting(
        crate::settings::SETTING_PROFILE_HLC,
        &hex_encode(&hlc_bytes),
    );
}

/// 读取已应用的最大 ProfileUpdated hlc；缺失返回最小值。
fn read_profile_hlc(svc: &Inner) -> Hlc {
    let hex = match svc.store.get_setting(crate::settings::SETTING_PROFILE_HLC) {
        Ok(Some(h)) => h,
        _ => {
            return Hlc {
                millis: 0,
                counter: 0,
            }
        }
    };
    match hex_decode(&hex)
        .ok()
        .and_then(|b| lanmsg_proto::from_cbor::<Hlc>(&b).ok())
    {
        Some(h) => h,
        None => Hlc {
            millis: 0,
            counter: 0,
        },
    }
}
