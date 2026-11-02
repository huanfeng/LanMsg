# LanMsg P2 阶段验收报告（2026-11-02）

## 任务完成状态

P2 计划（docs/superpowers/plans/2026-10-28-02-p2-sync-files.md）全部 14 个任务的代码部分已完成并逐个提交：

| 任务 | 内容 | 提交 |
|---|---|---|
| Task 1 | 协议类型、帧变体与数据连接帧 | `aa1890b` |
| Task 2 | 迁移 2 与存储扩展 | `2dd0fa0` |
| Task 3 | 连接层：通道字节、TLS 导出值、未初始化对端、数据连接 | `20ddac6` |
| Task 4 | 平台钩子、设置、资料与集成测试辅助 | `0220749` |
| Task 5 | 联系人双向确认与准入过滤（含 ConnManager 死条目修复） | `639837e` |
| Task 6 | 陌生人消息门控 | `a62ca41` |
| Task 7 | 我的设备配对（扫码 HMAC / SAS） | `4cfd92d` |
| Task 8 | 设备吊销与撤销分发 | `60e39f4` |
| Task 9 | 多设备同步、转交与离线补投 | `70fa6a8` |
| Task 10 | 已读、撤回与本地删除 | `7e9afd8` |
| Task 11 | Blob 发送：哈希、准备中、持有者与数据服务端 | `db01a56` |
| Task 12 | 传输引擎：分段并行、多源、断点续传 | `28b19cd` |
| Task 13+14 | 下载策略与 CLI 扩展、吞吐基准（一并完成） | `f9d03b6` |

## 自动验收项（全部通过）

- `cargo fmt --all --check`：通过
- `cargo clippy --workspace --all-targets -- -D warnings`：0 警告
- `cargo test --workspace`：160 通过 / 0 失败
- `bash scripts/e2e-two-daemons.sh`：`E2E OK`
- `bash scripts/bench-throughput.sh local 512`：`BENCH 278.5`（本机回环，512 MB）
- 占位检查：`grep -rnE 'todo!|unimplemented!|#\[ignore\]|TODO' crates/`（排除 TODO(#）无输出
- CLI 子命令齐全：join/pair-start/pair-join/pair-confirm/devices/remove-device/contacts/request-contact/requests/respond/send-files/send-folder/download/recall/read/settings/profile/files-of（`--help` 实测）

## 待两台/三台真实机器的手工验收（需用户参与）

1. **吞吐**（设计 §17，有线连接）：`bench-throughput.sh remote <对端IP>`，2 GiB 文件各测 3 次取中位数——千兆网 ≥ 110 MB/s；2.5G 网 ≥ 270 MB/s。连同两台机器配置补入本报告。不达标时记录瓶颈分析（CPU/磁盘），不改指标。
2. **扫码配对**（URI 文本代替二维码）：A `pair-start` → B `join` + `pair-join <uri>` → 两边 Done；Sas 方式两端显示相同 6 位数字。
3. **三台设备离线补齐**：关掉 C，A/B 互发若干消息，C 启动后 `log` 能看到全部。
4. **陌生人请求**：第三台未加联系人机器发消息 → `requests` 出现请求 → `respond once` 后可见。
5. **P1 遗留**：两台机器组播发现 + 双机聊天手工测试。

## 已知问题与遗留（记录在案，不阻塞阶段验收）

1. **tokio::fs::File 并发复用连接疑似卡死**（Task 12 发现，根因未明）：已用 spawn_blocking + std::fs 绕过。两机实测若见传输停滞优先怀疑此处。
2. **net 层无优雅关闭**（Task 7 发现）：发帧后立即 disconnect 会 RST 丢在途帧，Pairing 内用 200ms 缓冲规避。根治需冲刷写队列，建议 P3 处理。
3. **simultaneous_dial_keeps_one_connection 测试并行全量时偶发**（约 1/8）：双连接裁决时序抖动，单跑稳定，net 层 Task 5 后未再改动。
4. **本机被吊销仅清内存身份**：重启后 user.key 恢复但对端持续拒绝（无安全缺口），彻底清除留待后续。
5. **undo_recall 恢复状态近似为 sent**（原状态未记录，schema v2 未存 pre-recall 状态）。
6. **reissue_cert 与 SetProfile 顺序耦合**：先 SetProfile 后改设备名会重置显示名。
