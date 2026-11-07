// 缩略图子进程封装（P3 Task 8）。
// 解码与 ffmpeg 都在独立的 `lanmsg-thumb` 进程里完成：损坏/恶意文件最多拖垮子进程，
// 且有超时强杀，不影响守护进程常驻内存与稳定性。
// 注释中文，标识符英文。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use lanmsg_core::platform::{Thumbnail, ThumbnailProvider};
use lanmsg_proto::types::MediaInfo;

/// 子进程总时限（含 lanmsg-thumb 内部 ffprobe + ffmpeg 的 12 秒）。
const THUMB_TIMEOUT: Duration = Duration::from_secs(15);
/// 同时运行的最大子进程数。
const MAX_CONCURRENT: usize = 2;
/// 子进程 stdout/stderr 各自最多读取的字节数（超出部分丢弃，但仍会读空管道）。
const MAX_CAPTURE: u64 = 64 * 1024;
/// 子进程退出后等待读线程交出输出的时限（孙进程持有管道时读线程可能不结束）。
const DRAIN_WAIT: Duration = Duration::from_secs(1);

/// 调用 `lanmsg-thumb` 子进程生成缩略图的 `ThumbnailProvider`。
pub struct SubprocessThumbnailer {
    exe: PathBuf,
    cache_dir: PathBuf,
    timeout: Duration,
    /// 并发许可：已占用的子进程数 + 条件变量。
    slots: (Mutex<usize>, Condvar),
}

/// 临时输出文件名的进程内序号。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

impl SubprocessThumbnailer {
    /// 生产用：`lanmsg-thumb` 位于当前可执行文件同目录，超时 15 秒。
    pub fn new(cache_dir: PathBuf) -> Self {
        let name = if cfg!(windows) {
            "lanmsg-thumb.exe"
        } else {
            "lanmsg-thumb"
        };
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(name)))
            .unwrap_or_else(|| PathBuf::from(name));
        Self::with_timeout(exe, cache_dir, THUMB_TIMEOUT)
    }

    /// 指定子进程路径与超时（测试用）。
    pub fn with_timeout(exe: PathBuf, cache_dir: PathBuf, timeout: Duration) -> Self {
        Self {
            exe,
            cache_dir,
            timeout,
            slots: (Mutex::new(0), Condvar::new()),
        }
    }

    /// 取得一个并发许可（阻塞）；返回的守卫析构时归还。
    fn acquire(&self) -> SlotGuard<'_> {
        let (lock, cv) = &self.slots;
        let mut used = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *used >= MAX_CONCURRENT {
            used = cv.wait(used).unwrap_or_else(|e| e.into_inner());
        }
        *used += 1;
        SlotGuard(&self.slots)
    }

    /// 运行子进程并返回 (stdout, stderr)；成功退出才返回 Some。
    fn run_child(&self, input: &Path, mime: &str, out: &Path) -> Option<Vec<u8>> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg(input)
            .arg(mime)
            .arg(out)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_child(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("failed to start {}: {e}", self.exe.display());
                return None;
            }
        };
        // 读线程边跑边读，避免子进程输出写满管道而卡死；线程不 join，
        // 只通过通道取结果（孙进程持管道不放时不会拖住本线程）。
        let out_rx = capture(child.stdout.take());
        let err_rx = capture(child.stderr.take());

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) if Instant::now() >= deadline => break None,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => {
                    tracing::debug!("failed to wait for thumbnail subprocess: {e}");
                    kill_tree(&mut child);
                    return None;
                }
            }
        };
        let Some(status) = status else {
            tracing::debug!(
                "thumbnail subprocess timed out, killed: {}",
                input.display()
            );
            kill_tree(&mut child);
            return None;
        };
        let stdout = out_rx.recv_timeout(DRAIN_WAIT).unwrap_or_default();
        if !status.success() {
            let stderr = err_rx.recv_timeout(DRAIN_WAIT).unwrap_or_default();
            tracing::debug!(
                "thumbnail subprocess failed ({status}): {}",
                String::from_utf8_lossy(&stderr).trim()
            );
            return None;
        }
        Some(stdout)
    }
}

impl ThumbnailProvider for SubprocessThumbnailer {
    fn thumbnail(&self, path: &Path, mime: &str) -> Option<Thumbnail> {
        let thumbs_dir = self.cache_dir.join("thumbs");
        if let Err(e) = std::fs::create_dir_all(&thumbs_dir) {
            tracing::debug!("failed to create thumbnail directory: {e}");
            return None;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = thumbs_dir.join(format!("tmp-{}-{nanos:x}-{seq}.jpg", std::process::id()));

        let _slot = self.acquire();
        let result = self.run_child(path, mime, &tmp).and_then(|stdout| {
            let media = parse_media_info(&stdout)?;
            let jpeg = std::fs::read(&tmp).ok()?;
            (!jpeg.is_empty()).then_some(Thumbnail { jpeg, media })
        });
        let _ = std::fs::remove_file(&tmp);
        result
    }
}

/// 并发许可守卫：析构时归还并唤醒等待者。
struct SlotGuard<'a>(&'a (Mutex<usize>, Condvar));

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let (lock, cv) = self.0;
        let mut used = lock.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(1);
        cv.notify_one();
    }
}

/// 在后台线程读取管道（最多 MAX_CAPTURE 字节，其余读空丢弃），读完经通道返回。
fn capture<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    if let Some(pipe) = pipe {
        std::thread::spawn(move || {
            let mut pipe = pipe;
            let mut buf = Vec::new();
            let _ = (&mut pipe).take(MAX_CAPTURE).read_to_end(&mut buf);
            // 超出上限的部分继续读空，避免子进程写管道阻塞。
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            let _ = tx.send(buf);
        });
    }
    rx
}

/// 解析 stdout 中的 `{"width","height","duration_ms"}` JSON（取最后一个非空行）。
fn parse_media_info(stdout: &[u8]) -> Option<MediaInfo> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    match serde_json::from_str::<MediaInfo>(line.trim()) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::debug!("failed to parse thumbnail subprocess output: {e}");
            None
        }
    }
}

/// Unix：让子进程成为新进程组组长，超时后可整组强杀（含 ffmpeg 孙进程）。
#[cfg(unix)]
fn configure_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

/// Windows：不弹控制台窗口。
#[cfg(windows)]
fn configure_child(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// 强杀子进程及其后代并回收。
/// Unix：进程组 id 等于子进程 pid，调用系统 `kill -KILL -<pgid>` 杀整组
/// （不引入清单外的 libc/nix 依赖）。
/// Windows：只杀子进程（未引入 Job Object 所需依赖）；孙进程可能残留，
/// 但 lanmsg-thumb 自身对 ffmpeg 有 10 秒超时，残留时间有限。
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            tracing::debug!("failed to kill process group, falling back to killing the child only");
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// 写脚本到可执行位后立刻 exec 可能遇到 ETXTBSY（别的线程 fork 时继承了写句柄），
    /// 所以写脚本与运行都在这把锁内串行。
    static SCRIPT_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(unix)]
    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(windows)]
    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(format!("{name}.bat"));
        std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn thumbnailer_kills_hung_child() {
        let _g = SCRIPT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        // 脚本启动一个孙进程 sleep 并等待，模拟 lanmsg-thumb 启动的 ffmpeg 卡死。
        let exe = write_script(
            dir.path(),
            "hung.sh",
            &format!("sleep 60 &\necho $! > '{}'\nwait", pid_file.display()),
        );
        let t = SubprocessThumbnailer::with_timeout(
            exe,
            dir.path().to_path_buf(),
            Duration::from_secs(2),
        );
        let start = Instant::now();
        assert!(t
            .thumbnail(std::path::Path::new("/nonexistent"), "video/mp4")
            .is_none());
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "超时后必须及时返回"
        );

        // 孙进程也应随进程组被杀（Linux 下查 /proc；僵尸态视为已死）。
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(&pid_file)
                .unwrap()
                .trim()
                .to_string();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let alive = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                    Ok(stat) => !stat.rsplit(") ").next().unwrap_or("").starts_with('Z'),
                    Err(_) => false,
                };
                if !alive {
                    break;
                }
                assert!(Instant::now() < deadline, "孙进程 {pid} 未被杀死");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn thumbnailer_kills_hung_child() {
        let _g = SCRIPT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let exe = write_script(dir.path(), "hung", "timeout /t 60 /nobreak >nul");
        let t = SubprocessThumbnailer::with_timeout(
            exe,
            dir.path().to_path_buf(),
            Duration::from_secs(2),
        );
        let start = Instant::now();
        assert!(t
            .thumbnail(std::path::Path::new("C:\\nonexistent"), "video/mp4")
            .is_none());
        assert!(start.elapsed() < Duration::from_secs(20));
    }

    #[cfg(unix)]
    #[test]
    fn thumbnailer_parses_child_output() {
        use lanmsg_proto::types::MediaInfo;
        let _g = SCRIPT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // 约定：$1 输入、$2 mime、$3 输出。
        let exe = write_script(
            dir.path(),
            "ok.sh",
            "printf 'JPEGBYTES' > \"$3\"\necho '{\"width\":10,\"height\":20,\"duration_ms\":null}'",
        );
        let t = SubprocessThumbnailer::with_timeout(
            exe,
            dir.path().to_path_buf(),
            Duration::from_secs(10),
        );
        let got = t
            .thumbnail(std::path::Path::new("/x.png"), "image/png")
            .expect("应解析出缩略图");
        assert_eq!(got.jpeg, b"JPEGBYTES");
        assert_eq!(
            got.media,
            MediaInfo {
                width: 10,
                height: 20,
                duration_ms: None
            }
        );
        // 临时输出文件已清理。
        let leftover: Vec<_> = std::fs::read_dir(dir.path().join("thumbs"))
            .unwrap()
            .collect();
        assert!(leftover.is_empty(), "临时文件应已删除");
    }

    #[cfg(unix)]
    #[test]
    fn thumbnailer_returns_none_on_child_failure() {
        let _g = SCRIPT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let exe = write_script(dir.path(), "fail.sh", "echo 解码失败 >&2\nexit 1");
        let t = SubprocessThumbnailer::with_timeout(
            exe,
            dir.path().to_path_buf(),
            Duration::from_secs(10),
        );
        assert!(t
            .thumbnail(std::path::Path::new("/x.png"), "image/png")
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn thumbnailer_does_not_deadlock_on_large_stdout() {
        let _g = SCRIPT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // 子进程输出远超管道缓冲（64KB）的数据后才退出：不读管道就会卡死到超时。
        let exe = write_script(
            dir.path(),
            "big.sh",
            "head -c 300000 /dev/zero | tr '\\0' 'x'\necho",
        );
        let t = SubprocessThumbnailer::with_timeout(
            exe,
            dir.path().to_path_buf(),
            Duration::from_secs(10),
        );
        let start = Instant::now();
        assert!(t
            .thumbnail(std::path::Path::new("/x.png"), "image/png")
            .is_none());
        assert!(start.elapsed() < Duration::from_secs(5), "不应等到超时");
    }
}
