//! 缩略图短命进程：`lanmsg-thumb <输入路径> <mime> <输出.jpg>`。
//! 成功：写出 JPEG，stdout 打印一行 JSON 媒体信息，退出码 0；失败：stderr 打印原因，退出码 1。

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("用法: lanmsg-thumb <输入路径> <mime> <输出.jpg>");
        return ExitCode::from(1);
    }
    match run(&args[1], &args[2], &args[3]) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
    }
}

fn run(input: &str, mime: &str, output: &str) -> Result<String, String> {
    let (jpeg, info) =
        lanmsg_media::make_thumbnail(Path::new(input), mime).map_err(|e| e.to_string())?;
    std::fs::write(output, jpeg).map_err(|e| format!("failed to write output: {e}"))?;
    Ok(serde_json::json!({
        "width": info.width,
        "height": info.height,
        "duration_ms": info.duration_ms,
    })
    .to_string())
}
