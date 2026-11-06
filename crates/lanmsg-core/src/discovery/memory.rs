// P1 核心骨架：发现模式——内存总线（C6）。
// 共享同一 MemoryBus 的 Discovery 实例互相可见，用于测试与同机多实例。

use std::net::SocketAddr;
use tokio::sync::{broadcast, mpsc, watch};

use lanmsg_proto::types::Announce;

use super::OutPkt;

/// 广播通道容量。
const BUS_CAPACITY: usize = 64;

/// 内存总线：内部为 `broadcast::Sender<(Vec<u8>, SocketAddr)>（C6）。
#[derive(Clone)]
pub struct MemoryBus {
    tx: broadcast::Sender<(Vec<u8>, SocketAddr)>,
}

impl Default for MemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBus {
    /// 创建总线。容量 64；滞后超过容量的接收端按协议丢弃并记日志。
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BUS_CAPACITY);
        MemoryBus { tx }
    }

    /// 订阅总线，每个发现实例一个接收端。
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<(Vec<u8>, SocketAddr)> {
        self.tx.subscribe()
    }

    /// 向总线广播一包；无接收端时发送失败，属正常情况，忽略。
    pub(crate) fn broadcast(&self, bytes: Vec<u8>, from: SocketAddr) {
        let _ = self.tx.send((bytes, from));
    }
}

impl MemoryBus {
    /// 测试专用：向总线注入任意一包（如伪造指向“沉默对端”端口的通告）。
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn inject_for_tests(&self, bytes: Vec<u8>, from: SocketAddr) {
        self.broadcast(bytes, from);
    }
}

/// 源任务：把总线上的包转发进发现主循环的收件箱。
pub(crate) fn spawn_source(bus: &MemoryBus, inbox: mpsc::Sender<(Vec<u8>, SocketAddr)>) {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok((bytes, from)) => {
                    if inbox.send((bytes, from)).await.is_err() {
                        break; // 主循环已退出
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("内存总线接收滞后，丢失 {n} 包");
                }
            }
        }
    });
}

/// 汇任务：把主循环要发的包投到总线上，from 伪造为 127.0.0.1:<监听端口>。
pub(crate) async fn run_sink(
    bus: MemoryBus,
    announce_rx: watch::Receiver<Option<Announce>>,
    mut outbox: mpsc::Receiver<OutPkt>,
) {
    while let Some(pkt) = outbox.recv().await {
        // 监听端口取通告的 ports[0]；无端口时用 0。
        let port = announce_rx
            .borrow()
            .as_ref()
            .and_then(|a| a.ports.first().copied())
            .unwrap_or(0);
        bus.broadcast(pkt.bytes, SocketAddr::from(([127, 0, 0, 1], port)));
    }
}
