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
/// 注入 LANMSG_KEYSTORE=file：与测试进程并行的其他 crate 测试（真实凭据 API
/// 操作）并发写 Windows 凭据存储可瞬时 Access denied，daemon 会因 keystore
/// 错误启动失败；文件库不受影响，测试统一强制文件库。
fn spawn_daemon(dir: &tempfile::TempDir) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_lanmsgd"))
        .args([
            "--headless",
            "--no-multicast",
            "--port",
            "0",
            "--data-dir",
            dir.path().to_str().unwrap(),
        ])
        .env("LANMSG_KEYSTORE", "file")
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

    // Shutdown → 子进程 15 秒内以退出码 0 退出。
    client.call(Request::Shutdown).await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("子进程未在 15 秒内退出")
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

/// 显式指定子进程的 XDG_RUNTIME_DIR（客户端侧手工构造同一端点，
/// 避免污染测试进程环境）。注入 LANMSG_KEYSTORE=file：与测试并行的其他
/// crate 测试对 Windows 凭据存储的并发写可瞬时 Access denied，daemon 会因
/// keystore 错误启动失败；文件库不受影响，测试统一强制文件库。
fn spawn_daemon_env(dir: &tempfile::TempDir, args: &[&str]) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_lanmsgd"))
        .args(args)
        .env("LANMSG_KEYSTORE", "file")
        .env("XDG_RUNTIME_DIR", dir.path().join("run"))
        .spawn()
        .expect("lanmsgd 子进程启动失败")
}

/// 与 spawn_daemon_env 注入的 XDG_RUNTIME_DIR 对应、且 daemon 侧实际监听的
/// IPC 端点。Unix：手工构造 `<XDG_RUNTIME_DIR>/lanmsg/lanmsgd.sock`（不调
/// endpoint_name，避免早前测试 set_var 污染的进程环境参与计算）。Windows：
/// daemon 监听命名管道，名字由 endpoint_name 的 Windows 分支从 USERNAME +
/// blake3(data_dir) 计算，与 XDG_RUNTIME_DIR 无关，直接调用即可。
fn runtime_endpoint(dir: &tempfile::TempDir) -> String {
    #[cfg(windows)]
    {
        lanmsg_api::ipc::endpoint_name(dir.path())
    }
    #[cfg(unix)]
    {
        dir.path()
            .join("run")
            .join("lanmsg")
            .join("lanmsgd.sock")
            .to_string_lossy()
            .into_owned()
    }
}

/// 轮询连接指定端点，最多 10 秒。
async fn connect_endpoint_with_retry(endpoint: &str) -> IpcClient {
    let start = std::time::Instant::now();
    loop {
        match IpcClient::connect(endpoint).await {
            Ok(c) => return c,
            Err(_) if start.elapsed() < std::time::Duration::from_secs(10) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => panic!("端点 {endpoint} 无法连接: {e}"),
        }
    }
}

/// 两个 lanmsgd 子进程 --peer 互指，走完整 CLI 流程：init → 互加联系人 →
/// SendFiles 4MB → DownloadBlob → 等 TransferFinished → 比对内容。
#[tokio::test]
async fn two_daemons_transfer_file_via_cli_flow() {
    let _serial = SERIAL.lock().await;
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    // 单向静态拨号（B --peer A）：双向同时 --peer 会触发 simultaneous_dial
    // 裁决抖动（已知既有问题），单向与 e2e-two-daemons.sh 一致且稳定；
    // 连接建立后双向对等。
    let port_a = "53350";
    let port_b = "53351";
    let addr_a = format!("127.0.0.1:{port_a}");

    let mut a = spawn_daemon_env(
        &dir_a,
        &[
            "--headless",
            "--no-multicast",
            "--port",
            port_a,
            "--data-dir",
            dir_a.path().to_str().unwrap(),
        ],
    );
    let mut b = spawn_daemon_env(
        &dir_b,
        &[
            "--headless",
            "--no-multicast",
            "--port",
            port_b,
            "--data-dir",
            dir_b.path().to_str().unwrap(),
            "--peer",
            &addr_a,
        ],
    );

    let client_a = connect_endpoint_with_retry(&runtime_endpoint(&dir_a)).await;
    let client_b = connect_endpoint_with_retry(&runtime_endpoint(&dir_b)).await;

    // 各自初始化。
    for (c, name, dev) in [(&client_a, "甲", "甲机"), (&client_b, "乙", "乙机")] {
        c.call(Request::Init {
            display_name: name.into(),
            device_name: dev.into(),
            mode: InitMode::FirstDevice,
        })
        .await
        .unwrap();
    }
    let user_a = match client_a.call(Request::GetSelf).await.unwrap() {
        Response::SelfInfo { value } => value.user_id.unwrap(),
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    };

    // 等 B 的 ListPeers 出现 A（静态拨号建立连接）。
    let start = std::time::Instant::now();
    loop {
        let seen = matches!(
            client_b.call(Request::ListPeers).await.unwrap(),
            lanmsg_api::types::Response::Peers { ref value }
                if value.iter().any(|p| p.display_name == "甲")
        );
        if seen {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            panic!("10 秒内 B 未发现 A");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // B request-contact A → A respond add。
    client_b
        .call(Request::RequestContact {
            user: user_a.clone(),
        })
        .await
        .unwrap();
    let mut events_a = client_a.events();
    let req = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(10), events_a.recv()).await {
            Ok(Ok(lanmsg_api::types::Event::IncomingRequest { request })) => {
                if matches!(request.kind, lanmsg_api::types::RequestKind::Contact) {
                    break request.request;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => {}
            Err(_) => panic!("10 秒内未收到联系人请求"),
        }
    };
    client_a
        .call(Request::RespondRequest {
            request: req,
            action: lanmsg_api::types::RequestAction::AddContact,
        })
        .await
        .unwrap();

    // B 打开 DM 并发 4MB 文件。
    let conv = match client_b
        .call(Request::OpenDm { user: user_a })
        .await
        .unwrap()
    {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    };
    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("吞吐.bin");
    let payload: Vec<u8> = (0..(4 * 1024 * 1024)).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &payload).unwrap();
    client_b
        .call(Request::SendFiles {
            conv: conv.clone(),
            paths: vec![src.to_string_lossy().into_owned()],
        })
        .await
        .unwrap();

    // A 等 MessageAdded 拿到 hash。
    let hash = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(15), events_a.recv()).await {
            Ok(Ok(lanmsg_api::types::Event::MessageAdded { msg })) => {
                if let lanmsg_api::types::MessageBodyView::Files { items } = &msg.body {
                    if let Some(it) = items.iter().find(|it| !it.hash.is_empty()) {
                        break it.hash.clone();
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => {}
            Err(_) => panic!("15 秒内未收到文件消息"),
        }
    };

    // A 发起下载并等 TransferFinished ok。
    let transfer = match client_a
        .call(Request::DownloadBlob { hash: hash.clone() })
        .await
        .unwrap()
    {
        Response::TransferId { value } => value,
        other => panic!("期望 TransferId，实际 {other:?}"),
    };
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(30), events_a.recv()).await {
            Ok(Ok(lanmsg_api::types::Event::TransferFinished {
                transfer: t, ok, ..
            })) if t == transfer => {
                assert!(ok, "下载应成功");
                break;
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => {}
            Err(_) => panic!("30 秒内下载未完成"),
        }
    }

    // 比对内容：从 files-of 视角（ListMessages）取 local_path 读文件。
    let msgs = match client_a
        .call(Request::ListMessages {
            conv,
            before: None,
            limit: 100,
        })
        .await
        .unwrap()
    {
        Response::Messages { value } => value,
        other => panic!("期望 Messages，实际 {other:?}"),
    };
    let mut local_path = None;
    for m in msgs {
        if let lanmsg_api::types::MessageBodyView::Files { items } = &m.body {
            for it in items {
                if it.hash == hash {
                    local_path = it.local_path.clone();
                }
            }
        }
    }
    let local_path = local_path.expect("下载后应有本地路径");
    let got = std::fs::read(&local_path).expect("读取下载文件失败");
    assert_eq!(got, payload, "下载内容应与源文件一致");

    // 收尾：两个 daemon 退出。
    client_a.call(Request::Shutdown).await.unwrap();
    client_b.call(Request::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), a.wait()).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), b.wait()).await;
}

/// 收到 IPC Shutdown 后进程正常退出（码 0），且 Unix socket 文件由 Drop 清理。
#[cfg(unix)]
#[tokio::test]
async fn shutdown_request_removes_socket_file() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    isolate_runtime_dir(&dir);
    let mut child = spawn_daemon(&dir);
    let client = connect_with_retry(&dir).await;
    let sock = std::path::PathBuf::from(endpoint_name(dir.path()));
    assert!(sock.exists(), "运行中 socket 文件应存在");

    client.call(Request::Shutdown).await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("子进程未在 15 秒内退出")
        .expect("等待子进程失败");
    assert!(status.success(), "退出码应为 0，实际 {status}");
    assert!(
        !sock.exists(),
        "退出后 socket 文件应被清理: {}",
        sock.display()
    );
}

/// 收到 SIGTERM 后优雅关闭：进程退出且 socket 文件被清理。
#[cfg(unix)]
#[tokio::test]
async fn sigterm_shuts_down_gracefully_and_removes_socket() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    isolate_runtime_dir(&dir);
    let mut child = spawn_daemon(&dir);
    let _client = connect_with_retry(&dir).await;
    let sock = std::path::PathBuf::from(endpoint_name(dir.path()));
    assert!(sock.exists(), "运行中 socket 文件应存在");

    let pid = child.id().expect("子进程应有 pid");
    let st = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("执行 kill 失败");
    assert!(st.success());
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("子进程未在 15 秒内退出")
        .expect("等待子进程失败");
    assert!(status.success(), "优雅关闭退出码应为 0，实际 {status}");
    assert!(!sock.exists(), "退出后 socket 文件应被清理");
}

/// IPC Shutdown 后数据目录出现主动退出标记；再次启动（IPC 绑定后）标记被删除。
#[tokio::test]
async fn shutdown_writes_quit_marker_and_restart_removes_it() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    isolate_runtime_dir(&dir);
    let marker = dir.path().join(lanmsg_proto::consts::DAEMON_QUIT_MARKER);

    let mut child = spawn_daemon(&dir);
    let client = connect_with_retry(&dir).await;
    assert!(!marker.exists(), "运行中不应有退出标记");
    client.call(Request::Shutdown).await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("子进程未在 15 秒内退出")
        .expect("等待子进程失败");
    assert!(status.success());
    assert!(marker.exists(), "Shutdown 退出后应写退出标记");

    let mut child = spawn_daemon(&dir);
    let client = connect_with_retry(&dir).await;
    assert!(!marker.exists(), "再次启动成功后标记应被删除");
    client.call(Request::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(15), child.wait()).await;
}

/// SIGTERM 优雅关闭不写退出标记（系统关机后的自启不应被拦）。
#[cfg(unix)]
#[tokio::test]
async fn sigterm_does_not_write_quit_marker() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    isolate_runtime_dir(&dir);
    let mut child = spawn_daemon(&dir);
    let _client = connect_with_retry(&dir).await;
    let pid = child.id().expect("子进程应有 pid");
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("执行 kill 失败");
    tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("子进程未在 15 秒内退出")
        .expect("等待子进程失败");
    assert!(!dir
        .path()
        .join(lanmsg_proto::consts::DAEMON_QUIT_MARKER)
        .exists());
}
