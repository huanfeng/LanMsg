// 单机端到端验收缺陷修复的集成测试（见 docs/superpowers/reports/2026-11-06-e2e-acceptance.md）。
// 一律 node_offline 系 + link（手动互拨）。注释中文，标识符英文。

mod common;

use lanmsg_api::types::{MessageBodyView, Request, Response};
use lanmsg_proto::ids::{ConvId, UserId};

use common::{befriend, node_offline, pair, wait_until, TestNode};

async fn join_init(b: &TestNode) {
    b.try_call(Request::Init {
        display_name: "加入机".into(),
        device_name: "加入机".into(),
        mode: lanmsg_api::types::InitMode::JoinExisting,
    })
    .await
    .expect("Init JoinExisting 失败");
}

fn user(s: &str) -> UserId {
    UserId::from_hex(s).expect("非法 user_id")
}

async fn texts(n: &TestNode, conv: &str) -> Vec<String> {
    match n
        .try_call(Request::ListMessages {
            conv: conv.to_string(),
            before: None,
            limit: 50,
        })
        .await
    {
        Ok(Response::Messages { value }) => value
            .into_iter()
            .filter_map(|m| match m.body {
                MessageBodyView::Text { text } => Some(text),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// 缺陷 1：A1、A2 同用户在线，B 仅与 A1 相连；A1 发给 B 的消息与 B 的回复
/// 都应实时出现在 A2 的该 DM 会话中（无需重启/重连）。
#[tokio::test]
async fn online_sibling_receives_third_party_dm_in_realtime() {
    let mut a1 = node_offline().await;
    let mut a2 = node_offline().await;
    let a_user = a1.init_first("甲").await;
    join_init(&a2).await;
    pair(&mut a1, &mut a2).await;
    let mut b = node_offline().await;
    let b_user = b.init_first("乙").await;
    befriend(&mut a1, &mut b).await;

    // 等联系人同步到 A2。
    wait_until(
        || async {
            matches!(
                a2.try_call(Request::ListContacts).await,
                Ok(Response::Contacts { value }) if value.iter().any(|v| v.user_id == b_user)
            )
        },
        5,
    )
    .await;

    let conv = ConvId::dm(&user(&a_user), &user(&b_user)).to_hex();
    a1.call(Request::OpenDm {
        user: b_user.clone(),
    })
    .await;
    a1.call(Request::SendText {
        conv: conv.clone(),
        text: "去".into(),
    })
    .await;
    wait_until(
        || async { texts(&a2, &conv).await.contains(&"去".to_string()) },
        5,
    )
    .await;

    // B 回复（B 只连着 A1）。
    b.call(Request::OpenDm {
        user: a_user.clone(),
    })
    .await;
    b.call(Request::SendText {
        conv: conv.clone(),
        text: "回".into(),
    })
    .await;
    wait_until(
        || async { texts(&a2, &conv).await.contains(&"回".to_string()) },
        5,
    )
    .await;
}

/// 递归收集目录下所有文件的 (相对路径, 内容)，按路径排序。
fn collect_tree(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// 缺陷 2：含两级子目录的文件夹下载后，落地为
/// `<下载目录>/<会话标题>/<文件夹名>/<相对路径>`，结构与内容完全一致。
#[tokio::test]
async fn folder_download_preserves_structure() {
    use lanmsg_api::types::BlobState;

    let mut a = node_offline().await;
    a.init_first("A").await;
    let mut x = node_offline().await;
    x.init_first("X").await;
    befriend(&mut a, &mut x).await;

    let src = tempfile::tempdir().unwrap();
    let root = src.path().join("项目目录");
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    std::fs::write(root.join("sub/s1.bin"), vec![1u8; 4096]).unwrap();
    std::fs::write(root.join("sub/deep/d1.bin"), vec![2u8; 8192]).unwrap();
    std::fs::write(root.join("sub/deep/d2.bin"), vec![3u8; 100]).unwrap();

    let dl = tempfile::tempdir().unwrap();
    let Response::Settings { mut value } = x.call(Request::GetSettings).await else {
        panic!("期望 Settings")
    };
    value.download_dir = dl.path().to_string_lossy().into_owned();
    x.call(Request::SetSettings { settings: value }).await;

    let conv = match a.call(Request::OpenDm { user: x.user_id() }).await {
        Response::ConvId { value } => value,
        other => panic!("期望 ConvId，实际 {other:?}"),
    };
    a.call(Request::SendFolder {
        conv: conv.clone(),
        path: root.to_string_lossy().into_owned(),
    })
    .await;

    // X 收到 4 个条目后逐个下载（已自动下载的忽略错误），等全部落地。
    wait_until(
        || async {
            let Ok(Response::Messages { value }) = x
                .try_call(Request::ListMessages {
                    conv: conv.clone(),
                    before: None,
                    limit: 50,
                })
                .await
            else {
                return false;
            };
            for m in value {
                if let MessageBodyView::Folder { items, .. } = m.body {
                    if items.len() != 4 {
                        return false;
                    }
                    for it in &items {
                        if it.state != BlobState::Local {
                            let _ = x
                                .try_call(Request::DownloadBlob {
                                    hash: it.hash.clone(),
                                })
                                .await;
                        }
                    }
                    return items.iter().all(|it| it.state == BlobState::Local);
                }
            }
            false
        },
        20,
    )
    .await;

    let got = collect_tree(&dl.path().join("A").join("项目目录"));
    assert_eq!(got, collect_tree(&root), "落地结构应与源文件夹一致");
}

/// 缺陷 3：QR 配对后新设备的 GetSelf.display_name 等于原设备的用户显示名。
#[tokio::test]
async fn paired_device_inherits_display_name() {
    let mut a = node_offline().await;
    a.init_first("甲").await;
    let mut b = node_offline().await;
    join_init(&b).await;
    pair(&mut a, &mut b).await;
    match b.call(Request::GetSelf).await {
        Response::SelfInfo { value } => {
            assert_eq!(value.display_name.as_deref(), Some("甲"));
        }
        other => panic!("期望 SelfInfo，实际 {other:?}"),
    }
}
