# LanMsg P5（扩展与发布）阶段验收报告（2026-11-25）

结论：P5 计划的 23 个任务（Task 1–23，含 3B）全部完成并合入 `main`，每个任务经独立审阅（实施与审阅均为 sonnet 子代理，审阅不通过即进入修复轮；安全相关任务按安全视角审阅），审阅发现的应修项均已修复并复核。自动化检查全部通过；发布流水线在 `main` 上做过一次完整 dry-run（8 个构建 job 全绿）。**v0.1.0 已于 2026-11-26 公开发布**（仓库已转公开，GitHub Actions 账单限制随之解除）。剩余为需要真机的手工互操作验收（第 7 节）；发布过程中发现并修复的缺陷见第 8 节。

## 1. 提交清单

范围：`git log 5114d3d..HEAD`（首个 P5 提交 2026-11-08）。每行为“首个 → 修复轮/收尾”；合并提交不单列。

| 任务 | 内容 | 提交 |
|---|---|---|
| Task 1 | P5 数据表迁移与协议扩展 | 1f21699 → bf8faae |
| Task 2 | 网卡枚举、隧道识别与 `ListInterfaces` | fe875fc → b10e59b |
| Task 3 / 3B | 地址簿、重连器、手动添加；NAT 反向数据通道 | f2c64a2 → 24a6f8a、5602007、7f77a69；08563c9 → 5aab10e、b7f3fa7 |
| Task 4 | 单播通告目标与网段扫描 | 3977a88 → 86b21a4、333d291、2b7d5d2 |
| Task 5 | 本地文件登记与中转站主机端 | 43f4985 → 764d5a8、481e730 |
| Task 6 | 中转站同步、拉取端与按名下载 | 7083eac → 0b35182、3db6ff3 |
| Task 7 | 桌面端中转站页面与“发送到” | 79f2c05 → 50cea3b、be9db27、5749b97 |
| Task 8 | Android 中转站页面 | 9ee943f → 0c0bddb、24b0add |
| Task 9 | `lanmsg-localsend` 类型与发现 | 5114d3d → 1f372fc、1ad1323 |
| Task 10 | LocalSend 接收服务与发送客户端 | 041a213 → f56c294、b679c5a、4bf866f |
| Task 11 | core 的 LocalSend 桥（含首次联系入口） | a7f1e58 → fcadf11、e9e678d |
| Task 12 | Web 会话密码学与测试向量 | 19ccf72 → 5cbe7ce、a10b8e6 |
| Task 13 | Web 页面与 `lanmsg-web-assets` | 4736076 → 568ffa4、0bda949 |
| Task 14 | Web 服务端、会话与传输 | 9fa8c2e → 9ab8a13、240374d、cebb213 |
| Task 15 | 桌面端网页传输、网络设置与 LocalSend 端口设置 | 310d610 → 6b597fb、4a95f9b、ad02bf9 |
| Task 16 | Android 网页传输、网络设置与 LocalSend 端口设置 | f2e2cd7 → 113225b、39fb97b、5829b81 |
| Task 17 | wind-installer 新能力（防火墙、快捷方式 args / sendto） | 外部仓库 wind-installer v0.131.0 |
| Task 18 | `lanmsgd` 防火墙检测与一键放行 | cf48643 → 546d077、264ddfd、6fa097e、bb7994b |
| Task 19 | Windows 打包：安装包与 scoop | 10bb71e → b608614、ab9b24f |
| Task 20 | Linux 安装脚本、systemd 与构建 | e6314ca → 0a5440a |
| Task 21 | macOS 打包脚本 | 99796da → 40deb2a、7d29885 |
| Task 22 | CI 与发布流水线 | 6b041ad → 7406e37、9114e43、f0c80cd、a6a4341 |
| Task 23 | 应用内更新（23a 桌面/core/托盘/CLI；23b Android） | 7536c68 → e957ced；b25a68d → 58472cf |
| 收尾 | README/CHANGELOG 校正；CI 低项；Rust 低项；内存测量口径 | d7a01ce；ab8d806、dd02083；40ee55c、1c4fa04、a1a02c2、cd36aef；809f55a |

外部依赖：`windui` 以 git tag `v0.32.0` 依赖；`wind-installer v0.131.0` 已由另一会话发布。

## 2. 自动检查结果

| 检查 | 结果 |
|---|---|
| `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace`（合入收尾分支前的 main） | 898 通过，0 失败 |
| 收尾分支 `p5-polish` 的 fmt / clippy / `check-no-cjk` / 5 个 crate 测试 | 全部通过，839 通过，0 失败 |
| `node --test web/test/*.test.mjs` | 14 通过 |
| `bash scripts/check-packaging.sh --fast`、`bash packaging/linux/test-install.sh` | OK |
| 占位符扫描（`todo!`/`unimplemented!`/`#[ignore]`/`TODO`，仅跟踪文件） | 无命中（`lanmsg_ffi.kt` 为 uniffi 生成物，已被 gitignore） |
| `main` 的 CI（Linux / Windows / Android 三 job） | 全绿：`b3a797f`、`e797380`、`5a4dd19`、`bbc1acf`（`1983e3c` 当时因账单问题未能运行，仓库转公开后随后续提交一并验证，见第 6 节） |
| Release 流水线（首次 dry-run、修复后 dry-run、真实 tag 发布） | 三次均全绿；dry-run 为 `main` 上 8 个构建 job 全绿（Linux 桌面与无头 × 双架构、Android APK、macOS 通用、Windows × 双架构），Publish 按预期跳过 |
| `scripts/e2e-acceptance.sh`（合入全部收尾后的 main，先重新构建二进制） | ACCEPTANCE OK，13 个场景全部 PASS（文字、二维码/短码配对、离线补齐、陌生人请求、文件/文件夹传输与续传、吊销、撤回已读、回环吞吐） |
| 网页临时会话真实浏览器端到端（Chromium headless，非 CI；`docs/superpowers/reports/p5-web-e2e/`） | key 模式 11/11、code 模式 13/13：握手与宿主允许、双向文字、3.5 MB 中文名加密上传、5 MB 宿主文件解密下载（SHA-256 均一致）、宿主停止后页面显示“已结束”、无 console 错误 |

## 3. 独立审阅与安全结论

- 每个任务 2–5 个提交（含审阅修复轮），审阅发现并修复的典型问题：LocalSend 桥的 HTTPS 证书 pin 与 IP 绑定、PeerOnline 事件绕过屏蔽、Web 会话 seq 窗口与握手 IP 绑定、地址簿防滥用上限、`netsh` 输出 OEM 解码与规则块判断、Windows 命名管道“已有实例”识别。
- T23a（应用内更新）单独做了安全审阅：安装动作全部为数组式 argv、无 shell 拼接；下载先校验 SHA256 再落盘（缺校验和直接拒绝，fail-closed）；下载地址默认硬编码 GitHub 固定仓库，`LANMSG_UPDATE_URL` 仅作环境变量覆盖；rustls 统一 aws-lc-rs，依赖树中已无 ring。已知信任边界：`SHA256SUMS` 与安装包同源，无独立签名（与 wind-installer 现有约束一致）。
- 真实浏览器首轮跑出的唯一问题：自动请求 `/favicon.ico` 得到 404（无害的 console 噪声）。已加同源 `favicon.svg` 并在页面引用（CSP 的 `img-src` 不含 `data:`），带单测。这也是网页端 JS 第一次在真实浏览器里整条链路运行，此前只有密码学向量与 Rust 侧集成测试。
- 实施中发现的运行时必败缺陷（审阅与冒烟初看都漏掉）：更新客户端的 TLS 构造在 `rustls-tls-no-provider` 下缺根证书，更新检查与下载全部失败；已补 `webpki-roots` 并带回归测试。教训：新增外部调用必须做真实运行冒烟，而不是只看编译与单测。
- 收尾审阅：`LANMSG_KEYSTORE=file`（为 Windows CI 并发凭据写入而加的测试开关）收敛为仅 debug 构建生效；`SHA256SUMS` 解析容错与资产名路径校验；LocalSend 屏蔽判定在存储出错时 fail-closed。
- 遗留低项（均不阻塞）：`localsend_bridge.rs` 另有 3 处 `.ok().flatten()` 仍是 fail-open；Windows 资产名未拦保留名/`:`（只有发布者能触发）；`SHA256SUMS` 的 `*` 二进制模式前缀未兼容（当前流水线用文本模式，不触发）；Android 保存按钮复用 `net_unicast_save` 键名；更新断线复位后 core 侧 `update_busy` 可能仍为真（可恢复错误）。

## 4. 性能与资源（设计 §17，同机实测）

测量环境：Linux 无显示（headless），48 核共用机器，负载 14 左右；`lanmsgd` release（LTO、strip、panic=abort）21.8 MB（P3 时 16.5 MB）。

| 指标 | P3 版 | P5 后 | 说明 |
|---|---|---|---|
| 匿名堆 Private_Dirty，空库空闲 | 1.5 MB | 2.1 MB | 真正由 LanMsg 代码持有的内存 |
| 匿名堆，写入 1000 条消息后 | 3.0 MB | 3.7 MB | 增量 0.7 MB，远低于 8 MB 判据 |
| 文件映射常驻页（代码段） | 10.4 MB | 11.7 MB | 二进制变大 5.3 MB，只有被访问的页常驻，且可被系统回收 |
| rss，写入 1000 条消息后 | 13.0 MB | 14.9 MB | |
| 空闲 CPU（30 秒窗口） | 0 tick | 0 tick | 无后台忙轮询 |
| 线程数 | 5 | 5–6 | tokio worker ×2、blocking、async-io、主线程；建中转站后 +1 |

P5 功能对内存的影响（同一进程，单独运行）：启动 2.3 MB；开启网页传输 2.6 MB；建中转站并放入两个 8 MB 文件与一段文字 5.2 MB（与复制 16 MB 文件的哈希/拷贝路径有关）；执行一次更新检查（404 失败，首次加载 TLS）+0.13 MB。

**测量口径更正（重要）**：P3 报告与脚本原先用 `private`（Private_Clean + Private_Dirty）作失败判据。P5 验收时它报出 15.8 MB 超过 15.36 MB 阈值，经排查不是堆回归：Private_Clean 含只被本进程映射的二进制代码页，**同一状态下读数取决于当时有没有别的进程同时映射同一个二进制**（实测 2.2 / 9.8 / 10.8 / 15.8 MB 都出现过），而且二进制从 16.5 MB 长到 21.8 MB 也会抬高它。已改为以匿名堆（`dirty_kb`，阈值 `HEAP_THRESHOLD_KB` 默认 8192）作失败判据，`private` 总量降为超过 15360 KB 时的 WARN（提交 809f55a）。同机 isolated 运行时 `private` 总量为 13.1 MB，仍低于 15 MB。
吞吐：e2e 的 `bench_local`（回环 256 MB 文件）为 261.9 MB/s（机器负载约 14，共用，数值受影响；P3 同项为 267–278 MB/s，在噪声范围内，无回归迹象）。千兆/2.5G 真实网卡吞吐、真窗口常驻内存、界面 10 Hz 定时器空闲 CPU 仍需有显示与真实网络的机器测。

## 5. 已知限制与后续事项

- 没有真窗口常驻内存数据：本机无 DISPLAY，界面进程只测到离屏截图峰值（rss 14.5 MB），不等于常驻内存。
- 仓库为私有时，`install.sh`、scoop、应用内更新的匿名下载均返回 404；`install.sh` 失败时已提示可能原因（含私有仓库与 `LANMSG_BASE_URL`），应用内更新检查失败时提示“仓库可能为私有或离线”。
- 发布流水线的 `ubuntu:20.04` 容器已过标准支持期，apt 归档可用性与 glibc 基线（切 22.04 会抬高）需在首个正式发布前复核。
- Android 不做应用内静默安装：发现新版本只通知并引导到发布页下载。
- 更早登记的后续低项：`undo_recall`、`bundle_sent` 先置位、复合游标、`ImageViewer` 并发、GUI 自己提权成功后不通知 daemon（非中英文 Windows 上新连接会再见防火墙横幅）、TalkBack 语义、`LANMSG_BASE_URL` 语义。

## 6. GitHub Actions 账单

2026-11-25 把收尾合入 `main`（`1983e3c`）后，CI 三个 job 均“未启动”，注释为 *recent account payments have failed or your spending limit needs to be increased*，原因是私有仓库的 Actions 分钟数按量计费，本阶段的完整 Release dry-run（8 个 job，含 macOS 与 Windows）和多轮 CI 消耗较大。2026-11-26 用户把仓库转为公开后限制解除，`main` 的 CI 恢复运行并全绿。

## 7. 需真机验收项（用户手工）

以下为 P5 计划“阶段验收”中的手工项，**均未执行**，需要你在真机上完成并记录设备型号与版本：

1. **LocalSend 互操作**：LanMsg ↔ LocalSend 官方 App（Android 与 Windows 各一）互发文字与 3 个文件（含中文名、一个大于 1 GB）；LocalSend 侧能看到 LanMsg 设备；本机同时运行 LocalSend 时设置页出现端口占用提示且仍可向 LocalSend 发送。
2. **Web 临时会话**：Android Chrome 扫码（key 模式）与 iOS Safari 手输配对码（code 模式）各完成一次双向传输；30 分钟空闲后页面显示“会话已结束”；访客需在本机逐个允许。（桌面 Chromium 上的两种模式与“宿主停止后已结束”已由第 2 节的浏览器 e2e 覆盖；仍需真机浏览器、真实 Wi-Fi 与 30 分钟空闲、大于 1 GB 文件的确认弹窗。）
3. **中转站**：台式机建站放入 3 个文件（引用与复制各有）→ Android 查看清单并下载；台式机关机后 Android 显示“源离线”；Android 往中转站上传一张照片，台式机目录出现该文件。
4. **跨网段**：两台经 Tailscale 等互通但不在同一局域网的电脑，手动添加后互发消息，联系人行显示“VPN”角标；第三台本人设备经地址交换自动连上。
5. **安装渠道**：Windows 安装包与 scoop（`scoop install <本地 json>`）各装一次，并核对防火墙一键放行（`netsh` 的显示/添加/删除，安装器重复运行幂等，中文目录）；Ubuntu `curl | sh` 安装、NAS（或容器）无头版安装；Android APK 安装；macOS 记为外部依赖。
6. **应用内更新**：v0.1.0 已发布，但目前没有更旧的版本可以升级，所以“检查到新版本 → 下载 → 校验 → 安装”的整条链路还没有真实走过（只验证了对真实 Release 的检查、semver 比较与“已是最新”）。需要发布下一个版本后，用 0.1.0 验证各渠道安装（scoop 版应弹出命令行窗口执行 `scoop update lanmsg` 后退出）。

## 8. v0.1.0 发布与发布后验证（2026-11-26）

流程：`bump-version` 前发现 `CHANGELOG.md` 里有一节从未发布的 `[0.1.0]` 占位（会让脚本拒绝升版本），先删除，再把 `[Unreleased]` 归档为 `[0.1.0] - 2026-11-26`；`main` CI 绿后打 tag `v0.1.0`，Release 流水线全绿并发布。

**真实匿名下载验证**（不带 token）：全部资产 SHA-256 与 `SHA256SUMS` 一致；APK 由正式签名证书签名（指纹与 keystore 一致，`versionCode 100`）；`curl | sh` 无头安装与 `uninstall.sh` 卸载可用；Linux 桌面包无缺失库、glibc 要求 ≥ 2.30；scoop 清单由机器人自动提交回 `main`，版本、URL、哈希与发布一致；应用内更新检查对真实 Release 返回“已是最新”，安装渠道识别为 `Manual`（这也是上周 T23a 里 `builder error` 的最终确认：TLS 客户端修复后对真实 GitHub 正常）。

**发现的缺陷与处理**

- **A（影响发布）——Windows 安装器漏发。** 首次发布缺少 `lanmsg-windows-{x64,arm64}-setup.exe`：`wind-packer` 默认把安装器命名为 `<output_name>-<version>.exe`（带版本号），而发布资产、scoop 与应用内更新都约定不带版本号的名字；`actions/upload-artifact` 的 `if-no-files-found: error` 只在**所有**路径都缺失时才报错，zip 在就静默通过，所以 Windows job 与发布 job 都是绿的。此前 `check-packaging.sh` 只校验了清单里的 `output_name`，从未校验打包器**实际产出**的文件名。修复：`build-dist.ps1` 给 `wind-packer build` 显式传 `--output` 并立即检查产物存在；Windows job 上传前逐个核对；新增 `scripts/check-release-assets.sh` 与 `assets` job（dry-run 里也会跑，因为 Publish 在 dry-run 时被跳过，守卫不能只在第一次真发布时才执行）；`check-packaging.sh` 校验清单与 `release.yml` 文件头一致，并正反测试该脚本确实能抓出缺失。修复分支先在 Windows runner 上 dry-run 验证，再下载 artifact 实测文件名与 PE 头。
- **B（体验）——`install.sh --prefix` 试装会启用真实 systemd 服务。** 其实脚本早就有 `LANMSG_NO_SYSTEMD=1` 开关，只是 `--help` 没提；已补上。仍遗留：设了该开关时脚本依然会把 unit 文件写进 `~/.config/systemd/user/`（不启用）。
- **C（小）——`lanmsg-cli` 与 `lanmsgd` 不认 `--version`。** 已加（取自单一版本源），并有测试。
- ARM64 安装器被 `file` 识别为 x86-64 是**有意的**：wind-installer 只发布 x64 的 stub，ARM64 Windows 通过仿真运行它，安装进去的文件是真正的 ARM64 构建（`build-dist.ps1` 里有注释说明）。

**删除重发。** 修复合入后，用户选择删除已公开的 v0.1.0（Release 与 tag）并在修复后的提交上重新发布同一版本号，而不是发 v0.1.1。后果：重发的资产哈希与首次发布不同（除 `uninstall.sh` 外全部重新构建，另新增两个安装器），**首次发布期间下载过资产的人，其校验和与现在的 `SHA256SUMS` 对不上**；scoop 清单由重发流程自动更新为新哈希（已核对一致）。首次发布与重发之间约 40 分钟没有可用的 Release（`latest` 为 404）。

**没有验证到的**：Windows 安装器与 scoop 安装（本机是 Linux，只验证了文件名、PE 头、嵌入的清单版本与哈希，没有运行）；macOS dmg 的挂载与运行；真机上的 Android（只在模拟器上验证）；升级路径（第 7 节第 6 项）。

**发布版 APK 在模拟器上的实测**（Android 16 / x86_64，AVD `lanmsg-test`，安装的是从发布页匿名下载的正式签名 APK，截图在 `reports/p5-release-apk/`）：安装、启动、向导（填名字、选“这是我的第一台设备”）、创建身份、授予通知权限、进入主界面全程无崩溃（logcat 无 `FATAL`/`UnsatisfiedLink`），说明 Rust core 经 UniFFI 在 Android 上正常加载。App 内“我 → 设置 → 网络”页有 LocalSend 开关与端口（默认 53317）、手动添加设备、单播通告目标、网段扫描，网卡列表正确；联系人页有“新的请求”与“附近的人”。

与桌面 `lanmsgd`（release 同源的 debug 构建，另一进程）互传：手机在网络页手动添加 `10.0.2.2:<端口>` 后桌面看到手机并建立连接；桌面发联系人请求，手机收到“新的请求 (1)”（“PC 想加你为联系人”，四个选项齐全）并同意；**电脑→手机**文字与 3 MB 文件，文件按策略自动下载，字节与源文件一致；**手机→电脑**文字原样到达，经 App 的附件按钮与系统文件选择器发送 4 MB、2 MB 文件，桌面端下载后字节与源文件一致，2 MB 的那次桌面端 3 秒内自动下载完成。桌面启动自动检查在真实环境里自行执行（`update check ok, already up to date (v0.1.0)`），无 D-Bus 时密钥库按设计回退到文件库。

**这次互传测试的两个局限，写在这里避免被误读**：
- 模拟器在 NAT 后面，且桌面与手机同机、都想用 53320 端口：第一次手机→电脑的下载失败为 `source offline`，并且没有自动下载。原因是桌面把手机的回拨地址认成了 `127.0.0.1:53320`，连回了**它自己**。这是同机拓扑的伪影，不是产品缺陷；把桌面改到 53330 并用 `adb forward` 让桌面能真正连到手机后，同样的操作就自动下载成功。**NAT 后的手机把文件发给电脑**这个真实场景应由 T3B 的反向数据通道（DataDialBack）处理，本次拓扑无法构造出它，仍需要真机（手机与电脑在不同网段/NAT 下）验证。
- Android 系统分享入口（`ACTION_SEND`）没有用 `adb am start` 的方式验证通过：该方式构造的 intent 没有 URI 读取授权，LanMsg 读取别人的 MediaStore URI 时得到 `SecurityException`，App 按设计跳过并记 `Import failed, skipped`（不崩溃），这是 Android 权限模型在正常工作，不是 App 的缺陷，但也没有因此验证到真实的“相册→分享”路径，仍需真机。

**过程中的一次越界副作用**：用 `--prefix` 试装 `install.sh` 时，脚本在真实用户会话里启用并启动了 systemd 用户服务，并创建了默认数据目录 `~/.local/share/lanmsg`（确认是 02:13 新建、此前不存在，无既有数据）；已用发布的 `uninstall.sh` 卸载服务并删除该目录。之后的安装验证都在隔离的 `HOME`/`XDG_*` 下带 `LANMSG_NO_SYSTEMD=1` 进行，并核对用户环境未被触碰。

