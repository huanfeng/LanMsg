# LanMsg P3 阶段验收报告（2026-11-06）

测量与回归环境：Linux 7.0（PVE 容器），AMD EPYC 7402 48 线程，256 GB 内存，**无 DISPLAY**（无图形会话、无 Xvfb），
`GDK_SCALE`/`QT_SCALE_FACTOR` 均未设置（缩放 1.0）。工具链 cargo 1.99.1。

## 1. 任务提交清单（`git log --grep 'Plan: P3/'`，按时间正序）

| 提交 | 说明 | 所属 |
|---|---|---|
| `9756fe4` | feat(media): 新增图片解码与缩略图 crate 及 lanmsg-thumb 进程 | Task 6 |
| `34cc78f` | fix(media): ffmpeg 路径加固、统一超时与 EXIF 方向 | Task 6 |
| `2b282aa` | feat(core): 新增平台密钥库与 SetUiFocus | Task 7 |
| `d5583d8` | fix(core): 修复 Windows 目标编译 | Fix Windows |
| `2c23a00` | fix(core): 密钥库后端一次选定并持久化，出错不再重建身份 | Task 7 |
| `2573c21` | fix(core): 密钥库标记原子写入并以文件锁防并发首次选择 | Task 7 |
| `381a2df` | build: 声明最低 Rust 版本 1.89 | Task 7 |
| `32180ac` | feat(daemon): lanmsgd 进程结构、无头判定与缩略图子进程 | Task 8 |
| `58ce40e` | feat(daemon): 托盘、通知与拉起界面，Shutdown 改为优雅退出 | Task 9 |
| `9bf5f42` | fix(daemon): 弹通知前同步当前在线状态 | Task 9 |
| `4bf7637` | feat(desktop): 界面骨架：IPC 桥、Model、单实例、演示夹具 | Task 10 |
| `334f12e` | fix(api): IpcClient 读任务退出后关闭事件流 | Task 10 |
| `1a03ad2` | fix(desktop): 断线检测改用事件流关闭，减少误判与 toast | Task 10 |
| `069946e` | feat(desktop): 首次向导与会话列表 | Task 11 |
| `f5c89a4` | fix(desktop): 向导可返回上一步并防重复提交 | Task 11 |
| `74a6b86` `9b25cda` | docs(spec): 桌面端视觉规范 v1 / v1.1 | Visual |
| `40ebf20` `1438a9c` | style(desktop): 视觉规范打磨与文字对比度修复 | Visual |
| `93bf391` | feat(desktop): 聊天主区、消息行与下载交互 | Task 12 |
| `baa8801` | fix(desktop): 下载状态即时显示、历史窗口无缺口、撤回时限引用协议常量 | Task 12 |
| `93dc5ed` | feat(desktop): 输入区支持发送文字、文件、粘贴图片与拖入 | Task 13 |
| `dbb1a09` | fix(desktop): 粘贴文件不再同名覆盖，发送失败回填草稿 | Task 13 |
| `dc9cb21` | feat(desktop): 联系人页、我的设备与配对界面 | Task 14 |
| `072c1da` `1d3d5a7` | fix(desktop): 配对取消竞态、链接与地址校验、回复按本地序号匹配 | Task 14 |
| `3c0f1ba` | feat(core): 文件视图查询、消息文字搜索与文件转发 | Task 15 |
| `d5c344f` | feat(desktop): 文件页与设置页 | Task 15 |
| `70a7fb5` | fix(desktop): 设置允许 0 并按组保存，文件页与网格排版改进 | Task 15 |
| `b954e30` | fix(core): 文件查询凑满页并移出 worker 线程，回填跳过坏行，转发校验可见性 | Task 15 |
| `f306a01` `641d8fb` `b26769b` | docs(plan): 计划修订 | Plan |

Task 16（本报告与测量脚本）单独提交。Task 1–5 在 windui 仓库（/home/dufeng/develop/windinput/wind-ui-rust，当前 HEAD `05de6c7`）。

## 2. 自动检查结果

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all --check` | 通过 |
| clippy 本机 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |
| clippy Windows | 同上 `--target x86_64-pc-windows-gnu` | 0 警告 |
| 测试 | `cargo test --workspace --no-fail-fast` | **437 通过 / 0 失败**（含 `screenshots.rs` 全部 9 个页面） |
| P1 e2e | `bash scripts/e2e-two-daemons.sh` | `E2E OK` |
| 全量验收 | `bash scripts/e2e-acceptance.sh` | `ACCEPTANCE OK`（13 场景全过） |
| 占位检查 | `grep -rnE 'todo!\|unimplemented!\|#\[ignore\]\|TODO' crates/ \| grep -v 'TODO(#'` | 无输出 |
| windui 仓库 | `cargo test`（lib 1470 + 其余 114，26 ignored 为其既有）、`cargo clippy --all-targets`、`--no-default-features`、`cargo build --examples` | 通过 |
| 脚本语法 | `bash -n scripts/*.sh` | 通过 |

三个既有脚本新增 `LANMSG_BIN_DIR` 环境变量以覆盖二进制目录（默认行为不变），本次回归借此使用独立 target 目录。

### 发现的缺陷与不稳定项（未修复，不属本任务范围）

1. **`lanmsg-core --test blobs` 的 `download_from_my_other_device_when_sender_offline` 偶发失败**：
   回归首轮失败于 `blobs.rs:405`（“B 应从 A 下载成功，实际错误 Some("源离线")”）。随后该测试单跑 5/5 通过，整个 blobs 套件连跑 3 次、全工作区再跑一次均通过。
   失败发生时机与另一会话并发编译/测试重叠，疑似时序竞态（B 在得知 A 离线前就发起下载，或 A 离线判定与持有者列表更新存在窗口）。建议单独排查，勿默认为环境噪声。
   **注意**：回归期间工作区存在并发会话的未提交改动（`crates/lanmsg-core/src/net/*`、`service/*`、`tests/blobs.rs`、`consts.rs`、新文件 `net/timing.rs`、`tests/net_heartbeat.rs`），回归是针对包含这些改动的工作区运行的，与 HEAD 可能不完全一致；该失败也可能来自这些进行中的改动，需在其提交后复跑确认。
2. windui 仓库 `cargo test` 首轮 lib 有 1 个用例失败（输出被截断，未能记下用例名），之后 4 次重跑（1470 全过）无法复现。
3. `cargo test --workspace` 默认遇到首个失败的测试二进制即停止，会使后续（含截图测试）不运行；回归时应加 `--no-fail-fast`。

## 3. 内存与空闲 CPU

脚本：`scripts/measure-memory.sh <data_dir> [seconds]`（release 构建，`LANMSG_BIN_DIR` 指向 release 目录）。
本次命令：`LANMSG_BIN_DIR=<tgt>/release bash scripts/measure-memory.sh /tmp/lm-mem 60`，退出码 0，末行 `MEASURE OK`。
机器可读结果为输出中的 `RESULT {json}` 行（亦可用 `RESULT_JSON=<file>` 落盘）。显示缩放：1.0（无 `GDK_SCALE`，无 DISPLAY）。

| 进程 / 状态 | private_kb（Private_Clean+Private_Dirty） | pss_kb | rss_kb |
|---|---|---|---|
| lanmsgd 无头，空闲 60 s（空库） | **5636** | 5758 | 7192 |
| lanmsgd 无头，写入 1000 条消息后空闲 60 s | **7524** | 7610 | 9088 |
| lanmsg 离屏截图模式（`--demo chats --screenshot`）峰值 | 10284 | 11733 | 15732（VmHWM 15796） |

- 阈值：lanmsgd private < 15360 KB，两次均通过（余量约 50% 以上）。
- **界面进程数据仅供参考**：离屏截图进程是短命进程，取运行期间轮询的各指标最大值，**不等于真窗口常驻内存**；真窗口常驻内存、DPI 缩放影响需在有显示的真机用同一脚本测（有 DISPLAY 时脚本自动改为真窗口常驻 10 秒后测量并验证 SIGTERM 后 5 秒内退出）。
- lanmsgd **常驻托盘模式**（有 DISPLAY 时脚本自动走此模式）本机无法测，列为真机项。

**空闲 CPU**：lanmsgd 无头空闲 30 秒窗口内 utime+stime 增量 = **0 tick（0 ms）**（CLK_TCK=100），无后台忙轮询。
界面进程 100 ms `on_interval`（10 Hz 定时器）对空闲 CPU 的影响：**离屏模式无法测**（进程不常驻）；有 DISPLAY 的真机上脚本会输出 `lanmsg idle_cpu window_s=10 cpu_ticks=…`，待补测。

## 4. 截图清单

全部 9 页由 `crates/lanmsg-desktop/tests/screenshots.rs` 离屏渲染（`LANMSG_SHOT_DIR` 保留产物），已存入仓库
`docs/superpowers/reports/p3-screenshots/`：

`wizard.png`、`chats.png`、`chat.png`、`contacts.png`、`devices.png`、`pair.png`、`files.png`、`files-grid.png`、`settings.png`

## 5. 已知限制与缺能力汇总

来自提交信息与代码注释：

1. **windui 取不到容器宽度**：文件页网格列数固定，整行在内容区居中（`views/files.rs:31,629`；提交 `70a7fb5`）。
2. **windui 分段控件与下拉无“变更”回调**：文件页筛选由 `tick` 周期比对信号与 Model（`views/files.rs:166`）。
3. **windui 多行输入框不支持随内容增高**：输入区固定约 3 行，超出框内滚动（`views/composer.rs:20`）。
4. **原生 Wayland 读不到剪贴板图片**：粘贴图片回调不触发（`views/composer.rs:387`），Wayland 下只能用拖入或选择文件。
5. **Linux 无 StatusNotifier Watcher 时无托盘**：windui 自行降级，通知照常发出（`lanmsgd/src/tray.rs:104`）。
6. **无头判定只看环境变量**，显示连不上由 `run_tray` 兜底降级（`lanmsgd/src/runtime.rs:236`）。
7. 组字中的 Enter 由 windui 吞掉（`93dc5ed` 说明），Windows TSF 路径待真机确认。
8. 平台文件选择框不可用时提示“可把文件拖入窗口发送”（`PICK_UNAVAILABLE`），设置页目录选择同理。
9. 自启动仅 Linux/Windows 实现，其它平台返回“当前平台不支持开机自启”（`autostart.rs`）。
10. 消息类型未知时界面显示“不支持的消息类型，请升级”。
11. 本机被吊销仅清内存身份、`undo_recall` 状态近似、`reissue_cert` 与 `SetProfile` 顺序耦合等 P2 遗留未变（见 P2 报告）。
12. 本环境无 DISPLAY：真窗口、托盘、通知、IME、拖放等均未实测；截图与 `--demo` 仅覆盖渲染。

## 6. 需真机验收项清单

环境：Windows 一台 + Linux（含 KDE）一台，同局域网。请用 release 构建，并在有显示的会话中运行
`bash scripts/measure-memory.sh <短路径> 60` 补测托盘模式与真窗口内存/空闲 CPU/退出时限（Windows 用 `measure-memory.ps1`，**见下方说明**）。

- [ ] Windows 与 Linux KDE 托盘：状态勾选（忙碌/隐身）、退出；点“退出”后进程退出、IPC 端点消失
- [ ] 系统通知弹出；**点击通知打开界面并定位到该会话**
- [ ] IME 中文输入（Windows、Linux），**Windows TSF 组字中 Enter 不误发送**
- [ ] 剪贴板图片粘贴：Windows、X11（Wayland 已知不支持）
- [ ] 拖入文件与文件夹（含一个 200 MB 文件）；接收端“下载”按钮进度/速度实时更新；完成后“打开所在文件夹”
- [ ] 单实例激活：再次启动 `lanmsg` 激活已有窗口而非开新进程
- [ ] lanmsgd 被杀后界面重连（断线提示、恢复后自动刷新）
- [ ] 自启动开关（Windows 注册表/Linux autostart 条目）生效
- [ ] 两机组播发现（不加 `--no-multicast`）与吞吐（`bench-throughput.sh remote <IP>`，千兆 ≥ 110 MB/s）
- [ ] 手工核心场景 1–6（向导/配对、互发文字与截图、关窗后托盘仍在并收通知、文件页筛选与中英文搜索与转发、忙碌不弹通知但未读增加、隐身/退出）
- [ ] Windows 上 `file:` 前缀路径传给 ffmpeg 的缩略图/探测路径
- [ ] 有显示的真机：界面进程真窗口常驻内存、DPI（100%/150%/200%）影响、10 Hz 定时器空闲 CPU
- [ ] `measure-memory.ps1`（Windows）：**本任务未提供**。计划要求该脚本，但本机无 Windows/PowerShell 环境，无法验证，未写入未经验证的脚本，需在 Windows 真机补写并验证。
- [ ] Windows 发布版无控制台窗口：双击 `lanmsg.exe`、开机自启、界面拉起 `lanmsgd` 时均不闪出黑窗
- [ ] 有界面时托盘“退出”后 `lanmsgd` 不被界面复活：横幅显示“后台服务已退出”，点“启动后台服务”后恢复连接并清除退出标记
- [ ] 关闭窗口后（托盘常驻）来新消息仍弹系统通知；界面关闭前已清焦点，不会因“界面有焦点”而吞通知
- [ ] 托盘模式下 `lanmsgd` 内存实测（`scripts/measure-memory.sh`，有图形会话、托盘常驻、界面关闭）
- [ ] `measure-memory.ps1` 需在 Windows 真机补写并验证（本环境无法验证，仍未提供）
