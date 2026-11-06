# LanMsg 单机端到端功能验收报告（2026-11-06）

脚本：`scripts/e2e-acceptance.sh`（真实 `lanmsgd` + `lanmsg-cli`，每实例隔离 data-dir 与 XDG_RUNTIME_DIR，默认 `--no-multicast` + `--peer` 直连；`--only <场景>` 单跑）。
基线：HEAD=9f006d8，debug 构建。未修改 `crates/` 任何代码。

## 场景结果（两遍完整运行）

| 场景 | 第 1 遍 | 第 2 遍 | 说明 |
|---|---|---|---|
| text_chat | 通过 | 通过 | 6 条交替消息，双方顺序一致 |
| pair_qr | 通过 | 通过 | 两端同一 user_id，devices 两边均列出两台 |
| pair_profile（新增） | 失败 | 失败 | 缺陷 1 |
| pair_sas | 通过（含绕过） | 通过（含绕过） | 6 位数字两端相同；yes/yes Done；一端 no 双端 Failed。需组播，见缺陷 2、3 |
| offline_catchup | 通过 | 通过 | C 关闭期间 A/B 在我的设备会话、A 与 D 在私聊互发，C 重启后全部补齐且与 A 一致 |
| sibling_dm_sync（新增） | 失败 | 失败 | 缺陷 4 |
| stranger_request | 通过 | 通过 | 请求出现、once 后可见；block 后无新请求且消息不可见 |
| file_transfer | 通过 | 通过 | 64MB（约 190-200MB/s）与小图片 sha256 一致 |
| folder_transfer（新增） | 失败 | 失败 | 内容完整，目录结构不符，缺陷 5 |
| file_resume | 失败（脚本环境，见下） | 通过 | 768MB 下载中 kill -9 B，重启后续传，sha256 一致 |
| revoke | 通过 | 通过 | 移除后 B 收不到我的设备会话与私聊新消息；A 的 devices 直接不再列出该设备（无“已移除”标记，CLI 无此显示） |
| recall_read | 失败（脚本环境，见下） | 通过 | 通过 watch 事件观察到 Read 与 Recalled，撤回后对端 log 不再显示原文 |
| bench_local | 通过 | 通过 | 记录：256MB 平均 230.7 与 269.4 MB/s（无阈值） |

稳定性：除两个“环境类”失败外两遍一致。第 1 遍 file_resume 与 recall_read 均在“b 发现 a”处超时（静态 `--peer` 直连未建立）。怀疑是固定测试端口与同机并发的其他验收实例冲突（守护进程端口被占会静默回退随机端口，`--peer` 即落空；日志中曾见“端口被占用，回退到随机端口”），未能证实。脚本随后改为每次运行随机端口基数，之后单跑 recall_read、text_chat、revoke 各通过；此改动之后未重跑完整两遍。若再现，应视为静态直连的潜在偶发问题继续跟踪。

## 产品缺陷清单

1. **新设备配对后 `self` 的 display_name 为空**（pair_profile）。QR 配对完成后 B 的 `self` 显示 `display_name: -`，A 为 `甲`。初判：`crates/lanmsg-core/src/service/handlers.rs:92-96`（get_self 读 setting `display_name`）与 `crates/lanmsg-core/src/pairing.rs:~795-870`（加入方只写 users 行和 hello，未 `set_setting("display_name", bundle.display_name)`）。
2. **SAS 配对响应方会话 ID 不一致，CLI 看不到数字**（pair_sas，已绕过）。`pair-start --sas` 返回的 session 与随后 PairStatus 事件里的 session 不同（事件用的是加入方 `random_session()`，`pairing.rs:~428`），而 `lanmsg-cli pair-start` 按返回的 session 过滤事件（`crates/lanmsg-cli/src/main.rs` watch_pair_status），因此永远等不到 `ShowSas`；用返回的 session 做 `pair-confirm` 也不对应。另 `pair-join` 先调用再订阅事件（main.rs PairJoin 分支），可能错过快速出现的 ShowSas。脚本改用 `watch` 事件流取数字与会话 ID。
3. **SAS 按设备 ID 加入依赖组播发现表**（限制，非必然缺陷）。`pairing.rs:422-424` 用 `discovered.get(&d)`，静态 `--peer` 不会填充；`--no-multicast` 下 `pair-join <device>` 返回 NotFound，填充前还会返回 PeerOffline。pair_sas 因此启用组播（本机回环可用，场景耗时 23-43s）。
4. **在线的同用户设备收不到与第三方的私聊会话**（sibling_dm_sync）。A 与 D 私聊后，同用户在线设备 B 始终没有该会话，A 的外发消息和 D 的来信都不同步（C 离线后重启靠 offer 路径能补齐，说明只有实时推送路径有问题）。日志：`SyncPush conv 不匹配，丢弃`。初判：`crates/lanmsg-core/src/msg.rs:604-616`，会话不存在时只接受 `conv == dm(self, author_user)`，作者是自己的同用户设备时（author_user == self）恒不成立，应改为在已知联系人中匹配 `dm(self, 联系人)`。附带观察：B 的 `contacts` 里联系人 D 的显示名是短号而非“丁”。
5. **文件夹下载落地路径多一层同名目录且丢失文件夹名**（folder_transfer）。源 `项目目录/sub/deep/d1.bin` 落到 `<下载目录>/<会话标题>/sub/deep/d1.bin/d1.bin`（文件名重复为目录），且没有 `项目目录` 这一层。内容 sha256 全部正确。初判：`crates/lanmsg-core/src/blob/mod.rs:~84-92`（walk_dir 的 `rel_path` 含文件名本身）与 `crates/lanmsg-core/src/blob/transfer.rs:241-247`（dest_path_for 逐段拼接 rel_path 后再拼 name）；文件夹根名称也未进入 rel_path。
6. **（未证实）疑似向非成员推送**：probe 运行中 D（另一用户）日志出现一次来自 A 的 `SyncPush conv 不匹配，丢弃`，可能是 A 向 D 推送了不属于 D 的会话信封（隐私）。未能定位是哪个会话，建议审查 `sync.rs` 的 `shared_convs`/`run_offer` 推送范围。
7. 次要（CLI）：`log` 把 System 消息显示为 `<不支持的消息类型>`；同用户其他设备发的消息在 `log` 里作者一律显示“我”，无设备名。

## 无法自动化 / 仍需真机

- 真实组播发现（两台机器、两网卡）：脚本只在单机回环验证组播给 SAS 加入用，无法覆盖跨机、防火墙与网段情况。
- 两机吞吐（千兆 ≥110 MB/s、2.5G ≥270 MB/s）：本机回环数字不具代表性。
- 真二维码扫码：CLI 以 `lanmsg://` URI 文本代替。
- “A 的 devices 显示已移除”：CLI 的 `devices` 对被移除设备只是不再列出，没有“已移除”状态字段，只能验证“不再列出”。
- 已读状态：CLI `log` 不显示消息状态，已读、撤回均通过 `watch` 的 `MessageUpdated` 事件判断。

## Bench（bench_local，仅记录）

`bash scripts/bench-throughput.sh local 256`：230.7 MB/s（第 1 遍）、269.4 MB/s（第 2 遍）。另 file_transfer 中 64MB 下载 189-199 MB/s。
