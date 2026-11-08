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

- **T1**：迁移为 `MIGRATIONS[3]`（核实 schema.rs 当前 3 个元素）。`Frame` 追加 `AddressBook(50)`、`Station(60)` 及 `TYPE_*` 常量，同步 `type_id_values_match_spec` 测试。`station_blobs` 主键改为 `(item_id, rel_path, hash)`，`rel_path` 为 `TEXT NOT NULL DEFAULT ''`（文件夹内同内容文件不冲突；File 条目 `rel_path` 为空串）；另加索引 `station_items(expires_at)`、`localsend_peers(pseudo_device)`、`web_guests(pseudo_device)`。表 `known_addresses`、`manual_peers` 按 T3 修订后的列定义（见下）。
- **T2**：`LocalIface` 加 `tunnel` 字段，全仓 4 处构造同步。`ListInterfaces` 实现并移出兜底。
  - 实施补充：`PeerInfo.path` 原先在 `net/conn.rs` 硬编码为 `Lan`，现改为连接建立时用 `classify(peer_ip, cached_ifaces(), false)`（网卡列表 30 秒缓存，`NetworkChanged` 时作废）。`classify` 按最长前缀选网卡；枚举不到网卡时私网/回环/链路本地地址回退为 `Lan`（防 Android 11+ 限制 netlink 导致陌生人配对被拒）。
  - 已知限制：Android 上的隧道识别未经真机验证；公网陌生人入站将被拒（规则 7，预期行为）；`via_multicast` 目前在握手处恒为 false。
- **T3（重写）**：不新建 `Reconnector`，在 `service/redial.rs` 上增量修改：
  - `store/addresses.rs`：`known_addresses(device, addr, path, last_ok, source)`，每台设备最多 8 条；`manual_peers(addr, device NULL, added_at)`。函数名照原计划（`record_address`、`addresses_of`、`add_manual_peer`、`list_manual_peers`、`bind_manual_peer`、`is_manual_device`）。
  - `ConnEvent::Up` 已用 `dial_addr` 写 `devices.last_addr`（mod.rs:639），保留，并额外调用 `on_peer_up_record` 写 `known_addresses`（source = Direct）。
  - redial 取候选地址：`addresses_of(device)`，再加上 `devices.last_addr` 兜底，按顺序逐个尝试。
  - `AddManualPeer`（handlers.rs:438 已存在）改为持久化到 `manual_peers`；启动时对每条调用 `register_static_peer`（mod.rs:802），重启后仍会重连。
  - 地址簿 `net/addrbook.rs` 的 `build_for` / `merge` 规则照原计划；收到新地址后唤醒对应设备的 redial 任务。
  - Trust 规则 7 与规则 6 的"手动添加设备放行"照原计划。
  - 测试照原计划 6 个，名字不变。
  - 实施补充：`merge` 增加 `self_user` 参数（判断来源是否为自己的设备），另有 `merge_devices` 返回被写入地址的设备供唤醒重连；`add_manual_peer` 存解析后的 `ip:port`，连接失败返回 `PeerOffline` 但保留记录；地址簿除上线时发送外，联系人变化 1 秒后与每 30 秒再推一次（设备记录与地址到达顺序不固定）；`on_up` 对已吊销设备补一道断开（出站连接不经 accept_filter）。
  - 防滥用与准入（第 2 轮）：出站连接也经 accept_filter（`connect_outbound` 读到对端 Hello 后、等待确认前；准入查询出错一律拒绝），`on_up` 吊销检查保留为兜底。地址簿最多 256 个条目、每条目 8 个地址，同一对端 10 秒内最多处理 5 个，合并在 `spawn_blocking` 里单事务完成；只收私网/CGNAT/ULA 地址（测试钩子 `allow_loopback_for_tests` 例外），路径用本机网卡重判；每设备 Exchanged 最多 4 条且最先淘汰。手动对端记录规范化的 host:port（主机名启动时重新解析），上限 32，新增 `RemoveManualPeer`、`ListManualPeers`（C5 追加，Android 需重新生成 uniffi 绑定）。
  - 已知限制与行为变化（第 3 轮）：地址簿白名单（私网/CGNAT/ULA）不传播全局 IPv6、公网段 VPN、fe80 链路本地与 `::ffff:` 映射地址（这些仍可经发现、直连或手动添加使用）；手动添加公网 IP 的陌生人，出站也被规则 7 拒绝（v1 预期，公网/DDNS 留待后续）；出站准入对称后，我方 Hello 在过滤前已发出（协议顺序决定，过滤需要对端 Hello 里的证书）。主机名条目由后台任务连上后按存储键绑定设备；CLI 增加 `remove-peer`、`list-manual-peers`。
  - 手动对端主机名解析优先 IPv4，解析出的多个地址逐个尝试、失败回退下一个（每轮重连重新解析）。
- **T3B（新增）：NAT 下数据通道反向建立**。问题：`open_data` 固定向 `peer.dial_addr` 拨号（manager.rs:430），对端在 NAT 或端口转发后面、只能由它连进来时，文件传输失败。
  - 实施补充：反向数据连接使用新的通道字节 `CHANNEL_DATA_DIAL_BACK = 0x03`（DataHello 之后紧跟 `DataFrame::DialBack`，类型号 7），原因是普通数据连接在 DataHello 之后直接进入请求/应答，接受方无法预读下一帧来区分；令牌设备不符时不消耗令牌；拨回只拨对端自己的监听地址，不会被诱导去连第三方；测试钩子 `override_dial_addr_for_test` 模拟直拨不通。
  - 限制与提示：被请求方每设备并发 4、10 秒内 16 次、全局并发 32，超出丢弃（不回拒绝帧，避免新帧）；一个 Blob 下载最多 1 条元数据连接 + 4 条工作者连接，多文件并发下载经 NAT 对端时可能触及限制，由下载重试兜底。请求方最坏要 3 秒直拨超时 + 10 秒等待共约 13 秒才报 `PeerOffline`——T7/T15 界面任务注意：连接中状态期间不要显示失败。频率记录在对端 Down 后保留一个窗口。
  - 新帧 `Frame::DataDialBack { token: [u8; 16] }`，`TYPE_DATA_DIAL_BACK = 44`。
  - 新数据帧 `DataFrame::DialBack { token }`，在 `DataHello` 之后发送。
  - `open_data`：直拨失败（连接错误，或 3 秒超时）时，生成随机 token，登记一个 oneshot 等待者，经控制连接发出 `DataDialBack`，最多等 10 秒，超时返回 `PeerOffline`。
  - 收到 `DataDialBack` 的一方：照常向对方的监听地址（控制连接的远端 IP 加 Hello 中的 `listen_port`）建立数据连接，完成 `DataHello` 后发送 `DialBack{token}`。
  - 接受方：数据连接带 token 时，交给对应的等待者，不交给 `data_handler`。token 一次性有效，未知或过期的 token 直接关闭连接。对端身份必须与发起请求的设备一致（校验证书对应的 DeviceId）。
  - 测试：`data_channel_falls_back_to_dial_back`（A 的 dial_addr 指向一个已关闭的端口，B 只能被 A 连入；A 向 B 拉取文件成功）；`dial_back_token_single_use`；`dial_back_rejects_wrong_device`。
- **T4**：复用 `send_unicast`（multicast.rs:169）与 `SCAN_MIN_PREFIX`（consts.rs:99）。core 与桌面 `parse_targets`（settings.rs:134）规则保持一致，并把 `SettingsDto.unicast_targets` 真正接入发现。
  - 实施补充：单播目标不接受主机名（需要同步 DNS），只接受 IP、IP:端口、前缀不小于 /24 的 IPv4 网段，展开后最多 1024 个；桌面 `parse_targets` 已按同一规则收紧（IPv6 网段、/24 以下网段、端口 0 都拒绝）。发现主循环里，组播模式下来自本机网段之外的包记为 `via_multicast = false`（它们只可能是单播）。扫描只统计握手成功的不同设备数；探测端口固定为 `SERVICE_PORT`。
  - 第 2 轮补充：发现表记录 `via_multicast`，`discovered_peer_view` 据此 `classify`（单播来源的跨网段条目不再显示为局域网）；`ConnManager::connect` 的 `via_multicast` 参数未使用（连接后的路径由 conn.rs 按真实网卡判定）。扫描只尝试建立连接，`found` 统计握手被双方接受的设备：受信/同用户设备，以及局域网内的陌生设备；经 VPN 的陌生人入站会被对方拒绝（规则 7），公网陌生人出站也被拒，因此通常计 0。扫描建立的出站连接对陌生人会保持，连接本身不会产生请求或配对提示（请求只由消息、联系人请求触发）。扫描不限制私网范围（用户主动触发，网段大小已限制在 /22 内）。`ScanFinished.cidr` 用网络地址规范化；“扫描中”标志用 Drop 守卫复位，扫描任务可被关停打断；单播目标先去重再比上限，并过滤本机地址。
  - 第 3 轮补充：扫描连上的陌生设备会被写入本地 devices/users/已知地址，并出现在在线列表里（没有任何提示）。单播目标的本机地址过滤基于当前网卡缓存（30 秒 TTL，网络变化时作废）：换 IP 后到下一次刷新前可能漏过滤旧地址，同一台机器上跑多个实例时彼此的地址不属于“本机网卡地址”，不会被过滤。扫描探测端口可由测试钩子覆盖（`override_scan_port_for_test`）。
- **T5**：移动端拒绝建站：`InvalidArgument`，消息为英文。
  - 实施结果（43f4985、764d5a8）：
    - 符号链接：顶层路径先 `canonicalize`，按链接目标登记，条目名取目标文件名；文件夹内部的符号链接一律跳过。文件夹最多 10000 个文件，深度和总字节数不设上限。名字为 `.`、`..`、空、含分隔符，或者路径是根目录，一律返回 `InvalidArgument`。复制前必须满足 `dest != src`，且 `dest` 在 `item_dir` 之内。
    - 复制模式的目录只能用 `station::host::item_dir(data_dir, station, item)` 构造，布局是 `<data_dir>/Station/<station hex>/<item hex>/`，不得手拼。文件放在 `<dir>/<name>`；文件夹放在 `<dir>/<folder name>/<rel>`。`restore_local` 和 `remove_row` 都依赖这个布局。
    - `accept_remote`：`added_by` 取连接身份，`added_at`、`updated_at` 取本机时钟。这里只写库，不校验 `FolderEntry.rel_path`、文件夹名和 `BlobMeta.name`。
    - 签名：`StationHost::remove(&station, &item, now).await`（async）；`set_pinned(&station, &item, pinned, now)`。两者都会校验 `hosted()` 和条目所属站点。`sweep` 是阻塞函数，调用方要放进 `spawn_blocking`。`restore_local` 补登记时 mtime 记 0，迫使首次服务前重新计算哈希。
  - 交给 T6 的待办：
    - 远程内容写盘前，必须清洗 `FolderEntry.rel_path`（拒绝 `..`、绝对路径、盘符、反斜杠）、文件夹名和 `BlobMeta.name`，目标必须落在 `item_dir` 之内。
    - 复制模式登记会覆盖同哈希原有的本地行（例如消息附件的 Source 路径）；删除复制条目后，只会把本地行交还给其他中转站条目。改为：已有本地行且文件仍可读时，不覆盖。
    - `accept_remote` 重复投递已删除的条目会让它复活。需要定墓碑语义：已删除的条目不因旧投递复活。
    - `put_paths` 中途失败时可能留下孤儿副本目录，可由 `sweep` 清理。
- **T6**：`TransferEngine::download` 是 `self: Arc<Self>`，`spawn_download` 绑定 `MessageBlobRow`（transfer.rs:142、287）。先把下载来源抽象出来，再加 `download_to`。权限错误用稳定的英文键。
  - 实施结果（7083eac 及收尾提交）：
    - 签名：`TransferEngine::download_to(meta: BlobMeta, dest_dir: &Path)`（不是计划的 `(hash, name, size, dest_dir)`，下载过程要把分段表写回 `blobs`）；不检查本地是否已有该 Blob，`DownloadBlob` 在调用前自行返回 `already downloaded`。`station_dest_dir` 给出 `<下载目录>/Station/<站点名>/<子目录>`，目录名用英文。
    - `synced_until` 语义：只由 Snapshot 推进（取收到条目的最大 `updated_at`，不是 `now`），Delta 不推进；重连 Subscribe 的 `since` 回退 1 秒，重复项靠「updated_at 取新者」和「墓碑不可逆」去重。
    - 应答错误文本：主机对 `Rejected/InvalidArgument/NotFound` 只发内层文本，拉取端统一包成 `Rejected(文本)`，最终 `ApiError.message` 不带变体前缀。
    - 复制条目由主机顺序拉取，失败由 60 秒周期任务重试（重试失败只记 debug）；孤儿副本目录一小时后由 `sweep` 清理。
    - `sweep_orphan_dirs` 以条目目录自身的 mtime 判断是否过期：复制耗时超过 1 小时的大文件夹，在条目入库前有极低概率被误删。
    - 对非自己设备的拒绝应答固定 `req = 0` 且静默（对方不处理，也无法据此探测站点是否存在）。
    - 未测：文件夹条目的端到端拉取与按哈希下载（子目录注入）、站点名含特殊字符时的落地目录；联系人 Subscribe 只能验证「看不到站点」，拒绝应答本身无法在测试里观察。
- **T7 实施补充**：`StationView` 追加 `is_local: bool`（C5 追加，`host == 本机设备`；Android 的 uniffi 绑定重新生成后 Kotlin 构造处需补该字段）；`--station-put` 用它找本机站点，找不到时创建后重新列出并取最早的本机站点（多个进程同时放入时可能各建一个，留下的空站点需手动删除）；`--station-put` 失败只写 `logs/lanmsg.log` 并以退出码 1 结束：windui 的系统通知依赖托盘，无窗口、无托盘的进程不能用，Windows 上没有失败弹窗（已知限制）；断线 / 后台退出 / 拉起失败时 `station_putting` 清零；中转站会话不进桌面会话列表，因此不计入未读。
- **T7**：文案全部进 `strings.rs`；`--station-put` 加到 `parse_cli`（main.rs:50）；`Effect` 以现有定义为准。
- **T8**：错误状态存 `ErrorCode`。Android outbox 的 7 天清理不得删除中转站引用、主机尚未拉取的副本，要补测试。
- **T9 / T10**：照原计划，日志和错误改英文。LocalSend 协议字段以官方 v2 协议文档为准，需要联网核对。
  - T9 实际签名：`discovery::spawn(me, mcast_port, announce_enabled, events)`，组播端口与 HTTP 端口（`me.port`）相互独立。T10 的 `LsConfig` 拆成 `http_port` 与 `mcast_port` 两个字段（默认都取 `LOCALSEND_PORT`）；测试里 HTTP 端口用 0 时不要把 0 传给组播端口，或者不启用发现。
  - T10 的 `client.rs` 复用 T9 中构造宽松 TLS 客户端的函数，不要另建一个。`announce_enabled=false` 时不发通告、不 register，这条行为的测试在 T10 补上。
  - 已知限制：组播只加入默认网卡（discovery.rs `join_multicast_v4(.., UNSPECIFIED)`），联调时再决定是否按网卡逐个加入。
- **T11**：对端指纹先统一转成小写，再入库、计算 `ConvId::localsend` 和设备 ID。新增设置项 `localsend_port`（默认 53317，用户询问后追加；端口被占用时的提示里指引用户改端口）。 `RequestView.preview` 用 `RequestPreviewView::Files{..}`；新增 PIN 系统事件变体（全局修订 4）；把 `ServiceConfig.localsend_port`（mod.rs:52、141）接入。
  - 同一指纹可能在多个 IP 下有多条记录（`LocalSend::peers()` 键为 (ip, 指纹)）；桥不得按指纹取第一条。
  - `trusted` 自动接受须同时核对：来源 IP 与该指纹上次记录的 IP 一致（`LsEvent` 的 `peer.ip`），并按 `classify` 过滤非 Lan 来源。
  - 审批通过后若发送方收到 409（会话被占用）：界面按“对方忙”处理，不重复弹窗；`prepare-upload` 待审批名额有限（全局 8、每源 IP 1，超出 429）。
  - `LocalSend::shutdown` 目前只停服务与发现，不清理进行中会话的 `.part`，也不发 `SessionFinished`；T11 接入开关时处理。
  - **T11 落地说明（实现与审阅修订）**：
    - 信任模型：LocalSend 兼容的“信任” = 指纹 + 来源 IP，**不是密码学身份**。入站方向（prepare-upload 里的指纹是对端自报）无法校验；出站方向已按通告指纹固定证书（`lanmsg_localsend::pinned_client`，SHA-256 不符则握手失败，仍允许自签）。已信任对端换 IP 后重新询问，发送只发往记录的地址。
    - 发现阶段只放内存，不落库、不建会话；只有用户接受 / 加为联系人 / 屏蔽，或用户首次向在线对端发送（`ensure_conv`）时才落库，库内对端上限 512。已信任对端的别名、型号、地址只在用户确认时更新；别名过滤控制与 Unicode 格式字符。
    - C5 追加：`SystemEvent` / `SystemEventView` 的 `LocalSendPin`、`LocalSendBusy`；`SettingsDto.localsend_port`；`RequestView.source_addr`（来源 IP）与 `addr_changed`（已信任但地址不同，界面提示）。
    - 首次联系：发现到的设备在用户首次发送前没有会话行，界面需要由 `PeerOnline` 以外的入口取得会话 ID（`ConvId::localsend(小写指纹)`），由 T15/T16 的“附近设备”入口处理。
- **T12（安全审阅后修订，已实现）**：
  - HKDF 输出 128 字节：`k_page | k_host | k_enc_p2h | k_enc_h2p`（各 32）。加密按方向分离：页面发往宿主用 `k_enc_p2h`，宿主发往页面用 `k_enc_h2p`，防反射。`SessionKeys` 提供 `seal_to_page` / `open_from_page`（宿主端）与 `seal_to_host` / `open_from_host`（页面侧、测试用），原 `k_enc`、`seal`、`open` 删除。`vectors.json` 的每种模式含 `k_enc_p2h`、`k_enc_h2p`、`sealed_p2h`、`sealed_h2p`。
  - `derive` 与 `host_respond` 返回 `Result<_, CoreError>`：X25519 共享密钥非贡献性（零点等低阶公钥）时返回 `InvalidArgument`。
  - `WebSecrets::rotate_code(&mut self)` 同时重生成 `code`、`scrypt_salt` 并重算 `code_auth`；`CodeGuard::record_success()` 清零失败计数（确认成功时调用）。
  - `SessionKeys`、`WebSecrets` 实现 `ZeroizeOnDrop`，不再 `Clone`。
  - 配对码规范化 `normalize_code`：仅 ASCII 大写并去除空白。JS 侧必须做同样处理。
- **T13（对 T12 的补充）**：`crypto.js` 的 `derive` 返回 `{kPage, kHost, kEncP2h, kEncH2p, th}`，页面发送用 `kEncP2h`、接收用 `kEncH2p`；JS 配对码输入只做 ASCII 大写与去空白，输入 `0`、`1`、`I`、`O` 时提示“配对码不含 0、1、I、O”（字母表 `23456789ABCDEFGHJKLMNPQRSTUVWXYZ` 含 `L`，`L` 不得改写）。
- **T14（对 T12 的补充）**：加密帧的明文头必须带 `seq`（u64，每个方向独立；接收方按下方 T13 落地约定的滑动窗口拒绝已见值与过旧值，防重放）；文件分块帧另带 `file_id` 与 `chunk_idx`，并在认证范围内（放在明文头中随 secretbox 一起认证），防止分块被换位或跨文件替换。配对码确认失败走 `CodeGuard::record_failure`，成功走 `record_success`（只在 `verify_page_mac` 通过后调用；denied、超时不清零），达到上限时 `rotate_code`。
- **T13（落地约定，供 T14 对齐，第 2 轮修订）**：
  - **seq**：页面到宿主的所有加密 JSON（WS 消息、`upload/init`、请求头）与分块都带页面侧唯一递增的 `seq`，重试也换新 `seq`。宿主到页面的所有加密 JSON 与分块也带 `seq`，是**会话级单调计数器**（WS 帧与 HTTP 加密响应共用，不随 WS 重连重置）。两个方向的接收方都用“拒绝已见值与过旧值”的滑动窗口（窗口 4096，`crypto.js` 的 `newSeqWindow`），而不是严格连续，因为并发请求与 WS 会交错到达。
  - **分块明文**：`seq(u64 BE) || fid_len(u8) || fid(ASCII) || chunk_idx(u32 BE) || data`，下载响应同格式（页面校验 fid、idx 与 seq）。
  - **请求认证头**：`finish` 与 `download` 带 `X-LanMsg-Req: b64u(seal(k_enc_p2h, JSON{op, seq, fid[, idx]}))`，`op` 为 `"finish"` 或 `"download"`；宿主校验 `op` 与请求路径一致、fid（及 idx）与路径一致且 `seq` 未重放。`PUT` 分块明文中的 fid 与 chunk_idx 同样必须与 URL 路径一致，不一致拒绝。
  - **seq 计数器**：从 1 开始，保持在 2^53 以内（JS 安全整数）；超限则结束会话。宿主重试下载同一分块时必须重新封包、分配新 seq，不得缓存密文。
  - **T14 测试要求**：断言 HTTP 响应头含 CSP（含 `frame-ancestors 'none'`）、`X-Content-Type-Options: nosniff`、`Referrer-Policy: no-referrer`。
  - **WebSocket 升级认证（T14 落地）**：升级请求查询参数 `req=b64u(seal(k_enc_p2h, {op:"ws", seq}))`，宿主校验 `op` 与 `seq` 未重放，缺失或不符 403；`connectWs` 每次（含重连）换新 `seq`。
  - **握手与资源限制（T14 落地）**：`confirm`/`wait` 要求来源 IP 等于 `hello` 的来源 IP；未完成握手总数 ≤ 5、单 IP ≤ 2，满了拒绝新的（429）；会话活动时间只在解密成功且 `seq` 通过窗口后刷新；会话声明的上传总量 ≤ 64GiB（413），创建 `.part` 前检查可用磁盘空间（不足 507，页面文案 `no_space`）。
  - **部署限制（T14 落地）**：网页传输的来源判定按 TCP 对端地址，回环地址按 Lan 处理，因此**不得放在本机反向代理（nginx、Caddy、隧道客户端等）之后**，否则公网请求会以回环来源通过检查。`hello` 响应不再返回 `host_name`（改放加密的 `welcome` 帧）；上传文件名截断到 200 字节并保留扩展名；`WebStart` 时清理遗留的 `.{32位hex}.part`；WS 单条消息上限 2MiB。
  - **残留风险（T14）**：未完成握手有总数（5）与单 IP（2）上限，多个来源 IP 协同仍可暂时占满名额，使新浏览器短时间收到 429（握手 30 秒过期自动释放）；局域网内的信任模型下接受此风险。
  - **响应回显**：`upload/init` 响应（密封 JSON）回显 `client_key`；`finish` 响应（成功与 409）回显 `fid`；页面校验不符即失败。
  - **页面安全头（T14 在 HTTP 响应头发出）**：`Content-Security-Policy: default-src 'self'; connect-src 'self'; img-src 'self' blob:; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`（`frame-ancestors` 不能放 meta，只能放响应头）、`X-Content-Type-Options: nosniff`、`Referrer-Policy: no-referrer`。
  - 页面读出 `#k=` 后立刻 `history.replaceState` 清除地址栏片段；会话结束时清零并丢弃密钥。
- **T13**：网页文案放 `web/strings.js`（`zh`、`en` 两套，按 `navigator.language` 选择）；`check-no-cjk.py` 扩展到扫描 `web/`，`web/strings.js` 列入白名单。页面总大小 < 300KB 的限制不变。
- **T14**：访客会话标题由 core 返回空字符串，界面按会话类型显示本地化标题；上传目录名用英文。
- **T15**：手动添加的错误走 `error_text`；扫描结果和"公网"等文案进 `strings.rs`；托盘文案进 lanmsgd `strings.rs`。
- **T16**：测试断言 `ErrorCode`；`Conditions` 在现有 4 个字段的基础上追加 `webHosting`。
- **T17**：在 wind-installer 仓库（/home/dufeng/develop/windinput/wind-installer）实施，提权复用 `util/admin.rs`。遵守该仓库 AGENTS.md 和 `--only` 提交纪律，**不推送**。
- **T18**：托盘文案进 strings.rs；`--add-firewall-rule` 的命令行输出用英文。
  - 实施补充：netsh/提权/规则判断放在 `lanmsg-core/src/firewall.rs`（`windows` 依赖仅 Windows 编译），启动检查与命令行在 `lanmsgd/src/firewall_cmd.rs`。缺失状态用运行时共享的 `tokio::sync::watch` 传递：IPC 连接建立时按当前值补发 `FirewallMissing`，之后值变化再推送；缺失期间后台每 30 秒复查一次（不在每次连接时跑 netsh）。托盘菜单项只有 `enabled` 没有可见性（windui 限制），缺失时才可点。判断规则按 `-----` 分块：同一块内须含程序路径、已启用、入站、允许（中英文标签都认）；netsh 输出按 OEM 代码页解码（`GetOEMCP` + `MultiByteToWideChar`）。提权 `ShellExecuteExW` 设 `lpDirectory`，调用线程先 `CoInitializeEx`。
  - C5 追加（第 3 轮）：`Event::FirewallOk`（放在 `Event` 末尾），防火墙状态由缺失变为已放行时推送，界面据此隐藏提示；Android 绑定重新生成后若有对 `Event` 的穷尽 `when` 需补分支。托盘提权成功（命令退出码 0）直接把共享状态置为已放行，不依赖解析 netsh 输出复查。界面（桌面）自己提权成功后只在本地隐藏横幅，后台服务要等下一次定期复查才更新共享状态。
  - 后续项（wind-installer，未做）：`wind-installer/src/installer/firewall.rs` 有同样的两个问题——`run_netsh_output` 用 `from_utf8_lossy` 解码而非 OEM 代码页（现靠“路径含非 ASCII 时只要 show 成功就视为匹配”的回退），以及 `rule_matches_program` 不按规则块判断（同名规则被禁用、拦截或出站时也会被当作有效）。T17 对应的安装器改动应同步修正。
- **T19**：图标已有（`crates/lanmsg-desktop/assets/lanmsg.ico`、`scripts/gen-win-icon.py`），不需要占位图标；`packaging/assets` 从这里生成或引用。
  - 实施补充：`app.toml` 的 icon/logo 直接引用 `crates/lanmsg-desktop/assets/lanmsg.ico` 与 `packaging/assets/lanmsg-256.png`，不另存副本。暂不写 `url_protocol = "lanmsg"`（桌面端尚未处理 `lanmsg://`，与 Linux/macOS 一致，启用时一并处理）。sendto 快捷方式的 `--station-put` 依赖 T7。`build-dist.ps1` 增加 `-Version`（默认取 workspace 版本）、`-WindInstallerTag`（CI 用，Task 22 提供包含 firewall/sendto 能力的 wind-installer tag；未给 `-WindInstallerDir` 且无 tag 时报错）、`-SkipBuild`；wind-installer 只发布 x64 stub，arm64 安装包也用 x64 stub（经模拟运行）。脚本仿照 wind-installer 的 `pack.ps1`：先做 zip，再注入并加工卸载器（`prep-uninstaller`），最后 `build`。`scoop/lanmsg.json` 的仓库用 `@REPO@` 占位，由 `update-manifest.sh` 的 `REPO_SLUG`（默认 `huanfeng/lanmsg`）填充。`scripts/check-packaging.sh` 汇总 shellcheck、app.toml 解析、scoop 清单、macOS dry-run 与 Linux 安装测试（`--fast` 跳过后者），本机无 pwsh，ps1 未做语法检查，需在 CI（windows runner）验证；清单已用 wind-installer 的 `ProjectConfig::from_toml_str` 实测解析通过。Task 22 接入 CI 时调用 `scripts/check-packaging.sh`。
  - 第 2 轮：`[paths] data` 改为 `%APPDATA%\lanmsg\LanMsg\data`（core 默认数据目录不变，保护已有配对数据），去掉 `[localdata]`（缓存与日志在数据目录内）。已确认 wind-installer 原先的 `paths.data` 只是向导里数据目录输入框的初值，卸载“删除用户数据”写死删 `%APPDATA%\{app.id}`。第 3 轮：wind-installer 新增可选清单字段 `[paths] user_data`（提交 693db55，本地未推送；缺省行为不变，须以 `%APPDATA%\` / `%LOCALAPPDATA%\` 开头、其下至少一级、无 `..`/盘符/其他变量），LanMsg 用 `user_data = %APPDATA%\lanmsg\LanMsg`（directories 的应用目录；core 只用其下的 `data`，`config_dir`/`cache_dir` 当前代码未使用，`cache`、`logs` 均在 `data` 内；删整个应用目录可覆盖将来可能出现的 `config`，空的 `%APPDATA%\lanmsg` 会留下）。CI 下载的 wind-installer tag 必须包含该字段。`paths.rs` 的 Windows 路径注释已更正。scoop `autoupdate` 加 `hash.url = $baseurl/SHA256SUMS`（scoop 对 `<hash>  <文件名>` 多行格式按下载 URL 的文件名自动匹配，release 须上传 `SHA256SUMS` 资产）。
  - 后续核对（未做）：合并后核对防火墙规则名与 `lanmsgd --add-firewall-rule` 的 `FIREWALL_RULE_NAME` 一致（安装器按“规则名 + 程序路径”标识规则）；核对 `--station-put` 参数已由 T7 实现。已知局限：`SHA256SUMS` 与 wind-installer 工具来自同一 release，无独立信任根，靠固定 tag 缓解。
- **T20**：把 `lanmsgd` 的 windui 依赖改为 optional，归入 `desktop` feature。
  - 实施补充：`LANMSG_BASE_URL` 现为发布页根（默认 `https://github.com/huanfeng/lanmsg/releases`），下载目录为 `<根>/latest/download` 或 `<根>/download/<版本>`；`LANMSG_NO_SYSTEMD=1` 跳过 systemd 操作（测试用）。安装时二进制先复制到临时名再 `mv` 原子替换，服务已运行则 `restart`；`--prefix` 含空白、`|`、`&`、`\` 时拒绝；解压前预检成员路径。
  - 后续（未做）：`lanmsg.desktop` 暂不声明 `MimeType=x-scheme-handler/lanmsg`，待桌面端处理 `lanmsg://` 参数（T7/T15）后一并启用；`lanmsgd.service` 去掉了对用户单元无效的 `After=network-online.target`，桌面版若在图形会话就绪前启动会以无头运行，是否改为依赖 `graphical-session.target` 需评估。
- **T21**：只写脚本，在 CI 的 macOS runner 上产出未签名 dmg；本地无法验证，记为外部依赖。
  - 实施补充：未创建 `name.apk.lanmsg.daemon.plist`（LaunchAgent 由 `autostart.rs` 以标识 `app.lanmsg.daemon` 写入，与 bundle id `name.apk.lanmsg` 是两回事）。`build-dmg.sh` 支持 `--dry-run`（Linux 上只组装目录布局并打印 macOS 命令），测试为 `packaging/macos/test-build-dmg.sh`；`NSLocalNetworkUsageDescription` 默认英文，中文放 `zh-Hans.lproj/InfoPlist.strings`。`scripts/check-packaging.sh` 属 Task 19，尚未存在，Task 22 接入 CI 时把 `test-build-dmg.sh` 与 shellcheck 加上。lipo/sips/iconutil/hdiutil/签名公证未在 macOS 上实跑，记为外部依赖。
  - 第 2 轮：公证凭据存进构建时创建、退出时删除的临时 keychain（`store-credentials lanmsg-ci`），提交与 staple 用 `--keychain-profile`；`store-credentials` 一次性在命令行带密码，CI runner 单租户可接受。codesign 不再用 `--deep`，先逐个签 `Contents/MacOS/*` 再签 app。`CFBundleVersion` 只取版本号的点分数字部分，`CFBundleShortVersionString` 保留完整版本。
  - 后续（未做）：`Info.plist` 暂不注册 `lanmsg://`（`CFBundleURLTypes`），待 macOS 端处理 Apple Event `kAEGetURL` 时（与 T7/T15 的 `lanmsg://` 参数处理一起）启用；app 本身未 staple（只 staple 了 dmg，离线首次打开 app 时需联网校验）。
- **T22**：windui 依赖改为 `{ git = "https://github.com/huanfeng/wind-ui-rust", tag = "<tag>" }`。本地开发在 `.cargo/config.toml` 里用 `[patch."https://github.com/huanfeng/wind-ui-rust"]` 指回本地路径。注意：LanMsg 目前依赖 windui v0.31.0 之后的 12 个提交，其中 11 个还没推送，所以需要用户先在 windui 仓库发布新 tag（如 v0.32.0）并推送，CI 才能跑通。LanMsg 仓库地址为 `huanfeng/lanmsg`，**不推送、不创建远程仓库**，需要时由用户操作。

## 执行顺序

主线（串行，因为都要改热点文件）：T1 → T2 → T3 → T3B → T4 → T5 → T6 → T7 → T8 → T11 → T14 → T15 → T16 → T18 → T19 → T20 → T21 → T22。

旁线（与主线并行，不碰热点文件；使用独立的 CARGO_TARGET_DIR）：T9 → T10（新 crate `lanmsg-localsend`）；T12 → T13（`web/` 与 `lanmsg-web-assets`）；T17（外部仓库）。

## 用户追加需求（2026-11-08 第二批）：仓库、CI/Release 与自动更新

用户决定：
- 代码推送到已存在的私有仓库 `huanfeng/lanmsg`。旧 Flutter 代码已改名保存在 `flutter-legacy` 分支，`main` 是默认分支。仓库暂时保持私有。
- windui 依赖 tag `v0.32.0`（已发布，提交 78a088c）。wind-installer 依赖 tag `v0.131.0`（包含 firewall、sendto、args、user_data），CI 从源码构建它的工具，不依赖它的 Release 产物。
- Android 正式包使用 WindInputAndroid 的同一把签名。GitHub Secrets 已配置 `ANDROID_SIGNING_KEYSTORE_BASE64`、`ANDROID_SIGNING_STORE_PASSWORD`、`ANDROID_SIGNING_KEY_ALIAS`、`ANDROID_SIGNING_KEY_PASSWORD`；本地签名配置写在 `android/keystore.properties`（已被 gitignore 忽略）。
- 仓库私有时，匿名下载 Release 不可用（scoop 安装、curl 安装、应用内更新都受影响）。这些功能按公开仓库的方式实现；私有期间失败时要给出明确提示，不能崩溃。

### T22（重写）：CI 与发布流水线
- windui 依赖改为 `{ git = "https://github.com/huanfeng/wind-ui-rust", tag = "v0.32.0" }`，涉及 lanmsg-desktop 和 lanmsgd（optional）。不提交 `[patch]`。本地需要联调 windui 时，用 `cargo --config` 临时覆盖，写法记在 README 的开发约定里。
- 版本号只有一个来源：`Cargo.toml` 的 `[workspace.package].version`。Android 的 `versionName` 从它读取，`versionCode = major*10000 + minor*100 + patch`，在 gradle 里解析，不要手写。新增 `scripts/bump-version.sh <x.y.z>`，同步修改版本号和 CHANGELOG。
- `ci.yml`（push/PR 时触发）：
  - Linux 跑 fmt、clippy、test --workspace、check-no-cjk、`node --test web/test/*.test.mjs`、`scripts/check-packaging.sh --fast`。
  - Windows 跑 `cargo test -p lanmsg-desktop -p lanmsgd`。
  - Android 跑 `:app:testDebugUnitTest`。
- `release.yml`（推送 tag `v*` 或 workflow_dispatch 时触发；dispatch 默认只构建、不发布）：
  - Windows x64：build-dist.ps1 生成安装包和 scoop zip。
  - Linux x86_64：桌面版在 ubuntu:20.04 容器构建，无头版在 rust:alpine 用 musl 构建。aarch64 如果 runner 可用就一起构建，否则记为后续。
  - macOS：生成通用 dmg（未签名）。
  - Android：生成签名的 arm64 release APK，校验签名与 Secrets 中的密钥一致。可参考 /home/dufeng/develop/windinput/WindInputAndroid/.github/workflows/release.yml。
  - 生成 `SHA256SUMS`，创建 GitHub Release（附 CHANGELOG 对应段落），并用 `packaging/scoop/update-manifest.sh` 生成 `bucket/lanmsg.json` 提交回 main。仓库本身就是 scoop bucket，安装命令为 `scoop bucket add lanmsg https://github.com/huanfeng/lanmsg`。
  - 产物文件名以现有脚本为准，最终清单写进本附录 T22 条目，供 T23 使用。
- 验证方式：push 后 CI 通过；用 workflow_dispatch 跑一遍 release 只构建、不发布，所有 job 成功。主控确认后再打第一个 tag 正式发布。
- curl 一键安装：README 写明 `curl -fsSL https://github.com/huanfeng/lanmsg/releases/latest/download/install.sh | sh`，因此 install.sh 要作为 Release 资产上传。

### T23（新增）：应用内检查更新与自动更新
- core 新增 `update` 模块：
  - `Request::CheckUpdate`（C5 追加）请求 `https://api.github.com/repos/huanfeng/lanmsg/releases/latest`。地址可由设置或环境变量 `LANMSG_UPDATE_URL` 覆盖，便于测试。按语义化版本比较，返回 `UpdateInfo { version, notes, published_at, assets }`。
  - `Request::DownloadUpdate { asset }` 下载到数据目录下的 `updates/`，用 Release 中的 SHA256SUMS 校验，下载进度通过事件上报。
  - 请求失败（404、私有仓库、离线）返回明确的 ErrorCode，界面显示为"无法检查更新"。
  - HTTP 客户端复用 reqwest（rustls-tls-no-provider，显式使用 aws-lc-rs）。
- 安装渠道检测（新增 `InstallChannel` 枚举）：
  - Windows 安装版：运行下载好的安装包，然后退出界面和 lanmsgd。
  - Windows scoop 版：可执行文件位于 `scoop\apps\lanmsg\` 下。打开一个新的命令行窗口执行 `scoop update lanmsg`，然后退出界面和 lanmsgd。
  - Linux tarball 版：后台执行 Release 中的 install.sh，带 `--version` 参数，完成后提示重启。
  - macOS：下载 dmg 后打开它。
  - Android：core 下载 APK，Kotlin 通过 FileProvider 唤起系统安装器（需要 `REQUEST_INSTALL_PACKAGES` 权限，并处理"允许安装未知应用"的授权跳转）。
  - 其他情况（源码构建、无头版）：只提示有新版本，并给出 Release 页面链接。
- 设置：新增"启动时自动检查更新"开关（默认开，每天最多检查一次）。桌面端和 Android 的设置页都要有"检查更新"入口，显示当前版本、新版本、更新说明，以及"下载并安装"按钮。CLI 新增 `lanmsg-cli update check`。
- 安全：只从固定仓库下载，并校验 SHA256SUMS。SHA256SUMS 与安装包同源，这一限制和 wind-installer 相同，写进文档。
- 拆成两步：T23a 实现 core、桌面端、lanmsgd 和 CLI；T23b 实现 Android。两步都在 T22 产物清单确定之后进行。
