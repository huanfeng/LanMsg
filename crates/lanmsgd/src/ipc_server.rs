// P1 核心骨架：IPC 连接服务（Task 12）。
// 每个 IPC 连接：请求 → service.handle → 响应；同时推送事件。
// 注释中文，标识符英文。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_util::codec::Framed;

use lanmsg_api::ipc::{IpcCodec, IpcStream};
use lanmsg_api::types::{ApiResponse, IpcFrame, Request};
use lanmsg_core::Service;

/// 连接编号发号器（从 1 起；0 表示“没有连接持有焦点”）。
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// 服务单个 IPC 连接：转发事件 + 处理请求。
/// 收到 `Request::Shutdown` 时执行 `service.shutdown()`，再经 `shutdown_tx` 通知主线程退出
/// （不直接 `exit`：让 Drop 清理 IPC socket、托盘正常退出）；退出前在 `data_dir` 写主动退出标记。
///
/// `focus_owner` 记录“最后一次设置 ui_focus 的连接编号”：界面进程崩溃/被杀后连接断开，
/// 若焦点仍由本连接设置，则清除，避免残留焦点让该会话的新消息永远不弹通知。
pub(crate) async fn serve_connection(
    service: Service,
    stream: IpcStream,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    data_dir: PathBuf,
    focus_owner: Arc<AtomicU64>,
) {
    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    // 本连接最近一次 SetUiFocus 是否设置了 Some（设置 None 视为已自行清除）。
    let mut holds_focus = false;
    let mut events = service.subscribe();
    let (sink, mut source) = Framed::new(stream, IpcCodec).split();
    // sink 由事件任务与请求循环共享（发送互斥，保证帧完整）。
    let sink = Arc::new(tokio::sync::Mutex::new(sink));

    // 事件转发任务：连接断开时 send 失败退出。
    let ev_task = {
        let sink = sink.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(e) => {
                        if sink.lock().await.send(IpcFrame::Event(e)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        })
    };

    // 启动检查发现防火墙规则缺失：新连接补发一次（先复查，规则可能已被放行）。
    if crate::firewall_cmd::firewall_missing()
        && tokio::task::spawn_blocking(crate::firewall_cmd::check_missing)
            .await
            .unwrap_or(false)
    {
        let _ = sink
            .lock()
            .await
            .send(IpcFrame::Event(lanmsg_api::types::Event::FirewallMissing))
            .await;
    }

    // 请求处理循环。
    while let Some(frame) = source.next().await {
        let req = match frame {
            Ok(IpcFrame::Request(req)) => req,
            Ok(_) => continue,
            Err(_) => break,
        };
        let is_shutdown = matches!(req.body, Request::Shutdown);
        let focus_req = match &req.body {
            Request::SetUiFocus { conv } => Some(conv.is_some()),
            _ => None,
        };
        let result = service.handle(req.body).await;
        // 仅成功的 SetUiFocus 才算数（非法输入不改变焦点，见 handlers）。
        if let (Some(is_some), Ok(_)) = (focus_req, &result) {
            focus_owner.store(conn_id, Ordering::SeqCst);
            holds_focus = is_some;
        }
        if sink
            .lock()
            .await
            .send(IpcFrame::Response(ApiResponse { id: req.id, result }))
            .await
            .is_err()
        {
            break;
        }
        if is_shutdown {
            ev_task.abort();
            service.shutdown().await;
            crate::runtime::write_quit_marker(&data_dir);
            tracing::info!("Shutdown requested, notifying main thread to exit");
            shutdown_tx.send_replace(true);
            return;
        }
    }
    ev_task.abort();
    // 连接断开：焦点仍是本连接设置的才清除；之后别的连接设置过则不动。
    if holds_focus
        && focus_owner
            .compare_exchange(conn_id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        let _ = service.handle(Request::SetUiFocus { conv: None }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_api::ipc::{IpcClient, IpcListener};
    use lanmsg_core::discovery::DiscoveryMode;
    use lanmsg_core::keystore::FileKeyStore;
    use lanmsg_core::{PlatformHooks, ServiceConfig};
    use lanmsg_proto::ids::ConvId;

    /// 起一个离线 Service 与 IPC 监听，返回 (service, endpoint, focus_owner, 临时目录)。
    async fn setup() -> (Service, String, Arc<AtomicU64>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::start(ServiceConfig {
            data_dir: dir.path().to_path_buf(),
            listen_port: 0,
            discovery: DiscoveryMode::Disabled,
            keystore: Arc::new(FileKeyStore::new(dir.path().join("keys"))),
            static_peers: Vec::new(),
            platform: PlatformHooks::default(),
            localsend_port: 0,
        })
        .await
        .unwrap();
        let endpoint = dir.path().join("t.sock").to_str().unwrap().to_string();
        let listener = IpcListener::bind(&endpoint).unwrap();
        let focus_owner = Arc::new(AtomicU64::new(0));
        let (tx, _) = tokio::sync::watch::channel(false);
        let (svc, owner, data_dir) = (
            service.clone(),
            focus_owner.clone(),
            dir.path().to_path_buf(),
        );
        tokio::spawn(async move {
            while let Ok(stream) = listener.accept().await {
                tokio::spawn(serve_connection(
                    svc.clone(),
                    stream,
                    tx.clone(),
                    data_dir.clone(),
                    owner.clone(),
                ));
            }
        });
        (service, endpoint, focus_owner, dir)
    }

    /// 在独立线程/运行时里起一个 IPC 客户端并设置焦点；向返回的 Sender 发信号（或丢弃它）
    /// 即关闭该运行时，从而真正断开连接（IpcClient 的读任务不随 Drop 退出）。
    fn spawn_client(ep: &str, conv: ConvId) -> std::sync::mpsc::Sender<()> {
        let (close_tx, close_rx) = std::sync::mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let ep = ep.to_string();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let c = IpcClient::connect(&ep).await.unwrap();
                c.call(Request::SetUiFocus {
                    conv: Some(conv.to_hex()),
                })
                .await
                .unwrap();
                ready_tx.send(()).unwrap();
                // 阻塞到收到关闭信号；随后 rt 与 c 一并 Drop，连接断开。
                let _ = tokio::task::spawn_blocking(move || close_rx.recv()).await;
            });
        });
        ready_rx.recv().unwrap();
        close_tx
    }

    /// 等待条件成立（连接断开由服务端异步感知）。
    async fn wait_until(f: impl Fn() -> bool) -> bool {
        for _ in 0..50 {
            if f() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        f()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disconnect_clears_focus_set_by_that_connection() {
        let (svc, ep, _o, _dir) = setup().await;
        let conv = ConvId([1u8; 32]);
        let a = spawn_client(&ep, conv);
        assert_eq!(svc.ui_focus(), Some(conv));
        drop(a);
        assert!(
            wait_until(|| svc.ui_focus().is_none()).await,
            "断开后焦点应清除"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disconnect_keeps_focus_set_later_by_another_connection() {
        let (svc, ep, _o, _dir) = setup().await;
        let a = spawn_client(&ep, ConvId([1u8; 32]));
        let conv_b = ConvId([2u8; 32]);
        let b = spawn_client(&ep, conv_b);
        drop(a);
        // 给服务端处理 a 断开的时间，b 设置的焦点必须保留。
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(svc.ui_focus(), Some(conv_b));
        drop(b);
        assert!(wait_until(|| svc.ui_focus().is_none()).await);
    }
}
