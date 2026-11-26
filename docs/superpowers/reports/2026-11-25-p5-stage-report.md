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
6. **应用内更新**：v0.1.1 发布后，已用发布出来的 0.1.0 二进制在隔离环境里验证了检查、下载与校验（见第 10 节）；仍未验证的是各渠道的“安装”动作本身：Windows 安装器版静默安装、scoop 版弹出命令行窗口执行 `scoop update lanmsg` 后退出、Linux 执行 `install.sh`、macOS 打开 dmg，需要对应系统的真机。

## 8. v0.1.0 发布与发布后验证（2026-11-26）

流程：`bump-version` 前发现 `CHANGELOG.md` 里有一节从未发布的 `[0.1.0]` 占位（会让脚本拒绝升版本），先删除，再把 `[Unreleased]` 归档为 `[0.1.0] - 2026-11-26`；`main` CI 绿后打 tag `v0.1.0`，Release 流水线全绿并发布。

**真实匿名下载验证**（不带 token）：全部资产 SHA-256 与 `SHA256SUMS` 一致；APK 由正式签名证书签名（指纹与 keystore 一致，`versionCode 100`）；`curl | sh` 无头安装与 `uninstall.sh` 卸载可用；Linux 桌面包无缺失库、glibc 要求 ≥ 2.30；scoop 清单由机器人自动提交回 `main`，版本、URL、哈希与发布一致；应用内更新检查对真实 Release 返回“已是最新”，安装渠道识别为 `Manual`（这也是上周 T23a 里 `builder error` 的最终确认：TLS 客户端修复后对真实 GitHub 正常）。

**发现的缺陷与处理**

- **A（影响发布）——Windows 安装器漏发。** 首次发布缺少 `lanmsg-windows-{x64,arm64}-setup.exe`：`wind-packer` 默认把安装器命名为 `<output_name>-<version>.exe`（带版本号），而发布资产、scoop 与应用内更新都约定不带版本号的名字；`actions/upload-artifact` 的 `if-no-files-found: error` 只在**所有**路径都缺失时才报错，zip 在就静默通过，所以 Windows job 与发布 job 都是绿的。此前 `check-packaging.sh` 只校验了清单里的 `output_name`，从未校验打包器**实际产出**的文件名。修复：`build-dist.ps1` 给 `wind-packer build` 显式传 `--output` 并立即检查产物存在；Windows job 上传前逐个核对；新增 `scripts/check-release-assets.sh` 与 `assets` job（dry-run 里也会跑，因为 Publish 在 dry-run 时被跳过，守卫不能只在第一次真发布时才执行）；`check-packaging.sh` 校验清单与 `release.yml` 文件头一致，并正反测试该脚本确实能抓出缺失。修复分支先在 Windows runner 上 dry-run 验证，再下载 artifact 实测文件名与 PE 头。
- **B（体验）——`install.sh --prefix` 试装会启用真实 systemd 服务。** 其实脚本早就有 `LANMSG_NO_SYSTEMD=1` 开关，只是 `--help` 没提；已补上。仍遗留：设了该开关时脚本依然会把 unit 文件写进 `~/.config/systemd/user/`（不启用）。
- **C（小）——`lanmsg-cli` 与 `lanmsgd` 不认 `--version`。** 已加（取自单一版本源），并有测试。
- ARM64 安装器被 `file` 识别为 x86-64 是**有意的**：wind-installer 只发布 x64 的 stub，ARM64 Windows 通过仿真运行它，安装进去的文件是真正的 ARM64 构建（`build-dist.ps1` 里有注释说明）。

**删除重发。** 修复合入后，用户选择删除已公开的 v0.1.0（Release 与 tag）并在修复后的提交上重新发布同一版本号，而不是发 v0.1.1。后果：重发的资产哈希与首次发布不同（除 `uninstall.sh` 外全部重新构建，另新增两个安装器），**首次发布期间下载过资产的人，其校验和与现在的 `SHA256SUMS` 对不上**；scoop 清单由重发流程自动更新为新哈希（已核对一致）。首次发布与重发之间约 40 分钟没有可用的 Release（`latest` 为 404）。

**没有验证到的**：Windows 安装器与 scoop 在真实 Windows 上的安装（本机是 Linux；v0.1.1 发布后用 Wine 运行过安装器，见第 11 节，但那不是真实 Windows）；macOS dmg 的挂载与运行；真机上的 Android（只在模拟器上验证）；Windows 与 macOS 上的桌面 GUI（真窗口只在 Linux 上验证）；升级路径（第 7 节第 6 项）。

**发布版 APK 在模拟器上的实测**（Android 16 / x86_64，AVD `lanmsg-test`，安装的是从发布页匿名下载的正式签名 APK，截图在 `reports/p5-release-apk/`）：安装、启动、向导（填名字、选“这是我的第一台设备”）、创建身份、授予通知权限、进入主界面全程无崩溃（logcat 无 `FATAL`/`UnsatisfiedLink`），说明 Rust core 经 UniFFI 在 Android 上正常加载。App 内“我 → 设置 → 网络”页有 LocalSend 开关与端口（默认 53317）、手动添加设备、单播通告目标、网段扫描，网卡列表正确；联系人页有“新的请求”与“附近的人”。

与桌面 `lanmsgd`（release 同源的 debug 构建，另一进程）互传：手机在网络页手动添加 `10.0.2.2:<端口>` 后桌面看到手机并建立连接；桌面发联系人请求，手机收到“新的请求 (1)”（“PC 想加你为联系人”，四个选项齐全）并同意；**电脑→手机**文字与 3 MB 文件，文件按策略自动下载，字节与源文件一致；**手机→电脑**文字原样到达，经 App 的附件按钮与系统文件选择器发送 4 MB、2 MB 文件，桌面端下载后字节与源文件一致，2 MB 的那次桌面端 3 秒内自动下载完成。桌面启动自动检查在真实环境里自行执行（`update check ok, already up to date (v0.1.0)`），无 D-Bus 时密钥库按设计回退到文件库。

**这次互传测试的两个局限，写在这里避免被误读**：
- 模拟器在 NAT 后面，且桌面与手机同机、都想用 53320 端口：第一次手机→电脑的下载失败为 `source offline`，并且没有自动下载。原因是桌面把手机的回拨地址认成了 `127.0.0.1:53320`，连回了**它自己**。这是同机拓扑的伪影，不是产品缺陷；把桌面改到 53330 并用 `adb forward` 让桌面能真正连到手机后，同样的操作就自动下载成功。**NAT 后的手机把文件发给电脑**这个真实场景应由 T3B 的反向数据通道（DataDialBack）处理，本次拓扑无法构造出它，仍需要真机（手机与电脑在不同网段/NAT 下）验证。
- Android 系统分享入口（`ACTION_SEND`）没有用 `adb am start` 的方式验证通过：该方式构造的 intent 没有 URI 读取授权，LanMsg 读取别人的 MediaStore URI 时得到 `SecurityException`，App 按设计跳过并记 `Import failed, skipped`（不崩溃），这是 Android 权限模型在正常工作，不是 App 的缺陷，但也没有因此验证到真实的“相册→分享”路径，仍需真机。

**桌面 GUI 的真窗口实测（Linux，Ubuntu 22.04 GNOME Wayland + Xwayland，1440×900）**：用户提供了一台调试机，我把从发布页下载的 `lanmsg-linux-x86_64.tar.gz` 传过去并在对端再校验哈希（与发布页一致），解压到 `/tmp` 下，`HOME` 与 `XDG_*` 全部隔离、`LANMSG_NO_SYSTEMD=1`、不暴露用户的 D-Bus，用 `DISPLAY=:0` 起真窗口（截图在 `reports/p5-real-desktop/`）。这是桌面 GUI 第一次以真窗口运行，此前只有离屏截图。

- **窗口与布局**：1000×680 窗口，左侧图标栏（会话 / 联系人 / 文件 / 中转站 / 设置）、会话列表、搜索框、空状态页，中文与图标渲染正常；P5 新增的“中转站”入口在图标栏里。
- **实时刷新**：另起一个隔离的 `lanmsgd`（Bob）与它建立联系人并发消息，**没有任何手工操作**，窗口里自己出现了新会话 Bob（绿色在线点）、消息预览（中文与符号正常，过长省略）、时间与未读徽标 2。
- **冷启动**：窗口出现 11–16 ms，首帧有实际内容（窗口像素出现多种颜色）92–96 ms，5 次离散约 4 ms，每轮开始前都确认窗口树为 0；设计目标 < 500 ms。前提是 `lanmsgd` 已在运行（托盘常驻、界面按需打开的场景），不含 daemon 自身的启动。
- **常驻资源（空闲）**：GUI 匿名堆 5.0 MB、rss 21.8 MB、3 个线程；**30 秒空闲 CPU 仅 10 ms**，界面 10 Hz 定时器没有造成忙轮询（P3 报告里此前一直标“需有显示的真机”）。`lanmsgd` 匿名堆 2.2 MB，与本机一致。
- **关闭**：收到 SIGTERM 后 55 ms 退出，窗口无残留，后台 daemon 保持运行。没有 DISPLAY 时 `lanmsg --help` 给出清晰的错误（“无法连接 X 服务器…Wayland 会话需要启用 XWayland”）。

**发现的新缺陷（真实桌面环境才会暴露）——锁定的系统钥匙环会让 `lanmsgd` 首次启动无限阻塞。** 调试机上 `gnome-keyring` 的 `login` collection 是锁定的（`Locked = true`，常见于自动登录或未经 PAM 解锁）。在保留真实 D-Bus 的情况下首次启动 `lanmsgd`：12 秒以上没有 `lanmsgd started` 日志，IPC 不可用，且收到 SIGTERM 后虽然打出 `termination signal received, shutting down` 却不退出；`strace` 显示它连上了 `/run/user/1000/bus`，随后一个线程每 0.5 秒 `futex_wait` 超时并反复轮询。去掉 `DBUS_SESSION_BUS_ADDRESS` 后 250 ms 就绪并按设计回退到文件密钥库。定位到 `crates/lanmsg-core/src/keystore_os.rs` 的 `probe()`：同步调用 `keyring` 的 `set/get/remove`，没有任何超时。影响：GNOME 桌面自动登录的用户，开机自启 `lanmsgd` 会卡住，托盘不出现、界面一直连不上、也无法优雅退出。本机、CI、模拟器都没有 Secret Service，所以此前所有测试都没有覆盖到这个环境。

**已修复**（`b354a74`，随后合入 main）：这是决定密钥存放位置的安全敏感逻辑，所以修复范围刻意很窄——只限定**首次选定后端**（标记文件不存在）时对系统库的调用：放进辅助线程限时 8 秒（`KEYSTORE_PROBE_TIMEOUT`），超时则**本次**回退文件库且**不写标记**，下次启动再试；已有文件库密钥时下次启动会走原有的迁移逻辑，所以回退是可逆的。已选定 `os` 后端（标记存在）之后的读写**不限时**：那时回退文件库会重建身份，必须等用户解锁，这是原设计的意图。超时与“辅助线程没能给出结果（创建失败或 panic）”都归为瞬时错误、不写标记；后端明确报告不可用仍按原行为写 `file` 标记。超时分支不删除任何文件库密钥，也不回滚系统库。
- **测试**：用会挂起的后端覆盖首次安装、已有文件库密钥、下次启动迁移、`os` 模式不受限、辅助线程 panic；撤销限时后前两个测试会卡住 60 秒以上，说明确实抓得住原缺陷。
- **独立安全审阅通过**（无 blocker/应修）：任何路径下文件库密钥都不会丢，也不存在“标记为 os 但系统库里没有密钥”的状态；审阅提出的低项 3（辅助线程失败不应永久写 file 标记）已采纳并补测试，另两个极窄的并发窗口（超时后进程仍在运行时另一个进程恰好在钥匙环解锁后做首次选定可能各自产生一份身份；被放弃的辅助线程很久之后才完成的写入可能留下旧值）记录在 `select_backend` 的文档注释里，数据不会被销毁。
- **真机验证**（同一台调试机、同一锁定的 gnome-keyring、全新数据目录；二进制是在 `ubuntu:22.04` 兼容的构建镜像里编译的无头版，需要 GLIBC 2.34）：修复前 12 秒以上没有 `lanmsgd started`；修复后 **8.1 秒**出现该日志并带一句可读的告警（`system keystore did not answer … is the keyring locked?; using the file keystore for this run and retrying on the next start`），IPC 可用，身份创建 67 ms 且密钥落在文件库，**后端标记文件不存在**，SIGTERM 后 **54 ms** 退出（修复前收到信号却不退出）。对照：无 D-Bus（后端明确不可用）时仍 56 ms 启动并写 `file` 标记，与修复前一致。
- **没有验证到**：钥匙环被解锁后下次启动把文件库密钥迁移进系统库（只有单测覆盖，因为不能替用户解锁真实钥匙环）；用户在桌面上看到解锁弹窗并输入密码的完整交互。
- **另一个相关的小问题仍未处理**：`lanmsg` 界面会在检查 X 连接**之前**就拉起 `lanmsgd`，连不上显示时会留下一个用默认数据目录运行的孤儿 daemon。

**过程中的一次越界副作用**：用 `--prefix` 试装 `install.sh` 时，脚本在真实用户会话里启用并启动了 systemd 用户服务，并创建了默认数据目录 `~/.local/share/lanmsg`（确认是 02:13 新建、此前不存在，无既有数据）；已用发布的 `uninstall.sh` 卸载服务并删除该目录。之后的安装验证都在隔离的 `HOME`/`XDG_*` 下带 `LANMSG_NO_SYSTEMD=1` 进行，并核对用户环境未被触碰。

**在调试机上的一次同类失误**：为确认二进制能运行，我在没有隔离 `HOME`/`XDG_*` 的情况下执行了 `lanmsg --help`；它会先自动拉起同目录的 `lanmsgd`（用默认数据目录），再因为没有 DISPLAY 而报错退出，于是在用户的 `~/.local/share/lanmsg` 下留下了一个孤儿 daemon 和刚创建的数据（确认目录创建于同一分钟、探查时不存在、无既有数据）。发现后立即终止进程并删除该目录，之后所有测试都在完全隔离的环境里进行。调试机收尾时核对：无 lanmsg 进程与监听端口、`/tmp` 无残留、用户目录与 systemd 无新增，用户自己运行的 windui 演示程序没有被碰。另外，我曾把窗口列表里的一个 `windui — 文件拖出` 窗口当成可能的 LanMsg 缺陷，查进程归属后确认是用户三天前自己启动的演示程序，不属于 LanMsg，没有记为缺陷。

## 9. 发布后修复的 CI 脆弱点（2026-11-26）

v0.1.0 发布并修复钥匙环缺陷后，`main` 的 CI 连续出现偶发失败，查出两个不同的根因，都与功能代码无关，但会让 CI 随机变红、掩盖真正的失败：

- **Android 单元测试：Robolectric 每次联网下载 Android SDK jar。** 失败表现为 `MavenArtifactFetcher` → `IOException`，一批无关测试（先是 `MediaStoreSaverTest`，再是 `MainActivity*`）恰好落在下载窗口里失败，此前 7 次运行都是绿的，且出问题的提交只改了文档。第一次修复（给 `~/.m2/repository/org/robolectric` 加 `actions/cache`）**下得过早**：我只在特性分支上验证了“首次未命中、重跑命中”，而 GitHub Actions 的缓存**按分支隔离**，特性分支保存的缓存 `main` 读不到；并且 `actions/cache` 默认在 job 失败时不保存，下载失败本身就让 job 失败，缓存永远存不下来。合入 `main` 后 Android 照旧失败（缓存未命中）才暴露这一点。最终修法：拆成 `actions/cache/restore` 与 `always()` 的 `actions/cache/save`（已命中则跳过）。这样保存是安全的：反编译确认 `MavenArtifactFetcher` 先下载到临时文件，完成后才 `Files.move` 到最终位置，路径里只有完整的 jar。**在 `main` 上验证**：第一次运行未命中并保存 455 MB，重跑命中（`Cache restored from key: robolectric-Linux-…`），三个 job 全绿。没有加自动重试，那会把真实的偶发测试失败也吞掉。
- **`lanmsgd` 的 `firewall_ok_is_pushed_when_rule_becomes_present` 偶发超时：服务端的真实竞态。** `serve_connection` 把读取防火墙状态基线（`firewall_missing_now`）放在 `tokio::spawn` 出去的任务里，任务被调度的时刻不确定，连接建立到任务运行之间发生的状态变化会被当成基线而不推送，客户端永远收不到那次变化。测试只是在 CI 的 2 核 runner 上更容易撞上。把基线读取挪到 `spawn` 之前。限 2 核（`taskset -c 0,1`）反复运行该测试：修复前 80 次失败 3 次（约 4%），修复后 300 次 0 次失败。（中间我曾因 zsh 通配符没匹配到测试二进制路径，得到“失败 60 次”的无效数据，发现后重做，没有采用。）没有为它写专门的回归测试：确定性地构造“状态变化落在任务被调度之前”需要控制 `tokio::spawn` 的调度，测试会很脆弱；现有测试在修复后不再偶发失败，并在代码里留下了说明基线为何必须在 `spawn` 之前读取的注释。

教训（已写入记忆）：改 CI/缓存/环境类的东西，验证必须放到它真正起作用的环境（`main` 分支、对应 OS、真实桌面），特性分支或模拟环境上的成功只证明机制能工作；CI 偶发失败先看根因链，用“限核反复运行”区分真实竞态与外部网络，而不是“重跑到绿”。

## 10. v0.1.1 发布与升级路径验证（2026-11-26）

用户决定发布 v0.1.1。内容（只写用户能感知到的）：锁定钥匙环时 `lanmsgd` 首次启动不再无限阻塞；防火墙状态在界面刚连上后端的瞬间变化时界面偶尔收不到。流程与 v0.1.0 相同：`main` CI 三个 job 全绿后在该提交（`1bcb984`）上打 annotated tag，Release 流水线全绿（9 个构建 job、`Verify asset set` 资产清单守卫、`Publish`）。

**发布后验证（匿名下载）**：13 个资产（12 个约定资产 + `SHA256SUMS`，含两个 `setup.exe`，守卫没有被触发），12 个文件哈希与清单一致，`latest` 指向 v0.1.1；APK 由正式证书签名（指纹与 keystore 一致），`versionName 0.1.1`、`versionCode 101`；scoop 清单由机器人自动提交回 `main`，版本、URL 与两个架构的哈希都与发布一致。

**一处更正（我的错误）**：我最初把“`lanmsg-cli` 与 `lanmsgd` 支持 `--version`”写进了 0.1.1 的更新说明，并随 Release 公开发布；随后在对发布版 0.1.0 做升级测试时发现它的 `lanmsgd --version` 就已经输出 `0.1.0`——该改动（`3074af6`）和 Windows 安装器修复一起合入后才重发的 v0.1.0。我之前只核对了 `install.sh` 的改动是否已在 v0.1.0，漏了同一批提交里的这一项。用 `git merge-base --is-ancestor` 逐个核对后更正：钥匙环修复（`b354a74`）与防火墙基线修复（`4353b41`）确实不在 v0.1.0，保留；`--version` 在，去掉。已用 `gh release edit` 把已发布的 Release 说明改成更正后的内容（只改说明，tag 与资产没动），并提交 `CHANGELOG.md` 更正（`55fb365`）。

**升级路径（首次真实验证）**，全部在隔离的 `HOME`/`XDG_*` 下用**发布出来的 0.1.0 无头二进制**完成：
- **检查**：对真实 GitHub Release，`update check` 报告 `new version available: v0.1.1`，显示的是更正后的说明；渠道识别为 `Manual`。
- **下载与校验**：经 IPC 发 `DownloadUpdate{asset:"install.sh"}`，收到进度事件与 `UpdateDownloadFinished ok=true`，文件落在 `data/updates/`，SHA-256 与发布清单一致、字节与发布的 `install.sh` 相同、无 `.part` 残留。
- **fail-closed（用本地假 Release 服务器，`LANMSG_UPDATE_URL` 指向 127.0.0.1）**：资产内容与 `SHA256SUMS` 不符 → 拒绝（`checksum mismatch`），不留文件；`SHA256SUMS` 里没有该资产 → 拒绝（`has no entry for install.sh`），不留文件；资产名 `../evil.sh` → `InvalidArgument`，目录外无文件。T23a 安全审阅时这几条只靠代码阅读与单测确认过，现在在发布出来的二进制上端到端验证了。
- 小问题：`update check` 的 `published` 字段输出的是毫秒时间戳而不是可读日期。

**仍未验证**：各渠道的“安装”动作本身（Windows 安装器静默安装、scoop 版弹命令行窗口、Linux 执行 `install.sh`、macOS dmg）；Windows 安装器与 scoop 的真机安装、macOS dmg、Windows/macOS 上的桌面 GUI；真机 Android；NAT 后手机往电脑发文件。

## 11. Windows 安装器在 Wine 里的运行（2026-11-26）

本机有 Wine（缺 wine32，64 位程序不受影响）。用**全新的独立前缀**（`WINEPREFIX=/tmp/...`，完全不碰用户自己的 `~/.wine`，结束后核对其修改时间未变、里面没有任何 LanMsg 文件）运行**从 v0.1.1 发布页下载、哈希已校验**的 `lanmsg-windows-x64-setup.exe` 与 zip。

**验证到的**：
- Windows 版 `lanmsg-cli.exe` 与 `lanmsgd.exe`（来自 zip，以及安装后的）都能运行，输出 `0.1.1`。
- **静默安装** `setup.exe --silent`：12 个步骤走完（准备环境、停止旧进程、解压、释放文件、写卸载器清单、开机自启动、防火墙、快捷方式、卸载信息、启动服务、安装回执、完成），退出码 0；文件装到 `C:\Program Files\LanMsg\`（`lanmsg.exe`、`lanmsgd.exe`、`lanmsg-cli.exe`、`lanmsg-thumb.exe`、`LICENSE`、`uninstall.exe`）；桌面与开始菜单快捷方式（含中文名的 `卸载 LanMsg.lnk`）创建；服务被启动并在 `%APPDATA%\lanmsg\LanMsg\data\` 下创建了数据库，**这正是 README 里更正过的 Windows 数据目录，现在被真实安装器与真实 `lanmsgd.exe` 验证了**。
- **注册表**（用 `wine reg query` 查，我最初用 grep 注册表文件没找到，是因为 Wine 对键名里的反斜杠做了双重转义，不是没写）：`HKLM\…\Uninstall\LanMsg` 含 `DisplayName`、`DisplayVersion=0.1.1`、`InstallLocation`、`Publisher`、`UninstallString`，以及 `QuietUninstallString = "…\uninstall.exe" --silent --keep-user-data --reboot-as-success`，与 wind-installer README 的契约逐字一致；`HKCU\…\Run\LanMsg` 开机自启动项路径带引号。
- **覆盖安装（升级场景）**：旧 `lanmsgd.exe` 正在运行时再装一次，退出码 0，输出 `[2/12] 正在停止旧进程...`，文件与用户数据都在。
- **静默卸载**（用注册表里记录的 `QuietUninstallString`）：7 个步骤走完，退出码 0；程序文件、桌面与开始菜单快捷方式、`Uninstall\LanMsg` 键、`Run\LanMsg` 键全部清除，**`--keep-user-data` 生效，用户数据目录 7 个文件保留**。
- 首次启动时 `keystore.backend = os`：Wine 实现了凭据管理器，所以系统库探测成功，这顺带说明 Windows 的凭据后端在首次选定路径（含新的限时 `bounded` 线程）上能工作。

**不能由此证明的（Wine 不是 Windows）**：真实 Windows 上的 UAC 提示（`--silent` 未提权时会弹 UAC）；防火墙那一步 `[7/12]` 只是“走了这一步”，Wine 里没有 `netsh` 与 Windows 防火墙，**规则是否真的放行没有验证**，也没有验证中文系统上 `netsh` 输出的 OEM 解码；Windows Defender/SmartScreen 对未签名安装器的拦截；开机自启动是否真的在登录时拉起；scoop 的行为（scoop 本身、`scoop update lanmsg`、弹命令行窗口）完全没有验证；Windows 上的桌面 GUI 窗口（本次无图形）。另外我用 `grep` 进程名统计 `lanmsgd.exe` 得到的“3”不能解读为实例数（Wine 会为一个 Windows 进程起多个宿主进程），覆盖安装前后进程数不变也无法据此判断旧进程是否被停掉，所以我没有据此下结论。

