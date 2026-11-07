// 演示夹具截图：每个演示页面都能离屏渲染出非空白的 PNG。
// 注释中文，标识符英文。

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

/// 截图输出目录：环境变量 LANMSG_SHOT_DIR 指定时保留产物，否则用临时目录。
fn out_dir(tmp: &tempfile::TempDir) -> PathBuf {
    match std::env::var_os("LANMSG_SHOT_DIR") {
        Some(d) => {
            let p = PathBuf::from(d);
            std::fs::create_dir_all(&p).expect("创建截图目录");
            p
        }
        None => tmp.path().to_path_buf(),
    }
}

#[test]
fn every_demo_page_renders() {
    let tmp = tempfile::tempdir().expect("创建临时目录");
    let dir = out_dir(&tmp);
    for page in [
        "wizard",
        "chats",
        "chat",
        "contacts",
        "devices",
        "pair",
        "pair-sas",
        "pair-join",
        "files",
        "files-grid",
        "settings",
    ] {
        let png = dir.join(format!("{page}.png"));
        let status = Command::new(env!("CARGO_BIN_EXE_lanmsg"))
            .args(["--demo", page, "--screenshot"])
            .arg(&png)
            .status()
            .expect("启动 lanmsg");
        assert!(status.success(), "{page}: 退出码 {status:?}");
        let img = image::open(&png)
            .unwrap_or_else(|e| panic!("{page}: PNG 无法解码: {e}"))
            .to_rgba8();
        assert!(
            img.width() >= 1000 && img.height() >= 680,
            "{page}: 尺寸 {}x{}",
            img.width(),
            img.height()
        );
        let colors: HashSet<[u8; 4]> = img.pixels().map(|p| p.0).collect();
        assert!(
            colors.len() > 16,
            "{page}: 颜色数 {} 过少，疑似空白",
            colors.len()
        );
    }
}
