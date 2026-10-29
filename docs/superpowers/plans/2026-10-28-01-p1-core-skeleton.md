# P1 核心骨架 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **开工前必须读 `docs/superpowers/plans/2026-10-28-00-roadmap.md`（全局约束 + 共享契约 C1–C8）。**

**Goal:** 两台机器上的 `lanmsgd`（无头）能互相发现、建立双向认证的 TLS 连接，并通过 `lanmsg-cli` 收发文字消息，消息持久化在 SQLite 中。

**Architecture:** `lanmsg-proto` 提供无 IO 的协议类型、签名与帧编解码；`lanmsg-core` 的 `Service` 组合身份、存储、发现、连接管理和消息模块，并通过 `Service::handle(Request)` + 事件广播对外；`lanmsgd` 把 `Service` 暴露在本地 IPC 上，`lanmsg-cli` 是 IPC 客户端。

**Tech Stack:** Rust stable、tokio、rustls 0.23 + rcgen、ed25519-dalek 2、blake3、ciborium、rusqlite（bundled）、socket2、interprocess、clap。

**Spec:** `docs/superpowers/specs/2026-10-28-lanmsg-design.md`；共享契约：`docs/superpowers/plans/2026-10-28-00-roadmap.md`

## Global Constraints

路线图「全局约束」全部适用，此处不重复。本阶段额外约束：

- 本阶段不实现：配对、联系人/陌生人判定、多设备同步、文件。凡是已初始化的设备之间都可以互发文字（信任规则在 P2 加上）。
- 网络集成测试一律 `DiscoveryMode::Memory` 或 `static_peers`，不得依赖真实组播。

## Review Focus

1. **两端同时拨号**（发现后双方都认为该自己连）：最终只保留一条连接，双方都只收到一次 `PeerOnline`。→ Task 9 测试 `simultaneous_dial_keeps_one_connection`。
2. **TLS 证书公钥与 Hello 中的设备证书不一致**（中间人或伪造）：必须断开。→ Task 9 测试 `rejects_hello_cert_mismatching_tls_key`。
3. **同一条消息重复送达**：只入库一次、只发一次 `MessageAdded`。→ Task 10 测试 `duplicate_push_is_idempotent`。
4. **组播包被篡改 / 来自不同协议版本 / 是自己发的**：忽略且不报错。→ Task 7 测试 `ignores_tampered_and_self_announces`。
5. **`lanmsgd` 已在运行时再启动一个**：第二个进程应报"已在运行"并以非零码退出，而不是抢占 IPC 端点。→ Task 12 测试 `second_daemon_refuses_to_start`。

---

## 文件结构

```
Cargo.toml  rust-toolchain.toml  LICENSE  .gitignore  .github/workflows/ci.yml
crates/lanmsg-proto/src/
  lib.rs        consts.rs     ids.rs      types.rs     sign.rs     frame.rs    codec.rs
crates/lanmsg-core/src/
  lib.rs        error.rs      paths.rs    keystore.rs  identity.rs  hlc.rs
  store/mod.rs  store/schema.rs  store/users.rs  store/convs.rs  store/messages.rs
  discovery/mod.rs  discovery/memory.rs  discovery/multicast.rs
  net/mod.rs    net/tls.rs    net/path.rs  net/conn.rs  net/manager.rs
  msg.rs        testutil.rs   platform.rs
  service/mod.rs  service/handlers.rs  service/views.rs
crates/lanmsg-core/tests/  two_nodes.rs
crates/lanmsg-api/src/  lib.rs  types.rs  ipc.rs
crates/lanmsgd/src/  main.rs  ipc_server.rs
crates/lanmsgd/tests/  daemon.rs
crates/lanmsg-cli/src/  main.rs
scripts/e2e-two-daemons.sh
```

其余 crate（media、localsend、web-assets、desktop、ffi）在各自阶段创建，本阶段不建空壳。

---

### Task 1: 仓库与 workspace 初始化

**Files:**
- Create: `Cargo.toml`、`rust-toolchain.toml`、`.gitignore`、`LICENSE`、`.github/workflows/ci.yml`
- Create: `crates/lanmsg-proto/{Cargo.toml,src/lib.rs}`、`crates/lanmsg-core/{Cargo.toml,src/lib.rs}`、`crates/lanmsg-api/{Cargo.toml,src/lib.rs}`、`crates/lanmsgd/{Cargo.toml,src/main.rs}`、`crates/lanmsg-cli/{Cargo.toml,src/main.rs}`

**Interfaces:**
- Produces: 可编译的 workspace；`[workspace.package]` 提供 `version = "0.1.0"`、`edition = "2021"`、`license = "GPL-3.0-or-later"`、`repository`（留空字符串不可，省略该键）；`[workspace.dependencies]` 不使用（各 crate 用 `cargo add` 直接声明）。

- [ ] **Step 1: 初始化仓库**

```bash
cd /home/dufeng/develop/workspace/lanmsg
git init -b main
```
`docs/` 已存在，保留。

- [ ] **Step 2: 写 workspace 文件**

`Cargo.toml`：`[workspace] members = ["crates/*"]`、`resolver = "2"`，加 `[workspace.package]`，加 `[profile.release] lto = true, codegen-units = 1, strip = true, panic = "abort"`。
`rust-toolchain.toml`：`channel = "stable"`，`components = ["rustfmt", "clippy"]`。
`.gitignore`：`/target`、`*.part`、`.omc/`、`.remember/`。
`LICENSE`：GPL-3.0 全文，`curl -fsSL https://www.gnu.org/licenses/gpl-3.0.txt -o LICENSE`（无网络时在任务报告中注明，由人工补）。

- [ ] **Step 3: 建 5 个 crate 骨架**

`cargo new --lib crates/lanmsg-proto`、`--lib crates/lanmsg-core`、`--lib crates/lanmsg-api`、`--bin crates/lanmsgd`、`--bin crates/lanmsg-cli`。每个 `Cargo.toml` 的 `[package]` 用 `version.workspace = true`、`edition.workspace = true`、`license.workspace = true`。`lanmsg-cli` 的 `[[bin]] name = "lanmsg-cli"`。

- [ ] **Step 4: CI 工作流**

`.github/workflows/ci.yml`：push/PR 触发，矩阵 `ubuntu-latest`、`windows-latest`、`macos-latest`，步骤 `cargo fmt --all --check`（仅 ubuntu）、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。

- [ ] **Step 5: 验证**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 编译成功，0 个测试失败。

- [ ] **Step 6: Commit**

```bash
git add -- Cargo.toml rust-toolchain.toml .gitignore LICENSE .github crates docs
git commit -F - --only -- Cargo.toml rust-toolchain.toml .gitignore LICENSE .github crates docs <<'EOF'
chore: 初始化 workspace 与 CI

建立 Cargo workspace、五个初始 crate 和三平台 CI，附设计文档与实施计划。
EOF
```
（这是仓库第一次提交，`Cargo.lock` 也要加入：`git add -- Cargo.lock` 并加到 `--only` 路径中。）

---

### Task 2: 协议标识类型与常量

**Files:**
- Create: `crates/lanmsg-proto/src/consts.rs`、`crates/lanmsg-proto/src/ids.rs`
- Modify: `crates/lanmsg-proto/src/lib.rs`（`pub mod consts; pub mod ids;`）

**Interfaces:**
- Produces: 路线图 C1 全部常量；C2 全部类型与方法；`pub enum IdError { BadHex, BadLength, BadMsgId }`（thiserror）。

依赖：`cargo add -p lanmsg-proto serde@1 --features derive`、`blake3@1`、`data-encoding@2`、`thiserror@2`、`rand@0.8`。

- [ ] **Step 1: 写失败测试**（`ids.rs` 底部）

```rust
#[test]
fn device_id_hex_roundtrip() {
    let id = DeviceId([7u8; 32]);
    assert_eq!(id.to_hex().len(), 64);
    assert_eq!(DeviceId::from_hex(&id.to_hex()).unwrap(), id);
}
#[test]
fn from_hex_rejects_wrong_length() {
    assert!(matches!(DeviceId::from_hex("abcd"), Err(IdError::BadLength)));
}
#[test]
fn short_is_26_lowercase_base32() {
    let s = UserId([1u8; 32]).short();
    assert_eq!(s.len(), 26);
    assert!(s.chars().all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c)));
}
#[test]
fn dm_conv_id_is_order_independent() {
    let a = UserId([1u8; 32]); let b = UserId([2u8; 32]);
    assert_eq!(ConvId::dm(&a, &b), ConvId::dm(&b, &a));
    assert_ne!(ConvId::dm(&a, &b), ConvId::self_devices(&a));
}
#[test]
fn msg_id_string_roundtrip() {
    let m = MsgId { author: DeviceId([9u8; 32]), seq: 42 };
    let s = m.to_string();
    assert!(s.ends_with(":42"));
    assert_eq!(s.parse::<MsgId>().unwrap(), m);
    assert!("zz:1".parse::<MsgId>().is_err());
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p lanmsg-proto`
Expected: 编译失败（类型未定义）。

- [ ] **Step 3: 实现**

按 C1/C2 实现。32 字节类型用一个声明宏 `id32!(Name)` 生成公共实现，避免四份重复代码。serde 序列化为 CBOR bytes（用 `serde_bytes` 风格：手写 `Serialize` 调 `serialize_bytes`，`Deserialize` 接受 bytes 并校验长度 32）——不要序列化成 32 个整数的数组。`short()` 用 `data_encoding::BASE32_NOPAD` 后转小写。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p lanmsg-proto`
Expected: 5 passed。

- [ ] **Step 5: Commit**（路径：`crates/lanmsg-proto`、`Cargo.lock`）

---

### Task 3: 协议数据类型与签名

**Files:**
- Create: `crates/lanmsg-proto/src/types.rs`、`crates/lanmsg-proto/src/sign.rs`
- Modify: `crates/lanmsg-proto/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 的 ids。
- Produces:
  - C3 中标注 P1 的类型，以及 P2 类型（`BlobMeta`、`MediaInfo`、`FolderEntry`、`SystemEvent`）的**结构定义**（P1 就定义好，避免 P2 改线格式）。
  - `pub struct DeviceKey(ed25519_dalek::SigningKey)`：`generate() -> Self`、`from_bytes(&[u8;32]) -> Self`、`to_bytes(&self) -> [u8;32]`、`device_id(&self) -> DeviceId`、`sign(&self, msg: &[u8]) -> Signature64`、`to_pkcs8_der(&self) -> Vec<u8>`。
  - `pub struct UserKey(ed25519_dalek::SigningKey)`：同上方法，`user_id(&self) -> UserId`。
  - `pub fn verify(pubkey: &[u8;32], msg: &[u8], sig: &Signature64) -> Result<(), SignError>`
  - `pub trait Signed: Serialize + Clone { fn sig_mut(&mut self) -> &mut Signature64; fn sig(&self) -> &Signature64; }`，为 `DeviceCert`、`Envelope`、`Announce` 实现。
  - `pub fn sign_with_zeroed_sig<T: Signed>(value: &mut T, sign: impl FnOnce(&[u8]) -> Signature64)`
  - `pub fn verify_with_zeroed_sig<T: Signed>(value: &T, pubkey: &[u8;32]) -> Result<(), SignError>`
  - `impl DeviceCert { pub fn issue(user: &UserKey, device: DeviceId, device_name: String, device_type: DeviceType, created_at: i64) -> DeviceCert; pub fn verify(&self) -> Result<(), SignError> }`（key_epoch = 0；verify 用 `self.user.0` 作公钥）
  - `impl Envelope { pub fn verify(&self) -> Result<(), SignError> }`（用 `self.id.author.0`）
  - `impl Announce { pub fn verify(&self) -> Result<(), SignError> }`（先 `cert.verify()`，再用 `cert.device.0` 验 announce 自身）
  - `pub fn to_cbor<T: Serialize>(v: &T) -> Vec<u8>`、`pub fn from_cbor<T: DeserializeOwned>(b: &[u8]) -> Result<T, CodecError>`（放 `sign.rs` 或独立 `cbor.rs` 均可，但 crate 根要 `pub use`）

依赖：`cargo add -p lanmsg-proto ed25519-dalek@2 --features rand_core,pkcs8`、`ciborium@0.2`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn cert_issued_by_user_verifies_and_tamper_fails() {
    let user = UserKey::generate(); let dev = DeviceKey::generate();
    let mut cert = DeviceCert::issue(&user, dev.device_id(), "台式机".into(), DeviceType::Desktop, 1_700_000_000_000);
    assert!(cert.verify().is_ok());
    cert.device_name = "别人".into();
    assert!(cert.verify().is_err());
}
#[test]
fn envelope_signed_by_author_verifies() {
    let dev = DeviceKey::generate();
    let mut env = Envelope { conv: ConvId([0;32]), id: MsgId { author: dev.device_id(), seq: 1 },
        conv_seq: 1, hlc: Hlc { millis: 1, counter: 0 }, body: MsgBody::Text { text: "你好".into() },
        enc: None, sig: Signature64([0;64]) };
    sign_with_zeroed_sig(&mut env, |b| dev.sign(b));
    assert!(env.verify().is_ok());
    let mut other = env.clone(); other.id.author = DeviceKey::generate().device_id();
    assert!(other.verify().is_err());
}
#[test]
fn cbor_roundtrip_envelope() {
    // 构造同上 env，to_cbor 再 from_cbor，断言相等
}
```
第三个测试体按注释写完整（断言 `from_cbor::<Envelope>(&to_cbor(&env)).unwrap() == env`），所有类型需 `PartialEq`。

- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-proto`，编译失败。
- [ ] **Step 3: 实现**。`Signature64` 与 ids 一样序列化为 bytes。枚举用 serde 默认外部标签（CBOR map），`Hlc`/`VersionVector` 普通派生。
- [ ] **Step 4: 运行确认通过** — Expected: 全部通过。
- [ ] **Step 5: Commit**（`crates/lanmsg-proto`、`Cargo.lock`）

---

### Task 4: 帧定义与编解码

**Files:**
- Create: `crates/lanmsg-proto/src/frame.rs`、`crates/lanmsg-proto/src/codec.rs`

**Interfaces:**
- Consumes: Task 3 类型。
- Produces:
  - 路线图 C4 的 `Frame` 枚举。本阶段定义 P1 变体（Hello、Ping、Pong、SyncOffer、SyncPush、Ack、Presence、Error、Unknown）；P2/P5 变体由对应阶段追加，类型号已在 C4 预留。
  - `pub struct Hello`（字段见 C4）。
  - `impl Frame { pub fn type_id(&self) -> u16; pub fn encode_payload(&self) -> Vec<u8>; pub fn decode(type_id: u16, payload: &[u8]) -> Result<Frame, CodecError> }`；未知 type_id → `Ok(Frame::Unknown(type_id))`。
  - `pub struct FrameCodec;` 实现 `tokio_util::codec::{Encoder<Frame>, Decoder<Item = Frame>}`，线格式见 C4。
  - `pub enum CodecError { TooLarge(usize), Cbor(String), Io(std::io::Error), Truncated }`

依赖：`cargo add -p lanmsg-proto tokio-util@0.7 --features codec`、`bytes@1`；dev：`cargo add -p lanmsg-proto --dev tokio@1 --features full`、`futures@0.3`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn roundtrip_ping_and_hello() { /* encode → decode，断言相等，覆盖 Ping(7) 与一个带 cert: None 的 Hello */ }
#[test]
fn unknown_type_id_decodes_to_unknown() {
    let mut buf = BytesMut::new();
    buf.put_u32(2 + 1); buf.put_u16(777); buf.put_u8(0xa0); // CBOR 空 map
    assert_eq!(FrameCodec.decode(&mut buf).unwrap(), Some(Frame::Unknown(777)));
}
#[test]
fn rejects_frame_larger_than_max() {
    let mut buf = BytesMut::new();
    buf.put_u32((MAX_FRAME_LEN + 1) as u32);
    assert!(matches!(FrameCodec.decode(&mut buf), Err(CodecError::TooLarge(_))));
}
#[test]
fn partial_frame_waits_for_more_bytes() {
    // 只放入长度头和一半负载 → Ok(None)；补齐后 → Ok(Some(..))
}
```
注释处测试按描述写完整。

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**。解码时先检查长度头 > MAX_FRAME_LEN 立即报错（不等数据到齐）；长度 < 2 报 `Truncated`。
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 4A: 客户端 API 类型与 IPC 帧

**Files:**
- Create: `crates/lanmsg-api/src/{types.rs,ipc.rs}`
- Modify: `crates/lanmsg-api/src/lib.rs`（`pub mod types; pub mod ipc; pub use types::*;`）

**Interfaces:**
- Consumes: `lanmsg_proto::consts::MAX_FRAME_LEN`。
- Produces:
  - 路线图 C5 中**全部** DTO 与 `Request`/`Response`/`Event`/`ApiRequest`/`ApiResponse`/`ApiError`/`ErrorCode` 类型（一次定义齐，含标注为 P2–P5 的变体与字段；后续阶段只实现逻辑，不再改类型）。`Response` 除 `Ok` 外均为具名单字段 `{ value: T }`。
  - `ipc.rs`：`IpcFrame`；`pub struct IpcCodec;` 实现 `tokio_util::codec::{Encoder<IpcFrame>, Decoder<Item = IpcFrame>}`，线格式 `u32 BE 长度 | CBOR(IpcFrame)`，长度上限 `MAX_FRAME_LEN`；`pub enum IpcError { TooLarge(usize), Cbor(String), Io(std::io::Error) }`。（`endpoint_name`、`IpcClient`、`IpcListener` 在 Task 12 加入本文件。）
  - `uniffi` feature 在 P4 加，本阶段不加。

依赖：`cargo add -p lanmsg-api lanmsg-proto --path crates/lanmsg-proto`、`serde@1 --features derive`、`ciborium@0.2`、`thiserror@2`、`tokio-util@0.7 --features codec`、`bytes@1`。

- [ ] **Step 1: 写失败测试**（`ipc.rs` 测试模块）

```rust
#[test]
fn ipc_frame_roundtrip_request_and_event() {
    // 编码 IpcFrame::Request(ApiRequest{ id: 1, body: Request::SendText{ conv: "ab".into(), text: "你好".into() } })
    // 与 IpcFrame::Event(Event::PeerOffline{ device: "cd".into() })，解码后与原值相等
}
#[test]
fn response_uses_named_value_field() {
    let r = Response::MsgId { value: "x:1".into() };
    assert_eq!(r, Response::MsgId { value: "x:1".into() });
}
#[test]
fn ipc_rejects_oversized_frame() { /* 长度头 MAX_FRAME_LEN+1 → Err(IpcError::TooLarge(_)) */ }
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p lanmsg-api`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**（`crates/lanmsg-api`、`Cargo.lock`）

---

### Task 5: 密钥库、身份与 HLC

**Files:**
- Create: `crates/lanmsg-core/src/{error.rs,paths.rs,keystore.rs,identity.rs,hlc.rs}`
- Modify: `crates/lanmsg-core/src/lib.rs`

**Interfaces:**
- Consumes: proto 的 `DeviceKey`、`UserKey`、`DeviceCert`、`Hlc`。
- Produces:
  - C6 的 `KeyStore` trait；`pub enum KeyStoreError { Io(std::io::Error), Corrupt(String) }`。
  - `pub struct FileKeyStore { dir: PathBuf }`：`new(dir) -> Self`；文件名 = `name`，Unix 下权限 0600（目录 0700）；`save` 先写 `name.tmp` 再 rename。
  - 密钥名常量：`KEY_DEVICE = "device.key"`、`KEY_USER = "user.key"`（`identity.rs` 中 `pub const`）。
  - `pub struct Identity { pub device: DeviceKey, pub user: Option<UserKey>, pub cert: Option<DeviceCert> }`
    - `pub fn load_or_create(ks: &dyn KeyStore, store: &Store) -> Result<Identity, CoreError>`：设备密钥不存在则生成并保存；用户密钥存在则加载；证书从 store 的 `settings["self_cert"]`（CBOR 后 hex）读取。
    - `pub fn init_first_device(&mut self, ks: &dyn KeyStore, store: &Store, device_name: &str, device_type: DeviceType, now: i64) -> Result<(), CoreError>`：已有用户密钥 → `CoreError::AlreadyInitialized`。
    - `pub fn user_id(&self) -> Option<UserId>`、`pub fn device_id(&self) -> DeviceId`、`pub fn is_initialized(&self) -> bool`
  - `pub struct HlcClock { last: Hlc }`：`new() -> Self`、`pub fn now(&mut self, wall_millis: i64) -> Hlc`、`pub fn observe(&mut self, remote: Hlc, wall_millis: i64)`。算法：标准 HLC——`now`：`millis = max(wall, last.millis)`，若等于 `last.millis` 则 `counter = last.counter + 1` 否则 0；`observe`：`millis = max(wall, last.millis, remote.millis)`，counter 按三者中取得最大值的来源 +1（相同则取较大 counter +1）。
  - `pub fn default_data_dir() -> PathBuf`（C8）。
  - `pub enum CoreError { NotInitialized, AlreadyInitialized, NotFound(String), InvalidArgument(String), PeerOffline, Rejected(String), Io(std::io::Error), Store(StoreError), KeyStore(KeyStoreError), Proto(String), Tls(String) }` + `impl From<CoreError> for lanmsg_api::ApiError`（按名称映射到 `ErrorCode`；`Store/KeyStore/Proto/Tls` → `Internal`；`Io` → `Io`）。

依赖：`cargo add -p lanmsg-core lanmsg-proto --path crates/lanmsg-proto`、`lanmsg-api --path crates/lanmsg-api`、`tokio@1 --features full`、`thiserror@2`、`tracing@0.1`、`directories@5`；dev：`tempfile@3`。

> **顺序说明**：`Identity` 依赖 `Store`，因此 `identity.rs` 及其测试 `identity_persists_device_key_and_initializes_once` **在 Task 6 的 Step 3 末尾实现**；本任务只实现 `error.rs`、`paths.rs`、`keystore.rs`、`hlc.rs`。`CoreError::Store` 变体引用的 `StoreError` 在本任务先以 `pub enum StoreError {}` 占位于 `store/mod.rs`，Task 6 补全。

- [ ] **Step 1: 写失败测试**

```rust
// keystore.rs
#[test]
fn file_keystore_save_load_delete() {
    let dir = tempfile::tempdir().unwrap(); let ks = FileKeyStore::new(dir.path().join("keys"));
    assert_eq!(ks.load("a").unwrap(), None);
    ks.save("a", b"xyz").unwrap();
    assert_eq!(ks.load("a").unwrap().as_deref(), Some(&b"xyz"[..]));
    ks.delete("a").unwrap();
    assert_eq!(ks.load("a").unwrap(), None);
}
// hlc.rs
#[test]
fn hlc_monotonic_when_wall_clock_goes_back() {
    let mut c = HlcClock::new();
    let a = c.now(1000); let b = c.now(900);
    assert!(b > a);
}
#[test]
fn hlc_observe_remote_ahead() {
    let mut c = HlcClock::new();
    c.observe(Hlc { millis: 5000, counter: 3 }, 1000);
    let n = c.now(1000);
    assert!(n > Hlc { millis: 5000, counter: 3 });
}
// identity.rs（Task 6 完成后）
#[test]
fn identity_persists_device_key_and_initializes_once() {
    // 同一 tempdir 两次 load_or_create → device_id 相同；init_first_device 后 is_initialized；
    // 再次 init_first_device → Err(CoreError::AlreadyInitialized)；重新 load 后 cert 仍在且 verify 通过
}
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过** — `cargo test -p lanmsg-core`
- [ ] **Step 5: Commit**

---

### Task 6: SQLite 存储

**Files:**
- Create: `crates/lanmsg-core/src/store/{mod.rs,schema.rs,users.rs,convs.rs,messages.rs}`

**Interfaces:**
- Consumes: proto 类型。
- Produces（P2 及以后会直接调用这些名字）：
  - `pub struct Store { conn: std::sync::Mutex<rusqlite::Connection> }`。所有方法 `&self`、同步执行（SQLite 操作短，直接在 async 上下文调用；不跨 `.await` 持锁）。
  - `Store::open(path: &Path) -> Result<Store, StoreError>`、`Store::open_in_memory() -> Result<Store, StoreError>`：开启 WAL、`foreign_keys=ON`，执行迁移。
  - `pub fn schema_version(&self) -> Result<u32, StoreError>`（`PRAGMA user_version`）。
  - 迁移：`schema.rs` 中 `pub const MIGRATIONS: &[&str]`，第 i 个元素把版本从 i 升到 i+1，事务内执行。**后续阶段只能在数组末尾追加，不得修改已有元素。**
  - `pub enum StoreError { Sqlite(rusqlite::Error), Codec(String) }`
  - settings：`get_setting(&self, key: &str) -> Result<Option<String>>`、`set_setting(&self, key: &str, value: &str) -> Result<()>`
  - users（`users.rs`）：`pub struct UserRow { pub user: UserId, pub display_name: String, pub is_self: bool, pub updated_at: i64 }`；`upsert_user(&self, &UserRow)`、`get_user(&self, &UserId) -> Result<Option<UserRow>>`
  - devices（`users.rs`）：`pub struct DeviceRow { pub device: DeviceId, pub user: UserId, pub cert: DeviceCert, pub name: String, pub device_type: DeviceType, pub last_addr: Option<String>, pub last_path: Option<PathClass>, pub last_seen: Option<i64>, pub revoked: bool }`；`upsert_device(&self, &DeviceRow)`、`get_device(&self, &DeviceId) -> Result<Option<DeviceRow>>`、`devices_of_user(&self, &UserId) -> Result<Vec<DeviceRow>>`
  - conversations（`convs.rs`）：`pub enum ConvKindRow { SelfDevices, Dm, Guest, LocalSend, Station }`（存为小写字符串）；`pub struct ConvRow { pub conv: ConvId, pub kind: ConvKindRow, pub peer_user: Option<UserId>, pub title: Option<String>, pub created_at: i64 }`；`ensure_conversation(&self, &ConvRow) -> Result<()>`（已存在不覆盖）、`get_conversation(&self, &ConvId) -> Result<Option<ConvRow>>`、`list_conversations(&self) -> Result<Vec<ConvRow>>`
  - messages（`messages.rs`）：
    - `pub enum MsgStateRow { Sending, Sent, Delivered, Read, Failed, Recalled }`（小写字符串存储）
    - `pub struct StoredMessage { pub env: Envelope, pub state: MsgStateRow, pub received_at: i64 }`
    - `next_seqs(&self, author: &DeviceId, conv: &ConvId) -> Result<(u64, u64)>`：返回 `(seq, conv_seq)`，分别为该作者全局最大 seq+1、该会话内最大 conv_seq+1（无记录为 1）。
    - `insert_envelope(&self, env: &Envelope, state: MsgStateRow, received_at: i64) -> Result<bool>`：`INSERT OR IGNORE`，返回是否新插入；新插入时在同一事务内推进 `version_vectors`（若 `env.conv_seq == contiguous+1`，则向前扫描已存在的后续 conv_seq 直到断档）。
    - `set_message_state(&self, id: &MsgId, state: MsgStateRow) -> Result<()>`
    - `list_messages(&self, conv: &ConvId, before: Option<&MsgId>, limit: u32) -> Result<Vec<StoredMessage>>`：按 `(hlc_millis, hlc_counter, author, seq)` 降序取 `limit` 条（`before` 存在时取严格早于该消息的），返回时**升序**排列。
    - `get_message(&self, id: &MsgId) -> Result<Option<StoredMessage>>`
    - `version_vector(&self, conv: &ConvId) -> Result<VersionVector>`
    - `messages_after(&self, conv: &ConvId, author: &DeviceId, after_conv_seq: u64, limit: u32) -> Result<Vec<Envelope>>`（P2 同步用，本任务实现并测试）

**迁移 1（schema v1）**，写入 `MIGRATIONS[0]`：

```sql
CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE users(user_id BLOB PRIMARY KEY, display_name TEXT NOT NULL,
  is_self INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
CREATE TABLE devices(device_id BLOB PRIMARY KEY, user_id BLOB NOT NULL, cert BLOB NOT NULL,
  name TEXT NOT NULL, device_type TEXT NOT NULL, last_addr TEXT, last_path TEXT,
  last_seen INTEGER, revoked INTEGER NOT NULL DEFAULT 0);
CREATE INDEX devices_by_user ON devices(user_id);
CREATE TABLE conversations(conv_id BLOB PRIMARY KEY, kind TEXT NOT NULL, peer_user BLOB,
  title TEXT, created_at INTEGER NOT NULL);
CREATE TABLE messages(author BLOB NOT NULL, seq INTEGER NOT NULL, conv_id BLOB NOT NULL,
  conv_seq INTEGER NOT NULL, hlc_millis INTEGER NOT NULL, hlc_counter INTEGER NOT NULL,
  envelope BLOB NOT NULL, state TEXT NOT NULL, received_at INTEGER NOT NULL, text TEXT,
  PRIMARY KEY(author, seq), UNIQUE(conv_id, author, conv_seq));
CREATE INDEX messages_order ON messages(conv_id, hlc_millis, hlc_counter, author, seq);
CREATE TABLE version_vectors(conv_id BLOB NOT NULL, author BLOB NOT NULL,
  contiguous INTEGER NOT NULL, PRIMARY KEY(conv_id, author));
```
`messages.text` 冗余存放 `MsgBody::Text` 的文字（供列表预览与以后全文索引），`envelope` 存完整 CBOR。

依赖：`cargo add -p lanmsg-core rusqlite@0.32 --features bundled`。

- [ ] **Step 1: 写失败测试**（`store/messages.rs` 等底部，用 `Store::open_in_memory()`）

```rust
#[test]
fn migrations_set_user_version() { assert_eq!(store().schema_version().unwrap(), MIGRATIONS.len() as u32); }
#[test]
fn insert_envelope_is_idempotent() {
    let s = store(); let e = env(1, 1);
    assert!(s.insert_envelope(&e, MsgStateRow::Sent, 0).unwrap());
    assert!(!s.insert_envelope(&e, MsgStateRow::Sent, 0).unwrap());
}
#[test]
fn version_vector_tracks_contiguous_prefix_only() {
    let s = store();
    s.insert_envelope(&env(1, 1), MsgStateRow::Sent, 0).unwrap();
    s.insert_envelope(&env(3, 3), MsgStateRow::Sent, 0).unwrap();
    assert_eq!(s.version_vector(&CONV).unwrap().0[&AUTHOR], 1);
    s.insert_envelope(&env(2, 2), MsgStateRow::Sent, 0).unwrap();
    assert_eq!(s.version_vector(&CONV).unwrap().0[&AUTHOR], 3);
}
#[test]
fn list_messages_pages_backwards_in_ascending_order() {
    // 插入 hlc 为 1..=5 的 5 条；limit 2 → hlc [4,5]；before=第4条 limit 2 → [2,3]
}
#[test]
fn next_seqs_starts_at_one_and_increments() { /* 空库 (1,1)；插入 seq=1 conv_seq=1 后 (2,2)；换一个 conv → (2,1) */ }
#[test]
fn reopen_file_db_keeps_data() { /* tempdir 文件库，插入后 drop 再 open，get_message 仍在 */ }
```
测试辅助 `store()`、`env(seq, conv_seq)`（hlc.millis = seq）、常量 `CONV`、`AUTHOR` 在测试模块内定义；签名可用全零（存储层不验签）。

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**，然后实现 Task 5 中定义的 `Identity`（`identity.rs`）并补上其测试
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 7: 局域网发现

**Files:**
- Create: `crates/lanmsg-core/src/discovery/{mod.rs,memory.rs,multicast.rs}`

**Interfaces:**
- Consumes: proto `Announce`、consts。
- Produces:
  - `pub enum DiscoveryMode { Multicast, Memory(MemoryBus), Disabled }`（C6）
  - `#[derive(Clone)] pub struct MemoryBus`：`new() -> Self`；内部 `broadcast::Sender<(Vec<u8>, SocketAddr)>`。
  - `pub struct DiscoveryEvent { pub announce: Announce, pub from: SocketAddr, pub via_multicast: bool }`
  - `pub struct Discovery`：
    - `pub fn spawn(mode: DiscoveryMode, self_device: DeviceId, announce_rx: watch::Receiver<Option<Announce>>, events: mpsc::Sender<DiscoveryEvent>) -> Discovery`
    - `announce_rx` 的值为 `None` 时不发送任何通告（未初始化或隐身）；值变化时立即通告一次。
    - `pub fn shutdown(&self)`
  - 行为：
    - 接收：CBOR 解析 → `Announce::verify()` → 丢弃 `cert.device == self_device`、`proto != PROTO_VERSION` 的包 → 发出 `DiscoveryEvent`。解析/验签失败用 `tracing::debug!` 记录后丢弃。
    - 发送：启动时、值变化时、每 `ANNOUNCE_INTERVAL` 一次；收到某设备首次（或距上次回应 >10 秒）的通告时，向 `from` 单播回一次自己的通告。
    - Multicast 模式：`socket2` 绑定 `0.0.0.0:DISCOVERY_PORT`（`SO_REUSEADDR`，Unix 加 `SO_REUSEPORT`），对 `if-addrs` 枚举出的每个非回环 IPv4 网卡 `join_multicast_v4(DISCOVERY_V4, iface_ip)`，发送时对每个网卡 `set_multicast_if_v4` 后发送；IPv6 同理（`DISCOVERY_V6`，按网卡 index）。每 `IFACE_POLL_INTERVAL` 重新枚举网卡，有变化则重新加入组播并立即通告。IPv6 失败只记日志，不影响 IPv4。
    - Memory 模式：发送 = 往 bus 发 `(bytes, 伪造的 from 地址 127.0.0.1:<监听端口>)`。为此 `Announce.ports[0]` 即监听端口，Memory 模式下 `from` 取 `127.0.0.1:ports[0]`。

依赖：`cargo add -p lanmsg-core socket2@0.5 --features all`、`if-addrs@0.13`。

- [ ] **Step 1: 写失败测试**（`discovery/mod.rs`，Memory 模式）

```rust
#[tokio::test]
async fn memory_bus_delivers_announce_to_other_node() { /* 两个 Discovery 共享 bus，A 设置 Some(announce)，B 在 5 秒内收到 DiscoveryEvent 且 announce.cert.device == A */ }
#[tokio::test]
async fn ignores_tampered_and_self_announces() {
    // A 收不到自己的通告；手工往 bus 注入一份改了 display_name 但未重签的通告 → B 收不到
}
#[tokio::test]
async fn none_announce_sends_nothing() { /* A 的 watch 为 None，B 在 500ms 内收不到任何事件 */ }
```
测试中构造合法 `Announce` 的辅助函数放 `discovery/mod.rs` 的测试模块中（生成 UserKey/DeviceKey → issue cert → 签 announce）。

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: 手工冒烟（不入 CI）**：在有组播的局域网两台机器上留到 Task 12 的 e2e 脚本里验证。
- [ ] **Step 6: Commit**

---

### Task 8: TLS 配置与路径等级判定

**Files:**
- Create: `crates/lanmsg-core/src/net/{mod.rs,tls.rs,path.rs}`

**Interfaces:**
- Consumes: `DeviceKey::to_pkcs8_der()`。
- Produces:
  - `pub fn make_tls_configs(device: &DeviceKey) -> Result<(Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>), CoreError>`
    - 用 rcgen 以设备密钥生成自签证书（CN = 设备 hex 前 16 位，有效期 100 年）。
    - 服务端要求客户端证书（自定义 `ClientCertVerifier`），客户端用自定义 `ServerCertVerifier`；两者都**接受任意**证书链，但必须用 provider 的 `verify_tls13_signature` 校验握手签名，并只允许 TLS 1.3。
    - SNI 固定 `"lanmsg"`；verifier 忽略名称。
  - `pub fn peer_device_id(conn: &rustls::CommonState) -> Result<DeviceId, CoreError>`：从对端证书中用 x509-parser 取 SubjectPublicKeyInfo，要求算法为 Ed25519（OID 1.3.101.112），返回 32 字节公钥。
  - `pub const TLS_SNI: &str = "lanmsg";`
  - `path.rs`：
    - `pub struct LocalIface { pub name: String, pub addr: IpAddr, pub prefix: u8 }`
    - `pub fn local_ifaces() -> Vec<LocalIface>`（if-addrs，排除回环）
    - `pub fn classify(peer: IpAddr, ifaces: &[LocalIface], via_multicast: bool) -> PathClass`：`via_multicast` → `Lan`；peer 与任一网卡同网段 → `Lan`；peer 为回环 → `Lan`；peer 为私有 IPv4（10/8、172.16/12、192.168/16）、CGNAT（100.64/10）、IPv6 ULA（fc00::/7）、链路本地 → `Vpn`；其余 → `Internet`。（隧道网卡识别在 P5 扩展本函数。）

依赖：`cargo add -p lanmsg-core rustls@0.23`、`tokio-rustls@0.26`、`rcgen@0.13`、`x509-parser@0.16`、`ipnet@2`。

- [ ] **Step 1: 写失败测试**

```rust
// tls.rs
#[tokio::test]
async fn mutual_tls_exposes_both_device_ids() {
    // 两把 DeviceKey，各自 make_tls_configs；在 127.0.0.1:0 上 TlsAcceptor/TlsConnector 握手；
    // 服务端 peer_device_id == 客户端 device_id，客户端看到的 == 服务端 device_id
}
#[tokio::test]
async fn client_without_cert_is_rejected() { /* 用不带客户端证书的 rustls ClientConfig 连接 → 握手失败 */ }
// path.rs
#[test]
fn classify_table() {
    let ifaces = vec![LocalIface { name: "eth0".into(), addr: "192.168.1.10".parse().unwrap(), prefix: 24 }];
    assert_eq!(classify("192.168.1.20".parse().unwrap(), &ifaces, false), PathClass::Lan);
    assert_eq!(classify("10.8.0.2".parse().unwrap(), &ifaces, false), PathClass::Vpn);
    assert_eq!(classify("100.101.1.1".parse().unwrap(), &ifaces, false), PathClass::Vpn);
    assert_eq!(classify("8.8.8.8".parse().unwrap(), &ifaces, false), PathClass::Internet);
    assert_eq!(classify("8.8.8.8".parse().unwrap(), &ifaces, true), PathClass::Lan);
    assert_eq!(classify("127.0.0.1".parse().unwrap(), &ifaces, false), PathClass::Lan);
}
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 9: 控制连接与连接管理器

**Files:**
- Create: `crates/lanmsg-core/src/net/{conn.rs,manager.rs}`
- Test: `crates/lanmsg-core/src/net/manager.rs` 测试模块

**Interfaces:**
- Consumes: Task 4 `FrameCodec`、Task 8 TLS、Task 5 `Identity`。
- Produces:
  - `pub struct PeerInfo { pub device: DeviceId, pub cert: Option<DeviceCert>, pub display_name: String, pub addr: SocketAddr, pub dial_addr: SocketAddr, pub path: PathClass, pub caps: Vec<String>, pub outbound: bool }`（`addr` = TCP 对端实际地址；`dial_addr` = `(addr.ip(), hello.listen_port)`，所有回拨与地址记录只用 `dial_addr`）
  - `pub enum ConnEvent { Up(PeerInfo), Down(DeviceId), Frame(DeviceId, Frame) }`
  - `pub struct ConnManager`（`Clone`，内部 Arc）：
    - `pub async fn bind(identity: Arc<RwLock<Identity>>, port: u16, hello: watch::Receiver<HelloInfo>, accept_filter: Arc<dyn Fn(&PeerInfo) -> bool + Send + Sync>, events: mpsc::Sender<ConnEvent>) -> Result<ConnManager, CoreError>`：在 `0.0.0.0:port` 监听（port 被占用且非 0 时回退 0 并 `warn!`）。
    - `pub struct HelloInfo { pub display_name: String, pub cert: Option<DeviceCert> }`；发出的 `Hello.listen_port` 由 ConnManager 填入自身 `local_addr().port()`。
    - `pub fn local_addr(&self) -> SocketAddr`
    - `pub async fn connect(&self, addr: SocketAddr, via_multicast: bool) -> Result<DeviceId, CoreError>`：已连接则直接返回该设备。
    - `pub async fn send(&self, device: &DeviceId, frame: Frame) -> Result<(), CoreError>`：未连接 → `CoreError::PeerOffline`。
    - `pub fn peers(&self) -> Vec<PeerInfo>`、`pub fn is_connected(&self, device: &DeviceId) -> bool`
    - `pub async fn disconnect(&self, device: &DeviceId)`、`pub async fn shutdown(&self)`
  - 连接流程（`conn.rs`）：TCP → TLS → 双方各发 `Hello` → 校验：
    1. `peer_device_id(tls) == hello.cert.device`（cert 为 `None` 时跳过此条，且该连接标记为"未初始化对端"）；
    2. `hello.cert.verify()` 通过；
    3. `hello.proto == PROTO_VERSION`；
    4. 对端设备 ≠ 自己；
    5. `accept_filter(&peer_info)` 为真（Service 用它实现隐身与将来的吊销/屏蔽）。
    任何一条失败：发送 `Frame::Error { code, message }` 后关闭，不发 `Up` 事件。
  - 读写：每条连接一个读任务（解码帧 → `ConnEvent::Frame`，`Ping` 自动回 `Pong` 不上报），一个写任务（`mpsc` 队列，容量 256）。每 `PING_INTERVAL` 发 Ping，`PING_TIMEOUT` 内无任何入站帧则关闭。关闭时发 `ConnEvent::Down`（仅当曾发过 `Up`）。
  - 重复连接裁决：同一设备已有连接时又建立新连接，保留"拨号方 DeviceId 较小"的那条，关闭另一条；被关闭的那条不产生 `Up`/`Down`。

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn two_managers_connect_and_exchange_frames() { /* A.connect(B.local_addr) → 双方各收到一次 Up；A.send(B, Ping 以外的 Presence(Online)) → B 收到 Frame */ }
#[tokio::test]
async fn simultaneous_dial_keeps_one_connection() {
    // A、B 同时 connect 对方（tokio::join!）；等待 1 秒后 A.peers().len()==1、B.peers().len()==1；
    // 双方各自收到的 Up 事件恰好 1 次
}
#[tokio::test]
async fn rejects_hello_cert_mismatching_tls_key() {
    // 让 A 的 HelloInfo.cert 使用另一台设备的合法证书 → B 不产生 Up，A 收到 Error 后连接关闭
}
#[tokio::test]
async fn accept_filter_false_rejects_peer() { /* B 的 filter 恒 false → 无 Up */ }
#[tokio::test]
async fn down_event_when_peer_shuts_down() { /* 连接后 B.shutdown() → A 5 秒内收到 Down(B) */ }
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**（连续跑 5 次 `cargo test -p lanmsg-core net:: -- --test-threads=1` 均通过，确认不 flaky）
- [ ] **Step 5: Commit**

---

### Task 10: 文字消息收发

**Files:**
- Create: `crates/lanmsg-core/src/msg.rs`、`crates/lanmsg-core/src/testutil.rs`、`crates/lanmsg-core/src/service/views.rs`
- Create: `crates/lanmsg-core/src/service/mod.rs`（本任务只含 `pub mod views;`，Task 11 补全）
- Modify: `crates/lanmsg-core/src/lib.rs`（`pub mod msg; pub mod service; #[cfg(test)] pub mod testutil;`）

**Interfaces:**
- Consumes: Store（Task 6）、HlcClock、ConnManager、Identity。
- Produces:
  - `pub struct Messaging { store: Arc<Store>, identity: Arc<RwLock<Identity>>, clock: Arc<Mutex<HlcClock>>, conns: ConnManager, events: broadcast::Sender<lanmsg_api::Event> }`
  - `pub fn new(...) -> Messaging`（参数即上述字段）
  - `pub async fn send(&self, conv: &ConvId, body: MsgBody) -> Result<MsgId, CoreError>`：
    1. 未初始化 → `NotInitialized`；会话不存在 → `NotFound`。
    2. `next_seqs` → 构造 Envelope（hlc = clock.now(墙钟)）→ 设备密钥签名 → `insert_envelope(Sending)`。
    3. 发 `Event::MessageAdded` + `Event::ConversationUpdated`。
    4. 计算投递目标 `targets(conv)`：DM → 对端用户所有**已连接**设备；SelfDevices → 本人其他已连接设备（P1 实际为空）。
    5. 对每个目标 `conns.send(SyncPush { envelopes: vec![env] })`。没有任何目标在线时状态保持 `Sending`（P2 加待发队列）；至少一个 send 成功则置 `Sent`。
  - `pub async fn on_frame(&self, from: DeviceId, frame: Frame) -> Result<(), CoreError>`：
    - `SyncPush`：对每个 envelope：`verify()` 失败 → 丢弃并 `warn!`；作者设备未知（store 中无 DeviceRow）→ 丢弃；`clock.observe(env.hlc)`；会话不存在则按 DM 规则创建（`ConvId::dm(self_user, author_user)` 必须等于 `env.conv`，否则丢弃）；`insert_envelope(Delivered)`；**仅当新插入时**发 `MessageAdded`/`ConversationUpdated`。处理完回 `Ack { ids }`（包含重复的，以便对方确认）。
    - `Ack`：对每个 id，若是自己发的且状态为 `Sending`/`Sent` → 置 `Delivered`，发 `MessageUpdated`。
    - 其他帧：忽略（交给后续阶段的模块）。
  - `pub fn open_dm(&self, peer_user: &UserId) -> Result<ConvId, CoreError>`：`ensure_conversation(Dm)` 并返回 id。
  - 设备入库时机：ConnManager 发 `Up(PeerInfo)` 时，Service 把 `cert` 写入 `devices`、`display_name` 写入 `users`（见 Task 11）。
  - `service/views.rs`（本任务实现，Task 11 追加 `peer_view`）：
    - `pub fn message_view(store: &Store, self_device: &DeviceId, m: &StoredMessage) -> MessageView`：`author_device_name` 取 devices 表 name（无则 device short）；`is_mine` = 作者用户为自己；`body`：Text → `Text`，其余 P1 不会出现的类型 → `Unsupported`；`state` 由 `MsgStateRow` 一一映射。
    - `pub fn conv_summary(store: &Store, self_user: Option<&UserId>, conv: &ConvRow, peer_online: bool) -> ConversationSummary`：DM 标题 = 对端用户显示名（无则 user short）；SelfDevices 标题 = `"我的设备"`；`last_text`/`last_hlc_millis` 取最新一条；`unread` 本阶段恒 0（P2 实现）。
  - `testutil.rs`：
    ```rust
    pub struct Node { pub store: Arc<Store>, pub identity: Arc<RwLock<Identity>>, pub conns: ConnManager,
        pub msg: Messaging, pub events: broadcast::Receiver<Event>, pub user: UserId, pub device: DeviceId,
        _dir: tempfile::TempDir }
    pub async fn node() -> Node;                 // 已 init_first_device、端口 0、内存 Store、路由任务已启动
    pub async fn pair_of_nodes() -> (Node, Node); // 两个 node，A.conns.connect(B 地址)，等待双方 Up
    pub async fn next_event(rx: &mut broadcast::Receiver<Event>, pred: impl Fn(&Event) -> bool) -> Event; // 5 秒超时 panic
    ```
    `node()` 的路由任务：`Up(p)` → `upsert_device`/`upsert_user`；`Frame(d, f)` → `msg.on_frame(d, f)`。

- [ ] **Step 1: 写失败测试**（`msg.rs` 测试模块，可用两个真实 ConnManager + 两个内存 Store；为减少样板，写一个测试辅助 `pair_of_nodes() -> (Node, Node)`，放在 `crates/lanmsg-core/src/testutil.rs`，`#[cfg(test)] pub mod testutil;`）

```rust
#[tokio::test]
async fn text_message_reaches_peer_and_gets_delivered() {
    // A.open_dm(B.user) → A.send(Text "你好") → B 的事件流 5 秒内出现 MessageAdded，text == "你好"；
    // A 的事件流随后出现 MessageUpdated 且 state == Delivered；B 的 store 中该会话 1 条消息
}
#[tokio::test]
async fn duplicate_push_is_idempotent() {
    // 把同一个已签名 envelope 通过 on_frame 喂给 B 两次 → B 的 MessageAdded 事件只有 1 个，store 中 1 条
}
#[tokio::test]
async fn forged_envelope_is_dropped() { /* 改 text 后不重签 → B 无事件、无入库 */ }
#[tokio::test]
async fn send_without_peer_online_stays_sending() { /* B 下线后 A 发送 → 状态 Sending，返回 Ok */ }
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: Commit**

---

### Task 11: Service 组装

**Files:**
- Create: `crates/lanmsg-core/src/service/handlers.rs`、`crates/lanmsg-core/src/platform.rs`
- Modify: `crates/lanmsg-core/src/service/{mod.rs,views.rs}`、`crates/lanmsg-core/src/lib.rs`
- Create: `crates/lanmsg-core/tests/two_nodes.rs`

**Interfaces:**
- Consumes: 以上全部。
- Produces:
  - `lanmsg-core`：C6 的 `ServiceConfig`（**全部字段**，含 P5 才使用的 `localsend_port`，P1 中不读取它）、`Service`（`start`/`handle`/`subscribe`/`local_addr`/`shutdown`）；`PlatformHooks` 本阶段为空结构体 `#[derive(Default, Clone)] pub struct PlatformHooks {}`（P2 增加字段）。
  - `handle` 中**本阶段未实现**的请求一律返回 `ApiError { code: ErrorCode::Internal, message: "未实现：<变体名>".into() }`——这是唯一允许的"未实现"占位，后续阶段替换；不得使用 `todo!()`。
  - `views.rs` 追加：`pub fn peer_view(p: &PeerInfo, self_user: Option<&UserId>) -> PeerView`（`addr` 用 `dial_addr`）。
  - Service 内部启动顺序：打开 Store → `Identity::load_or_create` → TLS 配置 → `ConnManager::bind` → `Discovery::spawn` → 路由任务（`ConnEvent` → `Up` 时写库并发 `PeerOnline`；`Down` 发 `PeerOffline`；`Frame` 交给 `Messaging::on_frame`；`DiscoveryEvent` → 若未连接且 `self_device < announce.cert.device` 则 `connect`）→ 对每个 `static_peers` 后台拨号（失败按 `RETRY_MIN`..`RETRY_MAX` 指数退避重试，直到连上；连上后断开再重试）。
  - announce 内容：已初始化且状态非 `Invisible` 时为 `Some(..)`，`ports = vec![local_addr.port()]`。
  - accept_filter：状态为 `Invisible` 时，只接受 `cert.user == self_user` 的对端；对端 `cert` 为 `None` 时本阶段一律拒绝（P2 配对时放开）。
  - 已实现的请求：`GetSelf`、`Init{mode: FirstDevice}`（`JoinExisting` 返回 `ApiError { code: Internal, message: "未实现：Init(JoinExisting)" }`）、`SetStatus`（更新 announce 与 accept_filter，并向所有连接发 `Presence`）、`ListPeers`、`ListConversations`、`ListMessages`、`OpenDm`、`SendText`、`AddManualPeer`（解析 `ip:port` 或 `host:port`，`connect`）、`Shutdown`。
  - Response 一律按 C5 的具名单字段形状构造（如 `Response::SelfInfo { value }`）。
  - `PlatformHooks` 定义在 `crates/lanmsg-core/src/platform.rs`（`pub mod platform;`，crate 根 `pub use platform::PlatformHooks;`），P2 Task 4 在此文件扩展。
  - `Init{FirstDevice}` 的完整动作（同一流程内依次完成）：`identity.init_first_device` → `upsert_user(UserRow{ user, display_name, is_self: true, updated_at: now })` → `set_setting("display_name", ..)`、`set_setting("device_name", ..)` → `upsert_device`（自己的 DeviceRow）→ `ensure_conversation(ConvRow{ conv: ConvId::self_devices(&user), kind: SelfDevices, peer_user: None, title: None, created_at: now })` → 更新 announce 与 HelloInfo。
  - Init 时 `device_type`：编译目标为 android/ios → `Phone`，否则 `Desktop`。

- [ ] **Step 1: 写失败测试**（`crates/lanmsg-core/tests/two_nodes.rs`）

```rust
#[tokio::test]
async fn two_services_discover_and_chat_over_memory_bus() {
    // bus = MemoryBus::new()；A、B 各用 tempdir + FileKeyStore + listen_port 0 + Memory(bus.clone())
    // 两边 handle(Init{FirstDevice}) → 5 秒内 A 收到 PeerOnline(B) 且 ListPeers 含 B
    // A: OpenDm(B.user_id) → SendText("hello") → B 收到 MessageAdded；B: ListMessages 返回 1 条
}
#[tokio::test]
async fn static_peer_connects_without_discovery() { /* DiscoveryMode::Disabled，B.static_peers = [A.local_addr()] */ }
#[tokio::test]
async fn uninitialized_service_rejects_send() { /* 未 Init 时 SendText → ErrorCode::NotInitialized */ }
#[tokio::test]
async fn invisible_hides_from_other_users() {
    // A 设 Invisible 后 B 重启（新 Service 同一 data_dir）→ 3 秒内 B 收不到 PeerOnline(A)
}
#[tokio::test]
async fn init_first_device_creates_self_conversation() { /* Init 后 ListConversations 含 kind SelfDevices、title "我的设备" 的会话；GetSelf.display_name 为传入值 */ }
#[tokio::test]
async fn restart_keeps_identity_and_history() { /* shutdown 后同 data_dir 重启：GetSelf.user_id 不变，ListMessages 仍有历史 */ }
```

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过** — `cargo test -p lanmsg-core --test two_nodes`
- [ ] **Step 5: Commit**

---

### Task 12: `lanmsgd`（无头）、IPC 与 `lanmsg-cli`

**Files:**
- Modify: `crates/lanmsg-api/src/ipc.rs`（加 `endpoint_name`、`IpcClient`、`IpcListener` 封装）
- Create: `crates/lanmsgd/src/{main.rs,ipc_server.rs}`、`crates/lanmsgd/tests/daemon.rs`
- Create: `crates/lanmsg-cli/src/main.rs`
- Create: `scripts/e2e-two-daemons.sh`

**Interfaces:**
- Consumes: `Service`（C6）、`IpcCodec`。
- Produces:
  - `lanmsg_api::ipc::endpoint_name(data_dir: &Path) -> String`（C7）
  - `IpcClient`（C7）：`connect`、`call`、`events`。`call` 用自增 id 关联响应；连接断开时所有挂起调用返回 `ApiError { code: Io, .. }`。
  - `pub struct IpcListener`：`bind(endpoint: &str) -> io::Result<IpcListener>`、`accept() -> io::Result<IpcStream>`（基于 `interprocess::local_socket::tokio`）。Unix 下若 socket 文件已存在：先尝试连接，连得上说明已有实例运行 → 返回 `ErrorKind::AddrInUse`；连不上则删除残留文件后绑定。
  - `lanmsgd` 命令行（clap）：`--data-dir <PATH>`（默认 C8）、`--port <u16>`（默认 `SERVICE_PORT`）、`--peer <ADDR>`（可重复）、`--no-multicast`、`--log <LEVEL>`（默认 info）。启动：初始化 tracing（同时写 `<data_dir>/logs/lanmsgd.log`，按天滚动可不做，单文件追加即可）→ `Service::start` → `IpcListener::bind`（`AddrInUse` → 打印 `lanmsgd 已在运行` 并 `exit(2)`）→ 每个 IPC 连接：读 `ApiRequest` → `service.handle` → 回 `ApiResponse`；同时把 `subscribe()` 事件以 `IpcFrame::Event` 推送给该连接。收到 `Request::Shutdown` 或 Ctrl-C → `service.shutdown()` 后退出 0。
  - `lanmsg-cli` 子命令（全部通过 IpcClient）：

    | 子命令 | 行为 |
    |---|---|
    | `init --name <显示名> [--device-name <名>]` | `Init{FirstDevice}`，device_name 默认主机名 |
    | `self` | 打印 `SelfInfo` |
    | `peers` | 表格：device_short、display_name、device_name、addr、path |
    | `convs` | 表格：conv、title、unread、last_text |
    | `dm <user_id>` | `OpenDm`，打印 conv id |
    | `send <conv> <text>` | `SendText`，打印 msg id |
    | `log <conv> [--limit N]` | 打印 `[HH:MM:SS] 名字: 文本`（默认 limit 50） |
    | `watch` | 持续打印事件（每行一个 JSON） |
    | `add-peer <addr>` | `AddManualPeer` |
    | `status <online|invisible|busy>` | `SetStatus` |
    | `shutdown` | `Shutdown` |

    公共参数 `--data-dir`（用于计算端点）。连接失败时提示 `无法连接 lanmsgd（端点 …），请先启动 lanmsgd` 并以退出码 1 结束。`watch` 用 `serde_json` 输出——为此 `cargo add -p lanmsg-cli serde_json@1`（CLI 专用，已在此登记）。

依赖：`cargo add -p lanmsg-api interprocess@2 --features tokio`、`tokio@1 --features full`、`tokio-util@0.7 --features codec`、`ciborium@0.2`、`serde@1 --features derive`、`thiserror@2`；`lanmsgd`/`lanmsg-cli`：`clap@4 --features derive`、`anyhow@1`、`tracing-subscriber@0.3`、`tokio@1 --features full`。

- [ ] **Step 1: 写失败测试**（`crates/lanmsgd/tests/daemon.rs`，用 `env!("CARGO_BIN_EXE_lanmsgd")` 启动子进程，`--no-multicast --port 0 --data-dir <tempdir>`）

```rust
#[tokio::test]
async fn daemon_serves_get_self_over_ipc() {
    // 启动子进程 → 轮询 IpcClient::connect 最多 5 秒 → call(GetSelf) → initialized == false
    // → call(Init{FirstDevice}) → call(GetSelf).initialized == true → call(Shutdown) → 子进程 5 秒内退出码 0
}
#[tokio::test]
async fn second_daemon_refuses_to_start() {
    // 同一 data_dir 启动第二个 lanmsgd → 5 秒内以退出码 2 结束；第一个仍可 call(GetSelf)
}
```
Windows 命名管道名按 C7 带 data_dir 摘要，因此不同 `--data-dir` 的实例互不干扰，测试可以并行。

- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过** — `cargo test -p lanmsgd`
- [ ] **Step 5: 写 `scripts/e2e-two-daemons.sh`**：在同一台机器上用两个 data_dir、两个端口（`--port 0` 不便于互指，故用 53330/53331）启动两个 `lanmsgd --no-multicast`，B 带 `--peer 127.0.0.1:53330`；用 `lanmsg-cli` 分别 `init`，B `peers` 能看到 A，B 取 A 的 user_id（`lanmsg-cli --data-dir A self` 输出中解析）`dm` 后 `send`，A `log` 看到消息；最后两边 `shutdown`。脚本 `set -euo pipefail`，任一步失败非零退出。
- [ ] **Step 6: 运行脚本** — `bash scripts/e2e-two-daemons.sh`，Expected: 末行打印 `E2E OK`。
- [ ] **Step 7: Commit**

---

## 阶段验收

- [ ] `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` 全部通过。
- [ ] `bash scripts/e2e-two-daemons.sh` 输出 `E2E OK`。
- [ ] **手工**（两台同局域网 Linux/Windows 机器，不加 `--no-multicast`）：各自 `lanmsgd`、`lanmsg-cli init`，3 秒内 `lanmsg-cli peers` 看到对方，`path` 为 `Lan`，互发文字成功；把其中一台切 `status invisible`，另一台重启后看不到它。把结果（两台机器 OS、耗时）写进阶段报告。
- [ ] 检查改动文件中无 `todo!`/`unimplemented!`/`#[ignore]`/无编号 TODO：`grep -rnE 'todo!|unimplemented!|#\[ignore\]|TODO' crates/ | grep -v 'TODO(#'` 无输出。
