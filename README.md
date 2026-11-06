# LanMsg

局域网内直接收发消息与文件，无需账号与服务器。许可证：GPL-3.0-or-later。

## 安装与启动

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
