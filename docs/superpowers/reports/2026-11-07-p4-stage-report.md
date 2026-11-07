# LanMsg P4（Android）阶段验收报告（2026-11-07）

结论：P4 计划的 14 个任务（Task 1–13 与 8A）全部完成，每个任务经独立审阅（实施与审阅均为 sonnet 子代理，审阅不通过即进入修复轮）；手机 ↔ 电脑联调发现的缺陷已修复并复审；阶段总审结论为“可以进入下一阶段”。剩余为需要真机的手工验收（第 6 节）。

## 1. 提交清单（`git log 974c43c..HEAD`）

| 任务 | 内容 | 提交（首个 → 修复轮） | 审阅轮数 |
|---|---|---|---|
| Task 1 | `lanmsg-api` uniffi feature 与 `lanmsg-ffi` | c623f89 | 1 |
| Task 2 | Android 工程骨架与构建脚本 | f102651 | 1 |
| Task 3 | Kotlin core 桥（CoreApi、密钥库、缩略图、本地偏好） | cdcbfb9 → 30d631c | 2 |
| Task 4 | 移动端 UI 架构文档 `docs/ui-architecture.md` | f6d4f07 → 656d663、567e02d | 2 |
| Task 5 | 主题、导航、文案、通用组件 | 2c9c13d → ecdec2f、0240371 | 2 |
| Task 6 | 引导、会话列表、聊天、图片查看 | fc8ad84 → 1ef7df4 | 2 |
| Task 7 | 附件、系统分享、文件落地 | 584ef9e → bd0b604、967b923 | 2 |
| Task 8 | 文件视图 | 005700f → 473f24f | 2 |
| Task 8A | 消息搜索与转发 | 74c829a → 742d69d | 2 |
| Task 9 | 联系人、请求、我、设置 | 568ea77 → 5febef2、b7e26d1 | 2 |
| Task 10 | 我的设备与配对（扫码 + 数字比对） | 85258d9 → f83f979、16e3260 | 2 |
| Task 11 | CoreHolder、前台服务、后台档位 | 535916f → 3361470、4c209c0、f2b8a00、c0af374、398b003 | 4（第 4 轮经用户批准） |
| Task 12 | 通知 | c3638e4 → 1eacc04 | 2 |
| Task 13 | Release 构建、签名、混淆 | 4ad85b5 | 1 |
| 计划修订 | 头像取色 djb2 % 6、不用动态取色 | dd60917（P4/Plan） | — |
| 联调修复 | 见第 3 节 | b5d12cc、eece4b7、71df75b、ebd716c、c59fdb8、155a002、bd737fb | 已复审 |
| 收尾 | 总审低优先级项 | 837352b 及之后（P4/Cleanup） | 总审覆盖 |

过程中的决定（用户确认）：
- 日志与异常消息一律英文；Kotlin 侧已执行。Rust 侧既有中文在 P4 后单独清理（同时界面改为按 ErrorCode 显示本地化文案）。
- LanMsg 使用专用模拟器 `emulator-5556`（AVD `lanmsg-test`），`emulator-5554` 留给 WindInput 会话。
- Task 11 超出三轮限制再修一轮。

## 2. 自动检查结果（总审亲自运行）

| 检查 | 结果 |
|---|---|
| `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace`（含 `lanmsg-ffi`） | 481 通过，0 失败 |
| `scripts/e2e-acceptance.sh` | ACCEPTANCE OK（13 场景；联调修复后运行） |
| `./gradlew --rerun-tasks :app:testDebugUnitTest :app:lintDebug` | 254 通过；lint 0 error |
| `bash scripts/build-android.sh release` | 成功；三 ABI 的 `liblanmsg_ffi.so` 均在包内 |
| `docs/ui-architecture.md` §6 与 `strings.xml` | 157 键，零差异 |
| Review Focus 1–5 | 均有对应测试（CoreHolderTest、ChatViewModelTest、OutboxImporterTest、PairJoinScreenTest、NotifierTest） |
| 安全 | 分享只收 content URI 且拒绝本应用 URI；FileProvider 只暴露 `lanmsg/downloads/`、`outbox/`、`Download/LanMsg/`；deep link 白名单（`requests`、`chat/<conv>`，≤256）；前台服务仅 `specialUse`；无 GMS/ML Kit 依赖；密钥 AndroidKeyStore AES-GCM 包装 |

## 3. 手机 ↔ 电脑联调（emulator-5556 ↔ 本机 lanmsgd + lanmsg-cli）

网络：`adb forward/reverse` 建立双向连通，`lanmsgd --headless --no-multicast --peer 127.0.0.1:53330`。

| 场景 | 首次结果 | 处理 |
|---|---|---|
| S1 手机以数字比对加入电脑 | 失败：候选能列出、选中报“未发现该设备” | B1 修复（eece4b7）：`PairTarget::Device` 发现表未命中时回退已连接对端；新增无组播静态对端配对测试 |
| S2 电脑→手机文字 | 通过；配对产生的系统消息显示“不支持的消息类型” | B2 修复（eece4b7）：System 消息渲染为文本 |
| S3 手机→电脑文字 | 通过 | — |
| S4 双向文件（图片、5MB、小文件） | 通过，sha256 一致；下载失败无提示 | B5 修复（c59fdb8、bd737fb）：下载失败一次性提示 |
| S5 通知 | 消息通知不弹 | 根因：设置中“消息通知”开关处于关闭（非代码缺陷），打开后 5 秒内弹出（已实测）；同时发现 B4：“我的设备”会话中来自自己其他设备的消息两端都不通知——改为只排除本机发出（b5d12cc、71df75b） |
| S6 强制停止后恢复 | 通过，无 database is locked / Address already in use | — |

其他修复：配对完成后设备列表刷新（155a002）；CLI `log` 显示文件名与大小（ebd716c）。

## 4. 截图

预览站 http://192.168.5.11:8765/ ，分组 `P4-Task5` … `P4-Task13`、联调截图在 scratchpad `p4-shots/joint/`、`joint-fix/`。

## 5. 已知限制与后续事项

- **Rust 清理任务（P4 后，用户已定）**：日志与错误消息改英文、界面按 ErrorCode 本地化；`source_label`（“来自 X · 已下载”）与 System 文本改为结构化字段由各端拼装；core 提供“已被所有目标下载”的确认接口，Android“清理已分享副本”据此只删安全的副本（当前只删 7 天前）；配对二维码 URI 去掉 IPv6 链路本地等无用地址以缩小二维码。
- **网络**：入站连接的数据通道按 `peer_addr.ip():listen_port` 回拨，在 NAT/端口转发下不通（真实局域网无影响）；建议后续改为控制连接内多路复用或让对端出站。`SettingsDto.unicast_targets` 目前未被使用。
- **测试脚本**：`scripts/e2e-two-daemons.sh` 不读 `CARGO_TARGET_DIR`、端口写死；`XDG_RUNTIME_DIR` 路径过长会超出 `sun_path`。
- **签名**：无 `keystore.properties` 时 release 使用 debug 签名（预期行为，正式分发前需配置）。
- **未在真机验证**：扫码解码、Doze 下的省电档、大文件（200MB）传输、重启手机后自启。

## 6. 需真机验收项（用户手工）

前提：电脑运行桌面版或 `lanmsgd`，手机与电脑在同一 Wi-Fi；安装 `android/app/build/outputs/apk/release/app-release.apk`。

1. **扫码配对**：全新安装 → 引导选「加入我已有的设备」→ 扫电脑配对二维码。预期：配对成功，出现「我的设备」会话，历史补齐，设备列表自动出现电脑。
2. **数字比对**：拒绝相机权限 → 「改用数字比对」→ 选电脑 → 两端数字一致后确认。预期：配对成功、不崩溃。
3. **传输**：手机发 3 张照片 + 1 个 200MB 视频；电脑发文字、1 张图、1 个 100MB 文件。预期：电脑收齐；手机图片自动下载并进相册，100MB 文件显示「未下载」，点击后有进度并完成；下载中断网应弹出错误提示。
4. **实时档通知**：设置选「实时」并允许通知 → 锁屏 5 分钟 → 电脑发文字。预期：10 秒内弹出消息通知（标题为设备名），点击进入该会话。若不弹：确认设置中「消息通知」开关开启，并查看 `adb logcat -s Notifier` 是否有 `Notification suppressed`。
5. **省电档**：拔电、锁屏、连非信任 Wi-Fi。预期：常驻通知消失，电脑发消息后 20 分钟内收到通知；把当前 Wi-Fi 加入信任（需定位权限）后常驻通知恢复。
6. **关闭档**：选「关闭」后退出 App。预期：无常驻通知；重新打开后消息补齐。
7. **系统分享**：相册选两张图 →「分享」→ LanMsg → 选会话。预期：电脑收到两张图。
8. **免打扰与可见会话**：会话设免打扰后电脑发消息；正在看某会话（含打开图片查看页）时电脑发该会话消息。预期：都不弹通知，其他会话照常弹。
9. **搜索与转发**：搜索电脑发来的文字并点击结果；文件页长按已下载文件「转发」到另一会话。预期：跳转正确，对方收到。
10. **强制停止与重启（实时档）**：强制停止后再打开；重启手机。预期：服务自动恢复、无崩溃；`adb logcat | grep -E "database is locked|Address already in use|UnsatisfiedLink|ClassNotFound"` 无输出。
