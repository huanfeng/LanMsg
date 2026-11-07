# LanMsg 移动端 UI 架构规范

## §1 目的与适用范围

本文件是 Android / iOS / 鸿蒙原生界面的共同规范。页面、状态、动作、组件、文案均以本文为准；各平台只决定控件实现。
数据全部来自 `lanmsg-api` 的 Request / Response / Event（定义见路线图 C5：`docs/superpowers/plans/2026-10-28-00-roadmap.md` 的「C5. 客户端 API」一节）。
设计依据：`docs/superpowers/specs/2026-10-28-lanmsg-design.md`。视觉（色彩、文案原则）与桌面端 `docs/superpowers/specs/2026-11-06-desktop-visual-guide.md`（v1.1）保持一致，见 §5.1。

## §2 导航结构

```
启动
 ├─ GetSelf.initialized == false → 引导（Onboarding）
 │     ├─ 欢迎 → 填写名字 → 选择方式
 │     │     ├─ 「这是我的第一台设备」→ Init(FirstDevice) → 主界面
 │     │     └─ 「加入我已有的设备」→ Init(JoinExisting) → 配对加入页 → Done → 主界面
 └─ 已初始化 → 主界面（底部 4 个标签）
       ├─ 消息（会话列表）→ 聊天页 → 图片查看页；会话列表右上角搜索 → 搜索页（→ 聊天页）
       ├─ 文件（文件视图）
       ├─ 联系人 → 新的请求页 / 联系人详情页（→ 聊天页）
       └─ 我 → 我的设备页（→ 配对主持页 / 配对加入页）/ 设置页
外部分享进入 → 选择会话页 → 聊天页
文件卡片 / 文件视图「转发」→ 转发选择会话页 → 聊天页
点击消息通知 → 聊天页；点击请求通知 → 新的请求页
```

路由表（Android 用作 Navigation 路由字符串，iOS/鸿蒙用作页面标识）：

| 路由 | 参数 | 页面 |
|---|---|---|
| `onboarding` | — | 引导 |
| `convs` | — | 会话列表（标签 1） |
| `chat/{conv}` | conv：会话 id | 聊天 |
| `image/{conv}/{hash}` | — | 图片查看 |
| `files?conv={conv}` | conv 可选 | 文件视图（标签 2） |
| `contacts` | — | 联系人（标签 3） |
| `requests` | — | 新的请求 |
| `contact/{user}` | user：用户 id | 联系人详情 |
| `me` | — | 我（标签 4） |
| `devices` | — | 我的设备 |
| `pair/host` | — | 配对主持（本机出示二维码 / 比对数字） |
| `pair/join` | — | 配对加入（扫码 / 选设备 / 比对数字） |
| `settings` | — | 设置 |
| `share` | — | 选择会话（分享入口） |
| `search` | — | 搜索聊天记录 |
| `forward/{hashes}` | hashes：逗号分隔的 blob hash | 转发：选择会话 |


## §3 全局规则
- 事件订阅：每个页面在可见期间订阅事件；收到"事件落后"（Android：`CoreApi.lagged`）或 App 回到前台时，当前页面执行一次"全量刷新"（重新发该页的加载请求）。
- 合并规则：列表按 id 去重，后到的同 id 数据覆盖旧数据。
- 错误：请求失败用底部提示条显示 `错误：<message>`；`PeerOffline` 显示文案 `err_peer_offline`；`NotInitialized` 跳回引导。
- 时间格式：今天 `HH:mm`；昨天 `昨天 HH:mm`；今年 `M月d日`；更早 `yyyy/M/d`。
- 大小格式：`< 1024` → `N B`；`< 1024²` → `x.x KB`；`< 1024³` → `x.x MB`；否则 `x.x GB`（一位小数，去掉 `.0`）。
- 时长格式：`m:ss`，超过 1 小时 `h:mm:ss`。
- 头像：圆形，背景色取自 6 色调色板 `#3D6E8F #5B6E3A #7A5A8C #8C5A3D #3D7A6E #6B6478`（白字对比度均 ≥ 4.5:1），亮色与暗色主题下底色不变。取色算法与桌面端 `style.rs` 的 `avatar_color` 逐字一致，保证同一人在两端颜色相同：对 id 字符串的 UTF-8 字节序列做 djb2 哈希，`h = 5381`（u32），对每个字节 `b` 依次 `h = (h * 33 + b) mod 2^32`（u32 回绕），最终取调色板下标 `h mod 6`（按上面从左到右的顺序，下标 0 起）。Kotlin 用 `Int` 运算后以 `toUInt()` 或 `toLong() and 0xFFFFFFFF` 取模；id 对会话取会话 id，对人取用户 id。中间显示显示名第一个字符；"我的设备"会话用设备图标代替。

## §4 页面规格

（每页：用途 / 状态字段 / 加载 / 事件处理 / 用户动作 / 空状态与错误）

| 页面 | 状态字段 | 加载请求 | 事件处理 | 用户动作 → 请求 |
|---|---|---|---|---|
| 引导 | `step`(欢迎/名字/方式)、`displayName`、`deviceName`(默认系统设备名)、`busy`、`error` | `GetSelf` | — | 完成第一台：`Init{display_name, device_name, FirstDevice}`；加入已有：`Init{…, JoinExisting}` 后进入 `pair/join` |
| 会话列表 | `items: [ConversationSummary]`（按 `last_hlc_millis` 降序，空值排最后）、`loading` | `ListConversations` | `ConversationUpdated` → 按 conv upsert 并重排；`PeerOnline/PeerOffline` → 重拉 | 点行 → `chat/{conv}`；长按 → 菜单「免打扰 / 取消免打扰」（仅本机设置） |
| 聊天 | `conv`、`title`、`kind`、`peerOnline`、`messages: [MessageView]`（升序）、`hasMore`、`loadingMore`、`input`、`progress: {hash → (transfer, done, total)}`（`transfer` 取自 `TransferProgress.transfer`，`CancelTransfer{transfer}` 用它） | `ListConversations` 取本会话摘要；`ListMessages{conv, before=null, limit=50}` | `MessageAdded`(conv 相同) → 按 id 去重后追加；`MessageUpdated` → 按 id 替换；`TransferProgress{transfer, hash, done, total, …}` → 按 hash 更新 progress（含 transfer）；`TransferFinished` → 移除 progress 并重拉最新一页合并；`ConversationUpdated` → 更新标题/在线 | 发送文字：`SendText`；选择图片/视频（系统相册选择器）或文件（系统文件选择器）→ 复制到发件箱 → `SendFiles{conv, paths}`；点"未下载"的文件 → `DownloadBlob{hash}`；下载中点 × → `CancelTransfer{transfer}`（取 progress[hash].transfer）；长按自己 2 分钟内的消息 → 「撤回」→ `Recall`；长按任意消息 → 「删除」→ `DeleteLocal{msgs, also_my_devices=false}`、「复制」（文字）、「转发」（Files/Folder 消息，转发其全部文件 → `forward/{hashes}`）；滚到顶 → `ListMessages{before=最早一条 id}`；页面可见且最新一条变化 → `MarkRead{conv, upto=最新一条 id}` |
| 图片查看 | `item: FileItemView` | 从聊天页传入的 conv 中 `ListMessages` 查找 hash | `TransferFinished`(hash) → 刷新 | 未下载时显示缩略图 + 「下载原图」→ `DownloadBlob`；「保存到相册」（本机动作） |
| 文件视图 | `filter: FileFilter`（默认 kind=All、limit=200）、`items: [FileView]` | `QueryFiles{filter}` | `MessageAdded` / `MessageUpdated` / `TransferFinished` → 500ms 防抖后重查 | 顶部分段：全部/图片/视频/文档/压缩包/其他；筛选菜单：状态（全部/已下载/未下载）、来源设备；搜索框（`name_query`）；点项：已下载 → 打开，未下载 → `DownloadBlob`；长按：「打开所在会话」→ `chat/{conv}`、「转发」→ `forward/{hash}` |
| 联系人 | `contacts: [ContactView]`、`nearby: [PeerView]`（`!is_contact && !is_mine`）、`requestCount` | `ListContacts`、`ListPeers`、`ListRequests` | `ContactsChanged` → 重拉联系人；`PeerOnline/Offline` → 重拉 peers；`IncomingRequest` → 重拉请求数 | 顶部「新的请求 (N)」→ `requests`；点联系人 → `contact/{user}`；附近的人行尾「添加」→ `RequestContact{user}`，成功后提示 `toast_request_sent` |
| 新的请求 | `items: [RequestView]` | `ListRequests` | `IncomingRequest` → 重拉 | 每项四个按钮：接收一次/加为联系人/拒绝/屏蔽 → `RespondRequest{request, action}`，成功后从列表移除 |
| 联系人详情 | `contact: ContactView` | `ListContacts` 中按 user 查找 | `ContactsChanged` → 重拉 | 「发消息」→ `OpenDm{user}` → `chat/{conv}`；「自动接收」开关、「超过此大小仍询问」选择（不限/10MB/50MB/100MB/500MB）→ `SetContactPolicy` |
| 我 | `self: SelfInfo` | `GetSelf` | — | 改名 → `SetProfile`；状态选择（在线/隐身/忙碌）→ `SetStatus`；进入我的设备 / 设置 |
| 我的设备 | `devices: [DeviceView]` | `ListMyDevices` | `DevicesChanged` → 重拉 | 「添加我的设备」→ `pair/host`；「加入其他设备」→ `pair/join`；非本机设备长按 → 「移除此设备」→ 确认对话框 → `RemoveDevice` |
| 配对主持 | `method`(Qr/Sas)、`session`、`qrUri`、`state: PairStateDto` | `PairStart{Qr}` | `PairStatus`(session 相同) → 更新 state | 显示二维码；「改用数字比对」→ `PairCancel` + `PairStart{Sas}`；`ShowSas` 时显示 6 位数字和「一致」「不一致」→ `PairConfirm{matches}`；离开页面 → `PairCancel`；`Done` → 提示 `toast_pair_done` 并返回 |
| 配对加入 | `mode`(扫码/选设备)、`candidates: [PeerView]`、`session`、`state`、`cameraGranted` | `ListPeers`（选设备模式） | `PairStatus`；`PeerOnline/Offline` → 重拉候选 | 扫到 `lanmsg://pair?…` → `PairJoin{QrUri{uri}}`；选设备 → `PairJoin{Device{device}}`；`ShowSas` → 「一致/不一致」→ `PairConfirm`；无相机权限 → 显示 `pair_need_camera` 与按钮「改用数字比对」（切到选设备模式） |
| 设置 | `settings: SettingsDto`、本机项（通知开关、保存到相册、信任的 Wi-Fi） | `GetSettings` | — | 设备名、图片/文件自动下载上限（下拉：关闭/5MB/10MB/50MB/100MB/不限）、仅 Wi-Fi 下自动下载文件、LocalSend 兼容、后台模式（实时/省电/关闭）→ `SetSettings`（整体回写）；本机项直接存本地；「允许后台运行」→ 打开系统电池优化设置；「清理已分享文件的副本」→ 确认对话框 → 删除发件箱副本；「添加当前 Wi-Fi」运行时申请位置权限，拒绝显示 `settings_trusted_wifi_need_location` |
| 选择会话 | `items: [ConversationSummary]`、`pending: [本地文件路径]` | `ListConversations` | — | 点会话 → `SendFiles{conv, pending}` → `chat/{conv}` |
| 搜索 | `query`、`results: [MessageView]`、`loading`、`error` | —（输入后 300ms 防抖发 `SearchMessages{query, conv=null, limit=50}`；空白查询清空结果、不发请求） | — | 点结果 → `chat/{conv}`；结果行显示会话标题（来自 `ListConversations` 缓存）、作者设备名、时间、文字片段；无结果 → `search_empty`；core 未实现 → `err_core_outdated` |
| 转发选择会话 | `items: [ConversationSummary]`、`hashes` | `ListConversations` | — | 点会话 → `ForwardFiles{conv, hashes}` → 提示 `toast_forwarded` → `chat/{conv}`；core 未实现 → `err_core_outdated` |

## §5 组件规格

| 组件 | 内容与规则 |
|---|---|
| 会话行 | 头像、标题（单行省略）、右上时间、第二行 `last_text`（单行省略，空则显示 `conv_no_message`）、未读角标（>99 显示 `99+`，`signal_fill` 底白字）、对端在线时头像右下 8dp 绿点（`online`，见 §5.1）；免打扰会话标题后显示静音图标，未读角标改灰色 |
| 消息气泡 | 自己的消息靠右、`signal_fill` 底白字（见 §5.1，暗色下不是 `signal`）；他人靠左、`surface` 底 + 1px `line`、`ink` 字；最大宽度为屏宽 75%；`kind == SelfDevices` 或对方有多台设备时，气泡上方显示小字 `来自 {author_device_name}`；相邻两条间隔 > 5 分钟时插入居中时间分隔；`state`：Sending 显示转圈、Failed 显示 `danger` 色感叹号（点按重发 = 再次 `SendText` 同文本）、Delivered 不显示、Read 显示 `msg_read`、Recalled 整条替换为居中灰字 `msg_recalled_by_me`/`msg_recalled` |
| 文件卡片 | 左侧类型图标（按 mime：图片/视频/音频/文档/压缩包/其他），名称（中间省略）、大小、`source_label`（灰色小字）；右侧状态：Remote → 下载图标 + `file_not_downloaded`；Downloading → 环形进度 + 百分比 + × 按钮；Local → 无标记，点按打开；SourceOffline → 灰色 `file_source_offline`，不可点；Preparing → 转圈 + `file_preparing` |
| 媒体网格 | Files 消息中所有图片/视频条目组成网格：1 个时按宽高比显示（最长边 240dp），2–4 个 2 列，5 个以上 3 列，最多显示 9 个，第 9 格显示 `+N`；视频右下角显示时长；缩略图来自 `thumb_path`，没有就显示类型图标；Downloading 时叠加进度环 |
| 系统消息 | 居中灰色小字，`System{text}` 原样显示 |
| 不支持的消息 | 居中灰字 `msg_unsupported` |
| 路径角标 | `Lan` 不显示；`Vpn` → `path_vpn`；`Internet` → `path_internet`；`Relay` → `path_relay`；小号描边标签 |
| 空状态 | 图标 + 一句说明（文案见 §6）+ 可直接行动的入口：页面有可行动操作时必须给出按钮（例如“添加设备”按钮），页面确无可做之事时才只留图标和说明 |
| 确认对话框 | 标题、正文、「取消」+ 危险确认按钮（浅色：`danger` 实底白字；暗色：`surface` 底 + 1px `danger` 描边 + `danger` 文字，见 §5.1） |

### 5.1 视觉令牌（与桌面端视觉规范 v1.1 对齐）

浅色主题值如下；暗色主题按桌面规范第 4 节推导（`canvas #12181D`、`surface #1C252C`、`line #2B363F`、`ink #E6ECEF`、`ink_muted #A3B0BA`、`ink_subtle #8D9AA5`、`signal #3FB8AF`、`signal_soft #1E3533`、`online #3CC27A`、`danger #F08A5D`），跟随系统切换；头像底色不随主题变。各平台界面代码中不得出现表外色值字面量。

| 名称 | 值 | 用途 |
|---|---|---|
| `ink` | `#16202A` | 正文、标题 |
| `ink_muted` | `#4F5C68` | 预览、说明 |
| `ink_subtle` | `#5F6C78` | 时间戳、脚注 |
| `canvas` | `#F4F6F7` | 页面背景 |
| `surface` | `#FFFFFF` | 列表、卡片、输入框、对方气泡 |
| `line` | `#DCE2E6` | 分隔线、描边 |
| `signal` | `#0B7A75` | 强调色（深青）：选中标签、文字按钮、描边、选中色；实底（主按钮、未读角标、我的气泡）一律用 `signal_fill` |
| `signal_soft` | `#E8F4F2` | 选中底色、次要强调底 |
| `online` | `#1E9E5A` | 在线点 |
| `danger` | `#C2410C` | 错误文字、校验提示、危险确认按钮实底（浅色） |
| `signal_fill` | 浅色 `#0B7A75`；暗色 `#0B7A75` | 实底：主按钮、我的气泡、未读角标；白字 |
| `signal_fill_hover` | 浅色 `#096A65`；暗色 `#0B7F79` | `signal_fill` 悬停（有指针的平台） |
| `signal_fill_pressed` | 浅色 `#075A56`；暗色 `#096560` | `signal_fill` 按下 |

规则：
- 所有文字与其底色对比度 ≥ 4.5:1（含 12sp 小字）。不用渐变、不用阴影堆叠，层级靠底色与 1px `line`。
- 暗色主题 `signal`（`#3FB8AF`）亮度高，白字只有 2.42:1，**不得作实底**：实底（主按钮、我的气泡、未读角标）一律用 `signal_fill`（`#0B7A75`）配白字；`#3FB8AF` 只用于暗底上的文字、描边、选中色。白字对比度：`signal_fill` 5.18:1、暗色悬停 `#0B7F79` 4.85:1、暗色按下 `#096560` 6.90:1、浅色悬停 `#096A65` 6.44:1、浅色按下 `#075A56` 8.06:1，均 ≥ 4.5:1。浅色下 `signal` 与 `signal_fill` 同值。
- 危险确认按钮：桌面端没有 danger 实底。浅色下用 `danger` `#C2410C` 实底白字（5.18:1）；暗色 `danger` `#F08A5D` 白字只有 2.47:1，故暗色下改为描边按钮：`surface` 底、1px `danger` 描边、`danger` 文字（对 `surface` 6.29:1），按下态用系统默认涟漪/高亮。暗色下 `danger` 只作文字、描边、图标，不作白字实底。
- 主按钮用 `signal_fill` 实底白字，一个页面最多一个；次按钮 `surface` 底 + 1px `line`；文字按钮仅 `signal` 字。
- 禁用状态保留形状、透明度 45%，旁边或下方必须有一句说明为什么不可用（例如名字为空时显示“填写名字后继续”）。
- 空状态：图标 + 一句说明 + 一个能直接行动的入口，不写“暂无数据”。
- 文案原则：动作按钮写清会发生什么，结果提示与按钮同词；错误说明原因与办法，不道歉、不写“操作失败”；不用 `A · B · C` 拼接元信息；不用全大写。

## §6 文案表

（Android `strings.xml` 键名 = 本表"键"列；iOS/鸿蒙用同名键）

| 键 | 文案 |
|---|---|
| app_name | LanMsg |
| tab_messages | 消息 |
| tab_files | 文件 |
| tab_contacts | 联系人 |
| tab_me | 我 |
| onb_welcome_title | 欢迎使用 LanMsg |
| onb_welcome_body | 在局域网里，像聊天一样在手机和电脑之间互传文件 |
| onb_start | 开始 |
| onb_name_title | 你的名字 |
| onb_name_hint | 其他人会看到这个名字 |
| onb_name_required | 填写名字后继续 |
| onb_device_name | 本机名称 |
| onb_next | 下一步 |
| onb_mode_title | 这台设备是？ |
| onb_mode_first | 这是我的第一台设备 |
| onb_mode_first_desc | 创建新的身份 |
| onb_mode_join | 加入我已有的设备 |
| onb_mode_join_desc | 扫描电脑上的二维码，和已有设备同步 |
| conv_self_title | 我的设备 |
| conv_no_message | 暂无消息 |
| convs_empty | 还没有会话。同一局域网里的设备会自动出现在联系人中 |
| chat_input_hint | 输入消息 |
| chat_send | 发送 |
| chat_attach_media | 图片和视频 |
| chat_attach_file | 文件 |
| chat_from_device | 来自 %1$s |
| chat_peer_offline | 对方不在线，消息会在其上线后送达 |
| msg_recall | 撤回 |
| msg_delete | 删除 |
| msg_copy | 复制 |
| msg_recalled_by_me | 你撤回了一条消息 |
| msg_recalled | 对方撤回了一条消息 |
| msg_read | 已读 |
| msg_unsupported | 不支持的消息类型，请升级 LanMsg |
| msg_resend | 重新发送 |
| time_yesterday | 昨天 %1$s |
| time_month_day | M月d日 |
| file_not_downloaded | 未下载 |
| file_source_offline | 源设备离线 |
| file_preparing | 准备中 |
| file_download_original | 下载原图 |
| file_save_to_gallery | 保存到相册 |
| file_saved | 已保存 |
| file_save_failed | 保存失败 |
| file_open_conversation | 打开所在会话 |
| files_all | 全部 |
| files_image | 图片 |
| files_video | 视频 |
| files_document | 文档 |
| files_archive | 压缩包 |
| files_other | 其他 |
| files_state_downloaded | 已下载 |
| files_state_remote | 未下载 |
| files_filter | 筛选 |
| files_source_device | %1$s（%2$s） |
| files_search_hint | 搜索文件名 |
| files_empty | 还没有收发过文件 |
| contacts_requests | 新的请求 (%1$d) |
| requests_title | 新的请求 |
| contacts_nearby | 附近的人 |
| contacts_add | 添加 |
| contacts_empty | 还没有联系人 |
| toast_request_sent | 已发送请求 |
| req_accept_once | 接收一次 |
| req_add_contact | 加为联系人 |
| req_reject | 拒绝 |
| req_block | 屏蔽 |
| req_kind_contact | 想加你为联系人 |
| req_kind_message | 给你发来消息 |
| req_kind_file | 给你发来文件 |
| requests_empty | 没有新的请求 |
| contact_send_message | 发消息 |
| contact_auto_accept | 自动接收 |
| contact_ask_above | 超过此大小仍询问 |
| size_unlimited | 不限 |
| auto_off | 关闭 |
| me_status | 状态 |
| status_online | 在线 |
| status_invisible | 隐身 |
| status_busy | 忙碌 |
| me_my_devices | 我的设备 |
| me_settings | 设置 |
| me_edit_name | 修改名字 |
| devices_add | 添加我的设备 |
| devices_join | 加入其他设备 |
| devices_this | 本机 |
| devices_remove | 移除此设备 |
| devices_remove_confirm | 移除后，这台设备将无法再收发你的消息。确定移除“%1$s”吗？ |
| pair_host_title | 添加我的设备 |
| pair_qr_desc | 配对二维码 |
| pair_host_hint | 在另一台设备上打开 LanMsg，选择“加入我已有的设备”并扫描此二维码 |
| pair_use_sas | 改用数字比对 |
| pair_sas_hint | 请确认两台设备上显示的数字一致 |
| pair_match | 一致 |
| pair_mismatch | 不一致 |
| pair_join_title | 加入已有设备 |
| pair_join_scan_hint | 扫描已有设备上显示的二维码 |
| pair_join_pick | 选择要加入的设备 |
| pair_need_camera | 需要相机权限才能扫码 |
| pair_grant_camera | 授予权限 |
| pair_verifying | 正在验证… |
| pair_waiting | 等待对方… |
| toast_pair_done | 配对成功 |
| pair_failed | 配对失败：%1$s |
| settings_device_name | 设备名称 |
| settings_auto_image | 自动下载图片（小于） |
| settings_auto_file | 自动下载视频和文件（小于） |
| settings_wifi_only | 仅在 Wi-Fi 下自动下载文件 |
| settings_localsend | 兼容 LocalSend |
| settings_bg_mode | 后台模式 |
| bg_realtime | 实时（立即收到通知，耗电稍多） |
| bg_power_saving | 省电（亮屏、充电或在信任的 Wi-Fi 下保持连接，其余时间约每 15 分钟同步一次） |
| bg_off | 关闭（仅打开 App 时同步） |
| settings_trusted_wifi | 信任的 Wi-Fi |
| settings_trusted_wifi_add | 添加当前 Wi-Fi |
| settings_trusted_wifi_need_location | 读取 Wi-Fi 名称需要位置权限 |
| settings_trusted_wifi_not_connected | 当前没有连接 Wi-Fi |
| settings_trusted_wifi_remove | 移除 %1$s |
| settings_notifications | 消息通知 |
| settings_save_gallery | 收到的图片和视频保存到相册 |
| settings_battery | 允许后台运行 |
| settings_battery_desc | 在系统电池优化中将 LanMsg 设为“不优化”，避免后台连接被中断 |
| settings_clear_outbox | 清理已分享文件的副本 |
| settings_clear_outbox_desc | 副本占用 %1$s |
| settings_clear_outbox_confirm | 将删除 LanMsg 为分享而保存的文件副本，原文件不受影响。 |
| settings_clear_outbox_action | 清理副本 |
| share_pick_title | 发送到 |
| share_empty | 没有可用的会话 |
| share_need_setup | 请先完成初始化，再分享文件 |
| share_import_failed | 无法读取要发送的文件 |
| open_file_failed | 无法打开此文件 |
| path_vpn | VPN |
| path_internet | 公网 |
| path_relay | 中继 |
| notif_channel_messages | 消息 |
| notif_channel_requests | 请求 |
| notif_channel_transfers | 传输 |
| notif_channel_service | 后台连接 |
| notif_service_text | LanMsg 正在后台保持连接 |
| notif_new_files | [文件] %1$s |
| notif_new_request | %1$s %2$s |
| mute | 免打扰 |
| unmute | 取消免打扰 |
| cancel | 取消 |
| confirm | 确定 |
| back | 返回 |
| retry | 重试 |
| image_not_found | 找不到这张图片 |
| err_generic | 错误：%1$s |
| err_peer_offline | 对方不在线 |
| err_core_outdated | 该功能需要更新核心库 |
| search_hint | 搜索聊天记录 |
| search_empty | 没有找到相关消息 |
| action_forward | 转发 |
| forward_title | 转发到 |
| toast_forwarded | 已转发 |

## §7 通知规范
- 渠道：`messages`（高）、`requests`（高）、`transfers`（低，进度）、`service`（最低，前台服务常驻）。
- 消息通知：每个会话一条（通知 id = conv id 的 hashCode），标题 = 会话标题，正文 = 文字或 `notif_new_files`（文件名），同会话新消息更新同一条；点击 → `chat/{conv}`。
- 不弹通知的情况：消息是自己发的；本人状态为「忙碌」（Busy，仍计未读）；会话免打扰；用户正在看该会话；通知总开关关闭；系统未授予通知权限。
- 请求通知：标题 `LanMsg`，正文 `notif_new_request`（名字 + 请求类型文案），点击 → `requests`。

## §8 平台映射说明

Android = Compose + Navigation + ViewModel/StateFlow（本计划）；iOS = SwiftUI + NavigationStack + ObservableObject（后续）；鸿蒙 = ArkUI + Navigation（后续）。三端共享 §2 路由名、§4 状态字段名、§6 文案键。
