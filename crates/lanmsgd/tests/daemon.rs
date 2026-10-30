// P1 核心骨架：lanmsgd 守护进程集成测试（Task 12）。
// 用 CARGO_BIN_EXE_lanmsgd 启动子进程，经 IPC 验证 GetSelf/Init/Shutdown
// 与第二个实例拒绝启动（端点占用 → 退出码 2）。
// 注释中文，标识符英文。

use std::time::Duration;

use lanmsg_api::ipc::{endpoint_name, IpcClient};
use lanmsg_api::types::{InitMode, Request, Response};

/// Linux 下 XDG 端点固定为 `<XDG_RUNTIME_DIR>/lanmsg/lanmsgd.sock`，
/// 不含 data_dir 摘要——测试必须串行，且每个测试用独立的 XDG_RUNTIME_DIR。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 启动一个 lanmsgd 子进程（无组播、随机端口、指定数据目录）。
/// 先把 XDG_RUNTIME_DIR 指向本测试的临时目录，子进程与客户端计算同一端点。
fn spawn_daemon(dir: &tempfile::TempDir) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_lanmsgd"))
        .args([
            "--no-multicast",
            "--port",
            "0",
            "--data-dir",
            dir.path().to_str().unwrap(),
        ])
        .spawn()
        .expect("lanmsgd 子进程启动失败")
}

/// 为本测试设置独立的 XDG_RUNTIME_DIR（子进程会继承）。
fn isolate_runtime_dir(dir: &tempfile::TempDir) {
    std::env::set_var("XDG_RUNTIME_DIR", dir.path().join("run"));
}

/// 轮询连接 IPC 端点，最多 5 秒。
async fn connect_with_retry(dir: &tempfile::TempDir) -> IpcClient {
    let endpoint = endpoint_name(dir.path());
    let start = std::time::Instant::now();
    loop {
        match IpcClient::connect(&endpoint).await {
            Ok(c) => return c,
            Err(_) if start.elapsed() < Duration::from_secs(5) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("5 秒内无法连接端点 {endpoint}: {e}"),
        }
    }
}

#[tokio::test]
async fn daemon_serves_get_self_over_ipc() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    isolate_runtime_dir(&dir);
    let mut child = spawn_daemon(&dir);
    let client = connect_with_retry(&dir).await;

    // 未初始化：initialized == false。
    let resp = client.call(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value } = resp else {
        panic!("期望 SelfInfo");
    };
    assert!(!value.initialized);

    // Init{FirstDevice} 后 initialized == true。
    client
        .call(Request::Init {
            display_name: "测试者".into(),
            device_name: "测试设备".into(),
            mode: InitMode::FirstDevice,
        })
        .await
        .unwrap();
    let resp = client.call(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value } = resp else {
        panic!("期望 SelfInfo");
    };
    assert!(value.initialized);
    assert_eq!(value.display_name.as_deref(), Some("测试者"));

    // Shutdown → 子进程 5 秒内以退出码 0 退出。
    client.call(Request::Shutdown).await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("子进程未在 5 秒内退出")
        .expect("等待子进程失败");
    assert!(status.success(), "退出码应为 0，实际 {status}");
}

#[tokio::test]
async fn second_daemon_refuses_to_start() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    isolate_runtime_dir(&dir);
    let mut first = spawn_daemon(&dir);
    let client = connect_with_retry(&dir).await;

    // 同一 data_dir 启动第二个 lanmsgd → 5 秒内以退出码 2 结束。
    let mut second = spawn_daemon(&dir);
    let status = tokio::time::timeout(Duration::from_secs(5), second.wait())
        .await
        .expect("第二个 lanmsgd 未在 5 秒内退出")
        .expect("等待第二个 lanmsgd 失败");
    assert_eq!(status.code(), Some(2), "第二个 lanmsgd 应以退出码 2 结束");

    // 第一个仍可正常服务。
    let resp = client.call(Request::GetSelf).await.unwrap();
    let Response::SelfInfo { value } = resp else {
        panic!("期望 SelfInfo");
    };
    assert!(!value.initialized);

    first.kill().await.ok();
}
