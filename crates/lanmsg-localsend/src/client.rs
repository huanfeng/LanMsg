//! LocalSend 发送客户端。

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tokio_stream::wrappers::ReceiverStream;

use crate::server::LocalSend;
use crate::types::{LsFileMeta, LsPeer, LsProtocol, PrepareUploadRequest, PrepareUploadResponse};
use crate::LsError;

/// prepare-upload 要等对方点“接收”，超时放宽。
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);
/// 同时上传的文件数。
const UPLOAD_CONCURRENCY: usize = 2;
const CHUNK: usize = 64 * 1024;

fn base_url(peer: &LsPeer) -> String {
    let scheme = match peer.info.protocol {
        LsProtocol::Http => "http",
        LsProtocol::Https => "https",
    };
    format!(
        "{scheme}://{}/api/localsend/v2",
        std::net::SocketAddr::new(peer.ip, peer.info.port)
    )
}

fn http_err(e: reqwest::Error) -> LsError {
    LsError::Http(e.to_string())
}

impl LocalSend {
    /// 发送文字：一个 `text/plain` “文件”，正文放在 `preview`；对方返回 204 即完成。
    pub async fn send_text(&self, peer: &LsPeer, text: &str) -> Result<(), LsError> {
        let meta = LsFileMeta {
            id: uuid::Uuid::new_v4().to_string(),
            file_name: "message.txt".into(),
            size: text.len() as u64,
            file_type: "text/plain".into(),
            sha256: None,
            preview: Some(text.to_string()),
            metadata: None,
        };
        let id = meta.id.clone();
        let resp = self.prepare(peer, vec![meta]).await?;
        // 对方不接受文字消息的快捷路径时（200），把文字当文件内容上传
        let Some(resp) = resp else {
            return Ok(());
        };
        let token = resp
            .files
            .get(&id)
            .cloned()
            .ok_or_else(|| LsError::Json("missing file token".into()))?;
        let r = self
            .inner
            .client
            .post(format!("{}/upload", base_url(peer)))
            .query(&[
                ("sessionId", resp.session_id.as_str()),
                ("fileId", id.as_str()),
                ("token", token.as_str()),
            ])
            .body(text.as_bytes().to_vec())
            .send()
            .await
            .map_err(http_err)?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(LsError::Status(r.status().as_u16()))
        }
    }

    /// 依次（并发 2）发送文件；`progress` 收到 (文件名, 已发送, 总大小)。
    pub async fn send_files(
        &self,
        peer: &LsPeer,
        paths: &[PathBuf],
        progress: mpsc::Sender<(String, u64, u64)>,
    ) -> Result<(), LsError> {
        let mut metas = Vec::new();
        let mut by_id: BTreeMap<String, PathBuf> = BTreeMap::new();
        for path in paths {
            let p = path.clone();
            let (size, sha) = tokio::task::spawn_blocking(move || hash_file(&p))
                .await
                .map_err(|e| LsError::Http(e.to_string()))??;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let id = uuid::Uuid::new_v4().to_string();
            metas.push(LsFileMeta {
                id: id.clone(),
                file_type: mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
                file_name: name,
                size,
                sha256: Some(sha),
                preview: None,
                metadata: None,
            });
            by_id.insert(id, path.clone());
        }
        let Some(resp) = self.prepare(peer, metas.clone()).await? else {
            return Ok(());
        };

        let sem = Arc::new(Semaphore::new(UPLOAD_CONCURRENCY));
        let mut set = JoinSet::new();
        for meta in metas {
            let Some(token) = resp.files.get(&meta.id).cloned() else {
                continue;
            };
            let Some(path) = by_id.get(&meta.id).cloned() else {
                continue;
            };
            let (client, url, sid) = (
                self.inner.client.clone(),
                format!("{}/upload", base_url(peer)),
                resp.session_id.clone(),
            );
            let (sem, progress) = (sem.clone(), progress.clone());
            set.spawn(async move {
                let _permit = sem
                    .acquire()
                    .await
                    .map_err(|e| LsError::Http(e.to_string()))?;
                upload_one(&client, &url, &sid, &meta, &token, path, progress).await
            });
        }
        let mut first_err: Option<LsError> = None;
        while let Some(r) = set.join_next().await {
            let r = r.unwrap_or_else(|e| Err(LsError::Http(e.to_string())));
            if let Err(e) = r {
                if first_err.is_none() {
                    first_err = Some(e);
                    set.abort_all();
                }
            }
        }
        if let Some(e) = first_err {
            // 尽力通知对方清理
            let _ = self
                .inner
                .client
                .post(format!("{}/cancel", base_url(peer)))
                .query(&[("sessionId", resp.session_id.as_str())])
                .timeout(Duration::from_secs(5))
                .send()
                .await;
            return Err(e);
        }
        Ok(())
    }

    /// prepare-upload：204 返回 `None`，200 返回会话，其他状态码转错误。
    async fn prepare(
        &self,
        peer: &LsPeer,
        files: Vec<LsFileMeta>,
    ) -> Result<Option<PrepareUploadResponse>, LsError> {
        let req = PrepareUploadRequest {
            info: self.inner.info.clone(),
            files: files.into_iter().map(|f| (f.id.clone(), f)).collect(),
        };
        let r = self
            .inner
            .client
            .post(format!("{}/prepare-upload", base_url(peer)))
            .timeout(PREPARE_TIMEOUT)
            .json(&req)
            .send()
            .await
            .map_err(http_err)?;
        match r.status().as_u16() {
            204 => Ok(None),
            200 => r
                .json::<PrepareUploadResponse>()
                .await
                .map(Some)
                .map_err(|e| LsError::Json(e.to_string())),
            code => Err(LsError::Status(code)),
        }
    }
}

fn hash_file(path: &std::path::Path) -> Result<(u64, String), LsError> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    Ok((
        size,
        h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
    ))
}

async fn upload_one(
    client: &reqwest::Client,
    url: &str,
    session_id: &str,
    meta: &LsFileMeta,
    token: &str,
    path: PathBuf,
    progress: mpsc::Sender<(String, u64, u64)>,
) -> Result<(), LsError> {
    // 阻塞线程读文件并送入流（不使用 tokio::fs::File）
    let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>(4);
    let (name, total) = (meta.file_name.clone(), meta.size);
    tokio::task::spawn_blocking(move || {
        let mut f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                let _ = tx.blocking_send(Err(e));
                return;
            }
        };
        let mut done = 0u64;
        loop {
            let mut buf = vec![0u8; CHUNK];
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    done += n as u64;
                    if tx.blocking_send(Ok(buf)).is_err() {
                        break;
                    }
                    let _ = progress.try_send((name.clone(), done, total));
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    break;
                }
            }
        }
    });
    let r = client
        .post(url)
        .query(&[
            ("sessionId", session_id),
            ("fileId", meta.id.as_str()),
            ("token", token),
        ])
        .header("content-length", meta.size)
        .body(reqwest::Body::wrap_stream(ReceiverStream::new(rx)))
        .send()
        .await
        .map_err(http_err)?;
    if r.status().is_success() {
        Ok(())
    } else {
        Err(LsError::Status(r.status().as_u16()))
    }
}
