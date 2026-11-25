#!/usr/bin/env python3
# 最小 IPC 客户端：u32 BE 长度 + CBOR(IpcFrame)。用法: ipc.py <sock> <json-request-body>
import socket, struct, sys, json
def enc(o):
    if o is None: return b'\xf6'
    if o is True: return b'\xf5'
    if o is False: return b'\xf4'
    if isinstance(o, int):
        assert o >= 0
        return hd(0, o)
    if isinstance(o, str):
        b = o.encode(); return hd(3, len(b)) + b
    if isinstance(o, list): return hd(4, len(o)) + b''.join(enc(x) for x in o)
    if isinstance(o, dict): return hd(5, len(o)) + b''.join(enc(k)+enc(v) for k, v in o.items())
    raise TypeError(o)
def hd(m, n):
    if n < 24: return bytes([m<<5|n])
    if n < 256: return bytes([m<<5|24, n])
    if n < 65536: return bytes([m<<5|25]) + struct.pack('>H', n)
    if n < 2**32: return bytes([m<<5|26]) + struct.pack('>I', n)
    return bytes([m<<5|27]) + struct.pack('>Q', n)
def dec(b, i=0):
    ib = b[i]; m = ib >> 5; a = ib & 31; i += 1
    if m == 7:
        if a == 20: return False, i
        if a == 21: return True, i
        if a in (22, 23): return None, i
        if a == 25: return struct.unpack('>e', b[i:i+2])[0], i+2
        if a == 26: return struct.unpack('>f', b[i:i+4])[0], i+4
        if a == 27: return struct.unpack('>d', b[i:i+8])[0], i+8
        raise ValueError(ib)
    if a < 24: n = a
    elif a == 24: n = b[i]; i += 1
    elif a == 25: n = struct.unpack('>H', b[i:i+2])[0]; i += 2
    elif a == 26: n = struct.unpack('>I', b[i:i+4])[0]; i += 4
    elif a == 27: n = struct.unpack('>Q', b[i:i+8])[0]; i += 8
    else: raise ValueError('indef')
    if m == 0: return n, i
    if m == 1: return -1-n, i
    if m == 2: return bytes(b[i:i+n]), i+n
    if m == 3: return b[i:i+n].decode(), i+n
    if m == 4:
        r = []
        for _ in range(n):
            v, i = dec(b, i); r.append(v)
        return r, i
    if m == 5:
        r = {}
        for _ in range(n):
            k, i = dec(b, i); v, i = dec(b, i)
            r[k if not isinstance(k, list) else tuple(k)] = v
        return r, i
    if m == 6:
        return dec(b, i)
def recvn(s, n):
    buf = b''
    while len(buf) < n:
        c = s.recv(n - len(buf))
        if not c: raise EOFError
        buf += c
    return buf
def call(sockpath, body, rid=1, timeout=120):
    s = socket.socket(socket.AF_UNIX); s.settimeout(timeout); s.connect(sockpath)
    p = enc({"Request": {"id": rid, "body": body}})
    s.sendall(struct.pack('>I', len(p)) + p)
    while True:
        n = struct.unpack('>I', recvn(s, 4))[0]
        f, _ = dec(recvn(s, n))
        if isinstance(f, dict) and 'Response' in f and f['Response']['id'] == rid:
            s.close(); return f['Response']['result']
if __name__ == '__main__':
    print(json.dumps(call(sys.argv[1], json.loads(sys.argv[2])), default=str, ensure_ascii=False))
