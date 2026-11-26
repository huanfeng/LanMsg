# 更新日志

格式约定：每个版本一节，标题为 `## [x.y.z] - YYYY-MM-DD`，其上保留 `## [Unreleased]` 记录未发布改动。
发布流水线会把对应版本的小节作为 Release 说明；用 `scripts/bump-version.sh <x.y.z>` 升版本时会把 Unreleased 的内容归入新版本。

## [Unreleased]

## [0.1.0] - 2026-11-26

### 新增

- 文件中转站：把文件、文件夹或文字放进某台设备上的中转站，其它设备按需拉取；支持引用原文件或复制存放、设置保留时间、置顶；桌面端可拖入，或用 `lanmsg --station-put <路径>` 放入，Android 端也有中转站页。
- 网页传输：没有安装 LanMsg 的设备用浏览器扫码或输入配对码即可与本机互传，内容经应用层加密，访客需经本机允许，空闲 30 分钟后自动结束；桌面端、托盘与 Android 端均可开启。
- LocalSend 互通：与 LocalSend 应用互发文件；设置里可开关并修改 LocalSend 端口，端口被占用时给出提示。
- 跨网段与 VPN：手动添加设备地址（重启后仍会重连）、扫描指定网段、已连接设备间交换地址，并标注连接路径（局域网 / VPN）；对端在 NAT 后面时，文件传输可由对端反向建立连接。
- Windows 防火墙：检测到缺少放行规则时提示，并支持一键放行。
- 应用内更新：启动时自动检查（每天最多一次，默认开启，可在设置里关闭），设置页与托盘菜单可手动检查；Windows 安装器版静默安装，scoop 版调用 `scoop update lanmsg`，Linux 通过 `install.sh`，macOS 打开 dmg，Android 引导到发布页。
- 命令行客户端 `lanmsg-cli` 增加手动对端管理（`remove-peer`、`list-manual-peers`）与检查更新命令。
- 发布与安装：GitHub Actions 构建 Windows（x64 / ARM64）、Linux、macOS、Android 安装包并生成 `SHA256SUMS`；提供 Linux `install.sh` 与 scoop bucket。

### 变更

- 版本号统一取自根 `Cargo.toml`，Android 的版本名与版本号由其生成。
- 日志与错误消息统一为英文，界面文案只来自本地化资源。

### 修复

- 已屏蔽的 LocalSend 对端不再出现在“附近的人”里。
- 防火墙一键放行完成后状态即时刷新，托盘与横幅保持一致。
