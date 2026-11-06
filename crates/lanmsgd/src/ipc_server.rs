// P1 核心骨架：IPC 连接服务（Task 12）。
// 每个 IPC 连接：请求 → service.handle → 响应；同时推送事件。
// 注释中文，标识符英文。

use std::sync::Arc;

use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use tokio_util::codec::Framed;

use lanmsg_api::ipc::{IpcCodec, IpcStream};
use lanmsg_api::types::{ApiResponse, IpcFrame, Request};
use lanmsg_core::Service;

/// 服务单个 IPC 连接：转发事件 + 处理请求。
/// 收到 `Request::Shutdown` 时执行 `service.shutdown()`，再经 `shutdown_tx` 通知主线程退出
/// （不直接 `exit`：让 Drop 清理 IPC socket、托盘正常退出）。
pub(crate) async fn serve_connection(
    service: Service,
    stream: IpcStream,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
) {
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

    // 请求处理循环。
    while let Some(frame) = source.next().await {
        let req = match frame {
            Ok(IpcFrame::Request(req)) => req,
            Ok(_) => continue,
            Err(_) => break,
        };
        let is_shutdown = matches!(req.body, Request::Shutdown);
        let result = service.handle(req.body).await;
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
            tracing::info!("收到 Shutdown 请求，通知主线程退出");
            shutdown_tx.send_replace(true);
            return;
        }
    }
    ev_task.abort();
}
