//! 外部 ffmpeg/ffprobe 调用（运行期可选）；每个子进程 10 秒超时。

use crate::{decode_image, MediaError, Rgba};
use lanmsg_proto::types::MediaInfo;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

/// 先查当前可执行文件同目录，再查 `PATH`。
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let cand = dir.join(&file);
        if cand.is_file() {
            return Some(cand);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(&file))
        .find(|c| c.is_file())
}

/// 运行子进程并收集 stdout；超时则杀掉并返回 `Timeout`。
fn run_with_timeout(mut cmd: Command) -> Result<Vec<u8>, MediaError> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| MediaError::Decode("无法获取子进程输出".into()))?;
    // 单独线程读 stdout，避免管道写满导致子进程阻塞
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() >= TOOL_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(MediaError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = reader
        .join()
        .map_err(|_| MediaError::Decode("读取子进程输出的线程异常".into()))?;
    if !status.success() {
        return Err(MediaError::Decode(format!("外部工具退出码 {status}")));
    }
    Ok(out)
}

/// 取视频/HEIC 首帧（最长边 256）与媒体信息。
pub fn frame_and_probe(path: &Path) -> Result<(Rgba, MediaInfo), MediaError> {
    let ffprobe = find_tool("ffprobe").ok_or(MediaError::ToolMissing)?;
    let ffmpeg = find_tool("ffmpeg").ok_or(MediaError::ToolMissing)?;

    let mut probe = Command::new(ffprobe);
    probe
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=width,height:format=duration"])
        .args(["-of", "json"])
        .arg(path);
    let json: serde_json::Value = serde_json::from_slice(&run_with_timeout(probe)?)
        .map_err(|e| MediaError::Decode(e.to_string()))?;
    let stream = &json["streams"][0];
    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Err(MediaError::Decode("ffprobe 未返回视频尺寸".into()));
    }
    let duration_ms = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| (d * 1000.0) as u64);

    let mut grab = Command::new(ffmpeg);
    grab.args(["-v", "error", "-ss", "0", "-i"])
        .arg(path)
        .args(["-frames:v", "1"])
        .args(["-vf", "scale=256:256:force_original_aspect_ratio=decrease"])
        .args(["-f", "image2pipe", "-vcodec", "png", "-"]);
    let frame = decode_image(&run_with_timeout(grab)?)?;
    Ok((
        frame,
        MediaInfo {
            width,
            height,
            duration_ms,
        },
    ))
}
