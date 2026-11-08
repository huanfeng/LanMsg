// `--add-firewall-rule`：非 Windows 平台不做任何事，打印说明并以 0 退出（不启动服务）。
// 注释中文，标识符英文。

#[cfg(not(windows))]
#[test]
fn add_firewall_rule_flag_is_noop_on_non_windows() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_lanmsgd"))
        .arg("--add-firewall-rule")
        .output()
        .expect("failed to run lanmsgd");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("only needed on Windows"),
        "stdout: {stdout}"
    );
}
