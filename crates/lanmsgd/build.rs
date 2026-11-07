// 仅在目标为 Windows 时嵌入图标与版本信息；Linux/Android 构建不受影响。
// 图标文件两个 exe 共用 lanmsg-desktop 下的同一份。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../lanmsg-desktop/assets/lanmsg.ico");
    res.set("ProductName", "LanMsg");
    res.set("FileDescription", "LanMsg Daemon");
    if let Err(e) = res.compile() {
        panic!("failed to embed Windows resources: {e}");
    }
}
