# P5 计划修订附录（2026-11-08）

原计划 `2026-10-28-05-p5-extensions-release.md` 写于 P1–P4 实现之前。本附录记录与现有代码对照后的修订，**与原计划冲突时以本附录为准**。执行每个任务前先读原计划对应任务，再读本附录的「全局修订」与该任务条目。

## 用户决定（2026-11-08）

- 跨网段 NAT 回拨问题在 P5 一起修（新增 Task 3B）。
- LocalSend 兼容保持默认开启（`settings.rs` 现状不变）。
- windui 在 CI 中改为 git tag 依赖（仓库 `https://github.com/huanfeng/wind-ui-rust`），见 Task 22 条目。

## 全局修订

1. **英文日志与错误**：原计划中所有中文错误/日志字符串（第 20、326、328、400-402、482、725、776、943、946、989、1160 行等）一律改为英文；Rust 源必须通过 `python3 scripts/check-no-cjk.py`。界面只按 `ErrorCode` 显示本地化文案（桌面 `error_text.rs`、Android `ui/common/ErrorText.kt`），不显示 `ApiError.message`。测试断言 `ErrorCode`，不断言消息文本。
2. **界面文案只在资源文件**：桌面 `crates/lanmsg-desktop/src/strings.rs`、托盘 `crates/lanmsgd/src/strings.rs`、Android `strings.xml`（同步 `docs/ui-architecture.md` §6 文案表）、网页 `web/strings.js`（新文件，见 Task 13）。core 不生成任何用户可见文字：会话标题、系统事件、来源说明等用结构化字段（参照 `ConversationSummary.last_kind`、`FileItemView` 来源字段、`SystemEventView`、`RequestPreviewView`）。磁盘上的目录名用英文（如 `Station`、`Web uploads`，与 `My devices` 同一处理）。
3. **Unimplemented 兜底**：每实现一个请求，就把它从 `service/handlers.rs` 的 `Unimplemented` 兜底分支移除。
4. **C5 增补许可**：原计划"本阶段不新增 C5 类型"放宽为：允许**追加**变体/字段（不得改名、删除、改语义），并在提交说明中列出。已知需要：`SystemEventView` 的 LocalSend PIN 事件（Task 11）、Task 3B 的帧。
5. **签名漂移**：`views::file_item_view` 现为 6 个参数（views.rs:30）；桌面没有 `file_state_area`，只有私有 `state_area(ui, &[FileItemView])`（message_row.rs），需要时改为 `pub(crate)`；`launch.rs::open_gui` 签名为 `(data_dir, open_conv)`；Android `Conditions` 现有 4 个字段；`RequestView.preview` 是 `RequestPreviewView`。一律以代码为准。
6. **热点文件串行**：`service/handlers.rs`、`service/mod.rs`、desktop `model.rs`/`strings.rs`、`strings.xml` 同一时间只允许一个任务修改。
7. **隧道网段判 Vpn**（Task 2）：采纳原计划；但 `redial.rs` 的节奏判断（`on_local_subnet`）对隧道网卡上的已知设备仍按"同网段"节奏重连，不退到 5 分钟。陌生人规则 7 的影响由 Task 3 的"手动添加设备放行"解决。

## 任务条目

- **T1**：迁移为 `MIGRATIONS[3]`（核实 schema.rs 当前 3 个元素）。`Frame` 追加 `AddressBook(50)`、`Station(60)` 及 `TYPE_*` 常量，同步 `type_id_values_match_spec` 测试。表 `known_addresses`、`manual_peers` 按 T3 修订后的列定义（见下）。
- **T2**：`LocalIface` 加 `tunnel` 字段，全仓 4 处构造同步。`ListInterfaces` 实现并移出兜底。
- **T3（重写）**：不新建 `Reconnector`，在 `service/redial.rs` 上增量修改：
  - `store/addresses.rs`：`known_addresses(device, addr, path, last_ok, source)`，每台设备最多 8 条；`manual_peers(addr, device NULL, added_at)`。函数名照原计划（`record_address`、`addresses_of`、`add_manual_peer`、`list_manual_peers`、`bind_manual_peer`、`is_manual_device`）。
  - `ConnEvent::Up` 已用 `dial_addr` 写 `devices.last_addr`（mod.rs:639），保留，并额外调用 `on_peer_up_record` 写 `known_addresses`（source = Direct）。
  - redial 取候选地址：`addresses_of(device)`，再加上 `devices.last_addr` 兜底，按顺序逐个尝试。
  - `AddManualPeer`（handlers.rs:438 已存在）改为持久化到 `manual_peers`；启动时对每条调用 `register_static_peer`（mod.rs:802），重启后仍会重连。
  - 地址簿 `net/addrbook.rs` 的 `build_for` / `merge` 规则照原计划；收到新地址后唤醒对应设备的 redial 任务。
  - Trust 规则 7 与规则 6 的"手动添加设备放行"照原计划。
  - 测试照原计划 6 个，名字不变。
- **T3B（新增）：NAT 下数据通道反向建立**。问题：`open_data` 固定向 `peer.dial_addr` 拨号（manager.rs:430），对端在 NAT 或端口转发后面、只能由它连进来时，文件传输失败。
  - 新帧 `Frame::DataDialBack { token: [u8; 16] }`，`TYPE_DATA_DIAL_BACK = 44`。
  - 新数据帧 `DataFrame::DialBack { token }`，在 `DataHello` 之后发送。
  - `open_data`：直拨失败（连接错误，或 3 秒超时）时，生成随机 token，登记一个 oneshot 等待者，经控制连接发出 `DataDialBack`，最多等 10 秒，超时返回 `PeerOffline`。
  - 收到 `DataDialBack` 的一方：照常向对方的监听地址（控制连接的远端 IP 加 Hello 中的 `listen_port`）建立数据连接，完成 `DataHello` 后发送 `DialBack{token}`。
  - 接受方：数据连接带 token 时，交给对应的等待者，不交给 `data_handler`。token 一次性有效，未知或过期的 token 直接关闭连接。对端身份必须与发起请求的设备一致（校验证书对应的 DeviceId）。
  - 测试：`data_channel_falls_back_to_dial_back`（A 的 dial_addr 指向一个已关闭的端口，B 只能被 A 连入；A 向 B 拉取文件成功）；`dial_back_token_single_use`；`dial_back_rejects_wrong_device`。
- **T4**：复用 `send_unicast`（multicast.rs:169）与 `SCAN_MIN_PREFIX`（consts.rs:99）。core 与桌面 `parse_targets`（settings.rs:134）规则保持一致，并把 `SettingsDto.unicast_targets` 真正接入发现。
- **T5**：移动端拒绝建站：`InvalidArgument`，消息为英文。
- **T6**：`TransferEngine::download` 是 `self: Arc<Self>`，`spawn_download` 绑定 `MessageBlobRow`（transfer.rs:142、287）。先把下载来源抽象出来，再加 `download_to`。权限错误用稳定的英文键。
- **T7**：文案全部进 `strings.rs`；`--station-put` 加到 `parse_cli`（main.rs:50）；`Effect` 以现有定义为准。
- **T8**：错误状态存 `ErrorCode`。Android outbox 的 7 天清理不得删除中转站引用、主机尚未拉取的副本，要补测试。
- **T9 / T10**：照原计划，日志和错误改英文。LocalSend 协议字段以官方 v2 协议文档为准，需要联网核对。
- **T11**：`RequestView.preview` 用 `RequestPreviewView::Files{..}`；新增 PIN 系统事件变体（全局修订 4）；把 `ServiceConfig.localsend_port`（mod.rs:52、141）接入。
  - 同一指纹可能在多个 IP 下有多条记录（`LocalSend::peers()` 键为 (ip, 指纹)）；桥不得按指纹取第一条。
  - `trusted` 自动接受须同时核对：来源 IP 与该指纹上次记录的 IP 一致（`LsEvent` 的 `peer.ip`），并按 `classify` 过滤非 Lan 来源。
  - 审批通过后若发送方收到 409（会话被占用）：界面按“对方忙”处理，不重复弹窗；`prepare-upload` 待审批名额有限（全局 8、每源 IP 1，超出 429）。
  - `LocalSend::shutdown` 目前只停服务与发现，不清理进行中会话的 `.part`，也不发 `SessionFinished`；T11 接入开关时处理。
- **T13**：网页文案放 `web/strings.js`（`zh`、`en` 两套，按 `navigator.language` 选择）；`check-no-cjk.py` 扩展到扫描 `web/`，`web/strings.js` 列入白名单。页面总大小 < 300KB 的限制不变。
- **T14**：访客会话标题由 core 返回空字符串，界面按会话类型显示本地化标题；上传目录名用英文。
- **T15**：手动添加的错误走 `error_text`；扫描结果和"公网"等文案进 `strings.rs`；托盘文案进 lanmsgd `strings.rs`。
- **T16**：测试断言 `ErrorCode`；`Conditions` 在现有 4 个字段的基础上追加 `webHosting`。
- **T17**：在 wind-installer 仓库（/home/dufeng/develop/windinput/wind-installer）实施，提权复用 `util/admin.rs`。遵守该仓库 AGENTS.md 和 `--only` 提交纪律，**不推送**。
- **T18**：托盘文案进 strings.rs；`--add-firewall-rule` 的命令行输出用英文。
- **T19**：图标已有（`crates/lanmsg-desktop/assets/lanmsg.ico`、`scripts/gen-win-icon.py`），不需要占位图标；`packaging/assets` 从这里生成或引用。
- **T20**：把 `lanmsgd` 的 windui 依赖改为 optional，归入 `desktop` feature。
- **T21**：只写脚本，在 CI 的 macOS runner 上产出未签名 dmg；本地无法验证，记为外部依赖。
- **T22**：windui 依赖改为 `{ git = "https://github.com/huanfeng/wind-ui-rust", tag = "<tag>" }`。本地开发在 `.cargo/config.toml` 里用 `[patch."https://github.com/huanfeng/wind-ui-rust"]` 指回本地路径。注意：LanMsg 目前依赖 windui v0.31.0 之后的 12 个提交，其中 11 个还没推送，所以需要用户先在 windui 仓库发布新 tag（如 v0.32.0）并推送，CI 才能跑通。LanMsg 仓库地址为 `huanfeng/lanmsg`，**不推送、不创建远程仓库**，需要时由用户操作。

## 执行顺序

主线（串行，因为都要改热点文件）：T1 → T2 → T3 → T3B → T4 → T5 → T6 → T7 → T8 → T11 → T14 → T15 → T16 → T18 → T19 → T20 → T21 → T22。

旁线（与主线并行，不碰热点文件；使用独立的 CARGO_TARGET_DIR）：T9 → T10（新 crate `lanmsg-localsend`）；T12 → T13（`web/` 与 `lanmsg-web-assets`）；T17（外部仓库）。
