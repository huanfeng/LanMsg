# LanMsg

局域网内直接收发消息与文件，无需账号与服务器。许可证：GPL-3.0-or-later。

## 安装与启动

### 从发布包安装

发布页：<https://github.com/huanfeng/lanmsg/releases>（资产名不含版本号，校验和见 `SHA256SUMS`）。

- Linux（桌面版或 NAS 无头版）：`curl -fsSL https://github.com/huanfeng/lanmsg/releases/latest/download/install.sh | sh`
- Windows：运行 `lanmsg-windows-x64-setup.exe`（ARM64 机器用 `lanmsg-windows-arm64-setup.exe`），或使用 scoop：`scoop bucket add lanmsg https://github.com/huanfeng/lanmsg` 后 `scoop install lanmsg`
- macOS：打开 `lanmsg-macos-universal.dmg`（未签名，首次打开需在“系统设置 → 隐私与安全性”中放行）
- Android：安装 `lanmsg-android.apk`

仓库保持私有期间匿名无法下载发布资产，上述方式需要公开后才能使用。

### 构建

```sh
cargo build --release -p lanmsgd -p lanmsg-desktop -p lanmsg-cli -p lanmsg-media
```

产物在 `target/release/`。

### 可执行文件（须放在同一目录）

| 文件 | 作用 |
| --- | --- |
| `lanmsg` | 桌面界面 |
| `lanmsgd` | 后台服务（收发、发现、托盘、通知） |
| `lanmsg-thumb` | 缩略图生成辅助进程，由 `lanmsgd` 调用 |
| `lanmsg-cli` | 命令行客户端（可选） |

Windows 下文件名带 `.exe`。

### 首次运行

运行 `lanmsg`。后台服务未运行时，界面会自动拉起同目录的 `lanmsgd`，随后按向导设置名字即可。
托盘菜单“退出”会让后台服务停止；此后界面不会擅自把它再拉起，横幅上点“启动后台服务”即可恢复。

没有托盘宿主的 Linux（如未装 StatusNotifier 宿主的桌面）：托盘图标不会出现，直接在终端或启动器里运行 `lanmsg` 打开界面。
无图形会话时可用 `lanmsgd --headless` 只跑后台服务，用 `lanmsg-cli` 操作。

### Linux 依赖

- 图形：X11 或 XWayland
- 托盘：StatusNotifier 宿主（可选）
- 视频缩略图与时长：`ffmpeg`（可选，缺失时视频只显示类型图标）

### 数据目录

| 系统 | 位置 |
| --- | --- |
| Linux | `$XDG_DATA_HOME/lanmsg`，默认 `~/.local/share/lanmsg` |
| Windows | `%APPDATA%\LanMsg` |
| macOS | `~/Library/Application Support/LanMsg` |

`lanmsg` 与 `lanmsgd` 都接受 `--data-dir <目录>` 指定其它位置（同一数据目录只允许一个界面）。

### 两台机器互通

- 同一局域网内自动发现，无需配置。
- 跨网段（或组播被拦截）时，在“联系人”页的“附近的人”里手动添加对方地址。
- 两台机器各自完成向导后，互相添加为联系人即可聊天；另一台电脑加入自己的身份（手机端见后续版本），在“设置”或“联系人 → 我的设备”里配对。

## 开发约定

日志（`tracing`）与错误消息一律英文，界面文案只放本地化资源（桌面 `crates/lanmsg-desktop/src/strings.rs`、lanmsgd `crates/lanmsgd/src/strings.rs`、Android `strings.xml`）。提交前运行下面的检查，非测试、非注释的 Rust 字符串字面量里出现汉字或全角标点会让它以退出码 1 失败并逐行列出 `文件:行号`。不在检查范围内的有：白名单文件（两个 `strings.rs` 与桌面演示数据 `fixtures.rs`）、`tests/` 目录、`#[cfg(test)]` 修饰的 `mod`（按花括号配对，只剔除该模块本身）以及注释。脚本自带反例自测：`python3 scripts/check-no-cjk.py --self-test`。

```sh
python3 scripts/check-no-cjk.py
```

### 依赖与联调

- `windui` 以 git tag 方式依赖（`https://github.com/huanfeng/wind-ui-rust`，见 `crates/lanmsg-desktop/Cargo.toml` 与 `crates/lanmsgd/Cargo.toml`）。需要联调本地的 windui 时，不要改 `Cargo.toml` 也不要提交 `[patch]`，用 `--config` 临时覆盖（该命令会改动 `Cargo.lock`，提交前请 `git checkout Cargo.lock`）：

```sh
cargo --config 'patch."https://github.com/huanfeng/wind-ui-rust".windui.path="/绝对路径/wind-ui-rust"' build -p lanmsg-desktop
```

### 版本与发布

- 版本号只有一个来源：根 `Cargo.toml` 的 `[workspace.package].version`。Android 的 `versionName` 与 `versionCode`（`major*10000 + minor*100 + patch`）由 `android/app/build.gradle.kts` 解析该文件得到，不要手写。
- 升版本：`scripts/bump-version.sh <x.y.z>`（同步 `Cargo.toml`、`Cargo.lock`，并把 `CHANGELOG.md` 的 `[Unreleased]` 归入新版本）。提交后推送 `v<x.y.z>` tag，`release.yml` 构建全部安装包、生成 `SHA256SUMS`、创建 GitHub Release（说明取自 `CHANGELOG.md` 对应小节）并把 `bucket/lanmsg.json` 提交回 `main`。
- `release.yml` 也可手动触发（`workflow_dispatch`，`dry_run` 默认为 true）：只构建、不发布。
- Android 正式签名使用仓库 Secrets `ANDROID_SIGNING_KEYSTORE_BASE64`、`ANDROID_SIGNING_STORE_PASSWORD`、`ANDROID_SIGNING_KEY_ALIAS`、`ANDROID_SIGNING_KEY_PASSWORD`；本地签名见 `docs/android-build.md`。
- 提交前的打包检查：`bash scripts/check-packaging.sh --fast`。
