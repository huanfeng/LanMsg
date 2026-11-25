# 网页临时会话：真实浏览器端到端（手工 / 非 CI）

2026-11-25 在 P5 验收时补做。`web/` 的 JS 此前只有 Node 的密码学向量测试（`web/test/`）与 Rust 侧集成测试，
从未在真实浏览器里整条链路运行过；这里用 Playwright 驱动 Chromium，对真实的 `lanmsgd` 做 key 与 code 两种模式的双向传输。
**不属于 CI，也不进构建**：依赖 `playwright-core` 与一个 Chromium，仅在需要复核网页端时手工运行。

## 覆盖的检查

- 进入会话：key 模式（URL 的 `#k=` 片段）与 code 模式（手输 8 位配对码），宿主逐个允许访客后进入聊天页，连接指示为在线。
- 浏览器 → 宿主：中英文与符号的文字；3.5 MB、中文文件名的文件（多分片加密上传），宿主落盘文件的 SHA-256 与原文件一致。
- 宿主 → 浏览器：文字；5 MB 文件，浏览器解密、拼接并触发下载，内容哈希与原文件一致。
- 浏览器无 `console error` / `pageerror` / 请求失败。
- 宿主执行 `WebStop` 后，页面回到门页并显示 “Session ended”，不再提供输入框（`STOP=1`，只在最后一个模式里做）。

## 运行方式

1. 启动一个全新的无头宿主并初始化（数据目录与 socket 路径取短路径）：

   ```sh
   D=/tmp/lm-web; mkdir -p $D/run; export XDG_RUNTIME_DIR=$D/run
   target/debug/lanmsgd --no-multicast --port 0 --data-dir $D --headless &
   target/debug/lanmsg-cli --data-dir $D init --name host --device-name hostbox
   ```

2. 开启网页会话并让助手自动允许访客（保持运行，它会把会话信息写到 `session.json`）：

   ```sh
   python3 -I webhost.py $D/run/lanmsg/lanmsgd.sock "$PWD" &
   ```

3. 跑两种模式（脚本在本目录下写 `up/`、`down/`、截图与 `<mode>-results.json`）：

   ```sh
   export PLAYWRIGHT_CORE=/path/to/node_modules/playwright-core   # 可省，默认按包名解析
   export CHROMIUM=/path/to/chrome                                # 可省，默认用 Playwright 自带的
   node run.mjs key  $D/run/lanmsg/lanmsgd.sock "$PWD"
   STOP=1 node run.mjs code $D/run/lanmsg/lanmsgd.sock "$PWD"
   ```

4. 结束后停掉 `lanmsgd` 与 `webhost.py`（用 PID，不要用 `pkill -f`：它会匹配到自己的命令行）。

`ipc.py` 是一个最小的 IPC 客户端（`u32` 大端长度 + CBOR 的 `IpcFrame`），只用于本脚本。

## 结果（2026-11-25，本机 Chromium，宿主为 `main` 的 debug 构建）

- key 模式 11/11 通过；code 模式 13/13 通过。
- 首轮跑出的唯一问题：浏览器自动请求 `/favicon.ico` 得到 404（console 噪声）。已加同源 `favicon.svg` 并在 `index.html` 引用
  （页面 CSP 的 `img-src` 不含 `data:`，所以没有用 data URI），`lanmsg-web-assets` 有对应单测。
- 同名文件再次上传时宿主不会覆盖，而是改名为 `<名字> (1).bin`（实测，符合预期）。

截图：`code-0-gate.png`（配对码输入）、`key-3-final.png`（传输完成后的聊天页）、`code-4-ended.png`（宿主停止会话后）。

## 没有覆盖的部分

- 真正的手机浏览器（Android Chrome 扫码、iOS Safari 手输配对码）与真实 Wi-Fi 下的吞吐；
- 30 分钟空闲自动结束（只验证了宿主主动停止后的“已结束”页面）；
- 大于 1 GB 文件的确认弹窗路径。
