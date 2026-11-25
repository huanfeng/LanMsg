#!/usr/bin/env python3
# 网页临时会话真实浏览器 e2e 的宿主助手：
#   1) 连接 lanmsgd 的 IPC，发 WebStart，把会话信息写到 session.json；
#   2) 保持连接，收到 WebRequest 事件就自动发 WebRespond{allow:true}；
#   3) 其它事件全部打到标准输出（带时间戳），便于对照浏览器侧行为。
# 用法: webhost.py <sock> <outdir>
import json, os, socket, struct, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ipc  # 同目录的 ipc.py：复用 enc/dec/recvn

sock, outdir = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX)
s.settimeout(600)
s.connect(sock)


def send(rid, body):
    p = ipc.enc({"Request": {"id": rid, "body": body}})
    s.sendall(struct.pack(">I", len(p)) + p)


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


send(1, "WebStart")
rid = 2
while True:
    n = struct.unpack(">I", ipc.recvn(s, 4))[0]
    f, _ = ipc.dec(ipc.recvn(s, n))
    if "Response" in f:
        r = f["Response"]
        log("RESPONSE", r["id"], json.dumps(r["result"], default=str, ensure_ascii=False)[:400])
        if r["id"] == 1:
            with open(os.path.join(outdir, "session.json"), "w") as fh:
                json.dump(r["result"], fh, default=str)
    elif "Event" in f:
        ev = f["Event"]
        log("EVENT", json.dumps(ev, default=str, ensure_ascii=False)[:500])
        if isinstance(ev, dict) and "WebRequest" in ev:
            req = ev["WebRequest"]["request"]
            send(rid, {"WebRespond": {"request": req, "allow": True}})
            log("ALLOWED", req)
            rid += 1
