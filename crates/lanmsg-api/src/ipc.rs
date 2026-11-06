// P1 核心骨架：IPC 帧与编解码（C7）。
// 线格式：u32 BE 长度 | CBOR(IpcFrame)，长度上限 MAX_FRAME_LEN。
// endpoint_name / IpcListener / IpcClient 提供端点命名、服务端监听与客户端调用。
// 注释中文，标识符英文。

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::{Buf, BytesMut};
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use interprocess::local_socket::tokio::{prelude::*, Stream};
use interprocess::local_socket::{GenericFilePath, ListenerOptions};
use tokio::sync::{broadcast, oneshot};
use tokio_util::codec::{Decoder, Encoder, Framed};

use crate::types::{
    ApiError, ApiRequest, ApiResponse, ErrorCode, Event, IpcFrame, Request, Response,
};
use lanmsg_proto::consts::MAX_FRAME_LEN;

/// IPC 编解码错误。
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// 帧长度超过上限。
    #[error("帧长度 {0} 超过上限 {MAX_FRAME_LEN}")]
    TooLarge(usize),
    /// CBOR 编解码失败。
    #[error("CBOR 错误: {0}")]
    Cbor(String),
    /// IO 错误。
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// IPC 编解码器：u32 BE 长度 | CBOR(IpcFrame)。
pub struct IpcCodec;

impl Encoder<IpcFrame> for IpcCodec {
    type Error = IpcError;

    fn encode(&mut self, item: IpcFrame, dst: &mut BytesMut) -> Result<(), IpcError> {
        let mut buf = Vec::new();
        ciborium::into_writer(&item, &mut buf).map_err(|e| IpcError::Cbor(e.to_string()))?;
        if buf.len() > MAX_FRAME_LEN {
            return Err(IpcError::TooLarge(buf.len()));
        }
        dst.extend_from_slice(&(buf.len() as u32).to_be_bytes());
        dst.extend_from_slice(&buf);
        Ok(())
    }
}

impl Decoder for IpcCodec {
    type Item = IpcFrame;
    type Error = IpcError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<IpcFrame>, IpcError> {
        if src.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if len > MAX_FRAME_LEN {
            return Err(IpcError::TooLarge(len));
        }
        if src.len() < 4 + len {
            return Ok(None);
        }
        src.advance(4);
        let frame_bytes = src.split_to(len);
        let frame: IpcFrame =
            ciborium::from_reader(&frame_bytes[..]).map_err(|e| IpcError::Cbor(e.to_string()))?;
        Ok(Some(frame))
    }
}

/// 计算本机 IPC 端点名（C7）。
///
/// - Windows: 命名管道 `\\.\pipe\lanmsgd-<用户名>-<blake3(data_dir) 前 8 位 hex>`
/// - macOS / 无 `XDG_RUNTIME_DIR` 的 Unix: `<data_dir>/lanmsgd.sock`
/// - 其余 Unix: `<XDG_RUNTIME_DIR>/lanmsg/lanmsgd.sock`
pub fn endpoint_name(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        let canonical = data_dir
            .canonicalize()
            .unwrap_or_else(|_| data_dir.to_path_buf());
        let digest = blake3::hash(canonical.to_string_lossy().as_bytes());
        let hex = digest.to_hex();
        let short = &hex.to_string()[..8];
        let user = std::env::var("USERNAME").unwrap_or_default();
        format!(r"\\.\pipe\lanmsgd-{user}-{short}")
    }
    #[cfg(unix)]
    {
        if cfg!(target_os = "macos") {
            return data_dir.join("lanmsgd.sock").to_string_lossy().into_owned();
        }
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            if !xdg.is_empty() {
                return Path::new(&xdg)
                    .join("lanmsg")
                    .join("lanmsgd.sock")
                    .to_string_lossy()
                    .into_owned();
            }
        }
        data_dir.join("lanmsgd.sock").to_string_lossy().into_owned()
    }
}

/// IPC 服务端监听器。
pub struct IpcListener {
    inner: interprocess::local_socket::tokio::Listener,
}

/// IPC 连接流（interprocess 本地套接字流的别名）。
pub type IpcStream = Stream;

impl IpcListener {
    /// 绑定端点。Unix 下 socket 文件已存在时：先尝试连接，连得上说明已有实例
    /// 在运行 → 返回 `ErrorKind::AddrInUse`；连不上则删除残留文件后绑定。
    pub fn bind(endpoint: &str) -> io::Result<IpcListener> {
        #[cfg(unix)]
        {
            // 同步 connect 需要 Stream trait；仅 Unix 的残留 socket 探测使用。
            use interprocess::local_socket::traits::Stream as _;
            if Path::new(endpoint).exists() {
                let name = endpoint.to_fs_name::<GenericFilePath>()?;
                match interprocess::local_socket::Stream::connect(name) {
                    // 已有实例存活。
                    Ok(_) => {
                        return Err(io::Error::new(io::ErrorKind::AddrInUse, "端点已被占用"));
                    }
                    // 残留文件：删除后重新绑定。
                    Err(_) => {
                        let _ = std::fs::remove_file(endpoint);
                    }
                }
            }
            if let Some(parent) = Path::new(endpoint).parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let name = endpoint.to_fs_name::<GenericFilePath>()?;
        let inner = ListenerOptions::new().name(name).create_tokio()?;
        Ok(IpcListener { inner })
    }

    /// 接受一个连接。
    pub async fn accept(&self) -> io::Result<IpcStream> {
        self.inner.accept().await
    }
}

/// IPC 客户端（C7）：`call` 用自增 id 关联响应；连接断开时所有挂起调用
/// 返回 `ApiError { code: Io, .. }`，事件经 `events()` 订阅。
#[derive(Clone)]
pub struct IpcClient {
    send: Arc<tokio::sync::Mutex<SplitSink<Framed<Stream, IpcCodec>, IpcFrame>>>,
    next_id: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<ApiResponse>>>>,
    events: Arc<broadcast::Sender<Event>>,
}

/// 连接断开时的 Io 错误响应。
fn io_error_response(message: &str) -> ApiResponse {
    ApiResponse {
        id: 0,
        result: Err(ApiError {
            code: ErrorCode::Io,
            message: message.to_string(),
        }),
    }
}

impl IpcClient {
    /// 连接到指定端点，启动读循环任务。
    pub async fn connect(endpoint: &str) -> io::Result<IpcClient> {
        let name = endpoint.to_fs_name::<GenericFilePath>()?;
        let stream = Stream::connect(name).await?;
        let (events_tx, _events_rx) = broadcast::channel(256);
        let (sink, mut source) = Framed::new(stream, IpcCodec).split();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<ApiResponse>>>> = Arc::default();

        let pending_loop = pending.clone();
        let events_loop = events_tx.clone();
        tokio::spawn(async move {
            loop {
                match source.next().await {
                    Some(Ok(IpcFrame::Response(resp))) => {
                        let slot = pending_loop.lock().unwrap().remove(&resp.id);
                        if let Some(tx) = slot {
                            let _ = tx.send(resp);
                        }
                    }
                    Some(Ok(IpcFrame::Event(e))) => {
                        let _ = events_loop.send(e);
                    }
                    Some(Ok(IpcFrame::Request(_))) => {}
                    Some(Err(_)) | None => break,
                }
            }
            // 连接断开：所有挂起调用返回 Io 错误；events 发送端 drop 使订阅者收到 Closed。
            for (_id, tx) in pending_loop.lock().unwrap().drain() {
                let _ = tx.send(io_error_response("IPC 连接已断开"));
            }
        });

        Ok(IpcClient {
            send: Arc::new(tokio::sync::Mutex::new(sink)),
            next_id: Arc::new(AtomicU64::new(1)),
            pending,
            events: Arc::new(events_tx),
        })
    }

    /// 发送请求并等待响应。
    pub async fn call(&self, body: Request) -> Result<Response, ApiError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let send_result = {
            let mut sink = self.send.lock().await;
            sink.send(IpcFrame::Request(ApiRequest { id, body })).await
        };
        if let Err(e) = send_result {
            self.pending.lock().unwrap().remove(&id);
            return Err(ApiError {
                code: ErrorCode::Io,
                message: e.to_string(),
            });
        }
        match rx.await {
            Ok(resp) => resp.result,
            Err(_) => Err(ApiError {
                code: ErrorCode::Io,
                message: "IPC 连接已断开".into(),
            }),
        }
    }

    /// 订阅服务端推送的事件。
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApiRequest, Event, IpcFrame, Request, Response};
    use lanmsg_proto::consts::MAX_FRAME_LEN;
    use tokio_util::codec::Decoder;

    #[test]
    fn ipc_frame_roundtrip_request_and_event() {
        let frames = vec![
            IpcFrame::Request(ApiRequest {
                id: 1,
                body: Request::SendText {
                    conv: "ab".into(),
                    text: "你好".into(),
                },
            }),
            IpcFrame::Event(Event::PeerOffline {
                device: "cd".into(),
            }),
        ];

        let mut codec = IpcCodec;
        for frame in &frames {
            let mut dst = BytesMut::new();
            codec.encode(frame.clone(), &mut dst).expect("编码应成功");
            let mut src = dst;
            let decoded = codec.decode(&mut src).expect("解码应成功");
            assert_eq!(decoded, Some(frame.clone()));
        }
    }

    #[test]
    fn response_uses_named_value_field() {
        let r = Response::MsgId {
            value: "x:1".into(),
        };
        assert_eq!(
            r,
            Response::MsgId {
                value: "x:1".into()
            }
        );
    }

    #[test]
    fn ipc_rejects_oversized_frame() {
        let mut src = BytesMut::new();
        // 长度头为 MAX_FRAME_LEN + 1，无有效负载
        src.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());
        let mut codec = IpcCodec;
        let result = codec.decode(&mut src);
        assert!(matches!(result, Err(IpcError::TooLarge(_))));
    }
}
