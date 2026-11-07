// P2 Task 11：数据服务端与持有者帧处理。
// BlobServer 注册为 ConnManager 的数据处理器：serve 循环在 DATA_IDLE_TIMEOUT
// 内无请求即关闭；GetMeta/GetRange 都先做可见性检查（不区分"无权"与"没有"，
// 一律 NotFound），再做 verify_source 确认本地文件仍与记录一致。
// 注释中文，标识符英文。

use std::sync::{Arc, RwLock};

use lanmsg_proto::consts::DATA_IDLE_TIMEOUT;
use lanmsg_proto::frame::Frame;
use lanmsg_proto::ids::{BlobHash, DeviceId};
use lanmsg_proto::types::{BlobHave, BlobQuery};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::error::CoreError;
use crate::identity::Identity;
use crate::net::conn::{DataHandler, DataStream};
use crate::net::PeerInfo;
use crate::service::Inner;
use crate::store::{BlobLocalRow, Store};
use lanmsg_proto::data::{read_data_frame, write_data_frame, DataFrame};

use super::hash::{file_mtime, hash_file};

/// 传输流读写缓冲（计划建议 1 MiB）。
const STREAM_BUF: usize = 1024 * 1024;

/// 数据服务端：对外提供本地持有的 Blob。
pub struct BlobServer {
    store: Arc<Store>,
    identity: Arc<RwLock<Identity>>,
}

impl BlobServer {
    /// 构造 BlobServer。
    pub fn new(store: Arc<Store>, identity: Arc<RwLock<Identity>>) -> Arc<Self> {
        Arc::new(Self { store, identity })
    }

    /// 生成数据连接处理器：每条入站数据连接 spawn 一个 serve 循环。
    pub fn handler(self: Arc<Self>) -> DataHandler {
        Arc::new(move |peer: PeerInfo, stream: DataStream| {
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(e) = server.serve(peer, stream).await {
                    tracing::warn!(error = %e, "数据连接服务异常退出");
                }
            });
        })
    }

    /// 数据服务主循环：空闲超时、对端断开或协议错误即关闭连接。
    pub async fn serve(&self, peer: PeerInfo, mut stream: DataStream) -> Result<(), CoreError> {
        loop {
            let frame =
                match tokio::time::timeout(DATA_IDLE_TIMEOUT, read_data_frame(&mut stream)).await {
                    Ok(Ok(f)) => f,
                    // 超时 / EOF / 协议错误：一律关闭。
                    _ => return Ok(()),
                };

            let (hash, is_range) = match frame {
                DataFrame::GetMeta { hash } => (hash, None),
                DataFrame::GetRange { hash, offset, len } => (hash, Some((offset, len))),
                _ => continue,
            };

            // 可见性：对端证书在数据通道入站时已校验，必为 Some。
            let Some(cert) = peer.cert.clone() else {
                return Ok(());
            };
            let Some(self_user) = self.identity.read().unwrap().user_id() else {
                return Ok(());
            };
            if !self.store.blob_visible_to(&hash, &cert.user, &self_user)? {
                Self::write_frame(&mut stream, &DataFrame::NotFound { hash }).await?;
                continue;
            }

            // 本地源文件校验。
            let Some(local) = self.verify_source(&hash).await? else {
                Self::write_frame(&mut stream, &DataFrame::NotFound { hash }).await?;
                continue;
            };

            match is_range {
                None => {
                    let segments = self.store.blob_segments(&hash)?.unwrap_or_default();
                    Self::write_frame(
                        &mut stream,
                        &DataFrame::Meta {
                            hash,
                            size: local.size,
                            segment_hashes: segments,
                        },
                    )
                    .await?;
                }
                Some((offset, len)) => {
                    // 越界 → 416。
                    if offset >= local.size || len > local.size - offset {
                        Self::write_frame(
                            &mut stream,
                            &DataFrame::Error {
                                code: 416,
                                message: "requested range out of bounds".into(),
                            },
                        )
                        .await?;
                        continue;
                    }
                    Self::write_frame(&mut stream, &DataFrame::RangeStart { offset, len }).await?;
                    self.stream_range(&mut stream, &local, offset, len).await?;
                }
            }
        }
    }

    /// 本地源校验：无记录、文件消失或哈希改变都返回 None（并清理本地行）；
    /// (size, mtime) 变化但哈希相同则更新记录。
    /// 注：计划签名为同步 fn，因重哈希必须 spawn_blocking，改为 async。
    pub async fn verify_source(&self, hash: &BlobHash) -> Result<Option<BlobLocalRow>, CoreError> {
        let Some(row) = self.store.get_local(hash)? else {
            return Ok(None);
        };
        let meta = match std::fs::metadata(&row.path) {
            Ok(m) => m,
            Err(_) => {
                self.store.remove_local(hash)?;
                return Ok(None);
            }
        };
        let size = meta.len();
        let mtime = file_mtime(&meta);
        if size == row.size && mtime == row.mtime {
            return Ok(Some(row));
        }
        // (size, mtime) 不符：重新哈希比对。
        let path = row.path.clone();
        let hashed = match tokio::task::spawn_blocking(move || hash_file(&path)).await {
            Ok(Ok(h)) => h,
            _ => {
                self.store.remove_local(hash)?;
                return Ok(None);
            }
        };
        if hashed.hash.0 == hash.0 {
            let fresh = BlobLocalRow {
                hash: *hash,
                path: row.path.clone(),
                kind: row.kind,
                size: hashed.size,
                mtime: hashed.mtime,
            };
            self.store.set_local(&fresh)?;
            Ok(Some(fresh))
        } else {
            self.store.remove_local(hash)?;
            Ok(None)
        }
    }

    /// RangeStart 之后按 1 MiB 缓冲 seek + 读写 len 字节。
    async fn stream_range(
        &self,
        stream: &mut DataStream,
        local: &BlobLocalRow,
        offset: u64,
        len: u64,
    ) -> Result<(), CoreError> {
        let mut file = tokio::fs::File::open(&local.path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut remaining = len;
        let mut buf = vec![0u8; STREAM_BUF];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                // 文件比记录短（被截断）：中断传输，连接交给上层超时处理。
                break;
            }
            stream.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
        stream.flush().await?;
        Ok(())
    }

    async fn write_frame(stream: &mut DataStream, f: &DataFrame) -> Result<(), CoreError> {
        write_data_frame(stream, f)
            .await
            .map_err(|e| CoreError::Proto(e.to_string()))
    }

    /// Frame::BlobQuery：回 BlobHave（本地持有且 verify_source 通过、且对方可见）。
    pub(crate) async fn handle_blob_query(&self, inner: &Inner, from: DeviceId, q: BlobQuery) {
        let Some(from_user) = inner.store.get_device(&from).ok().flatten().map(|r| r.user) else {
            return;
        };
        let Some(self_user) = inner.state.read().unwrap().self_user else {
            return;
        };
        let mut hashes = Vec::new();
        for h in q.hashes {
            let visible = self
                .store
                .blob_visible_to(&h, &from_user, &self_user)
                .unwrap_or(false);
            if !visible {
                continue;
            }
            if self
                .verify_source(&h)
                .await
                .map(|o| o.is_some())
                .unwrap_or(false)
            {
                hashes.push(h);
            }
        }
        let _ = inner
            .conns
            .send(&from, Frame::BlobHave(BlobHave { hashes }))
            .await;
    }

    /// Frame::BlobHave：把发送方记为各哈希的持有者。
    pub(crate) async fn handle_blob_have(&self, from: DeviceId, h: BlobHave) {
        let now = crate::service::now_millis();
        for hash in h.hashes {
            let _ = self.store.add_holder(&hash, &from, now);
        }
    }
}
