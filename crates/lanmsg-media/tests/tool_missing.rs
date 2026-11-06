//! 修改 PATH 的测试，独占一个测试二进制，避免干扰其它测试。

#[cfg(not(windows))]
#[test]
fn video_without_ffmpeg_is_tool_missing() {
    use lanmsg_media::{make_thumbnail, MediaError};
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty-path");
    std::fs::create_dir(&empty).unwrap();
    let mut mp4 = vec![0, 0, 0, 0x18];
    mp4.extend_from_slice(b"ftypisom\0\0\0\0");
    mp4.extend_from_slice(&[0u8; 64]);
    let file = dir.path().join("fake.mp4");
    std::fs::write(&file, mp4).unwrap();
    // SAFETY: 本测试二进制只有这一个测试，无并发读写环境变量
    unsafe { std::env::set_var("PATH", &empty) };
    assert!(matches!(
        make_thumbnail(&file, "video/mp4"),
        Err(MediaError::ToolMissing)
    ));
}
