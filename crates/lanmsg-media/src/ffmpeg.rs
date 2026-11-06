//! 外部 ffmpeg/ffprobe 调用（运行期可选）；两步共用 12 秒总期限。

use crate::{decode_image, MediaError, Rgba};
use lanmsg_proto::types::MediaInfo;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// ffprobe + ffmpeg 共用的总期限，须小于父进程的 15 秒强杀。
const TOOL_DEADLINE: Duration = Duration::from_secs(12);
/// ffmpeg 输出上限（缩略帧 PNG 远小于此值）。
const MAX_TOOL_OUTPUT: u64 = 4 * 1024 * 1024;

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

/// 运行子进程并收集 stdout；超过 `deadline` 则杀掉并返回 `Timeout`，输出超上限返回错误。
fn run_with_timeout(mut cmd: Command, deadline: Instant) -> Result<Vec<u8>, MediaError> {
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
        // 多读 1 字节用于判断是否超限；读满后丢弃管道，子进程会因写失败退出
        let _ = (&mut stdout)
            .take(MAX_TOOL_OUTPUT + 1)
            .read_to_end(&mut buf);
        buf
    });
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() >= deadline {
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
    if out.len() as u64 > MAX_TOOL_OUTPUT {
        return Err(MediaError::TooLarge);
    }
    if !status.success() {
        return Err(MediaError::Decode(format!("外部工具退出码 {status}")));
    }
    Ok(out)
}

/// 输入路径转为绝对路径并加 `file:` 前缀，避免 `-` 开头的文件名被当选项、相对路径被当协议。
fn input_arg(path: &Path) -> Result<OsString, MediaError> {
    let abs = std::path::absolute(path)?;
    let mut arg = OsString::from("file:");
    arg.push(abs.as_os_str());
    Ok(arg)
}

fn probe_args(input: &OsStr) -> Vec<OsString> {
    let mut v: Vec<OsString> = [
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height:format=duration",
        "-of",
        "json",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    v.push(input.to_os_string());
    v
}

fn grab_args(input: &OsStr) -> Vec<OsString> {
    let mut v: Vec<OsString> = ["-v", "error", "-ss", "0", "-i"]
        .iter()
        .map(OsString::from)
        .collect();
    v.push(input.to_os_string());
    v.extend(
        [
            "-frames:v",
            "1",
            "-vf",
            "scale=256:256:force_original_aspect_ratio=decrease",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-",
        ]
        .iter()
        .map(OsString::from),
    );
    v
}

/// 取视频/HEIC 首帧（最长边 256）与媒体信息。
pub fn frame_and_probe(path: &Path) -> Result<(Rgba, MediaInfo), MediaError> {
    let ffprobe = find_tool("ffprobe").ok_or(MediaError::ToolMissing)?;
    let ffmpeg = find_tool("ffmpeg").ok_or(MediaError::ToolMissing)?;

    let deadline = Instant::now() + TOOL_DEADLINE;
    let input = input_arg(path)?;

    let mut probe = Command::new(ffprobe);
    probe.args(probe_args(&input));
    let json: serde_json::Value = serde_json::from_slice(&run_with_timeout(probe, deadline)?)
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

    // 第二步使用剩余时间；已无剩余直接超时
    if Instant::now() >= deadline {
        return Err(MediaError::Timeout);
    }
    let mut grab = Command::new(ffmpeg);
    grab.args(grab_args(&input));
    let frame = decode_image(&run_with_timeout(grab, deadline)?)?;
    Ok((
        frame,
        MediaInfo {
            width,
            height,
            duration_ms,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_prefixed_filename_never_looks_like_option() {
        let arg = input_arg(Path::new("-x.mp4")).unwrap();
        let text = arg.to_string_lossy().into_owned();
        assert!(text.starts_with("file:"), "{text}");
        assert!(Path::new(&text["file:".len()..]).is_absolute(), "{text}");
        for args in [probe_args(&arg), grab_args(&arg)] {
            let pos = args.iter().position(|a| a == &arg).unwrap();
            // 输入位置的元素不以 - 开头；且它之外只有固定选项以 - 开头
            assert!(!args[pos].to_string_lossy().starts_with('-'));
            assert!(
                args.iter()
                    .filter(|a| a.to_string_lossy().contains("x.mp4"))
                    .count()
                    == 1
            );
        }
    }
}
