# P5 扩展功能与发布 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **开工前必须读 `docs/superpowers/plans/2026-10-28-00-roadmap.md`（全局约束 + 共享契约 C1–C8），并浏览 P2/P3/P4 计划中被本计划引用的接口（见下文「依赖的前序接口」）。**

**Goal:** 完成文件中转站、Web 临时会话、LocalSend v2 兼容、跨网段连接与路径标记，并产出 Windows / Linux / macOS / Android 四个平台的安装渠道与发布流水线。

**Architecture:** 四个功能都挂在 `lanmsg-core` 的 `Service` 上，经 `Service::handle` 与事件流对外（C6）。中转站复用 P2 的 Blob 元数据 + 按需拉取机制，只新增"货架"数据模型与 `Station` 帧；Web 临时会话是 core 内嵌的 axum HTTP 服务，页面静态资源来自 `lanmsg-web-assets`，内容经应用层加密；LocalSend 是独立 crate `lanmsg-localsend`（不依赖 core），core 通过 `localsend_bridge` 把它接入会话模型；跨网段在 P1 连接管理之上加地址簿、重连器、单播通告与网段扫描。打包脚本放 `packaging/`，发布由 `.github/workflows/release.yml` 驱动。

**Tech Stack:** Rust（tokio、axum 0.7 + ws、reqwest 0.12、rustls、x25519-dalek、crypto_secretbox、hkdf、scrypt、qrcode）、纯 HTML/JS/CSS（vendored tweetnacl 1.0.3、scrypt-js 3.0.1）、wind-installer、scoop、bash、hdiutil、GitHub Actions。

**Spec:** `docs/superpowers/specs/2026-10-28-lanmsg-design.md`（§6.3、§6.5、§11、§13、§14、§18）；共享契约：`docs/superpowers/plans/2026-10-28-00-roadmap.md`

## Global Constraints

路线图「全局约束」全部适用。本阶段额外约束：

- 新增依赖（已在本计划登记，可直接 `cargo add`）：`x25519-dalek@2`（features `static_secrets`）、`crypto_secretbox@0.1`、`hkdf@0.12`、`scrypt@0.11`（关默认 features）、`qrcode@0.14`（关默认 features）、`serde_json@1`（core、localsend）、`uuid@1`（features `v4`）、`base64@0.22`、`mime_guess@2`、`sha2@0.10`（localsend 指纹）、`axum@0.7` 开 `ws` feature。JS 第三方库只允许 `tweetnacl@1.0.3`（`nacl-fast.min.js`）与 `scrypt-js@3.0.1`（`scrypt.js`），以文件形式放在 `web/vendor/`，不得引入其他 JS 依赖或构建工具。
- Web 页面总大小（`web/` 下全部文件之和）< 300KB，由 Task 13 的测试守护。
- Web 临时会话、LocalSend 兼容只在 `PathClass::Lan` 上生效：来源地址经 `classify` 判定不是 `Lan` 的请求一律 403 / 忽略。
- 中转站主机只能是桌面或无头设备（`DeviceType::Desktop | Laptop | Headless`）；Android 上 `StationCreate` 返回 `InvalidArgument("移动设备不能作为中转站主机")`。
- 所有新表的迁移**追加**到 `MIGRATIONS` 末尾（P1 Task 6 规则），本阶段只追加**一个**元素（Task 1），即 `MIGRATIONS[3]`（schema v3 → v4；P2 为 `MIGRATIONS[1]`，P3 为 `MIGRATIONS[2]`）。
- C5 的全部类型（含本阶段用到的 `ScanFinished`、`WebSessionChanged`、`StationSetPinned`、`StationPut.expires_at`、`StationPutItem::Blobs`、`StationItemView.{unavailable, pinned}`、`WebSessionInfo.urls`、`SettingsDto.localsend_listening`）与 `ServiceConfig.localsend_port` 已由 P1 一次定义；本阶段**只实现逻辑**，不新增或修改这些类型。
- 共享状态一律用 `std::sync::{Mutex, RwLock}`，任何锁都不得跨 `.await` 持有（先在块内取出所需数据再 await）。
- wind-installer 仓库的修改遵守该仓库 `AGENTS.md` 的「新增一种能力的标准套路」与「缺省即不执行」原则，并同样遵守 `--only` 提交纪律；LanMsg 开发期以 `path` 方式引用其本地构建产物（设计 §18.1）。
- macOS 真机验收与签名公证是外部依赖（设计 §18.2）：本阶段只写脚本、在 CI 产出未签名 dmg。

## 依赖的前序接口

本计划调用下列前序阶段产出的名字（均已在对应计划中定义）：

| 名字 | 来源 | 本计划用途 |
|---|---|---|
| store：`put_blob`、`blob_segments`、`set_local`、`get_local`、`remove_local`、`add_holder`、`holders`、`message_blobs_by_hash`、`blob_visible_to`；`BlobLocalRow { hash, path, kind: BlobLocalKind { Source, Downloaded }, size, mtime }`（`crates/lanmsg-core/src/store/blobs.rs`） | P2 Task 2 | 本地文件登记、持有者、可见性 |
| `crate::blob::hash::{hash_file, HashedFile, file_mtime}`、`crate::blob::mime::mime_from_name` | P2 Task 11 | 新增的 `register_local_file`（Task 5）复用 |
| `crate::blob::serve::BlobServer::verify_source` | P2 Task 11 | 中转站引用文件校验 |
| `crate::blob::transfer::{TransferEngine, unique_path, sanitize_component}`，`TransferEngine::download(&self, hash: BlobHash) -> Result<String, CoreError>` | P2 Task 12 | 新增的 `TransferEngine::download_to`（Task 6）基于它重构 |
| `crate::platform::{PlatformHooks, ThumbnailProvider, Thumbnail}` | P2 Task 4 | 登记文件时生成缩略图 |
| `crate::settings::load_settings(store: &Store) -> Result<SettingsDto, CoreError>` | P2 Task 4 | 下载目录、LocalSend 开关 |
| `crate::trust::Trust::accept_peer(&self, p: &PeerInfo, status: Status, pair_window_open: bool) -> bool`（规则 7：陌生人 `Vpn` 仅 `p.outbound` 接受） | P2 Task 5 | Task 3 增加"手动添加的设备"放行 |
| `crate::trust::Trust::gate_inbound(&self, from: &PeerInfo, env: &Envelope, now: i64) -> Result<GateDecision, CoreError>`（规则 6） | P2 Task 6 | 同上 |
| `service/handlers.rs` 中 `Request::RespondRequest` 分支（按 `RequestRow.kind` 分派到 `Trust::respond_contact` / `Trust::respond_message_request`）与 `Request::ListRequests` 分支 | P2 Task 5/6 | Task 11 增加 `ls-` 前缀分流 |
| `crate::sync::SyncEngine::shared_convs`、`crate::sync::conv_member_ok` | P2 Task 9 | Task 11/14 增加回归测试，确认 LocalSend / Guest 会话不同步 |
| `crate::service::Inner`、`crate::service::ingest::after_ingest(svc: &Inner, from: DeviceId, m: &StoredMessage)`、`crate::service::views::file_item_view(svc: &Inner, author_user: &UserId, author_device: &DeviceId, meta: &BlobMeta) -> FileItemView` | P2 Task 4/13 | 中转站条目视图 |
| 测试辅助 `crates/lanmsg-core/tests/common/mod.rs`：`TestNode { svc, dir, events }`、`node(&MemoryBus)`、`node_with(&MemoryBus, TempDir, PlatformHooks)`、`TestNode::{call, try_call, init_first, wait_event, no_event, user_id}`、`wait_until`、`pair(a, b)`、`befriend(a, b)` | P2 Task 4/5/7 | 本计划全部 core 集成测试 |
| `crates/lanmsgd/src/tray.rs` 的 `run_tray(rt: DaemonRuntime, data_dir: PathBuf)`、`notify.rs`、`launch.rs::open_gui`、`runtime.rs::{DaemonRuntime, is_headless}`、`--headless` 参数 | P3 Task 8/9 | 防火墙菜单项；无头构建 |
| 界面：`bridge.rs` 的 `UiMsg`/`ReplyTag`/`Bridge`；`model.rs` 的 `Model`/`Page`/`Effect`；`ui_state.rs` 的 `UiState`；`fixtures.rs` 的 `DemoPage`/`demo_model`；`views/shell.rs`；`views/settings.rs` 的"网络"分组；`views/pair.rs` 的 `pub fn qr_rgba(data: &str, px: u32) -> (u32, u32, Vec<u8>)`；`views/message_row.rs` 的 `pub fn file_state_area(ui, item)`；`strings.rs`；`tests/screenshots.rs` 的 `every_demo_page_renders`；`autostart.rs::set_enabled`（macOS LaunchAgent `app.lanmsg.daemon`） | P3 Task 10–15 | 桌面增量页面 |
| Android：`name.apk.lanmsg.core.CoreApi`（`request`/`events`/`lagged`）、`call<T>` 扩展、`FfiCoreApi`、测试用 `testutil/FakeCoreApi`（`handler`、`requests`、`emit`）、`Fixtures`、`MainDispatcherRule`；`ui/nav/Routes.kt`、`ui/nav/LanMsgNavHost.kt`；`ui/common/QrImage.kt`（`QrEncoder`、`QrImage`）、`PathBadge`；`ui/convs/ConversationListScreen.kt`；`ui/settings/SettingsScreen.kt`；`service/BackgroundPolicy.kt`（`Conditions`、`shouldRunService`）、`DeviceConditions`；`ServiceLocator.coreHolder.api`；`docs/ui-architecture.md` §2 路由、§4 状态、§6 文案表 | P4 Task 3–11 | Android 增量页面 |
| `scripts/build-android-core.sh [debug\|release]`、`scripts/build-android.sh [debug\|release]`、`android/keystore.properties` | P4 Task 2/13 | 发布流水线 |

`Response` 变体统一为具名字段 `{ value: T }`（路线图 C5），本计划写作 `Response::ConvId { value }` 等。

---

## Review Focus

1. **浏览器拿到的口令被截获 / 有人在同网段乱猜配对码**：手动码模式下，连续 5 次确认失败必须作废当前码并在宿主重新生成，单 IP 每分钟最多 10 次 `hello`。→ Task 12 测试 `code_mode_rotates_after_five_failures`、Task 14 测试 `hello_rate_limited_per_ip`。
2. **中转站引用的原文件被用户移动 / 删除 / 修改**：条目显示为不可用，而不是把旧内容当新内容发出去，也不是让下载卡死。→ Task 5 测试 `reference_item_marked_unavailable_when_file_changes`。
3. **本机同时运行着 LocalSend 本体（53317 被占用）**：LanMsg 不得对外通告 LocalSend 身份，但仍能发现 LocalSend 设备并向它发送。→ Task 10 测试 `port_busy_disables_announce_but_keeps_sending`。
4. **网段扫描输入过大或格式错误**（`10.0.0.0/8`、`abc`、IPv6 段）：立即返回 `InvalidArgument`，不发出任何探测。→ Task 4 测试 `scan_rejects_too_large_or_invalid_cidr`。
5. **Web 上传传到一半网页刷新或断网**：已收到的分块保留，同一页面重连后只补缺块；页面关闭超时后 `.part` 文件被清理，不在下载目录留下半截文件。→ Task 14 测试 `upload_resumes_missing_chunks_only`、`idle_timeout_cleans_partial_uploads`。

---

## 文件结构

```
crates/lanmsg-proto/src/
  frame.rs                 # 追加 AddressBook(50)、Station(60)
  station.rs               # Station 帧子消息与数据类型（新）
  addrbook.rs              # AddressBook 帧类型（新）
crates/lanmsg-core/src/
  store/schema.rs          # MIGRATIONS 追加 1 个元素
  store/addresses.rs       # known_addresses、manual_peers（新）
  store/stations.rs        # stations、station_items（新）
  store/localsend.rs       # localsend_peers（新）
  net/path.rs              # 扩展：TunnelKind、隧道识别
  net/addrbook.rs          # 地址簿交换与合并（新）
  net/reconnect.rs         # 重连器（新，取代 P1 中 static_peers 的重试逻辑）
  net/scan.rs              # 网段扫描（新）
  discovery/unicast.rs     # 单播通告目标（新）
  station/mod.rs  station/host.rs  station/client.rs   # 中转站（新）
  localsend_bridge.rs      # LocalSend 接入（新）
  web/mod.rs  web/crypto.rs  web/server.rs  web/session.rs  web/transfer.rs  # Web 临时会话（新）
  firewall.rs              # Windows 防火墙检测（新，cfg(windows)）
crates/lanmsg-localsend/src/
  lib.rs  types.rs  discovery.rs  server.rs  client.rs  cert.rs
crates/lanmsg-localsend/tests/fixtures/*.json
crates/lanmsg-web-assets/src/lib.rs   build.rs
web/
  index.html  style.css  app.js  crypto.js
  vendor/nacl-fast.min.js  vendor/scrypt.js  vendor/LICENSES.txt
  test/crypto.test.mjs  test/vectors.json
crates/lanmsgd/src/firewall_cmd.rs     # --add-firewall-rule（新）
crates/lanmsg-core/src/blob/register.rs  # register_local_file（新）
crates/lanmsg-desktop/src/views/stations.rs  views/web_transfer.rs  views/network.rs  station_put.rs
android/app/src/main/java/name/apk/lanmsg/ui/station/{StationScreen.kt,StationViewModel.kt}
android/app/src/main/java/name/apk/lanmsg/ui/web/{WebHostScreen.kt,WebHostViewModel.kt}
android/app/src/main/java/name/apk/lanmsg/ui/settings/{NetworkSettingsScreen.kt,NetworkSettingsViewModel.kt}
packaging/
  windows/app.toml  windows/build-dist.ps1  assets/lanmsg.ico  assets/lanmsg-256.png  assets/logo.png
  scoop/lanmsg.json  scoop/update-manifest.sh
  linux/install.sh  linux/uninstall.sh  linux/lanmsgd.service  linux/lanmsg.desktop  linux/build-tarball.sh
  macos/build-dmg.sh  macos/Info.plist
.github/workflows/release.yml
（wind-installer 仓库）src/manifest.rs  src/installer/firewall.rs  src/installer/shortcut.rs  src/installer/steps.rs
  src/installer/plan.rs  src/installer/receipt.rs  tests/step_plan.rs  tests/manifest_parse.rs  README.md  app.toml
```

---

### Task 1: P5 数据表迁移与协议扩展

**Files:**
- Modify: `crates/lanmsg-core/src/store/schema.rs`（`MIGRATIONS` 末尾追加 1 个元素，即 `MIGRATIONS[3]`）
- Create: `crates/lanmsg-proto/src/station.rs`、`crates/lanmsg-proto/src/addrbook.rs`
- Modify: `crates/lanmsg-proto/src/frame.rs`、`crates/lanmsg-proto/src/lib.rs`

**Interfaces:**
- Produces（`lanmsg_proto`）：

```rust
// addrbook.rs
pub struct AddressBook { pub entries: Vec<AddressEntry> }
pub struct AddressEntry { pub device: DeviceId, pub addrs: Vec<KnownAddr> }
pub struct KnownAddr { pub addr: String /* "ip:port"，IPv6 为 "[ip]:port" */, pub path: PathClass, pub last_ok: i64 }

// station.rs
pub struct ItemId(pub [u8; 16]);            // serde 为 bytes；to_hex/from_hex（32 位 hex）；ItemId::random()
pub enum StationAcl { OwnerOnly, Contacts { users: Vec<UserId> } }   // v1 只用 OwnerOnly
pub struct StationInfo { pub station: ConvId, pub name: String, pub host: DeviceId, pub owner: UserId,
    pub acl: StationAcl, pub created_at: i64, pub deleted: bool }
pub enum StationContent { Text { text: String }, File { blob: BlobMeta },
    Folder { name: String, entries: Vec<FolderEntry> } }
pub struct StationItem { pub item: ItemId, pub added_by: DeviceId, pub added_at: i64,
    pub expires_at: Option<i64>, pub content: StationContent, pub pinned: bool,
    pub removed: bool, pub available: bool, pub updated_at: i64 }
pub enum StationMsg {
    Announce { stations: Vec<StationInfo> },                        // 主机 → 自己的设备：我托管的站点
    Subscribe { station: ConvId, since: i64 },                      // 拉取端 → 主机：要 updated_at > since 的条目
    Snapshot { info: StationInfo, items: Vec<StationItem>, now: i64 },  // 主机 → 拉取端
    Delta { station: ConvId, items: Vec<StationItem> },              // 主机主动推送（含 removed=true）
    Put { station: ConvId, req: u64, item: StationItem, mode_copy: bool },  // 拉取端 → 主机
    Remove { station: ConvId, req: u64, item: ItemId },
    SetPinned { station: ConvId, req: u64, item: ItemId, pinned: bool },  // 拉取端 → 主机
    Result { req: u64, ok: bool, error: Option<String> },           // 主机 → 拉取端
}
```
- `Frame::AddressBook(AddressBook)` 类型号 **50**，`Frame::Station(StationMsg)` 类型号 **60**（C4 已预留）。
- 迁移（`MIGRATIONS[3]`，原样写入）：

```sql
CREATE TABLE known_addresses(device_id BLOB NOT NULL, addr TEXT NOT NULL, path TEXT NOT NULL,
  last_ok INTEGER NOT NULL, source TEXT NOT NULL, PRIMARY KEY(device_id, addr));
CREATE TABLE manual_peers(addr TEXT PRIMARY KEY, added_at INTEGER NOT NULL, device_id BLOB);
CREATE TABLE stations(station_id BLOB PRIMARY KEY, name TEXT NOT NULL, host_device BLOB NOT NULL,
  owner_user BLOB NOT NULL, acl BLOB NOT NULL, created_at INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0, synced_until INTEGER NOT NULL DEFAULT 0);
CREATE TABLE station_items(item_id BLOB PRIMARY KEY, station_id BLOB NOT NULL, added_by BLOB NOT NULL,
  added_at INTEGER NOT NULL, expires_at INTEGER, content BLOB NOT NULL, pinned INTEGER NOT NULL DEFAULT 0,
  removed INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1, updated_at INTEGER NOT NULL,
  store_mode TEXT, local_path TEXT, local_size INTEGER, local_mtime INTEGER);
CREATE INDEX station_items_by_station ON station_items(station_id, updated_at);
CREATE TABLE station_blobs(item_id BLOB NOT NULL, hash BLOB NOT NULL, name TEXT NOT NULL,
  size INTEGER NOT NULL, mime TEXT NOT NULL, rel_path TEXT NOT NULL DEFAULT '', meta BLOB NOT NULL,
  PRIMARY KEY(item_id, rel_path, hash));
CREATE INDEX station_blobs_by_hash ON station_blobs(hash);
CREATE TABLE localsend_peers(fingerprint TEXT PRIMARY KEY, pseudo_device BLOB NOT NULL, alias TEXT NOT NULL,
  device_model TEXT, device_type TEXT, last_addr TEXT, protocol TEXT NOT NULL, port INTEGER NOT NULL,
  last_seen INTEGER NOT NULL, trusted INTEGER NOT NULL DEFAULT 0, blocked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE web_guests(conv_id BLOB PRIMARY KEY, pseudo_device BLOB NOT NULL, user_agent TEXT NOT NULL,
  ip TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER);
```
  - `content` / `acl` 列存 CBOR；`store_mode`（`reference`/`copy`）、`local_path`、`local_size`、`local_mtime` 只在主机上有值。
  - `stations.synced_until`：拉取端已同步到的主机 `updated_at` 水位。
  - `station_blobs`：中转站条目引用的每个 Blob 一行（File 一行；Folder 每个 entry 一行，`rel_path` 为 entry 路径），`meta` 为 `BlobMeta` 的 CBOR；主机与拉取端都写，用于按哈希反查名字/大小（下载）与可见性判定。

- [ ] **Step 1: 写失败测试**

```rust
// proto: frame.rs 测试模块
#[test]
fn station_and_addressbook_frames_roundtrip() {
    // 构造 Frame::Station(StationMsg::Put{..含 File blob..}) 与 Frame::AddressBook(..)，
    // encode→decode 相等；type_id 分别为 60、50
}
// core: store/schema.rs 测试模块
#[test]
fn p5_migration_creates_tables() {
    // open_in_memory 后对 7 张新表各执行一次 SELECT count(*)，均返回 0
}
```
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-proto -p lanmsg-core`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**（`crates/lanmsg-proto`、`crates/lanmsg-core/src/store/schema.rs`）

---

### Task 2: 网卡枚举、隧道识别与 `ListInterfaces`

**Files:**
- Modify: `crates/lanmsg-core/src/net/path.rs`
- Modify: `crates/lanmsg-core/src/service/handlers.rs`

**Interfaces:**
- Consumes: P1 `LocalIface`、`classify`。
- Produces:
  - `pub enum TunnelKind { Tailscale, ZeroTier, WireGuard, Tun }`，`fn as_str(&self) -> &'static str`（`"tailscale"`/`"zerotier"`/`"wireguard"`/`"tun"`）。
  - `LocalIface` 追加字段 `pub tunnel: Option<TunnelKind>`（P1 中构造 `LocalIface` 的测试同步补 `tunnel: None`）。
  - `pub fn detect_tunnel(name: &str, addr: IpAddr) -> Option<TunnelKind>`，规则按顺序：
    1. 名称（小写）以 `tailscale` 开头，或包含 `tailscale` → `Tailscale`；
    2. 名称以 `zt` 开头且长度 > 2，或包含 `zerotier` → `ZeroTier`；
    3. 名称以 `wg` 开头，或包含 `wireguard` → `WireGuard`；
    4. 名称以 `utun`/`tun`/`ppp`/`tap` 开头：地址在 `100.64.0.0/10` → `Tailscale`，否则 `Tun`；
    5. 其余 `None`。
  - `classify` 改为：`via_multicast` → `Lan`；peer 落在某网卡网段内：该网卡 `tunnel.is_some()` → `Vpn`，否则 `Lan`；其余规则同 P1。
  - `Request::ListInterfaces` → `Response::Interfaces { value: Vec<InterfaceView> }`：`name`、`addr`（字符串）、`prefix`、`tunnel_kind = tunnel.map(as_str)`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn detect_tunnel_table() {
    assert_eq!(detect_tunnel("tailscale0", "100.100.1.2".parse().unwrap()), Some(TunnelKind::Tailscale));
    assert_eq!(detect_tunnel("utun4", "100.70.0.1".parse().unwrap()), Some(TunnelKind::Tailscale));
    assert_eq!(detect_tunnel("utun4", "10.0.0.1".parse().unwrap()), Some(TunnelKind::Tun));
    assert_eq!(detect_tunnel("ztabcdef12", "10.147.17.5".parse().unwrap()), Some(TunnelKind::ZeroTier));
    assert_eq!(detect_tunnel("ZeroTier One [abc]", "10.147.17.5".parse().unwrap()), Some(TunnelKind::ZeroTier));
    assert_eq!(detect_tunnel("wg0", "10.8.0.2".parse().unwrap()), Some(TunnelKind::WireGuard));
    assert_eq!(detect_tunnel("eth0", "192.168.1.2".parse().unwrap()), None);
}
#[test]
fn same_subnet_over_tunnel_is_vpn() {
    let ifaces = vec![LocalIface { name: "tailscale0".into(), addr: "100.101.1.1".parse().unwrap(), prefix: 10, tunnel: Some(TunnelKind::Tailscale) }];
    assert_eq!(classify("100.90.2.2".parse().unwrap(), &ifaces, false), PathClass::Vpn);
}
// tests/two_nodes.rs 追加
#[tokio::test]
async fn list_interfaces_returns_ok() { /* handle(ListInterfaces) 返回 Response::Interfaces { value }（内容随机器而定，只断言不报错且 value 中不含回环地址） */ }
```
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 3: 地址簿、重连器与手动添加

**Files:**
- Create: `crates/lanmsg-core/src/store/addresses.rs`、`crates/lanmsg-core/src/net/addrbook.rs`、`crates/lanmsg-core/src/net/reconnect.rs`
- Modify: `crates/lanmsg-core/src/service/mod.rs`（用 `Reconnector` 取代 P1 的 static_peers 重试循环）、`service/handlers.rs`（`AddManualPeer` 完整流程）、`crates/lanmsg-core/src/trust.rs`（`accept_peer` 规则 7、`gate_inbound` 规则 6）
- Test: `crates/lanmsg-core/tests/cross_subnet.rs`

**Interfaces:**
- Consumes: Task 1 表与 `AddressBook`；P1 `ConnManager`、`ConnEvent`；P2 `Trust::accept_peer`、`Trust::gate_inbound`、`tests/common`。
- Produces:
  - store（`addresses.rs`）：
    - `pub struct KnownAddrRow { pub device: DeviceId, pub addr: SocketAddr, pub path: PathClass, pub last_ok: i64, pub source: AddrSource }`
    - `pub enum AddrSource { Direct, Exchanged, Manual }`（存 `direct`/`exchanged`/`manual`）
    - `record_address(&self, row: &KnownAddrRow) -> Result<()>`：upsert；每设备最多保留 8 条，超出删 `last_ok` 最旧的。
    - `addresses_of(&self, device: &DeviceId) -> Result<Vec<KnownAddrRow>>`：按路径优先级（Lan < Vpn < Internet < Relay）再按 `last_ok` 降序。
    - `add_manual_peer(&self, addr: &str, now: i64) -> Result<()>`、`list_manual_peers(&self) -> Result<Vec<(String, Option<DeviceId>)>>`、`bind_manual_peer(&self, addr: &str, device: &DeviceId) -> Result<()>`、`is_manual_device(&self, device: &DeviceId) -> Result<bool>`
  - `net/addrbook.rs`：
    - `pub fn build_for(store: &Store, self_user: &UserId, peer_user: &UserId) -> Result<AddressBook, CoreError>`：对端是自己的设备（`peer_user == self_user`）→ 包含所有已知设备（自己的 + 联系人的，排除被吊销的）；对端是联系人 → 只包含自己的设备；其余返回空。
    - `pub fn merge(store: &Store, from: &PeerInfo, book: &AddressBook, now: i64) -> Result<usize, CoreError>`：只接受 `devices` 表中已存在且未吊销的设备；对端不是自己设备时，只接受属于 `from` 所属用户的条目；`path = Internet` 或 `Relay` 的条目丢弃（v1 不用）；写入 `source = Exchanged`；返回写入条数。
  - `net/reconnect.rs`：
    - `pub struct Reconnector`：`pub fn spawn(conns: ConnManager, store: Arc<Store>, static_peers: Vec<SocketAddr>) -> Reconnector`、`pub fn kick(&self, device: DeviceId)`（立即重试一次）、`pub fn add_target(&self, addr: SocketAddr)`、`pub fn shutdown(&self)`。
    - 目标集合 = `static_peers` ∪ `manual_peers` ∪ 所有"自己的设备 + 联系人设备"中**当前未连接**者。每 5 秒扫描一次；每个目标独立退避：首次失败等 `RETRY_MIN`，之后翻倍，上限 `RETRY_MAX`；连上清零。设备目标按 `addresses_of` 的顺序逐个尝试地址，第一个成功即止。
  - Service 接线：
    - `ConnEvent::Up(peer)`：`record_address(KnownAddrRow { device: peer.device, addr: peer.dial_addr, path: peer.path, last_ok: now, source: Direct })`——**必须用 `peer.dial_addr`（对端 IP + Hello.listen_port），不得用 `peer.addr`**：被对方拨入时 `addr` 是对端的临时源端口，无法回拨；若对端是自己的设备或联系人，向其发送 `Frame::AddressBook(build_for(..))`。
    - 收到 `Frame::AddressBook` → `merge`；有新地址则对相关设备 `kick`。
    - `AddManualPeer { addr }`：解析 `host[:port]`（无端口用 `SERVICE_PORT`；IPv6 需 `[ip]:port`），DNS 解析失败 → `InvalidArgument("无法解析地址：…")`；`add_manual_peer` → `conns.connect(addr, false)`；成功则 `bind_manual_peer`，返回 `Response::Ok`；连接失败返回 `PeerOffline`，但地址保留在 `manual_peers` 里由重连器继续尝试。
    - 准入（修改 `crates/lanmsg-core/src/trust.rs`）：`Trust::accept_peer` 规则 7 的 `Vpn` 分支改为 `p.outbound || self.store.is_manual_device(&p.device).unwrap_or(false)`；`Trust::gate_inbound` 规则 6 同样把"`Vpn` 且 `from.outbound`"扩展为"`Vpn` 且（`from.outbound` 或 `is_manual_device(&from.device)`）"。即：用户手动添加过的 vpn 对端，之后由对方主动连入也被接受。

- [ ] **Step 1: 写失败测试**（`tests/cross_subnet.rs`，`mod common;` 引用 P2 测试辅助；`DiscoveryMode::Disabled` 的节点用 `node_with` 时把 bus 换成不共享的新 `MemoryBus`，即彼此不会经发现互连）

```rust
#[tokio::test]
async fn manual_peer_connects_and_is_remembered() {
    // a = node(&bus1)，b = node(&bus2)（不同 bus，互不发现），各 init_first；
    // b.call(AddManualPeer{addr: format!("127.0.0.1:{}", a.svc.local_addr().port())}) → Ok，b.wait_event(PeerOnline(a), 5)；
    // b.svc.shutdown()，用同一 dir 经 node_with 重启 → 10 秒内再次 PeerOnline(a)（来自 manual_peers）
}
#[tokio::test]
async fn address_book_lets_my_devices_find_contact() {
    // p1 = node(&bus1) init_first；p2 = node(&bus1)；pair(&mut p1, &mut p2)；
    // c = node(&bus2) init_first；p1 AddManualPeer(c) 后 befriend(&mut p1, &mut c)；
    // p2 与 c 不同 bus 且从未直连 → p2 收到 p1 的 AddressBook 后 10 秒内 wait_event(PeerOnline(c))
}
#[tokio::test]
async fn address_book_from_contact_cannot_inject_third_party() {
    // 单元级：内存 Store 中 C 为联系人设备、X 为未知设备；merge(from = C 的 PeerInfo, book 含 X) → Ok(0)，addresses_of(X) 为空
}
#[tokio::test]
async fn add_manual_peer_rejects_unparsable_addr() { /* try_call(AddManualPeer{"not a host:99999"}) → ErrorCode::InvalidArgument */ }
#[test]
fn manual_vpn_device_accepted_inbound() {
    // 内存 Store + Trust：陌生人 PeerInfo{path: Vpn, outbound: false}：is_manual_device 为 false → accept_peer 返回 false；
    // bind_manual_peer 后 → true
}
#[test]
fn up_records_dial_addr_not_source_port() {
    // 内存 Store；PeerInfo { addr: "10.0.0.2:50123", dial_addr: "10.0.0.2:53320", path: Vpn, .. }
    // on_peer_up_record(&store, &peer, now) → addresses_of(device)[0].addr == "10.0.0.2:53320"
}
```
Up 时的地址记录抽成 `pub(crate) fn on_peer_up_record(store: &Store, peer: &PeerInfo, now: i64) -> Result<(), CoreError>`（放 `net/addrbook.rs`），Service 的 `ConnEvent::Up` 分支调用它。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**（连续 3 次）
- [ ] **Step 5: Commit**

---

### Task 4: 单播通告目标与网段扫描

**Files:**
- Create: `crates/lanmsg-core/src/discovery/unicast.rs`、`crates/lanmsg-core/src/net/scan.rs`
- Modify: `crates/lanmsg-core/src/discovery/mod.rs`、`service/handlers.rs`

**Interfaces:**
- Consumes: `SettingsDto.unicast_targets`（C5，P2 已实现读写）。
- Produces:
  - `pub fn parse_unicast_targets(items: &[String]) -> Result<Vec<SocketAddr>, CoreError>`：每项可以是 `ip`、`ip:port`、`host`、`host:port` 或 IPv4 CIDR（前缀 ≥ 24）；无端口用 `DISCOVERY_PORT`；CIDR 展开为全部主机地址（去掉网络号与广播地址）；任一项非法 → `InvalidArgument(项)`。`SetSettings` 时先校验，非法则拒绝保存。
  - Discovery 增加 `pub fn set_unicast_targets(&self, targets: Vec<SocketAddr>)`：每次周期通告时，额外向每个目标单播发送同一份 `Announce` 字节；由此收到的回应 `via_multicast = false`。Memory 模式下该函数为空操作。
  - `pub fn validate_scan_cidr(cidr: &str) -> Result<ipnet::Ipv4Net, CoreError>`：必须是 IPv4，前缀 ≥ `SCAN_MIN_PREFIX`（22），否则 `InvalidArgument("只支持 /22 及更小的 IPv4 网段")`；解析失败 `InvalidArgument("网段格式错误")`。
  - `pub async fn scan(conns: ConnManager, net: Ipv4Net, port: u16) -> ScanReport`；`pub struct ScanReport { pub probed: u32, pub found: u32 }`。并发 64，每地址先 TCP connect 超时 800ms，连得上再 `conns.connect(addr, false)`（完整 TLS + Hello）；跳过本机地址。
  - `Request::ScanSubnet { cidr }`：同步校验，通过即返回 `Response::Ok`，扫描在后台进行；完成时发 `Event::ScanFinished { cidr: String, probed: u32, found: u32 }`（C5 已定义）。同一时刻只允许一个扫描，重复请求返回 `InvalidArgument("正在扫描中")`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn scan_rejects_too_large_or_invalid_cidr() {
    assert!(validate_scan_cidr("10.0.0.0/8").is_err());
    assert!(validate_scan_cidr("10.0.0.0/21").is_err());
    assert!(validate_scan_cidr("abc").is_err());
    assert!(validate_scan_cidr("fd00::/120").is_err());
    assert_eq!(validate_scan_cidr("192.168.4.0/22").unwrap().prefix_len(), 22);
}
#[test]
fn parse_unicast_targets_expands_cidr_and_defaults_port() {
    let v = parse_unicast_targets(&["10.1.2.3".into(), "10.9.0.0/30".into()]).unwrap();
    assert_eq!(v.len(), 3); // 10.1.2.3 + 10.9.0.1、10.9.0.2
    assert!(v.iter().all(|a| a.port() == DISCOVERY_PORT));
    assert!(parse_unicast_targets(&["10.0.0.0/16".into()]).is_err());
}
#[tokio::test]
async fn scan_finds_peer_on_loopback_range() {
    // A 监听在 127.0.0.1:<p>；B 调 scan(127.0.0.0/30, p) → found == 1 且 B 收到 PeerOnline(A)
}
```
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 5: 本地文件登记与中转站主机端

**Files:**
- Create: `crates/lanmsg-core/src/blob/register.rs`（新增，P2 中没有）
- Create: `crates/lanmsg-core/src/store/stations.rs`、`crates/lanmsg-core/src/station/{mod.rs,host.rs}`
- Modify: `crates/lanmsg-core/src/blob/mod.rs`（`pub mod register;`）、`crates/lanmsg-core/src/store/blobs.rs`（`blob_visible_to`）、`service/mod.rs`（构造 `StationHost`、每 60 秒 `sweep`）、`service/handlers.rs`

**Interfaces:**
- Consumes: Task 1 类型与表；P2 `hash_file`、`file_mtime`、`mime_from_name`、`put_blob`、`set_local`、`add_holder`、`ThumbnailProvider`、`THUMB_MAX_BYTES`。
- Produces:
  - `blob/register.rs`（中转站、LocalSend、Web 共用；**P5 新增**）：
    ```rust
    pub async fn register_local_file(store: Arc<Store>, self_device: DeviceId,
        thumbnailer: Option<Arc<dyn ThumbnailProvider>>, path: PathBuf, kind: BlobLocalKind, now: i64)
        -> Result<BlobMeta, CoreError>;
    ```
    流程：`spawn_blocking(hash_file(&path))` → `name` = 文件名、`mime = mime_from_name(name)` → mime 为 `image/*`/`video/*` 且 `thumbnailer` 存在时 `spawn_blocking(thumbnail(&path, mime))`，`jpeg.len() <= THUMB_MAX_BYTES` 才采用（同时取 `media`）→ 组 `BlobMeta { hash, size, name, mime, media, thumb }` → **同一事务**内 `put_blob(&meta, &segment_hashes)`、`set_local(&BlobLocalRow { hash, path, kind, size, mtime })`、`add_holder(&hash, &self_device, now)`。路径不存在或不是普通文件 → `InvalidArgument`。
  - `store/blobs.rs` 的 `blob_visible_to` 规则扩展（修改 P2 函数，签名不变）：`user == self_user` 时，满足以下任一即为 `true`：被 `message_blobs` 引用（P2 原规则）、被 `station_blobs` 引用、或存在 `blob_local` 行。理由：自己的设备共享用户密钥，完全可信；中转站文件与手机放入中转站的文件都不在消息里。对其他用户的规则不变。
  - store（`stations.rs`）：`upsert_station(&self, &StationInfo) -> Result<()>`、`get_station(&self, &ConvId) -> Result<Option<StationInfo>>`、`list_stations(&self) -> Result<Vec<StationInfo>>`、`set_synced_until(&self, &ConvId, i64) -> Result<()>`、`synced_until(&self, &ConvId) -> Result<i64>`；`pub struct StationItemRow { pub item: StationItem, pub station: ConvId, pub store_mode: Option<StoreModeRow>, pub local_path: Option<PathBuf>, pub local_size: Option<u64>, pub local_mtime: Option<i64> }`、`pub enum StoreModeRow { Reference, Copy }`（存 `reference`/`copy`）；`upsert_station_item(&self, &StationItemRow) -> Result<()>`（同一事务内按 `item.content` 重写该条目的 `station_blobs` 行）、`station_items(&self, station: &ConvId, since: i64, include_removed: bool) -> Result<Vec<StationItemRow>>`、`get_station_item(&self, &ItemId) -> Result<Option<StationItemRow>>`、`station_blob(&self, hash: &BlobHash) -> Result<Option<(ConvId, BlobMeta, Option<String>)>>`（返回所属站点、元数据、`rel_path`；取第一条）。
  - `station/host.rs`：
    ```rust
    // RwLock / Mutex 均为 std::sync 版本，不跨 .await 持有
    pub struct StationHost { store: Arc<Store>, identity: Arc<RwLock<Identity>>, data_dir: PathBuf,
        thumbnailer: Option<Arc<dyn ThumbnailProvider>>, changes: broadcast::Sender<(ConvId, Vec<StationItem>)> }
    impl StationHost {
        pub fn new(store: Arc<Store>, identity: Arc<RwLock<Identity>>, data_dir: PathBuf,
            thumbnailer: Option<Arc<dyn ThumbnailProvider>>) -> StationHost;
        pub fn subscribe_changes(&self) -> broadcast::Receiver<(ConvId, Vec<StationItem>)>;
        pub fn create(&self, name: &str, now: i64) -> Result<StationInfo, CoreError>;
        pub async fn put_paths(&self, station: &ConvId, paths: &[PathBuf], mode: StoreModeRow, expires_at: Option<i64>, added_by: DeviceId, now: i64) -> Result<Vec<StationItem>, CoreError>;
        pub fn put_text(&self, station: &ConvId, text: &str, expires_at: Option<i64>, added_by: DeviceId, now: i64) -> Result<StationItem, CoreError>;
        pub fn put_blobs(&self, station: &ConvId, metas: Vec<BlobMeta>, expires_at: Option<i64>, added_by: DeviceId, now: i64) -> Result<Vec<StationItem>, CoreError>;
        pub fn accept_remote(&self, from: &PeerInfo, station: &ConvId, item: StationItem, now: i64) -> Result<StationItem, CoreError>;
        pub fn mark_available(&self, item: &ItemId, local_path: &Path, now: i64) -> Result<(), CoreError>;
        pub fn remove(&self, station: &ConvId, item: &ItemId, now: i64) -> Result<(), CoreError>;
        pub fn set_pinned(&self, item: &ItemId, pinned: bool, now: i64) -> Result<(), CoreError>;
        pub fn sweep(&self, now: i64) -> Result<usize, CoreError>;
    }
    ```
    - `create`：名称 trim 后为空或超过 64 字符 → `InvalidArgument`；`station = ConvId::random()`、`host = 本机设备`、`owner = 本机用户`、`acl = OwnerOnly`；`upsert_station` 并 `ensure_conversation(ConvRow { kind: ConvKindRow::Station, peer_user: None, title: Some(name) })`。
    - `put_paths`：每个路径一个条目；普通文件 → `StationContent::File`，目录 → `StationContent::Folder`（遍历规则与 P2 `send_folder` 相同：跳过符号链接，`rel_path` 用 `/`，文件数上限 `MAX_FOLDER_ENTRIES`）。`Copy` 模式先把文件复制到 `<data_dir>/station/<station hex>/<item hex>/<文件名或文件夹名>/…` 再对副本 `register_local_file(kind = Downloaded)`；`Reference` 模式对原路径 `register_local_file(kind = Source)` 并在条目行记录 `local_path`、`local_size`、`local_mtime`。站点不在本机托管 → `InvalidArgument("该中转站不在本机")`。
    - `put_text`：文字超过 64KB → `InvalidArgument`。
    - 过期时间（`put_paths`/`put_text`/`put_blobs`/`accept_remote` 共用校验）：`expires_at = Some(t)` 且 `t <= now` → `InvalidArgument("过期时间必须晚于当前时间")`；写入 `item.expires_at`。
    - `put_blobs`：每个 `BlobMeta` 一个 `StationContent::File` 条目，`available = true`、`store_mode = None`、无 `local_path`；**不复制、不下载**——内容由该 blob 已知的持有者提供（拉取端照常 `DownloadBlob`）。`metas` 为空 → `InvalidArgument`。
    - `set_pinned`：更新 `pinned` 与 `updated_at = now`，经 `changes` 发送；条目不存在 → `NotFound`。
    - `accept_remote`：`from.cert` 的 `user` 必须等于站点 `owner`，否则 `Rejected("无权访问")`；文字条目直接写入（`available = true`）；文件/文件夹条目：`mode_copy == true` 时写入 `available = false`、`store_mode = Copy`，并对每个 blob `add_holder(hash, from.device)`；`mode_copy == false`（来自 `StationPutItem::Blobs`）时写入 `available = true`、`store_mode = None`，不下载；返回写入后的条目。调用方（Task 6）仅对 `mode_copy == true` 的条目用 `TransferEngine::download_to` 拉取到 `<data_dir>/station/<station hex>/<item hex>/`，完成后调 `mark_available`。
    - `remove`：标 `removed = true`、`updated_at = now`；`Copy` 模式删除复制目录（失败只 `warn!`）。
    - `sweep`：(a) `expires_at <= now` 且未删除 → 按 `remove` 处理；(b) `Reference` 条目对 `local_path` 取元数据：不存在或 `(size, mtime)` 与记录不同 → `available = false`；重新一致 → `available = true`。返回变化条数。
    - 每次写入/变化后经 `changes` 发送 `(station, 变化条目)`。
  - API（主机侧，`handlers.rs`）：`StationCreate{name}` → `Response::ConvId { value: hex }`（Android 上见 Global Constraints 的拒绝）；`StationPut{station, item, mode, expires_at}`：站点在本机 → 按 `item` 分派 `Paths` → `put_paths`、`Text` → `put_text`、`Blobs{hashes}` → 先 `resolve_blob_metas` 再 `put_blobs`（`StoreMode::Reference/Copy` 映射 `StoreModeRow`，`Blobs` 忽略 `mode`）；否则走 Task 6 的远程 Put；`StationRemove{station, item}`、`StationSetPinned{station, item, pinned}`（本机 → `set_pinned`）同理。
  - `pub(crate) fn resolve_blob_metas(store: &Store, hashes: &[String]) -> Result<Vec<BlobMeta>, CoreError>`（放 `station/host.rs`）：每个 hex 先 `message_blobs_by_hash(h)` 取第一行的 `meta`，没有再 `station_blob(h)` 取 meta；hex 非法 → `InvalidArgument`；都找不到 → `NotFound(hash)`。

- [ ] **Step 1: 写失败测试**（`blob/register.rs`、`store/blobs.rs`、`station/host.rs` 测试模块，内存 Store + tempdir）

```rust
#[tokio::test]
async fn register_local_file_writes_blob_local_and_holder() {
    // 写一个 3 字节文件 → register_local_file(kind Source) → get_local(hash).is_some()、holders(hash) == [self]、
    // blob_segments(hash) 长度 1、meta.mime == "text/plain"（文件名 a.txt）
}
#[test]
fn own_user_can_see_locally_held_blob_but_others_cannot() {
    // set_local 一个未被任何消息引用的 hash → blob_visible_to(hash, self_user, self_user) == true；对其他用户 == false
}
#[tokio::test]
async fn put_text_and_file_then_list() { /* create("我的中转站") → put_text("hi") + put_paths([file], Reference) → station_items(since 0, false) 2 条；station_blob(file hash) 返回该站点 */ }
#[tokio::test]
async fn copy_mode_survives_original_deletion() { /* Copy 模式放入后删除原文件 → sweep 后条目仍 available，get_local 路径在 station 目录下 */ }
#[tokio::test]
async fn reference_item_marked_unavailable_when_file_changes() {
    // Reference 模式放入 → 改写原文件内容（size 变）→ sweep 返回 1，条目 available == false；
    // 删除原文件同样 available == false
}
#[tokio::test]
async fn expired_item_is_removed_by_sweep() { /* expires_at = now-1 → sweep 后 removed == true */ }
#[test]
fn accept_remote_rejects_other_users() { /* from.cert.user != owner → Err(CoreError::Rejected(_)) */ }
#[test]
fn put_with_expiry_records_expires_at() { /* put_text("x", Some(now + 86_400_000)) → station_items 中该条 expires_at == Some(now + 86_400_000) */ }
#[test]
fn put_rejects_past_expiry() { /* put_text("x", Some(now - 1)) → Err(CoreError::InvalidArgument(_)) */ }
#[test]
fn set_pinned_updates_row() { /* put_text → set_pinned(item, true, now+5) → get_station_item 的 pinned == true、updated_at == now+5；不存在的 ItemId → Err(NotFound) */ }
#[test]
fn put_blobs_creates_items_without_local_copy() {
    // 先 put_blob(meta) 模拟一条消息里已知的文件（resolve_blob_metas 经 message_blobs_by_hash 能找到）→
    // resolve_blob_metas(&[hex]) 得到 meta → put_blobs → 条目 available == true、local_path == None，<data_dir>/station 下无新文件；
    // resolve_blob_metas(&["00"*32 的 hex]) → Err(NotFound)
}
```
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-core register station`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**；并确认 P2 原有测试 `stranger_cannot_fetch_unshared_blob` 仍通过（`cargo test -p lanmsg-core --test blobs`）。
- [ ] **Step 5: Commit**

---

### Task 6: 中转站同步、拉取端与按名下载

**Files:**
- Create: `crates/lanmsg-core/src/station/client.rs`
- Modify: `crates/lanmsg-core/src/blob/transfer.rs`（新增 `download_to`）、`station/mod.rs`、`service/mod.rs`（帧路由）、`service/handlers.rs`（`StationList`/`StationItems`/远程 `StationPut`/`StationRemove`/`StationSetPinned`、`DownloadBlob`）、`service/views.rs`
- Test: `crates/lanmsg-core/tests/station.rs`

**Interfaces:**
- Consumes: Task 5；P2 `TransferEngine`、`file_item_view`、`tests/common::{node, pair, befriend}`。
- Produces:
  - `TransferEngine` 新增（**P5 新增**，写在 `blob/transfer.rs`）：
    ```rust
    pub async fn download_to(&self, hash: BlobHash, name: &str, size: u64, dest_dir: &Path) -> Result<String, CoreError>;
    ```
    把 P2 `download` 的流程拆成私有 `start(&self, hash: BlobHash, size: u64, dest_path: PathBuf) -> Result<String, CoreError>`（P2 `download` 流程第 3 步起的全部逻辑：候选源、GetMeta、工作者、复核、`set_local(Downloaded)`、`add_holder`、`BlobHave`、事件）；`download` 改为"第 1–2 步算出 name/size/dest_path 后调 `start`"，行为不变；`download_to` 直接以 `dest_dir.join(sanitize_component(name))` 为目标调 `start`。完成时仍对引用该 blob 的消息发 `MessageUpdated`（无引用则不发），另外若该 hash 出现在 `station_blobs` 中，发 `Event::StationUpdated { station }`。
  - `DownloadBlob { hash }` 处理（修改 `handlers.rs` 该分支）：`message_blobs_by_hash` 非空 → 原 `download`；为空且 `station_blob(hash)` 有值 → `download_to(hash, meta.name, meta.size, <settings.download_dir>/中转站/<sanitize(站点名)>/<rel_path 父目录各段>)`；都没有 → `NotFound`。
  - 协议行为（`station/client.rs` + Service 帧路由）：
    - 主机与**自己的设备**连接 `Up` 时发送 `StationMsg::Announce { stations: 本机托管的全部站点 }`。
    - 拉取端收到 `Announce` → `upsert_station` + `ensure_conversation(Station)` → 对每个站点发 `Subscribe { station, since: synced_until }`。
    - 主机收到 `Subscribe` → 回 `Snapshot { info, items: updated_at > since 的全部条目（含 removed）, now }`；拉取端逐条 `upsert_station_item`（`store_mode`/`local_*` 为 `None`），并对每个文件 blob `add_holder(hash, info.host)`，最后 `set_synced_until(now)`。
    - 主机 `StationHost::subscribe_changes` 收到变化 → 对当前已连接的自己设备发 `Delta`；拉取端处理同 Snapshot，`synced_until = max(条目 updated_at)`。
    - 拉取端远程 `StationPut`：`Paths` 的文件先 `register_local_file(kind = Source)`（得 `BlobMeta`）→ 发 `Put { req, item, mode_copy: true }`；`Text` 发 `Put { mode_copy: true }`（主机对文字不下载，字段无意义）；`Blobs{hashes}` → `resolve_blob_metas` → 每个 meta 一个 `Put { mode_copy: false }`。`item.added_by = 本机`、`item.expires_at = 请求的 expires_at`。每个 `Put` 等 `Result{req}` 最多 30 秒；主机未连接 → `PeerOffline`；`ok == false` → `Rejected(error)`。`StationRemove` 同理发 `Remove`，`StationSetPinned` 发 `SetPinned`。
    - `req` 由拉取端 `AtomicU64` 自增分配；等待表为 `std::sync::Mutex<HashMap<u64, oneshot::Sender<(bool, Option<String>)>>>`，取出 sender 后再 await。
    - 主机收到 `SetPinned` → 校验站点主人（同 `Put`）→ `set_pinned` → `Result`。
    - 主机收到 `Put` → `accept_remote` → 回 `Result{ok:true}` → 仅对 `mode_copy == true` 的文件条目 `download_to(hash, name, size, <data_dir>/station/<station hex>/<item hex>/)`，`TransferFinished{ok:true}` 后 `mark_available`。收到 `Remove` → `remove` → `Result`。
    - 非站点主人的设备发来的 `Subscribe`/`Put`/`Remove` → 回 `Result { req, ok: false, error: Some("无权访问") }`（`Subscribe` 无 req，用 `req: 0`），不发 Snapshot。
  - 视图（`views.rs` 新增 `pub(crate) fn station_view(svc: &Inner, s: &StationInfo) -> StationView` 与 `pub(crate) fn station_item_view(svc: &Inner, s: &StationInfo, row: &StationItemRow) -> StationItemView`）：
    - `StationList` → `Response::Stations { value }`：`host_online = host == 本机 || conns.is_connected(&host)`，`item_count` = 未删除条目数。
    - `StationItems{station}` → `Response::StationItems { value }`：未删除条目，置顶在前，其余按 `added_at` 降序；`added_by` = 设备名（查 `devices`，无则设备短指纹）；`file` = `file_item_view(svc, &s.owner, &row.item.added_by, &meta)`（Folder 取第一个 entry 的 meta、名字替换为文件夹名、大小为总和）；`available == false` → `unavailable = Some(true)`；`pinned = row.item.pinned`。
    - 中转站在 `ListConversations` 中以 `ConvKind::Station` 出现，`title` = 站点名，`last_text` = 最新条目的文字前 50 字或文件名。
  - 事件：站点或条目变化（本地或同步而来）→ `Event::StationUpdated { station: hex }`。

- [ ] **Step 1: 写失败测试**（`tests/station.rs`，`mod common;`：`h = node(&bus)` `init_first`，`p = node(&bus)`，`pair(&mut h, &mut p)`）

```rust
#[tokio::test]
async fn phone_sees_station_items_after_connect() {
    // h: StationCreate{"货架"} + StationPut(Text "a") → p 10 秒内 StationList 含该站点、StationItems 1 条
}
#[tokio::test]
async fn delta_pushes_new_items_live() { /* 已连接状态下 h 再放入一条 → p wait_event(StationUpdated)，StationItems 2 条 */ }
#[tokio::test]
async fn phone_downloads_station_file_by_hash() {
    // h 放入 1MB 文件（Reference）→ p DownloadBlob{hash} → TransferFinished{ok:true}；
    // p 下载目录 "中转站/货架/" 下文件内容一致
}
#[tokio::test]
async fn phone_put_file_is_pulled_by_host() {
    // p: StationPut(Paths[小文件], Copy) → h 的条目 30 秒内 unavailable 不为 Some(true)，且 h 的 data_dir/station/.. 中存在内容相同的文件
}
#[tokio::test]
async fn phone_pins_item_remotely() { /* h 放入 3 条文字 → p StationSetPinned(最早一条, true) → Ok；p 10 秒内 StationItems 第一条为该条且 pinned == true */ }
#[tokio::test]
async fn phone_puts_known_blob_without_upload() {
    // h 与 p 的"我的设备"会话里 h 先 SendFiles 一个 1MB 文件（p 已同步到该消息，未下载）；
    // p: StationPut{item: Blobs{[hash]}, mode: Reference, expires_at: None} → Ok；
    // h 的 StationItems 出现该文件，unavailable 为 None，且 h 侧无新的 TransferProgress（没有回传）
}
#[tokio::test]
async fn host_offline_marks_items_source_offline() { /* h.svc.shutdown() → p 的 StationItems 中文件条目 file.state == SourceOffline，StationView.host_online == false */ }
#[tokio::test]
async fn contact_cannot_subscribe() { /* c = node(&bus) init_first，befriend(&mut h, &mut c)；c 侧直接构造并发送 Subscribe 帧（经 ConnManager 测试入口）→ 收到 Result{ok:false}，c 无该站点 */ }
```
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-core --test station`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**；P2 `tests/blobs.rs` 全部仍通过（`download` 行为未变）。
- [ ] **Step 5: Commit**

---

### Task 7: 桌面端中转站页面与"发送到"

**Files:**
- Create: `crates/lanmsg-desktop/src/views/stations.rs`、`crates/lanmsg-desktop/src/station_put.rs`
- Modify: `crates/lanmsg-desktop/src/model.rs`、`src/bridge.rs`（`ReplyTag` 追加变体）、`src/views/{mod.rs,shell.rs,files.rs}`、`src/strings.rs`、`src/fixtures.rs`、`src/main.rs`、`tests/screenshots.rs`

**Interfaces:**
- Consumes: C5 `StationList`/`StationItems`/`StationCreate`/`StationPut`/`StationRemove`/`StationSetPinned`/`DownloadBlob`、`Event::StationUpdated`；P3 `Model`/`Effect`/`UiState`/`views::message_row::file_state_area`/`Bridge`。
- Produces:
  - `bridge.rs`：`ReplyTag` 追加 `Stations`、`StationItems { station: String }`。
  - `model.rs`：`Page` 追加 `Stations`；`Model` 追加字段 `pub stations: Vec<StationView>`、`pub current_station: Option<String>`、`pub station_items: Vec<StationItemView>`、`pub station_mode: StoreMode`（默认 `Reference`）、`pub station_expiry: ExpiryChoice`（默认 `Never`）；新增类型 `pub enum ExpiryChoice { Never, Day1, Day7, Day30 }`，方法 `pub fn expires_at(&self, now_millis: i64) -> Option<i64>`（Never → None，其余 → `Some(now + 天数 × 86_400_000)`）；新增方法 `pub fn station_set_pinned(&mut self, item: String, pinned: bool) -> Vec<Effect>`、`pub fn put_blobs_to_station(&mut self, station: String, hashes: Vec<String>) -> Vec<Effect>`（`StationPut{station, item: Blobs{hashes}, mode: Reference, expires_at: None}`）；以及 `pub fn open_station(&mut self, station: String) -> Vec<Effect>`（设 `current_station`，`Call(StationItems{station}, StationItems{station})`）、`pub fn station_put(&mut self, paths: Vec<PathBuf>, now_millis: i64) -> Vec<Effect>`、`pub fn station_put_text(&mut self, text: String, now_millis: i64) -> Vec<Effect>`、`pub fn station_remove(&mut self, item: String) -> Vec<Effect>`。`station_put`/`station_put_text` 的 `expires_at = station_expiry.expires_at(当前毫秒)`（`now_millis` 由视图层传入当前时间，便于测试）。规则：`set_page(Stations)` → `Call(StationList, Stations)`；`Stations` 回复 → 替换 `stations`，`current_station` 为空且列表非空时自动 `open_station(第一个)`；`StationItems{station}` 回复且等于 `current_station` → 替换 `station_items`；`Event::StationUpdated{station}` → `Call(StationList)`，且等于 `current_station` 时再 `Call(StationItems)`；`Connected`/`Resync` 时若 `page == Stations` 同 `set_page(Stations)`。
  - `views/stations.rs`：`pub fn stations_page(ui: &UiState) -> Element`：
    - 左列站点列表（名称、主机设备名、在线点、条目数）；底部按钮 `strings::STATION_NEW = "新建中转站"`（点击弹输入框，确认 → `StationCreate`）。
    - 右侧：顶栏（站点名、主机状态 `STATION_HOST_ONLINE = "主机在线"` / `STATION_HOST_OFFLINE = "主机离线，仅可查看清单"`、按钮"添加文件""添加文件夹""添加文字"、存储模式下拉 `"引用原文件（默认）"` / `"复制到中转站"`，写入 `station_mode`；保留时间下拉 `STATION_EXPIRY = "保留时间"`：`"不过期"` / `"1 天"` / `"7 天"` / `"30 天"`，写入 `station_expiry`，作用于之后的添加）；条目列表：缩略图或类型图标、名称（文字条目前 80 字）、大小、`added_by` + 时间、状态区（文件条目直接复用 `file_state_area(ui, item)`；`unavailable == Some(true)` 时改为灰字 `STATION_UNAVAILABLE = "已失效（原文件被移动或修改）"`）、操作（文字条目"复制"；`STATION_PIN = "置顶"` / `STATION_UNPIN = "取消置顶"` → `station_set_pinned`；删除，删除前二次确认）；置顶条目名称前显示 📌；有 `expires_at` 的条目在时间后追加 `"· {M月D日} 过期"`。
    - 右侧区域挂 `Element::on_drop_files` → `station_put`。
    - 空状态：无站点 `STATION_EMPTY = "还没有中转站。在常开的电脑上新建一个，手机就能随时来取文件。"`；有站点无条目 `STATION_ITEMS_EMPTY = "把文件拖到这里，或点击上方按钮添加。"`。
  - `views/shell.rs`：左侧图标栏在"文件"之后加"中转站"入口（→ `set_page(Page::Stations)`）。
  - `views/files.rs`（P3 文件视图）：右键菜单追加 `FILES_PUT_STATION = "放入中转站"`：`stations` 为空时先 `Call(StationList, Stations)` 再展开；子菜单列出全部站点名，点击 → `put_blobs_to_station(station, vec![item.hash])`；没有任何站点时菜单项置灰并提示 `"请先在"中转站"页新建"`。
  - `fixtures.rs`：`DemoPage` 追加 `Stations`（2 个站点，其中 1 个主机离线；5 个条目：文字、已下载图片、未下载视频、源离线文件、已失效文件）；`main.rs` 的 `--demo` 接受 `stations`；`tests/screenshots.rs` 的页面数组追加 `"stations"`。
  - `station_put.rs`：`pub fn run_station_put(data_dir: &Path, paths: Vec<PathBuf>) -> anyhow::Result<()>`：不创建窗口、不注册单实例；单线程 tokio 运行时内 `IpcClient::connect(endpoint_name(data_dir))`（连接失败 → 按 P3 `Bridge` 同样方式拉起 `lanmsgd` 后重试 5 秒）→ `StationList` 取 `host_device == 本机 device_id` 的第一个站点，没有则 `StationCreate{"我的中转站"}` → `StationPut{station, Paths{paths}, Reference}`；任一步失败返回错误（`main` 打印到日志 `<data_dir>/logs/lanmsg.log` 并以退出码 1 结束）。`main.rs`：参数 `--station-put <PATH>...` 存在时在一切窗口逻辑之前调用它并退出。供 Windows "发送到"快捷方式（Task 17/19）使用。
- [ ] **Step 1: 写失败测试**（`model.rs` 测试模块）

```rust
#[test] fn set_page_stations_requests_list() { /* set_page(Stations) 的 Effects 含 Call(StationList, ReplyTag::Stations) */ }
#[test] fn stations_reply_auto_opens_first() { /* 回复 2 个站点 → current_station == 第一个，Effects 含 StationItems 请求 */ }
#[test] fn station_updated_refreshes_current_items_only() { /* StationUpdated(当前) → 2 个 Call；StationUpdated(其他) → 只有 StationList */ }
#[test] fn station_put_uses_selected_mode() { /* station_mode = Copy → Call(StationPut{.., mode: Copy}) */ }
#[test] fn expiry_choice_sets_expires_at() { /* station_expiry = Day7，station_put_text("a", 1_000) → Call 中 expires_at == Some(1_000 + 7*86_400_000)；Never → None */ }
#[test] fn pin_emits_set_pinned() { /* station_set_pinned("i1", true) → Call(StationSetPinned{station: 当前, item: "i1", pinned: true}) */ }
#[test] fn put_blobs_to_station_uses_blobs_item() { /* → Call(StationPut{station: "s1", item: Blobs{hashes: ["h"]}, mode: Reference, expires_at: None}) */ }
```
  以及 `main.rs` 参数解析测试 `station_put_arg_parses_paths`（`--station-put a b` → 两个路径）。
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-desktop`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**（含 `every_demo_page_renders`）；`cargo run -p lanmsg-desktop -- --demo stations --screenshot /tmp/stations.png`，目视检查后把路径写入任务报告。
- [ ] **Step 5: Commit**

---

### Task 8: Android 中转站页面

**Files:**
- Create: `android/app/src/main/java/name/apk/lanmsg/ui/station/{StationScreen.kt,StationViewModel.kt}`
- Modify: `ui/files/{FilesScreen.kt,FilesViewModel.kt}`（"放入中转站"动作）、`ui/nav/Routes.kt`（`const val STATION = "station/{conv}"`、`fun station(conv: String) = "station/${Uri.encode(conv)}"`）、`ui/nav/LanMsgNavHost.kt`（注册该路由）、`ui/convs/ConversationListScreen.kt`（`ConvKind.STATION` 的会话点击 → `Routes.station(conv)`，其余仍 `Routes.chat`）、`res/values/strings.xml`、`docs/ui-architecture.md`（§2 加路由 `station/{conv}`，§4 加页面状态，§6 加文案键）
- Test: `android/app/src/test/java/name/apk/lanmsg/ui/station/StationViewModelTest.kt`、`android/app/src/test/java/name/apk/lanmsg/ui/files/FilesViewModelTest.kt`（追加用例）

**Interfaces:**
- Consumes: `CoreApi`、`call<T>`、`FakeCoreApi`、`Fixtures`、`MainDispatcherRule`；P4 `files/OutboxImporter`（选择文件后落地为路径）、`ui/chat/FileCard.kt`（文件条目状态区复用）。
- Produces:
  ```kotlin
  class StationViewModel(core: CoreApi, station: String, importer: Importer, clock: () -> Long = System::currentTimeMillis) : ViewModel() {
      enum class Expiry(val days: Int?) { NEVER(null), DAY1(1), DAY7(7), DAY30(30) }
      data class UiState(val name: String = "", val hostOnline: Boolean = false,
                         val items: List<StationItemView> = emptyList(), val expiry: Expiry = Expiry.NEVER,
                         val busy: Boolean = false, val error: String? = null)
      val state: StateFlow<UiState>
      fun refresh()                          // StationList（取本站点）+ StationItems
      fun putUris(uris: List<Uri>)            // importer.import → StationPut(Paths, StoreMode.COPY, expiresAt)
      fun putText(text: String)               // StationPut(Text, StoreMode.COPY, expiresAt)
      fun setExpiry(e: Expiry)                // 作用于之后的放入；expiresAt = days?.let { clock() + it * 86_400_000L }
      fun setPinned(item: String, pinned: Boolean)  // StationSetPinned
      fun download(hash: String)              // DownloadBlob
      fun remove(item: String)                // StationRemove
  }
  ```
  - 订阅 `core.events`：`Event.StationUpdated` 且 `station` 相同 → `refresh()`；`core.lagged` → `refresh()`。
  - 页面：顶栏（站点名 + 主机状态）；主机离线横幅 `station_host_offline_banner = "主机离线，暂时无法下载"`；条目列表（文件条目用 `FileCard` 的状态区：未下载点"下载"、已下载点"打开"（P4 `FileOpener`）；文字条目点击复制到剪贴板并 Toast `toast_copied = "已复制"`；`unavailable == true` 显示 `station_item_unavailable = "已失效"`）；条目长按菜单 `station_pin = "置顶"` / `station_unpin = "取消置顶"` / `station_remove = "删除"`；置顶条目显示图钉图标；顶栏菜单 `station_expiry = "保留时间"`（`station_expiry_never = "不过期"`、`station_expiry_1d = "1 天"`、`station_expiry_7d = "7 天"`、`station_expiry_30d = "30 天"`，当前选中打勾）；右下角 FAB `station_add`（菜单：`station_add_file = "选择文件"` / `station_add_text = "输入文字"`）；下拉刷新；空状态 `station_items_empty = "中转站里还没有东西"`。手机端放入一律 `StoreMode.COPY`。
- [ ] **Step 1: 写失败测试**

```kotlin
class StationViewModelTest {
    @get:Rule val main = MainDispatcherRule()
    @Test fun loads_items_and_host_state() = runTest {
        // fake.handler：StationList → Response.Stations(listOf(stationView("s1", hostOnline = false)))；StationItems → 2 条
        // vm = StationViewModel(fake, "s1", fakeImporter) → state.items.size == 2，hostOnline == false
    }
    @Test fun station_updated_event_triggers_refresh() = runTest { /* emit(Event.StationUpdated("s1")) → fake.requests 中 StationItems 出现第二次 */ }
    @Test fun put_text_uses_copy_mode() = runTest { /* putText("hi") → 最后一个请求为 StationPut(station="s1", item=Text("hi"), mode=COPY, expiresAt=null) */ }
    @Test fun expiry_applies_to_put() = runTest { /* clock = { 1000L }；setExpiry(DAY1)；putText("x") → expiresAt == 1000 + 86_400_000 */ }
    @Test fun set_pinned_sends_request() = runTest { /* setPinned("i1", true) → 请求 StationSetPinned(station="s1", item="i1", pinned=true) */ }
}
// FilesViewModelTest 追加
@Test fun put_to_station_sends_blobs_item() = runTest { /* vm.putToStation("s1", "h1") → 请求 StationPut(station="s1", item=Blobs(listOf("h1")), mode=REFERENCE, expiresAt=null) */ }
```
  （`stationView(..)`、`stationItem(..)` 追加到 `testutil/Fixtures.kt`。）
  - `FilesViewModel` 新增 `val stations: StateFlow<List<StationView>>`（首次调用 `loadStations()` 时 `StationList`）与 `fun putToStation(station: String, hash: String)`；`FilesScreen` 文件项长按菜单追加 `files_put_station = "放入中转站"` → 弹出站点单选对话框（无站点时显示 `files_no_station = "还没有中转站，请先在电脑上新建"`），选中后 `putToStation`，成功 Toast `toast_put_station = "已放入中转站"`。
- [ ] **Step 2: 运行确认失败** — `cd android && ./gradlew :app:testDebugUnitTest --tests '*StationViewModelTest*'`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 9: `lanmsg-localsend` 类型与发现

**Files:**
- Create: `crates/lanmsg-localsend/{Cargo.toml,src/lib.rs,src/types.rs,src/discovery.rs,src/cert.rs}`
- Create: `crates/lanmsg-localsend/tests/fixtures/{announce.json,register.json,prepare_upload_request.json,prepare_upload_response.json,text_message_request.json}`

**协议事实**（已对照官方 `github.com/localsend/protocol` README，页面标注为 v2.2，示例中 `version` 字段为 `"2.0"`）：

- 组播 UDP `224.0.0.167:53317`（`LOCALSEND_MCAST` / `LOCALSEND_PORT`）。
- 通告 JSON 字段：`alias`（string）、`version`（string，`"2.0"`）、`deviceModel`（string|null）、`deviceType`（`"mobile"|"desktop"|"web"|"headless"|"server"`|null，未知值按 `desktop`）、`fingerprint`（string）、`port`（number）、`protocol`（`"http"|"https"`）、`download`（bool，可缺省，默认 false）、`announce`（bool）。
- 收到 `announce: true` 的通告：优先向对方 `POST {protocol}://{ip}:{port}/api/localsend/v2/register`，body 为自己的信息（同字段，无 `announce`），响应体为对方信息；失败则组播回一份 `announce: false` 的通告。
- 指纹：HTTPS 模式为证书的 SHA-256；HTTP 模式为随机字符串。HTTPS 模式下请求体里的 `fingerprint` 字段被忽略（双方仍发送）。
- REST（前缀 `/api/localsend/v2`）：`POST /register`；`POST /prepare-upload?pin=`；`POST /upload?sessionId=&fileId=&token=`（body 为文件原始字节）；`POST /cancel?sessionId=`；`POST /prepare-download`、`GET /download`（v1 不实现这两个——LanMsg 不提供"下载模式"）。
- `prepare-upload` 请求：`{"info": {…自己的信息，无 announce…}, "files": {"<fileId>": {"id","fileName","size","fileType","sha256"?,"preview"?,"metadata"?: {"modified"?,"accessed"?}}}}`；响应 200：`{"sessionId": "...", "files": {"<fileId>": "<token>"}}`；204 = 无需传输（文字消息就是这样结束的）；400 无效 body；401 需要/错误 PIN；403 拒绝；409 被其他会话占用；429 过多请求；500 未知。
- `upload`：400 缺参数；403 token 或 IP 不符；409 被占用；422 sha256 不符；500。
- 文字消息：作为 `fileType: "text/plain"` 的单个"文件"，`preview` 字段即文字内容；接收方显示后返回 **204**，不再上传。（实现前用真实 LocalSend App 抓一次包确认，记录到 `text_message_request.json` 夹具。）

**Interfaces:**
- Produces（`lanmsg_localsend`）：

```rust
pub enum LsProtocol { Http, Https }                         // serde 小写
pub struct LsInfo { pub alias: String, pub version: String, pub device_model: Option<String>,
    pub device_type: Option<String>, pub fingerprint: String, pub port: u16,
    pub protocol: LsProtocol, pub download: bool }          // serde rename camelCase
pub struct LsAnnounce { #[serde(flatten)] pub info: LsInfo, pub announce: bool }
pub struct LsFileMeta { pub id: String, pub file_name: String, pub size: u64, pub file_type: String,
    pub sha256: Option<String>, pub preview: Option<String>, pub metadata: Option<LsFileTimes> }
pub struct LsFileTimes { pub modified: Option<String>, pub accessed: Option<String> }
pub struct PrepareUploadRequest { pub info: LsInfo, pub files: BTreeMap<String, LsFileMeta> }
pub struct PrepareUploadResponse { pub session_id: String, pub files: BTreeMap<String, String> }
pub struct LsPeer { pub info: LsInfo, pub ip: IpAddr }
pub enum LsError { Io(std::io::Error), Http(String), Status(u16), Json(String), Tls(String), PortBusy }
```
  - JSON 字段名严格为 camelCase（`fileName`、`fileType`、`deviceModel`、`deviceType`、`sessionId`）；`download` 缺省反序列化为 `false`；可空字段为 `null` 或缺省都要能解析。
  - `cert.rs`：`pub struct LsCert { pub cert_der: Vec<u8>, pub key_der: Vec<u8> }`；`pub fn load_or_create(dir: &Path) -> Result<LsCert, LsError>`（文件 `localsend.crt.der` / `localsend.key.der`，rcgen 自签 ECDSA P-256，CN `LanMsg`，有效期 100 年）；`pub fn fingerprint(cert_der: &[u8]) -> String` = SHA-256 的**大写** hex（与官方实现一致性在手工互操作时核对；因 HTTPS 模式对方忽略该字段，不一致也不影响互通，只影响对方"记住设备"）。
  - `discovery.rs`：`pub struct LsDiscovery`；`pub fn spawn(me: LsInfo, announce_enabled: bool, events: mpsc::Sender<LsPeerEvent>) -> Result<LsDiscovery, LsError>`；`pub enum LsPeerEvent { Seen(LsPeer), }`；绑定 UDP `0.0.0.0:53317`（`SO_REUSEADDR`，Unix 加 `SO_REUSEPORT`）并加入组播；`announce_enabled == false` 时**只收不发、也不调用 register**；收到自身指纹的通告忽略；`announce_enabled` 时启动即通告一次，此后每 30 秒一次。`pub async fn register_reply(client: &reqwest::Client, me: &LsInfo, peer: &LsPeer) -> Result<LsInfo, LsError>`。

依赖：`cargo add -p lanmsg-localsend lanmsg-proto --path crates/lanmsg-proto`、`tokio@1 --features full`、`serde@1 --features derive`、`serde_json@1`、`thiserror@2`、`tracing@0.1`、`socket2@0.5 --features all`、`rcgen@0.13`、`sha2@0.10`、`reqwest@0.12 --no-default-features --features rustls-tls,json,stream`、`axum@0.7`、`axum-server@0.7 --features tls-rustls`（已登记于路线图依赖清单）、`uuid@1 --features v4`、`mime_guess@2`；dev：`tempfile@3`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn parses_official_announce_fixture() {
    let a: LsAnnounce = serde_json::from_str(include_str!("../tests/fixtures/announce.json")).unwrap();
    assert_eq!(a.info.version, "2.0"); assert!(a.announce);
    assert_eq!(a.info.protocol, LsProtocol::Https);
}
#[test]
fn download_field_defaults_false_and_nulls_parse() {
    let j = r#"{"alias":"x","version":"2.0","deviceModel":null,"deviceType":null,"fingerprint":"f","port":53317,"protocol":"http","announce":true}"#;
    let a: LsAnnounce = serde_json::from_str(j).unwrap();
    assert!(!a.info.download); assert_eq!(a.info.device_type, None);
}
#[test]
fn prepare_upload_serializes_camel_case() {
    // 构造 PrepareUploadRequest 序列化后的字符串包含 "fileName"、"fileType"，不含 "file_name"
}
#[test]
fn fingerprint_is_uppercase_sha256_hex() { assert_eq!(fingerprint(b"abc").len(), 64); /* 且全为 0-9A-F */ }
```
  夹具文件按上面"协议事实"手写（字段取官方示例值：alias `"Nice Orange"`、deviceModel `"Samsung"`、deviceType `"mobile"`、fingerprint `"random string"`、port 53317、protocol `"https"`、download true、announce true）。
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-localsend`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 10: LocalSend 接收服务与发送客户端

**Files:**
- Create: `crates/lanmsg-localsend/src/server.rs`、`crates/lanmsg-localsend/src/client.rs`
- Modify: `crates/lanmsg-localsend/src/lib.rs`
- Test: `crates/lanmsg-localsend/tests/loopback.rs`

**Interfaces:**
- Consumes: Task 9。
- Produces:

```rust
pub struct LsConfig { pub alias: String, pub device_model: String, pub device_type: String,
    pub download_dir: PathBuf, pub cert_dir: PathBuf, pub port: u16 }  // port 生产为 LOCALSEND_PORT，测试可为 0
pub enum LsDecision { Accept, Reject }
pub enum LsEvent {
    PeerSeen(LsPeer),
    IncomingFiles { session: String, peer: LsPeer, files: Vec<LsFileMeta>, reply: oneshot::Sender<LsDecision> },
    TextReceived { peer: LsPeer, text: String, reply: oneshot::Sender<LsDecision> },
    FileReceived { session: String, peer: LsPeer, file: LsFileMeta, path: PathBuf },
    Progress { session: String, file_id: String, done: u64, total: u64 },
    SessionFinished { session: String, ok: bool, error: Option<String> },
}
pub struct LocalSend;   // Clone，内部 Arc
impl LocalSend {
    pub async fn start(cfg: LsConfig, events: mpsc::Sender<LsEvent>) -> Result<LocalSend, LsError>;
    pub fn listening(&self) -> bool;          // TCP 端口绑定成功
    pub fn info(&self) -> LsInfo;
    pub fn peers(&self) -> Vec<LsPeer>;
    pub async fn send_text(&self, peer: &LsPeer, text: &str) -> Result<(), LsError>;
    pub async fn send_files(&self, peer: &LsPeer, paths: &[PathBuf],
        progress: mpsc::Sender<(String /*file name*/, u64, u64)>) -> Result<(), LsError>;
    pub async fn shutdown(&self);
}
```
  - 启动：先尝试绑定 TCP `0.0.0.0:port`（HTTPS，证书来自 `cert::load_or_create`）。失败 → `listening = false`，`LsDiscovery` 以 `announce_enabled = false` 启动（只发现不通告），`tracing::warn!("53317 已被占用（可能运行着 LocalSend），LocalSend 兼容只能发送")`。成功 → `announce_enabled = true`。
  - 服务端路由：
    - `POST /api/localsend/v2/register` → 记录对端（IP 取 TCP 对端地址）→ 200 + 自己的 `LsInfo`。
    - `POST /api/localsend/v2/prepare-upload` → body 上限 1MB；有 `pin` 参数也忽略（v1 不设 PIN）；同一时刻已有进行中的会话 → 409；文件全部为 `text/plain` 且只有一个、带 `preview` → 发 `TextReceived`，等待 `reply` 最多 60 秒：`Accept` → 204，`Reject`/超时 → 403；其他 → 发 `IncomingFiles`，等待同上：`Accept` → 200 + `sessionId`（uuid v4）+ 每文件随机 token，`Reject`/超时 → 403。
    - `POST /api/localsend/v2/upload` → 校验 sessionId、fileId、token 与来源 IP（不符 403，缺参 400）；流式写 `<download_dir>/<fileName>.part`（文件名做清理：去掉路径分隔符与 `..`，重名追加 ` (1)`、` (2)`…）；若请求提供 `sha256` 则校验，不符 → 删除 `.part` 并 422；完成后改名、发 `FileReceived`；全部文件完成 → `SessionFinished { ok: true }`。
    - `POST /api/localsend/v2/cancel` → 删除该会话的 `.part`，`SessionFinished { ok: false, error: Some("对方取消") }`。
  - 客户端：`reqwest::Client` 关闭证书校验（`danger_accept_invalid_certs(true)`，仅本 crate 内使用，并注释"LocalSend 使用自签证书 + TOFU"），超时：连接 5 秒；`prepare-upload` 120 秒（等待对方点接收）。`send_text` = 一个 `text/plain` 文件、`size` 为 UTF-8 字节数、`preview` 为文字、`fileName` 为 `"message.txt"`；期望 204（200 时把文字作为文件内容上传）。`send_files` 依次（并发 2）上传；401 → `LsError::Status(401)`（界面提示"对方开启了 PIN，暂不支持"），403 → `Status(403)`。

- [ ] **Step 1: 写失败测试**（`tests/loopback.rs`：两个 `LocalSend` 实例都绑定 `port: 0`，通过 `peers` 之外的方式构造 `LsPeer { ip: 127.0.0.1, info.port: 对方实际端口 }`）

```rust
#[tokio::test]
async fn send_text_is_accepted_with_204() { /* B 收到 TextReceived 回 Accept → A.send_text Ok；B 文本一致 */ }
#[tokio::test]
async fn send_file_roundtrip_and_sanitized_name() { /* 文件名 "../evil.txt" → B 下载目录中出现 "evil.txt"，内容一致，目录外无文件 */ }
#[tokio::test]
async fn reject_returns_403() { /* B 回 Reject → A 得到 Err(Status(403)) */ }
#[tokio::test]
async fn upload_with_wrong_token_is_403() { /* 直接用 reqwest 调 /upload 带错误 token → 403 */ }
#[tokio::test]
async fn port_busy_disables_announce_but_keeps_sending() {
    // 先占用一个 TCP 端口，再用该端口启动 LocalSend → listening()==false；它仍能 send_text 给另一个实例
}
```
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 11: core 的 LocalSend 桥

**Files:**
- Create: `crates/lanmsg-core/src/store/localsend.rs`、`crates/lanmsg-core/src/localsend_bridge.rs`
- Modify: `service/mod.rs`、`service/handlers.rs`（`SendText`/`SendFiles` 对 LocalSend 会话分流；`Request::RespondRequest` 与 `Request::ListRequests` 两个分支加 `ls-` 前缀分流）、`service/views.rs`（伪设备作者名）
- Test: `crates/lanmsg-core/tests/localsend_bridge.rs`、`crates/lanmsg-core/tests/sync.rs`（追加回归测试）

**Interfaces:**
- Consumes: Task 10；Task 5 `register_local_file`；P2 `Messaging::{send, send_files}`、`Store::{next_seqs, insert_envelope, set_message_state}`、`settings::load_settings`、`SyncEngine::shared_convs`。
- Produces:
  - store：`pub struct LsPeerRow { pub fingerprint: String, pub pseudo_device: DeviceId, pub alias: String, pub device_model: Option<String>, pub device_type: Option<String>, pub last_addr: Option<String>, pub protocol: String, pub port: u16, pub last_seen: i64, pub trusted: bool, pub blocked: bool }`；`upsert_ls_peer`、`get_ls_peer(&str)`、`ls_peer_by_pseudo(&DeviceId)`、`set_ls_trust(&str, trusted: bool, blocked: bool)`。
  - 伪设备 ID：`DeviceId(blake3("lanmsg-ls-dev" || fingerprint))`——只用作消息作者标识，永远不会出现在 TLS 连接中。
  - `pub struct LocalSendBridge`：`pub async fn start(store: Arc<Store>, identity: Arc<RwLock<Identity>>, clock: Arc<Mutex<HlcClock>>, events: broadcast::Sender<Event>, thumbnailer: Option<Arc<dyn ThumbnailProvider>>, cfg: LsConfig) -> Result<LocalSendBridge, CoreError>`、`pub async fn stop(&self)`、`pub fn pending_requests(&self) -> Vec<RequestView>`、`pub async fn respond(&self, request: &str, action: RequestAction) -> Result<(), CoreError>`、`pub async fn send_text(&self, conv: &ConvId, text: &str) -> Result<(), CoreError>`、`pub async fn send_paths(&self, conv: &ConvId, paths: &[PathBuf]) -> Result<(), CoreError>`、`pub fn peer_online(&self, conv: &ConvId) -> bool`。Service 在"已初始化且 `settings.localsend_enabled`"时启动；`SetSettings` 改变该开关时启动/停止。`alias = 显示名`，`device_model = "LanMsg"`，`device_type` = 本机为手机 `"mobile"`、否则 `"desktop"`，`download_dir = <设置的下载目录>/LanMsg/LocalSend`，`cert_dir = <data_dir>/keys`。
  - 事件映射：
    - `PeerSeen(p)`：`p.ip` 经 `classify` 不是 `Lan` → 忽略；否则 `upsert_ls_peer`，`ensure_conversation { conv: ConvId::localsend(&fp), kind: LocalSend, title: alias }`，发 `PeerOnline(PeerView { device_id: 伪设备 hex, user_id: "", display_name: alias, device_type: deviceType 或 "desktop", path: Lan, is_contact: trusted, is_mine: false, .. })`。LocalSend 对端 90 秒未再出现 → `PeerOffline`。
    - `TextReceived`/`IncomingFiles`：`blocked` → 立即 `Reject`；`trusted` → `Accept`；否则生成 `RequestView { request: "ls-<uuid>", kind: Message 或 File, from_user: "", from_name: alias, from_short: 指纹前 8 位, preview: 文字前 100 字或"N 个文件，共 X MB" }`，发 `IncomingRequest`，把 `reply` 存入 `pending: HashMap<String, oneshot::Sender<LsDecision>>`。
    - `respond(request, action)`：`AcceptOnce` → Accept；`AddContact` → Accept + `trusted = true`；`Reject` → Reject；`Block` → Reject + `blocked = true`；信任变化时发 `ContactsChanged`；请求不存在（已超时）→ `NotFound`。
    - 分流（修改 `service/handlers.rs`）：`Request::RespondRequest { request, action }` 分支开头加 `if request.starts_with("ls-") { return bridge.respond(&request, action) 的结果 }`，其余保持 P2 原逻辑（`get_request` → 按 kind 调 `Trust::respond_contact` / `Trust::respond_message_request`）；`Request::ListRequests` 分支把 `bridge.pending_requests()` 追加到 P2 结果末尾。桥未启动时两处都按无 LocalSend 请求处理（`ls-` 请求返回 `NotFound`）。
    - 文字被接受 → 在 LocalSend 会话中插入本地消息：`Envelope { conv, id: MsgId { author: 伪设备, seq }, conv_seq, hlc: clock.now(墙钟), body: Text, enc: None, sig: 全零 }`（`(seq, conv_seq) = store.next_seqs(&伪设备, &conv)`），`store.insert_envelope(&env, MsgStateRow::Delivered, now)`，新插入则发 `MessageAdded` + `ConversationUpdated`。LocalSend 会话的消息不验签、不同步：P2 `SyncEngine::shared_convs` 只返回 `SelfDevices`/`Dm` 会话、`conv_member_ok` 对其他 kind 返回 false，已满足；本任务在 `crates/lanmsg-core/tests/sync.rs` 追加回归测试 `localsend_conv_is_never_synced` 固定这一行为，**不修改** `sync.rs` 代码。
    - `FileReceived` → `register_local_file(store, 本机设备, thumbnailer, path, BlobLocalKind::Downloaded, now)` 得到 `BlobMeta` → 同上插入 `MsgBody::Files { items: vec![meta] }`。
  - 发送（修改 `handlers.rs` 的 `SendText`/`SendFiles` 分支）：`conv` 为 `ConvKindRow::LocalSend` 时先检查 `bridge.peer_online(&conv)`，否则 `PeerOffline`；然后照常调用 P2 `Messaging::send`/`Messaging::send_files` 生成本机签名的消息（LocalSend 会话没有投递目标，状态停在 `Sending`），拿到 `MsgId` 后在后台调用 `bridge.send_text`/`bridge.send_paths`：成功 → `set_message_state(Delivered)`，失败 → `Failed`，各发一次 `MessageUpdated`；`Status(401)` 失败时另插入一条本地 `MsgBody::System` 可见消息（伪设备作者、文字 "对方开启了 PIN，暂不支持"）。`SendFolder` 到 LocalSend 会话 → `InvalidArgument("LocalSend 不支持发送文件夹")`。
  - 视图：消息作者设备在 `devices` 表中不存在但能在 `localsend_peers` 中按伪设备找到 → `author_device_name = alias`、`is_mine = false`。

- [ ] **Step 1: 写失败测试**（`tests/localsend_bridge.rs`：一个 Service（开启 LocalSend，端口 0 通过测试专用配置注入）+ 一个独立 `lanmsg_localsend::LocalSend` 作为"LocalSend App"）

```rust
#[tokio::test]
async fn incoming_text_from_stranger_becomes_request_then_message() {
    // App.send_text("hi") → Service 发 IncomingRequest(kind Message)；RespondRequest(AcceptOnce) → App 得到 Ok；
    // Service 的 LocalSend 会话出现文字 "hi"，作者名为 App 的 alias
}
#[tokio::test]
async fn add_contact_makes_next_transfer_automatic() { /* AddContact 后第二次 send_text 不再产生 IncomingRequest */ }
#[tokio::test]
async fn blocked_peer_is_rejected_silently() { /* Block 后 App.send_text → Err(Status(403))，无事件 */ }
#[tokio::test]
async fn send_files_to_localsend_conv_marks_delivered() { /* App 侧自动 Accept；Service SendFiles → MessageUpdated(Delivered)，App 下载目录有文件 */ }
#[tokio::test]
async fn ls_request_appears_in_list_requests() { /* 待处理时 ListRequests 含 request 以 "ls-" 开头的项 */ }
// tests/sync.rs 追加
#[tokio::test]
async fn localsend_conv_is_never_synced() {
    // a、b 已 pair；a 的 Store 中直接插入一个 LocalSend 会话与一条伪设备消息；断开重连触发同步 → b 的 ListConversations 不含该会话
}
```
  为测试注入端口：`LocalSendBridge` 从 `ServiceConfig.localsend_port`（C6 已定义；生产为 `LOCALSEND_PORT`，测试为 0）读取。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 12: Web 会话密码学（Rust 侧 + 测试向量）

**Files:**
- Create: `crates/lanmsg-core/src/web/{mod.rs,crypto.rs}`
- Create: `web/test/vectors.json`（由本任务的测试生成并提交）

**算法（Rust 与 JS 必须逐字节一致）**：

- 记号：`b64u` = base64url 无填充；`||` = 字节拼接；字符串均为 UTF-8。
- 两种模式：`key`（二维码 / 链接，URL 片段 `#k=<b64u(32 字节)>`）与 `code`（手动输入 8 位码）。
- 宿主每次 `WebStart` 生成：`url_key`（32 随机字节）、`code`（8 字符，字母表 `23456789ABCDEFGHJKLMNPQRSTUVWXYZ`，即 32 个字符、每字符 5 比特）、`scrypt_salt`（16 随机字节）。
- 认证密钥 `auth`：
  - key 模式：`auth = url_key`；
  - code 模式：`auth = scrypt(password = code（大写）, salt = scrypt_salt, N = 16384, r = 8, p = 1, dkLen = 32)`。
- 握手：页面生成 X25519 临时密钥对 `(page_sk, page_pk)` 与 `page_nonce`（16 字节）；宿主生成 `(host_sk, host_pk)` 与 `host_nonce`（16 字节）及 `session`（32 位小写 hex，随机 16 字节）。
- `transcript = "lanmsg-web-v1" || mode_byte(key=0x01, code=0x02) || page_pk || host_pk || page_nonce || host_nonce || session(ASCII)`；`th = SHA-512(transcript)`。
- `shared = X25519(己方私钥, 对方公钥)`（32 字节）。
- `okm = HKDF-SHA512(ikm = shared, salt = auth, info = th, L = 96)`；`k_page = okm[0..32]`，`k_host = okm[32..64]`，`k_enc = okm[64..96]`。
- `page_mac = HMAC-SHA512(k_page, "page" || th)[0..32]`；`host_mac = HMAC-SHA512(k_host, "host" || th)[0..32]`；比较用常数时间。
- 封包：`seal(k_enc, plaintext) = nonce(24 随机字节) || XSalsa20-Poly1305(k_enc, nonce, plaintext)`（即 NaCl `secretbox`）；`open` 反之，长度 < 24+16 或认证失败 → 错误。

**Interfaces:**
- Produces（`crate::web::crypto`）：
  - `pub enum WebMode { Key, Code }`
  - `pub struct WebSecrets { pub url_key: [u8; 32], pub code: String, pub scrypt_salt: [u8; 16] }`；`WebSecrets::generate() -> WebSecrets`；`pub fn auth_key(&self, mode: WebMode) -> [u8; 32]`（code 模式的 scrypt 结果在 `generate` 时预计算并缓存）
  - `pub struct HostHandshake { pub session: String, pub host_pk: [u8; 32], pub host_nonce: [u8; 16], keys: SessionKeys }`
  - `pub fn host_respond(mode: WebMode, auth: &[u8; 32], page_pk: &[u8; 32], page_nonce: &[u8; 16]) -> HostHandshake`
  - `pub struct SessionKeys { pub k_page: [u8; 32], pub k_host: [u8; 32], pub k_enc: [u8; 32], pub th: [u8; 64] }`
  - `pub fn derive(mode: WebMode, auth: &[u8;32], my_sk: &[u8;32], their_pk: &[u8;32], page_pk: &[u8;32], host_pk: &[u8;32], page_nonce: &[u8;16], host_nonce: &[u8;16], session: &str) -> SessionKeys`（供测试向量与 `host_respond` 共用）
  - `impl SessionKeys { pub fn verify_page_mac(&self, mac: &[u8]) -> bool; pub fn host_mac(&self) -> [u8; 32]; pub fn seal(&self, pt: &[u8]) -> Vec<u8>; pub fn open(&self, ct: &[u8]) -> Result<Vec<u8>, CoreError> }`
  - `pub struct CodeGuard { failures: u32 }`：`pub fn record_failure(&mut self) -> bool`（返回 `true` 表示已达 5 次、调用方必须轮换 code 与 scrypt_salt 并清零）。

依赖：`cargo add -p lanmsg-core x25519-dalek@2 --features static_secrets`、`crypto_secretbox@0.1`、`hkdf@0.12`、`hmac@0.12`、`sha2@0.10`、`scrypt@0.11 --no-default-features`、`base64@0.22`、`serde_json@1`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn page_and_host_derive_same_keys() { /* 两侧各用自己的私钥调用 derive，三把密钥与 th 均相等 */ }
#[test]
fn wrong_code_fails_page_mac() { /* 页面用错误 code 推导 → host.verify_page_mac(page_mac) == false */ }
#[test]
fn seal_open_roundtrip_and_tamper_detected() { /* open(seal(x)) == x；翻转一个字节 → Err */ }
#[test]
fn code_alphabet_and_length() { /* generate() 100 次，code 长度 8 且字符均在字母表中 */ }
#[test]
fn code_mode_rotates_after_five_failures() { /* CodeGuard 前 4 次 false，第 5 次 true */ }
#[test]
fn write_test_vectors() {
    // 用固定私钥/随机数/口令（全部写死在测试中）计算 key 与 code 两种模式的 auth、th、k_page、k_host、k_enc、
    // page_mac、host_mac，以及对固定明文 "你好 LanMsg" + 固定 nonce 的 seal 结果，
    // 以 b64u 写入 ../../web/test/vectors.json（存在且内容相同则不改写；内容不同则测试失败并提示重新生成）
}
```
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**（为了生成固定 nonce 的向量，`seal` 内部调用 `seal_with_nonce(pt, nonce)`，后者 `pub(crate)`）
- [ ] **Step 4: 运行确认通过**，确认生成了 `web/test/vectors.json`
- [ ] **Step 5: Commit**（含 `web/test/vectors.json`）

---

### Task 13: Web 页面与 `lanmsg-web-assets`

**Files:**
- Create: `web/{index.html,style.css,app.js,crypto.js}`、`web/vendor/{nacl-fast.min.js,scrypt.js,LICENSES.txt}`、`web/test/crypto.test.mjs`
- Create: `crates/lanmsg-web-assets/{Cargo.toml,build.rs,src/lib.rs}`

**Interfaces:**
- Consumes: Task 12 算法与 `vectors.json`；Task 14 的 HTTP / WebSocket 协议（下方定义，本任务写客户端，Task 14 写服务端）。
- Produces:
  - vendored 文件来源（下载后在 `LICENSES.txt` 记录 URL、版本、许可证与 SHA-256）：
    - `https://cdn.jsdelivr.net/npm/tweetnacl@1.0.3/nacl-fast.min.js`（Unlicense）
    - `https://cdn.jsdelivr.net/npm/scrypt-js@3.0.1/scrypt.js`（MIT）
  - `web/crypto.js`：普通脚本，定义全局 `window.LanMsgCrypto`，同时在 `typeof module !== "undefined"` 时 `module.exports = LanMsgCrypto`（供 Node 测试）。导出：`b64uEncode(u8)`、`b64uDecode(str)`、`sha512(u8)`（`nacl.hash`）、`hmacSha512(key, msg)`、`hkdfSha512(ikm, salt, info, len)`（RFC 5869）、`scryptCode(code, salt) -> Promise<Uint8Array>`、`newHandshake() -> {sk, pk, nonce}`、`derive(mode, auth, mySk, theirPk, pagePk, hostPk, pageNonce, hostNonce, session) -> {kPage, kHost, kEnc, th}`、`pageMac(keys)`、`checkHostMac(keys, mac)`、`seal(kEnc, u8)`、`open(kEnc, u8) -> Uint8Array | null`。与 Task 12 算法逐字节一致。
  - `web/test/crypto.test.mjs`：用 Node（≥18，`node:test` + `createRequire` 加载 vendor 与 `crypto.js`）读取 `vectors.json`，对两种模式逐项比对 `auth`（code 模式经 scrypt）、`th`、三把密钥、两个 MAC、固定 nonce 的密文（`crypto.js` 暴露 `sealWithNonce` 仅供测试），全部相等即通过。
  - 页面行为（`app.js`）：
    1. 读取 `location.hash` 中的 `k`：有 → key 模式；无 → 显示"输入宿主屏幕上的 8 位配对码"输入框（自动转大写、去空格）→ code 模式（`POST /api/web/hello` 返回的 `scrypt_salt` 用于计算 auth）。
    2. 握手：`hello` → 计算 → `confirm` → 显示"等待对方允许…" → 长轮询 `wait` 直到 `allowed`（校验 `host_mac`，失败显示"连接校验失败，请重新扫码"并停止）或 `denied`（显示"对方拒绝了连接"）。
    3. 允许后打开 WebSocket，显示聊天界面：顶栏（宿主名、连接状态点）、消息流（文字气泡、文件卡片：名称 / 大小 / 进度 / "下载"按钮、图片下载完成后显示内联预览）、底部输入框 + 发送按钮 + "选择文件"按钮（`multiple`）；整页支持拖拽文件上传。
    4. 上传：每个文件先 `upload/init`，然后按 `chunk_size` 顺序（并发 3）`PUT` 缺失分块，失败分块重试 3 次；全部完成后 `finish`。WebSocket 断开 → 自动重连（1s、2s、4s… 上限 15s），重连后对未完成上传重新 `init`（同 `client_key`）只补缺块。
    5. 下载：并发 3 拉取分块、解密、存入数组，完成后 `new Blob(parts, {type: mime})` + `URL.createObjectURL` + 隐藏 `<a download>` 触发保存；文件 > 1GB 时先 `confirm("文件较大，浏览器可能占用大量内存，继续吗？")`。
    6. 页面关闭或 `{"t":"bye"}` → 显示"会话已结束"。
    7. 移动端适配：视口宽度 < 600px 时单列布局，按钮最小 44px。
  - `index.html` 以 `<script src="vendor/nacl-fast.min.js">`、`vendor/scrypt.js`、`crypto.js`、`app.js` 顺序加载；加 `<meta http-equiv="Content-Security-Policy" content="default-src 'self'; connect-src 'self' ws: ; img-src 'self' blob: data:; style-src 'self'">`；不引用任何外部 URL。
  - `lanmsg-web-assets`：`build.rs` 遍历仓库根 `web/` 目录（排除 `web/test/`），生成 `OUT_DIR/assets.rs`：`static ASSETS: &[(&str, &[u8], &str)]`（路径、`include_bytes!` 内容、MIME），并设置 `cargo:rerun-if-changed=../../web`。`src/lib.rs`：`pub fn get(path: &str) -> Option<(&'static [u8], &'static str)>`（`""` 或 `"/"` 映射 `index.html`；路径含 `..` 返回 `None`）、`pub fn total_size() -> usize`。

- [ ] **Step 1: 写失败测试**

```rust
// crates/lanmsg-web-assets/src/lib.rs
#[test] fn root_maps_to_index() { assert_eq!(get("/").unwrap().1, "text/html; charset=utf-8"); }
#[test] fn traversal_rejected() { assert!(get("/../Cargo.toml").is_none()); }
#[test] fn test_dir_not_embedded() { assert!(get("/test/vectors.json").is_none()); }
#[test] fn page_under_300kb() { assert!(total_size() < 300 * 1024, "web/ 超出 300KB：{}", total_size()); }
```
  以及 `node --test web/test/crypto.test.mjs`（先写测试，crypto.js 未实现时失败）。
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-web-assets`；`node --test web/test/crypto.test.mjs`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**（两条命令都通过）；CI 工作流 `ci.yml` 增加 `actions/setup-node`（Node 20）与 `node --test web/test/` 步骤（仅 ubuntu）。
- [ ] **Step 5: Commit**

---

### Task 14: Web 服务端、会话与传输

**Files:**
- Create: `crates/lanmsg-core/src/web/{server.rs,session.rs,transfer.rs}`
- Modify: `web/mod.rs`、`service/handlers.rs`（`WebStart`/`WebStop`/`WebRespond`；`SendText`/`SendFiles` 对 `ConvKind::Guest` 分流）、`service/views.rs`（访客伪设备作者名）
- Test: `crates/lanmsg-core/tests/web.rs`

**Interfaces:**
- Consumes: Task 12、13；`lanmsg_web_assets::get`；Task 5 `register_local_file`；P2 `Messaging::{send, send_files}`、`Store::{next_seqs, insert_envelope}`、`TransferEngine::unique_path`、`sanitize_component`。
- Produces（HTTP 协议，JSON 字段名固定）：

| 方法 路径 | 请求 | 响应 |
|---|---|---|
| `GET /`、`GET /<静态文件>` | — | `lanmsg_web_assets::get`；不存在 404；响应头 `Cache-Control: no-store` |
| `POST /api/web/hello` | 明文 JSON `{"mode":"key"\|"code","page_pk":b64u,"page_nonce":b64u}` | `{"session":hex,"host_pk":b64u,"host_nonce":b64u,"host_name":str,"scrypt_salt":b64u}` |
| `POST /api/web/confirm` | `{"session":hex,"page_mac":b64u}` | 成功 `{"status":"pending"}`；MAC 错 403 `{"error":"bad_mac"}` |
| `GET /api/web/wait?session=` | — | 长轮询 ≤ 25 秒：`{"status":"pending"}` / `{"status":"allowed","host_mac":b64u}` / `{"status":"denied"}` |
| `GET /api/web/ws?session=`（WebSocket） | 二进制帧，每帧 = `seal(JSON)` | 同左 |
| `POST /api/web/upload/init` | 头 `X-LanMsg-Session`；body = `seal({"client_key":str,"name":str,"size":u64,"mime":str})` | `seal({"fid":hex,"chunk_size":1048576,"have":[u32…]})` |
| `PUT /api/web/upload/{fid}/{index}` | 同头；body = `seal(分块明文)` | 204；分块序号越界 400；解密失败 400 |
| `POST /api/web/upload/{fid}/finish` | 同头 | `seal({"msg":str})`；缺块 409 `seal({"missing":[u32…]})` |
| `GET /api/web/download/{fid}/{index}` | 同头 | `seal(分块明文)`，`Content-Type: application/octet-stream` |

  - WebSocket JSON 消息（封包内）：
    - 宿主 → 页面：`{"t":"welcome","host_name":str}`；`{"t":"msg","id":str,"from":"host"|"page","kind":"text","text":str,"ts":i64}`；`{"t":"msg","id":str,"from":…,"kind":"file","file":{"fid":hex,"name":str,"size":u64,"mime":str,"chunks":u32},"ts":i64}`；`{"t":"ack","cid":str,"id":str}`；`{"t":"bye","reason":str}`。连接（含重连）建立后先发 `welcome`，再按时间顺序重放本会话全部消息。
    - 页面 → 宿主：`{"t":"text","cid":str,"text":str}`（≤ 64KB）；`{"t":"ping"}`（每 20 秒，宿主不回复，只刷新活动时间）。
  - 会话规则（`session.rs`）：
    - `pub struct WebHost`：`pub async fn start(…) -> Result<WebSessionInfo, CoreError>`、`pub async fn stop(&self)`、`pub fn respond(&self, session: &str, allow: bool) -> Result<(), CoreError>`、`pub async fn send_text(&self, conv: &ConvId, text: &str) -> Result<(), CoreError>`、`pub async fn send_files(&self, conv: &ConvId, paths: &[PathBuf]) -> Result<(), CoreError>`。
    - 监听 `0.0.0.0:WEB_PORT`，被占用则端口 0；每个请求先取 TCP 对端地址做 `classify`，非 `Lan` → 403。
    - `WebStart` → `Response::WebSession { value: WebSessionInfo }`；`WebSessionInfo`：`url = "http://<ip>:<port>/#k=<b64u(url_key)>"`，`ip` = 第一个 `tunnel == None` 的私有 IPv4 网卡地址（没有则第一个非回环 IPv4）；`qr_uri = url`；`code = secrets.code`；另附 `urls = Some(所有候选网卡生成的 URL 列表)`。
    - 限制：未完成握手的会话最多 5 个，30 秒过期；单 IP 每分钟 `hello` ≤ 10 次（超出 429）；code 模式 `confirm` 失败交给 `CodeGuard`，达到 5 次 → 轮换 `code` 与 `scrypt_salt`，发 `Event::WebSessionChanged { info: Some(新的 WebSessionInfo) }`（界面刷新显示的码）；`WebStop` 或超时结束时发 `WebSessionChanged { info: None }`。
    - `confirm` 成功 → 发 `Event::WebRequest { request: session, user_agent, ip }`；`WebRespond { request, allow }`：允许 → 创建访客会话：`conv = ConvId::random()`，`ensure_conversation(Guest, title = "访客 · MM-DD HH:mm")`，`web_guests` 记一行，伪设备 = `DeviceId(blake3("lanmsg-guest" || conv))`；拒绝 → `wait` 返回 `denied`。
    - 已允许会话 `WEB_IDLE_TIMEOUT` 内无任何请求 / ping → 结束：WebSocket 发 `bye`，删除该会话的全部 `.part`，`web_guests.ended_at = now`。`WebStop` → 结束全部会话并停止监听；`url_key`、`code` 作废。同一 `WebStart` 下可有多个并发访客会话。
    - 页面发来的文字 / 完成的上传 → 在访客会话中以伪设备作者插入本地消息（签名全零、`Delivered`，做法同 Task 11），发 `MessageAdded`；上传文件落在 `<settings.download_dir>/<sanitize_component(会话标题)>/`（同名用 `unique_path`），然后 `register_local_file(.., BlobLocalKind::Downloaded, ..)` 并以 `MsgBody::Files` 插入。Guest 会话不同步：P2 `SyncEngine::shared_convs` 只返回 `SelfDevices`/`Dm`，已满足；本任务在 `crates/lanmsg-core/tests/sync.rs` 追加回归测试 `guest_conv_is_never_synced`，**不修改** `sync.rs`。
    - 宿主 `SendText`/`SendFiles` 到访客会话（修改 `handlers.rs` 这两个分支：`ConvKindRow::Guest` 时先查 `WebHost` 中该会话是否仍活动，否则 `PeerOffline`）→ 调 P2 `Messaging::send`/`send_files` 生成本机签名消息 → 后台调用 `WebHost::send_text`/`send_files` 推送 WS `msg`（文件分配 `fid`，按 1MB 分块供下载）→ 推送成功置 `Delivered` 并发 `MessageUpdated`。`SendFolder` 到访客会话 → `InvalidArgument("网页传输不支持文件夹")`。
  - `transfer.rs`：`pub struct UploadState { pub fid: String, pub path_part: PathBuf, pub size: u64, pub chunk_size: u32, pub have: BitVec 或 Vec<bool> }`；分块写入用 `seek + write_all`；`finish` 校验全部到齐后 `rename`（同名追加 ` (1)`）。

- [ ] **Step 1: 写失败测试**（`tests/web.rs`：Service 调 `WebStart`；测试内用 `reqwest` + `tokio-tungstenite`（**dev 依赖新增**）扮演页面，密码学调用 Task 12 的 `derive` 作页面侧）

```rust
#[tokio::test] async fn key_mode_handshake_and_allow() { /* hello→confirm→WebRequest 事件→WebRespond(allow)→wait 返回 allowed 且 host_mac 校验通过 */ }
#[tokio::test] async fn deny_returns_denied() {}
#[tokio::test] async fn hello_rate_limited_per_ip() { /* 11 次 hello，第 11 次 429 */ }
#[tokio::test] async fn text_both_directions_over_ws() { /* 页面 text → 宿主访客会话出现消息；宿主 SendText → 页面收到 msg */ }
#[tokio::test] async fn upload_resumes_missing_chunks_only() {
    // 3MB 文件：传 chunk 0、2 后断开 WS；同 client_key 再 init → have == [0,2]；补 1 → finish 成功，内容一致
}
#[tokio::test] async fn host_file_downloadable_by_chunks() { /* 宿主 SendFiles → 页面收到 file 消息 → 按 chunks 下载解密拼接与原文件一致 */ }
#[tokio::test] async fn idle_timeout_cleans_partial_uploads() {
    // 通过测试专用的 WebHost 超时参数（Duration::from_secs(1)）验证：超时后 .part 被删除、WS 收到 bye
}
#[tokio::test] async fn requests_without_session_header_are_403() {}
// tests/sync.rs 追加
#[tokio::test] async fn guest_conv_is_never_synced() { /* a、b 已 pair；a 有一个访客会话与消息；重连同步后 b 的 ListConversations 不含它 */ }
```
  为测试可调超时，`WebHost::start` 接收 `idle_timeout: Duration` 参数（生产传 `WEB_IDLE_TIMEOUT`）。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**；手工：手机浏览器扫码打开，与桌面互传一张图片和一个 100MB 文件，记录到任务报告。
- [ ] **Step 5: Commit**

---

### Task 15: 桌面端网页传输与网络设置

**Files:**
- Create: `crates/lanmsg-desktop/src/views/web_transfer.rs`、`crates/lanmsg-desktop/src/views/network.rs`
- Modify: `crates/lanmsg-desktop/src/model.rs`、`src/bridge.rs`（`ReplyTag` 追加）、`src/views/{mod.rs,shell.rs,settings.rs}`、`src/strings.rs`、`src/fixtures.rs`、`src/main.rs`（`--demo` 追加 `web`）、`tests/screenshots.rs`；`crates/lanmsgd/src/tray.rs`（菜单项"开启网页传输"）

**Interfaces:**
- Consumes: `WebStart`/`WebStop`/`WebRespond`、`Event::WebRequest`/`WebSessionChanged`；`AddManualPeer`、`ScanSubnet`、`ListInterfaces`、`GetSettings`/`SetSettings`、`Event::ScanFinished`；P3 `views::pair::qr_rgba`、`Element::image_rgba`、`views/settings.rs` 的"网络"分组（已有 LocalSend 开关与单播通告目标多行输入）。
- Produces:
  - `bridge.rs`：`ReplyTag` 追加 `Web`、`Interfaces`、`ManualPeer`。
  - `model.rs`：
    - `pub struct WebUi { pub info: WebSessionInfo, pub url_index: usize, pub pending: Vec<WebRequestUi> }`、`pub struct WebRequestUi { pub request: String, pub user_agent: String, pub ip: String, pub received_at_millis: i64 }`
    - `Model` 追加字段 `pub web: Option<WebUi>`、`pub web_open: bool`（网页传输窗口是否显示）、`pub interfaces: Vec<InterfaceView>`、`pub scanning: Option<String>`（正在扫描的 CIDR）、`pub scan_result: Option<String>`、`pub manual_result: Option<String>`、`pub firewall_missing: bool`。
    - 方法：`pub fn web_start(&mut self) -> Vec<Effect>`（`web_open = true`；`web` 为空时 `Call(WebStart, Web)`）、`pub fn web_stop(&mut self) -> Vec<Effect>`（`Call(WebStop)`，`web = None`，`web_open = false`）、`pub fn web_respond(&mut self, request: &str, allow: bool) -> Vec<Effect>`（从 `pending` 移除并 `Call(WebRespond)`）、`pub fn web_expire(&mut self, now_millis: i64) -> Vec<Effect>`（`pending` 中超过 60 秒的项按拒绝处理）、`pub fn add_manual_peer(&mut self, addr: String) -> Vec<Effect>`、`pub fn scan(&mut self, cidr: String) -> Vec<Effect>`（`scanning` 非空时忽略）、`pub fn load_interfaces(&mut self) -> Vec<Effect>`。
    - 规则：`Web` 回复 → `web = Some(WebUi { info, url_index: 0, pending: [] })`；`Event::WebSessionChanged{info}` → `Some(i)` 时替换 `web.info`（`web` 为 None 时新建 `WebUi`），`None` 时 `web = None`、`web_open = false`；`Event::WebRequest` → 追加到 `pending`（按 request 去重，`web_open = true`）；`Event::ScanFinished{cidr, probed, found}` → `scanning = None`、`scan_result = Some(format!("已探测 {probed} 个地址，发现 {found} 台设备"))`；`ManualPeer` 回复 Ok → `manual_result = Some("已连接")`，Err → `Some(e.message)`；`Event::FirewallMissing` → `firewall_missing = true`（Task 18 使用）。
  - 纯函数（`views/web_transfer.rs`，带单测）：`pub fn group_code(code: &str) -> String`（8 字符 → `"XXXX XXXX"`，其他长度原样）、`pub fn ua_summary(ua: &str) -> String`（含 `Edg/` → "Edge"、`Chrome/` → "Chrome"、`Firefox/` → "Firefox"、`Safari/` 且不含 `Chrome/` → "Safari"，否则前 40 字符；并在含 `Android`/`iPhone`/`Windows`/`Mac OS X`/`Linux` 时追加 " · " 与对应系统名）。`views/network.rs`：`pub fn path_badge_text(p: &PathClassDto) -> Option<&'static str>`（`Lan` → None、`Vpn` → "VPN"、`Internet` → "公网"、`Relay` → "中继"）、`pub fn scan_cidr_for(iface: &InterfaceView) -> String`（前缀 ≥ 22 用网卡自身网段，否则取本机地址所在 /24）。
  - `views/web_transfer.rs`：`pub fn web_transfer_window(ui: &UiState) -> Element`（`web_open` 为真时由 `shell` 以模态面板叠加显示）：二维码（`qr_rgba(当前 url, 220)` → `Element::image_rgba`）、URL 文本 + "复制"按钮（`urls` 有多个时下拉切换 `url_index`）、配对码大字 `group_code(code)` 与说明 `WEB_CODE_HINT = "无法扫码时，在浏览器打开地址后输入此配对码"`、已连接访客列表（`convs` 中 `ConvKind::Guest` 的会话，行内"打开会话" → `open_conv`）、按钮 `WEB_STOP = "结束网页传输"`、`WEB_HIDE = "隐藏"`（只把 `web_open` 置 false，不停止）。`pending` 非空时在面板顶部显示第一条：`"有浏览器请求连接\nIP：{ip}\n浏览器：{ua_summary}"` + "允许" / "拒绝"；`shell` 每秒调用一次 `web_expire`（windui 定时器）。
  - `views/network.rs`：`pub fn network_section(ui: &UiState) -> Element`，插入 `views/settings.rs` 的"网络"分组（保留 P3 已有的 LocalSend 开关与单播通告目标）：
    - "手动添加设备"：输入框（占位 `192.168.2.10 或 host:53320`）+ "连接"按钮 → `add_manual_peer`；下方显示 `manual_result`。
    - "网段扫描"：进入设置页时 `load_interfaces`；每个网卡一行：名称、`addr/prefix`、隧道角标（`tunnel_kind` → Tailscale / ZeroTier / WireGuard / 隧道）、IPv4 网卡带"扫描此网段"按钮（`scan(scan_cidr_for(iface))`）；另有手输 CIDR 输入框 + "扫描"；扫描中显示转圈与 `scanning`，结束显示 `scan_result`。
    - LocalSend 开关下：`settings.localsend_listening == Some(false)` 时显示 `LOCALSEND_PORT_BUSY = "53317 端口被占用（可能正在运行 LocalSend），只能向 LocalSend 设备发送"`。
  - 路径角标：P3 联系人页、设备页、`ListPeers` 列表行在名字后追加 `path_badge_text` 的小标签（`Element::badge`）。
  - `lanmsgd/src/tray.rs` 的 `run_tray`：菜单在"打开 LanMsg"之后加 `开启网页传输`，点击 → 以参数 `--web` 调用 P3 `launch.rs` 的启动逻辑（新增 `pub fn open_gui_with(data_dir: &Path, extra_args: &[&str]) -> std::io::Result<()>`，`open_gui` 改为调用它）。界面 `main.rs` 新增参数 `--web`：首次 `Connected` 后执行 `web_start()`；界面已在运行时，单实例转发来的 argv 含 `--web` 同样执行 `web_start()`。
  - `fixtures.rs`：`DemoPage` 追加 `Web`（`web = Some(..)`，含 2 个 URL、1 个待允许请求、1 个访客会话）；`tests/screenshots.rs` 页面数组追加 `"web"`。
- [ ] **Step 1: 写失败测试**（`model.rs`、`views/web_transfer.rs`、`views/network.rs` 测试模块）

```rust
#[test] fn group_code_4_4() { assert_eq!(group_code("K7QM3XAP"), "K7QM 3XAP"); assert_eq!(group_code("abc"), "abc"); }
#[test] fn ua_summary_cases() { /* Android Chrome UA → "Chrome · Android"；iPhone Safari UA → "Safari · iPhone" */ }
#[test] fn path_badge_text_cases() { /* Lan None；Vpn "VPN"；Internet "公网"；Relay "中继" */ }
#[test] fn scan_cidr_for_small_and_large_prefix() { /* 192.168.1.10/24 → "192.168.1.0/24"；10.1.2.3/8 → "10.1.2.0/24"；10.1.4.5/22 → "10.1.4.0/22" */ }
#[test] fn web_request_event_queues_and_expires() {
    // apply(Event(WebRequest{..})) → pending.len()==1、web_open；web_expire(now + 61_000) → pending 为空，Effects 含 WebRespond{allow:false}
}
#[test] fn scan_finished_sets_result_and_clears_scanning() {}
#[test] fn web_arg_starts_web_on_connect() { /* main 参数 --web 解析为 true（参数解析纯函数测试） */ }
```
- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-desktop`
- [ ] **Step 3: 实现**（二维码复用 P3 `views::pair::qr_rgba`，不新增依赖）
- [ ] **Step 4: 运行确认通过**（含 `every_demo_page_renders`）；`--demo web` 与 `--demo settings` 截图，路径写入任务报告。
- [ ] **Step 5: Commit**

---

### Task 16: Android 网页传输与网络设置

**Files:**
- Create: `android/app/src/main/java/name/apk/lanmsg/ui/web/{WebHostScreen.kt,WebHostViewModel.kt}`、`android/app/src/main/java/name/apk/lanmsg/ui/settings/{NetworkSettingsScreen.kt,NetworkSettingsViewModel.kt}`
- Modify: `ui/nav/Routes.kt`（`const val WEB_HOST = "web/host"`、`const val NETWORK = "settings/network"`）、`ui/nav/LanMsgNavHost.kt`、`ui/me/MeScreen.kt`（入口"网页传输"）、`ui/settings/SettingsScreen.kt`（入口"网络"）、`service/BackgroundPolicy.kt`、`service/DeviceConditions.kt`、`service/LanMsgService.kt`（常驻通知文字）、`res/values/strings.xml`、`docs/ui-architecture.md`（§2 路由、§4 状态、§6 文案）
- Test: `ui/web/WebHostViewModelTest.kt`、`ui/settings/NetworkSettingsViewModelTest.kt`、`service/BackgroundPolicyTest.kt`（追加用例）

**Interfaces:**
- Consumes: `CoreApi`、`call<T>`、`FakeCoreApi`、`Fixtures`、`MainDispatcherRule`；P4 `QrImage`、`Conditions`、`BackgroundPolicy.shouldRunService`、`DeviceConditions`、`ServiceLocator.coreHolder`。
- Produces:
  ```kotlin
  class WebHostViewModel(core: CoreApi, setWebHosting: (Boolean) -> Unit) : ViewModel() {
      data class PendingRequest(val request: String, val userAgent: String, val ip: String, val receivedAt: Long)
      data class UiState(val info: WebSessionInfo? = null, val urlIndex: Int = 0,
                         val guests: List<ConversationSummary> = emptyList(), val pending: PendingRequest? = null,
                         val error: String? = null)
      val state: StateFlow<UiState>
      fun start()                 // setWebHosting(true)；WebStart → info
      fun stop()                  // WebStop；setWebHosting(false)；info = null
      fun respond(allow: Boolean) // WebRespond(pending.request, allow)；pending = null
      fun selectUrl(i: Int)
  }
  class NetworkSettingsViewModel(core: CoreApi) : ViewModel() {
      data class UiState(val interfaces: List<InterfaceView> = emptyList(), val scanning: String? = null,
                         val scanResult: String? = null, val manualResult: String? = null,
                         val settings: SettingsDto? = null, val error: String? = null)
      fun load(); fun addManualPeer(addr: String); fun scan(cidr: String)
      fun setUnicastTargets(lines: List<String>); fun setLocalSendEnabled(v: Boolean)
  }
  ```
  - 事件：`WebHostViewModel` 订阅 `Event.WebRequest`（设 `pending`，60 秒后自动 `respond(false)`）、`Event.WebSessionChanged`（`info` 非空替换，为空清空页面状态并返回上一页）、`Event.ConversationUpdated`（`kind == GUEST` 时刷新 `guests`）；`NetworkSettingsViewModel` 订阅 `Event.ScanFinished`。
  - 后台：`Conditions` 追加字段 `val webHosting: Boolean = false`；`BackgroundPolicy.shouldRunService` 在 `webHosting == true` 时恒为 true（优先于档位，包括 `OFF`）；`DeviceConditions` 新增 `fun setWebHosting(on: Boolean)`（更新 `conditions` StateFlow）；`WebHostViewModel` 的工厂把 `setWebHosting` 绑定到 `ServiceLocator` 中的 `DeviceConditions.setWebHosting`。`LanMsgService` 在 `webHosting` 时常驻通知文字用 `notif_service_web_text = "网页传输中"`，否则 P4 原 `notif_service_text`。
  - 页面：网页传输页元素同 Task 15（二维码 `QrImage(url)`、URL + 复制、配对码分组显示、访客列表点击 → `Routes.chat(conv)`、"结束网页传输"）；`pending` 非空弹 `AlertDialog`（`web_request_title = "有浏览器请求连接"`，正文 IP 与浏览器摘要，按钮 `web_allow = "允许"` / `web_deny = "拒绝"`）。网络设置页元素同 Task 15 的 `network_section`（含 `localsend_port_busy` 提示）。
  - 文案键（写入 `strings.xml` 与 `docs/ui-architecture.md` §6）：`web_title`、`web_code_hint`、`web_stop`、`web_request_title`、`web_allow`、`web_deny`、`web_guests`、`notif_service_web_text`、`net_title`、`net_manual_add`、`net_manual_hint`、`net_connect`、`net_unicast_targets`、`net_scan`、`net_scan_this`、`net_scan_result`（`已探测 %1$d 个地址，发现 %2$d 台设备`）、`localsend_port_busy`；中文取值与 Task 15 文案一致。
- [ ] **Step 1: 写失败测试**

```kotlin
class WebHostViewModelTest {
    @get:Rule val main = MainDispatcherRule()
    @Test fun start_sets_hosting_and_info() = runTest {
        // fake.handler：WebStart → Response.WebSession(webSessionInfo())；var hosting = false
        // vm.start() → hosting == true，state.info != null
    }
    @Test fun web_request_then_allow() = runTest { /* emit(Event.WebRequest("r1","UA","192.168.1.5")) → pending != null；respond(true) → 最后请求 WebRespond("r1", true)，pending == null */ }
    @Test fun stop_clears_hosting() = runTest { /* stop() → hosting == false，请求含 WebStop */ }
}
class NetworkSettingsViewModelTest {
    @Test fun scan_finished_sets_result() = runTest { /* scan("192.168.1.0/24") → scanning 非空；emit(ScanFinished(..,254,2)) → scanning == null，scanResult 非空 */ }
    @Test fun manual_peer_error_shown() = runTest { /* AddManualPeer 抛 ApiException(INVALID_ARGUMENT,"无法解析地址") → manualResult == "无法解析地址" */ }
}
// BackgroundPolicyTest 追加
@Test fun web_hosting_forces_service_even_when_off() { /* shouldRunService(OFF, Conditions(..., webHosting = true)) == true */ }
```
  （`webSessionInfo()` 追加到 `testutil/Fixtures.kt`。）
- [ ] **Step 2: 运行确认失败** — `cd android && ./gradlew :app:testDebugUnitTest`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 17: wind-installer 新能力：`[[firewall]]` 与快捷方式 `args` / `sendto`

**仓库：** `/home/dufeng/develop/windinput/wind-installer`（先读其 `AGENTS.md`、`CONTRIBUTING.md`，按「新增一种能力的标准套路」执行；开工前 `git status --short --branch` 记录快照，提交只用 `--only` 本任务路径）

**Files:**
- Modify: `src/manifest.rs`、`src/installer/steps.rs`、`src/installer/plan.rs`、`src/installer/receipt.rs`、`src/installer/shortcut.rs`、`README.md`（能力段表格）、`app.toml`（示例补两段说明）
- Create: `src/installer/firewall.rs`
- Test: `tests/step_plan.rs`、`tests/manifest_parse.rs`、`tests/receipt_roundtrip.rs`

**Interfaces:**
- Produces（清单格式）：

```toml
[[firewall]]
name      = "LanMsg"            # 规则显示名（必填）
program   = "lanmsgd.exe"       # 相对安装目录（必填）
protocol  = "any"               # any / tcp / udp，缺省 any
profiles  = ["private", "domain"]   # 缺省 ["private", "domain"]；可含 "public"

[[shortcut]]
target   = "lanmsg.exe"
name     = "LanMsg 中转站"
location = "sendto"             # 新值：%APPDATA%\Microsoft\Windows\SendTo
args     = "--station-put"      # 新字段：快捷方式参数，缺省空
```
  - `manifest.rs`：`pub struct FirewallInfo { pub name: String, pub program: String, #[serde(default)] pub protocol: FirewallProtocol, #[serde(default = "default_profiles")] pub profiles: Vec<String> }`；`AppManifest` 加 `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub firewall: Vec<FirewallInfo>`；`ShortcutLocation` 加 `SendTo`（serde 名 `"sendto"`）；`ShortcutInfo` 加 `#[serde(default)] pub args: String`。
  - `firewall.rs`：`pub fn add_rule(rule: &FirewallInfo, install_dir: &Path) -> io::Result<()>`：执行 `netsh advfirewall firewall add rule name="<name>" dir=in action=allow program="<install_dir>\<program>" enable=yes profile=<profiles 逗号连接> protocol=<any|tcp|udp>`（同名同程序规则先 `delete` 再 `add`，保证幂等）；`pub fn delete_rule(name: &str, program_abs: &str) -> io::Result<()>`：`netsh advfirewall firewall delete rule name="<name>" program="<program_abs>"`。命令以 `CREATE_NO_WINDOW` 运行，非零退出码转为错误并带上输出。
  - `receipt.rs`：`ReceiptEntry` 追加 `FirewallRuleAdded { name: String, program: String }`（`program` 为绝对路径）；卸载时调用 `delete_rule`，失败只记日志不中断卸载。
  - `steps.rs`：`struct AddFirewallRules`（显示名"配置防火墙"），逐条 `add_rule` 并写回执；`plan.rs`：`if !m.firewall.is_empty() { plan.push(Box::new(AddFirewallRules)); }`，放在 `ExtractFiles` 之后、快捷方式之前。
  - `shortcut.rs`：`SendTo` 位置解析为 `%APPDATA%\Microsoft\Windows\SendTo`；`args` 非空时写入 `IShellLink::SetArguments`；回执沿用 `ShortcutCreated`。
- [ ] **Step 1: 写失败测试**

```rust
// tests/step_plan.rs
#[test]
fn firewall_is_opt_in() {
    assert!(!plan_names(MINIMAL, InstallMode::Standard).iter().any(|n| n.contains("防火墙")));
    let toml = format!("{}\n[[firewall]]\nname = \"X\"\nprogram = \"x.exe\"\n", MINIMAL);
    assert!(plan_names(&toml, InstallMode::Standard).iter().any(|n| n.contains("防火墙")));
}
#[test]
fn portable_mode_never_touches_firewall() { /* 便携模式即使声明 [[firewall]] 也不入计划 */ }
// tests/manifest_parse.rs
#[test]
fn shortcut_sendto_with_args_parses() { /* location="sendto"、args="--station-put" 解析正确；缺省 args 为空 */ }
#[test]
fn firewall_defaults() { /* 只写 name/program → protocol Any、profiles ["private","domain"] */ }
// tests/receipt_roundtrip.rs
#[test]
fn firewall_receipt_roundtrips() {}
```
- [ ] **Step 2: 运行确认失败** — `cargo test --tests`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过** — `cargo test --tests && cargo clippy --lib`（该仓库的自检命令）；`tests/ui_defaults.rs::defaults_are_domain_neutral` 仍通过（新代码不得出现 LanMsg 字样）。
- [ ] **Step 5: Commit（在 wind-installer 仓库）**

```bash
cd /home/dufeng/develop/windinput/wind-installer
git add -- src/installer/firewall.rs
git commit -F - --only -- src/manifest.rs src/installer/firewall.rs src/installer/steps.rs src/installer/plan.rs src/installer/receipt.rs src/installer/shortcut.rs README.md app.toml tests/step_plan.rs tests/manifest_parse.rs tests/receipt_roundtrip.rs <<'EOF'
feat: 新增 [[firewall]] 能力段与快捷方式 args / sendto 位置

为需要局域网入站连接的应用提供可撤销的防火墙放行规则；快捷方式支持参数与「发送到」菜单。两者均缺省不执行。
EOF
```

---

### Task 18: `lanmsgd` 防火墙检测与一键放行

**Files:**
- Create: `crates/lanmsg-core/src/firewall.rs`（`#[cfg(windows)]`）、`crates/lanmsgd/src/firewall_cmd.rs`
- Modify: `crates/lanmsgd/src/main.rs`（`--add-firewall-rule`）、`crates/lanmsgd/src/tray.rs`（`run_tray` 菜单项与通知）、`crates/lanmsgd/src/ipc_server.rs`（新连接时补发 `FirewallMissing`）、`crates/lanmsg-desktop/src/views/shell.rs`、`src/strings.rs`

**Interfaces:**
- Produces:
  - `pub const FIREWALL_RULE_NAME: &str = "LanMsg";`（与 `packaging/windows/app.toml` 中 `[[firewall]].name` 一致）
  - `pub fn firewall_rule_present(program: &Path) -> bool`：运行 `netsh advfirewall firewall show rule name="LanMsg" verbose`，输出中包含 `program` 的路径（不区分大小写）即为 `true`；命令失败视为 `false`。
  - Service 启动后（仅 Windows）检查一次；不存在 → 发 `Event::FirewallMissing`，并记录 `firewall_missing = true`；`lanmsgd` 的 IPC 服务对每个新连接在 `firewall_missing` 时补发一次该事件。
  - 托盘（`tray.rs::run_tray`）：事件转发器收到 `FirewallMissing` 时，把菜单项 `允许 LanMsg 通过防火墙` 的可见性 `Signal<bool>` 置真，并调用一次 `TrayHandle::notify_tagged("LanMsg", "Windows 防火墙可能阻止其他设备发现本机，点击允许", "firewall")`；`on_notification_click` 中 tag 为 `"firewall"` 时不调用 `open_gui`，改为执行提权；菜单项点击同样执行提权。提权 = `crate::firewall_cmd::elevate_add_rule() -> std::io::Result<bool>`：`ShellExecuteExW`（verb `runas`、文件 = `current_exe()`、参数 `--add-firewall-rule`、`SEE_MASK_NOCLOSEPROCESS`）并等待子进程，退出码 0 返回 `Ok(true)`；用户在 UAC 取消返回 `Ok(false)`。成功后重新 `firewall_rule_present`，通过则把菜单项可见性置假。
  - `lanmsgd --add-firewall-rule`：不启动服务，执行与 wind-installer 相同的 `netsh … add rule`（name `LanMsg`、program = `current_exe()`、`protocol=any`、`profile=private,domain`），成功退出 0，失败打印错误退出 1。非 Windows 平台该参数打印"仅 Windows 需要"并退出 0。
  - 界面：`Model.firewall_missing`（Task 15 已加字段与事件规则）为真时，`views/shell.rs` 在顶部状态条位置（与 P3 `DISCONNECTED_BANNER` 同一位置、同一样式，断开横幅优先）显示 `strings::FIREWALL_BANNER = "Windows 防火墙可能阻止其他设备发现本机"` + 按钮 `FIREWALL_ALLOW = "允许"`：点击以 `runas` 启动同目录 `lanmsgd.exe --add-firewall-rule`（`ShellExecuteExW`，等待退出码 0 后把 `firewall_missing` 置假）。仅 `cfg(windows)` 编译该按钮逻辑。
- [ ] **Step 1: 写失败测试**：`firewall.rs` 中把 netsh 输出解析拆为纯函数 `fn output_mentions_program(output: &str, program: &Path) -> bool` 并测试（输出样例写死在测试中，含中文系统的"程序:"与英文"Program:"两种标签）；`lanmsgd` 测试 `add_firewall_rule_flag_is_noop_on_non_windows`（`#[cfg(not(windows))]`，子进程退出码 0）。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**；Windows 手工：删除规则 → 启动 lanmsgd 出现托盘提示 → 点击、UAC 确认 → 规则出现、提示消失。
- [ ] **Step 5: Commit**

---

### Task 19: Windows 打包：安装包与 scoop

**Files:**
- Create: `packaging/windows/app.toml`、`packaging/windows/build-dist.ps1`、`packaging/assets/{lanmsg.ico,lanmsg-256.png,logo.png}`、`packaging/scoop/lanmsg.json`、`packaging/scoop/update-manifest.sh`

**Interfaces:**
- Produces:
  - `packaging/windows/app.toml`（完整内容如下，`version` 由 `build-dist.ps1` 从 workspace 版本替换 `@VERSION@`）：

```toml
[app]
id                = "LanMsg"
display_name      = "LanMsg"
version           = "@VERSION@"
publisher         = "LanMsg"
description       = "局域网聊天式文件互传"
main_exe          = "lanmsg.exe"
setting_exe       = "lanmsg.exe"
start_menu_folder = "LanMsg"
url_protocol      = "lanmsg"
process_names     = ["lanmsg", "lanmsgd", "lanmsg-thumb"]

[theme]
accent = "#2F7BF6"

[paths]
install  = '%ProgramFiles%\{id}'
portable = '%USERPROFILE%\{id}'
data     = '%APPDATA%\{id}'

[autostart]
exe = "lanmsgd.exe"

[[shortcut]]
target   = "lanmsg.exe"
name     = "LanMsg"
location = "start_menu"

[[shortcut]]
target   = "lanmsg.exe"
name     = "LanMsg"
location = "desktop"

[[shortcut]]
target   = "lanmsg.exe"
name     = "LanMsg 中转站"
location = "sendto"
args     = "--station-put"

[[shortcut]]
target   = "uninstall.exe"
name     = "卸载 LanMsg"
location = "start_menu"

[[firewall]]
name     = "LanMsg"
program  = "lanmsgd.exe"
protocol = "any"
profiles = ["private", "domain"]

[startup]
prestart = true
exe      = "lanmsgd.exe"

[localdata]
cache_dirs = ["cache", "logs"]
remove_dir_when_empty = true

[package]
compression = "lzma"
source_dir  = "../../target/dist/windows-@ARCH@"
output_name = "lanmsg-windows-@ARCH@-setup"
output_dir  = "../../target/dist"
logo        = "../assets/logo.png"
icon        = "../assets/lanmsg.ico"
```
    `url_protocol = "lanmsg"` 让 `lanmsg://pair?...` 链接能拉起 `lanmsg.exe`。P3 未处理该参数，本任务在 `crates/lanmsg-desktop/src/main.rs` 中增加：位置参数以 `lanmsg://pair?` 开头时，进入配对页（P3 `Page`/配对加入流程）并在首次 `Connected` 后发送 `PairJoin { target: PairTarget::QrUri { uri } }`；已运行时经单实例转发的 argv 同样处理；测试 `pair_uri_arg_is_recognized`（参数解析纯函数）。macOS `Info.plist` 的 `CFBundleURLTypes` 与 Linux `.desktop` 的 `x-scheme-handler/lanmsg` 使用同一入口。
  - `build-dist.ps1 -Arch x64|arm64 [-WindInstallerDir <路径>]`：`cargo build --release --target <x86_64|aarch64>-pc-windows-msvc -p lanmsgd -p lanmsg-desktop -p lanmsg-cli -p lanmsg-media`（`lanmsg-thumb` 是 media 的 bin）→ 复制 `lanmsg.exe`、`lanmsgd.exe`、`lanmsg-cli.exe`、`lanmsg-thumb.exe`、`LICENSE` 到 `target/dist/windows-<arch>/` → 生成 `target/dist/lanmsg-windows-<arch>.zip`（scoop 用）→ 替换 `@VERSION@`/`@ARCH@` 生成临时清单 → `wind-packer build --config <临时清单> --stub wind-installer.exe`。`wind-packer`/stub 来源：给了 `-WindInstallerDir` 就用该目录下 `target/release/` 的本地构建（开发期 path 方式），否则从 wind-installer 的 GitHub Release 下载**固定 tag**（脚本顶部变量 `$WindInstallerTag`，取包含 Task 17 能力的首个版本）。
  - `packaging/scoop/lanmsg.json`：

```json
{
  "version": "@VERSION@",
  "description": "局域网聊天式文件互传",
  "homepage": "https://github.com/huanfeng/lanmsg",
  "license": "GPL-3.0-or-later",
  "architecture": {
    "64bit": { "url": "https://github.com/huanfeng/lanmsg/releases/download/v@VERSION@/lanmsg-windows-x64.zip", "hash": "@SHA256_X64@" },
    "arm64": { "url": "https://github.com/huanfeng/lanmsg/releases/download/v@VERSION@/lanmsg-windows-arm64.zip", "hash": "@SHA256_ARM64@" }
  },
  "bin": ["lanmsg-cli.exe"],
  "shortcuts": [["lanmsg.exe", "LanMsg"]],
  "notes": "首次运行若提示防火墙，请在托盘菜单中点击「允许 LanMsg 通过防火墙」。",
  "checkver": { "github": "https://github.com/huanfeng/lanmsg" },
  "autoupdate": {
    "architecture": {
      "64bit": { "url": "https://github.com/huanfeng/lanmsg/releases/download/v$version/lanmsg-windows-x64.zip" },
      "arm64": { "url": "https://github.com/huanfeng/lanmsg/releases/download/v$version/lanmsg-windows-arm64.zip" }
    }
  }
}
```
    仓库地址 `huanfeng/lanmsg` 为假定值，集中在 `packaging/scoop/update-manifest.sh` 与 `release.yml` 顶部变量里，确认后统一替换。
  - `update-manifest.sh <version> <SHA256SUMS 路径>`：填充模板，输出 `target/dist/lanmsg.json`。
- [ ] **Step 1: 写失败测试**：`scripts/check-packaging.sh`（新建，CI ubuntu 运行）：用 `python3 -c 'import tomllib…'` 解析 `app.toml` 成功、`jq . packaging/scoop/lanmsg.json` 成功、`update-manifest.sh 1.2.3 <样例 SHA256SUMS>` 输出中不含 `@` 占位符。先写脚本，跑一次确认失败（文件尚不存在）。
- [ ] **Step 2: 运行确认失败** — `bash scripts/check-packaging.sh`
- [ ] **Step 3: 实现**；图标：从 windui 的 `assets/windui-256.png` 风格另做 LanMsg 图标不在本计划范围——若没有设计稿，用 256×256 纯色圆角方块 + 白色 "L" 生成占位图标，并在任务报告中标为"待替换"（这是唯一允许的占位资源）。
- [ ] **Step 4: 运行确认通过**；Windows 手工：`build-dist.ps1 -Arch x64 -WindInstallerDir ...` 生成安装包，安装后：开始菜单 / 桌面 / 发送到 均有快捷方式、`lanmsgd` 自启、防火墙规则存在；卸载后规则与快捷方式消失。
- [ ] **Step 5: Commit**

---

### Task 20: Linux 安装脚本、systemd 与构建

**Files:**
- Create: `packaging/linux/{install.sh,uninstall.sh,lanmsgd.service,lanmsg.desktop,build-tarball.sh}`
- Modify: `crates/lanmsgd/Cargo.toml`（windui 依赖改为 `optional = true`，新增 `[features] default = ["desktop"]`、`desktop = ["dep:windui"]`）、`crates/lanmsgd/src/main.rs`（`tray`、`notify`、`launch` 模块加 `#[cfg(feature = "desktop")]`；未开 `desktop` 时 `is_headless` 的结果视为恒真，直接 `block_until_shutdown()`）

**Interfaces:**
- Produces:
  - 产物结构（`build-tarball.sh <target-triple> <out-name>` 生成）：
    - 桌面版 `lanmsg-linux-<x86_64|aarch64>.tar.gz`：`bin/{lanmsg,lanmsgd,lanmsg-cli,lanmsg-thumb}`、`share/applications/lanmsg.desktop`、`share/icons/hicolor/256x256/apps/lanmsg.png`、`lib/systemd/user/lanmsgd.service`、`LICENSE`。
    - 无头版 `lanmsgd-linux-<x86_64|aarch64>-musl.tar.gz`：`bin/{lanmsgd,lanmsg-cli}`、`lib/systemd/user/lanmsgd.service`（ExecStart 带 `--headless`）、`LICENSE`。
  - `lanmsgd.service`：

```ini
[Unit]
Description=LanMsg 后台服务
After=network-online.target

[Service]
ExecStart=%h/.local/bin/lanmsgd @HEADLESS@
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
```
  - `lanmsg.desktop`：`Name=LanMsg`、`Comment=局域网聊天式文件互传`、`Exec=lanmsg %U`、`Icon=lanmsg`、`Type=Application`、`Categories=Network;FileTransfer;`、`MimeType=x-scheme-handler/lanmsg;`。
  - `install.sh [--headless] [--version vX.Y.Z] [--prefix DIR]`（默认 `~/.local`；`POSIX sh` 兼容，`set -eu`）：
    1. `uname -m` → `x86_64` / `aarch64`（`arm64` 归一为 `aarch64`），其他报错退出。
    2. 非 `--headless` 时检查 glibc：`ldd --version 2>&1 | head -n1` 解析版本 < 2.31（或检测到 musl）→ 提示"系统 glibc 低于 2.31，请使用 --headless 安装无界面版本"并退出 1。
    3. 下载地址 `${LANMSG_BASE_URL:-https://github.com/huanfeng/lanmsg/releases/latest/download}`（`--version` 时用 `…/releases/download/<ver>`）；用 `curl -fsSL` 或 `wget -qO-` 下载 tarball 与 `SHA256SUMS`，`sha256sum -c` 校验失败即退出。
    4. 解压到 `$prefix`（`bin/` → `$prefix/bin`，`share/` → `$prefix/share`，service → `~/.config/systemd/user/`，替换 `@HEADLESS@`）。
    5. 有 `systemctl --user` 时 `daemon-reload` + `enable --now lanmsgd`；否则提示手动运行 `lanmsgd`。有 `update-desktop-database` 时执行。
    6. `$prefix/bin` 不在 `PATH` 时提示添加。
    7. 末行打印 `LanMsg 安装完成`。
  - `uninstall.sh [--purge] [--prefix DIR]`：`systemctl --user disable --now lanmsgd`（忽略失败）→ 删除安装的文件 → `--purge` 时再删除 `${XDG_DATA_HOME:-~/.local/share}/lanmsg`（先打印将删除的路径并要求输入 `yes` 确认，`--yes` 跳过确认）。
  - 构建基线（写在 Task 22 的工作流中）：桌面版在 `ubuntu:20.04` 容器内构建（glibc 2.31），aarch64 用 GitHub `ubuntu-24.04-arm` runner + 同一容器镜像的 arm64 版本；无头版在 `rust:alpine` 容器内按架构原生构建 `--target <arch>-unknown-linux-musl -p lanmsgd -p lanmsg-cli`，`lanmsgd` 用 `--no-default-features`（关闭 `desktop` feature，不链接 windui）。
- [ ] **Step 1: 写失败测试**：`packaging/linux/test-install.sh`（CI ubuntu 运行）：用 `build-tarball.sh` 打一个本机架构的包到临时目录，起 `python3 -m http.server` 提供该目录与生成的 `SHA256SUMS`，`LANMSG_BASE_URL=http://127.0.0.1:<port> sh install.sh --prefix <tmp>/prefix`（容器中无 systemd，验证脚本走"提示手动运行"分支），断言 `<tmp>/prefix/bin/lanmsgd --help` 退出 0 与 `.desktop` 存在；再 `sh uninstall.sh --prefix <tmp>/prefix` 后文件消失；篡改 tarball 一个字节后 install 必须失败；另外 `cargo build -p lanmsgd --no-default-features` 必须成功，且 `cargo tree -p lanmsgd --no-default-features | grep -c windui` 为 0。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**；`shellcheck packaging/linux/*.sh` 无警告（CI 中执行）。
- [ ] **Step 4: 运行确认通过**；手工：一台 Ubuntu 22.04 桌面跑 `curl -fsSL …/install.sh | sh`，GNOME 下无托盘时 `lanmsgd` 仍在后台、从应用菜单可打开界面。
- [ ] **Step 5: Commit**

---

### Task 21: macOS 打包脚本（未签名 dmg + 可选签名公证）

**Files:**
- Create: `packaging/macos/{build-dmg.sh,Info.plist,name.apk.lanmsg.daemon.plist}`
- （不修改 Rust 代码：macOS 开机自启沿用 P3 `crates/lanmsg-desktop/src/autostart.rs` 的 LaunchAgent `app.lanmsg.daemon`，首次向导中已调用 `autostart::set_enabled(true)`）

**Interfaces:**
- Produces:
  - `build-dmg.sh <version>`：分别 `cargo build --release --target x86_64-apple-darwin` 与 `aarch64-apple-darwin`，`lipo -create` 合成通用二进制；组装 `LanMsg.app/Contents/{Info.plist, MacOS/{lanmsg,lanmsgd,lanmsg-cli,lanmsg-thumb}, Resources/lanmsg.icns}`（icns 由 `packaging/assets/lanmsg-256.png` 经 `sips` + `iconutil` 生成）；`Info.plist`：`CFBundleIdentifier = name.apk.lanmsg`、`CFBundleExecutable = lanmsg`、`CFBundleShortVersionString = <version>`、`LSMinimumSystemVersion = 11.0`、`CFBundleURLTypes` 注册 `lanmsg` scheme、`NSLocalNetworkUsageDescription = "LanMsg 需要访问本地网络来发现和连接你的其他设备"`、`NSBonjourServices` 不需要（不用 Bonjour）。
  - 签名公证仅在环境变量 `APPLE_DEVELOPER_ID`、`APPLE_ID`、`APPLE_TEAM_ID`、`APPLE_APP_PASSWORD` 全部存在时执行：`codesign --deep --options runtime --timestamp -s "$APPLE_DEVELOPER_ID"` → `hdiutil create` → `xcrun notarytool submit --wait` → `xcrun stapler staple`；缺任一变量则打印"未配置 Apple 签名，生成未签名 dmg"并只做 `hdiutil create -volname LanMsg -srcfolder … -format UDZO target/dist/lanmsg-macos-universal.dmg`。
  - 自启：由 P3 `autostart.rs` 负责（写 `~/Library/LaunchAgents/app.lanmsg.daemon.plist`，`ProgramArguments` 为与 `lanmsg` 同目录的 `lanmsgd`，即 `LanMsg.app/Contents/MacOS/lanmsgd`），本任务只需保证 `.app` 内 `lanmsg` 与 `lanmsgd` 同目录。
- [ ] **Step 1: 写失败测试**：`scripts/check-packaging.sh` 增加：`python3 -c 'import plistlib,sys; d=plistlib.load(open(sys.argv[1],"rb")); assert d["CFBundleIdentifier"]=="name.apk.lanmsg"; assert d["CFBundleExecutable"]=="lanmsg"' packaging/macos/Info.plist`（macOS runner 上另跑 `plutil -lint`）。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**；CI macOS runner 执行 `build-dmg.sh 0.0.0-ci` 产出未签名 dmg（真机验收为外部依赖，记录在阶段报告）。
- [ ] **Step 5: Commit**

---

### Task 22: 发布流水线

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `.github/workflows/ci.yml`（加 `scripts/check-packaging.sh`、`packaging/linux/test-install.sh`、`shellcheck`）

**Interfaces:**
- Produces:
  - 触发：推送 `v*` tag；`workflow_dispatch` 时只构建不发布（`dry_run` 输入，默认 true）。
  - 顶部 `env`：`REPO_SLUG: huanfeng/lanmsg`、`WIND_INSTALLER_TAG: <Task 19 中的同一 tag>`。
  - Jobs（全部上传 artifact，名称即最终资产名）：

| Job | Runner / 容器 | 产物 |
|---|---|---|
| `windows` 矩阵 x64/arm64 | `windows-latest` | `lanmsg-windows-<arch>-setup.exe`、`lanmsg-windows-<arch>.zip` |
| `linux-desktop` 矩阵 | x86_64：`ubuntu-latest` + 容器 `ubuntu:20.04`；aarch64：`ubuntu-24.04-arm` + 同镜像 | `lanmsg-linux-<arch>.tar.gz` |
| `linux-headless` 矩阵 | 同上 runner + 容器 `rust:alpine` | `lanmsgd-linux-<arch>-musl.tar.gz` |
| `macos` | `macos-latest` | `lanmsg-macos-universal.dmg`（有 Apple secrets 则签名公证） |
| `android` | `ubuntu-latest`（setup-java 17、Android SDK、NDK、`cargo install cargo-ndk`） | `lanmsg-android.apk`（secrets `ANDROID_KEYSTORE_B64`/`ANDROID_KEYSTORE_PASSWORD`/`ANDROID_KEY_ALIAS`/`ANDROID_KEY_PASSWORD` 存在时解码为 `android/release.jks` 并写 `android/keystore.properties`（键 `storeFile`/`storePassword`/`keyAlias`/`keyPassword`，P4 Task 13 定义）；然后 `bash scripts/build-android.sh release`，把 `android/app/build/outputs/apk/release/app-release.apk` 改名为资产名；secrets 缺失时 P4 构建自动用 debug 签名，资产名改为 `lanmsg-android-unsigned.apk`） |
| `release`（needs 全部，`dry_run` 为 false 时） | `ubuntu-latest` | 汇总全部 artifact + `packaging/linux/install.sh`、`uninstall.sh` → `sha256sum * > SHA256SUMS` → `update-manifest.sh` 生成 `lanmsg.json` → `gh release create $TAG --generate-notes` 上传全部文件 |

  - 容器 `ubuntu:20.04` 中先安装：`build-essential curl pkg-config libfontconfig1-dev libx11-dev libxkbcommon-dev libwayland-dev libdbus-1-dev`，再用 rustup 安装 stable。
  - 资产名不含版本号（与 wind-installer 一致，版本由 tag 表达）；`install.sh` 依赖的名字以本表为准。
- [ ] **Step 1: 写失败测试**：`scripts/check-packaging.sh` 增加 `actionlint .github/workflows/*.yml`（CI 安装 actionlint）与"release.yml 中声明的资产名集合 == install.sh / scoop 模板引用的资产名集合"的 grep 检查。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**；在 fork 或本仓库用 `workflow_dispatch`（`dry_run: true`）跑一次，全部 job 成功，artifact 齐全，结果链接写入任务报告。
- [ ] **Step 5: Commit**

---

## 阶段验收

- [ ] `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` 全部通过；`node --test web/test/`、`bash scripts/check-packaging.sh`、`bash packaging/linux/test-install.sh` 通过；Android `./gradlew testDebugUnitTest` 通过；wind-installer `cargo test --tests` 通过。
- [ ] **手工互操作**（记录设备型号与版本）：
  - LanMsg ↔ LocalSend 官方 App（Android 与 Windows 各一）：互发文字、互发 3 个文件（含中文名、>1GB 一个）；LocalSend 侧能看到 LanMsg 设备；本机同时运行 LocalSend 时 LanMsg 设置页出现端口占用提示且仍可向 LocalSend 发送。抓包核对 `text_message_request.json` 夹具与真实请求一致。
  - Web 临时会话：Android Chrome 扫码（key 模式）与 iOS Safari 手输配对码（code 模式）各完成一次双向传输；30 分钟空闲后页面显示"会话已结束"。
  - 中转站：台式机建站放入 3 个文件（引用 + 复制各有）→ Android 查看清单并下载；台式机关机后 Android 显示"源离线"；Android 往中转站上传一张照片，台式机目录出现该文件。
  - 跨网段：两台经 Tailscale 互通、不在同一局域网的电脑，手动添加后互发消息，联系人行显示"VPN"角标；第三台本人设备经地址交换自动连上。
  - 安装渠道：Windows 安装包与 scoop（`scoop install <本地 json>`）各装一次；Ubuntu `curl | sh` 安装、NAS（或容器）无头版安装；Android APK 安装。macOS 记为外部依赖。
- [ ] `grep -rnE 'todo!|unimplemented!|#\[ignore\]|TODO' crates/ web/ packaging/ android/app/src | grep -v 'TODO(#'` 无输出；唯一允许的占位是 Task 19 中标注"待替换"的图标资源。

---

## 本计划对前序代码与外部仓库的修改汇总

C5/C6 中本阶段用到的类型与字段已由路线图统一、P1 一次定义，此处不再列出。以下是本阶段任务**自己执行**的改动，汇总以便审阅：

1. P1 `LocalIface` 追加 `pub tunnel: Option<TunnelKind>`，`classify` 规则调整（Task 2，同时改 P1 中构造 `LocalIface` 的测试）。
2. Service `ConnEvent::Up` 记录地址改用 `PeerInfo.dial_addr`（Task 3，`on_peer_up_record`）。
3. P2 `Trust::accept_peer` 规则 7 与 `gate_inbound` 规则 6 增加"手动添加的设备"（Task 3）。
4. P2 `blob_visible_to` 对本人放宽（Task 5）。
5. P2 `TransferEngine::download` 拆出私有 `start` 并新增 `download_to`（Task 6）。
6. P2 同步规则（`SyncEngine::shared_convs` 只同步 `SelfDevices`/`Dm`）不改代码，只追加回归测试 `localsend_conv_is_never_synced`、`guest_conv_is_never_synced`（Task 11、14）。
7. 迁移 `MIGRATIONS[3]`：`known_addresses`、`manual_peers`、`stations`、`station_items`、`station_blobs`、`localsend_peers`、`web_guests`（Task 1）。
8. 设计 §13 的加密细节（已同步到设计文档）：XSalsa20-Poly1305（tweetnacl secretbox）；手动码模式为"X25519 临时密钥交换 + scrypt(配对码) 作 HKDF salt + 双向 HMAC 确认"（Task 12）。
9. P3：`Page::Stations`、`DemoPage::{Stations, Web}`、`ReplyTag::{Stations, StationItems, Web, Interfaces, ManualPeer}`、`ExpiryChoice` 与 `Model` 新字段、文件视图"放入中转站"（Task 7、15）；`launch.rs` 新增 `open_gui_with(data_dir, extra_args)`；`lanmsg`（界面）新增 `--station-put`、`--web` 与 `lanmsg://` 参数处理。
10. P4：`Conditions` 追加 `webHosting`、`DeviceConditions.setWebHosting`、`Routes` 追加 `STATION`/`WEB_HOST`/`NETWORK`、`FilesViewModel.putToStation`（Task 8、16）。
11. `lanmsgd` 新增 cargo feature `desktop`（默认开，musl 无头构建关闭）与命令行参数 `--add-firewall-rule`（Task 18、20）。
12. wind-installer 清单格式新增 `[[firewall]]` 段、`[[shortcut]].args` 字段与 `location = "sendto"`（Task 17，在 wind-installer 仓库内）。
13. 依赖（已登记于路线图依赖清单）：`x25519-dalek@2`、`crypto_secretbox@0.1`、`hkdf@0.12`、`scrypt@0.11`、`qrcode@0.14`、`uuid@1`、`base64@0.22`、`mime_guess@2`、`axum-server@0.7`、`serde_json@1`、dev `tokio-tungstenite@0.24`；JS vendored `tweetnacl@1.0.3`、`scrypt-js@3.0.1`；Android `com.google.zxing:core:3.5.3`。
