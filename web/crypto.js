// LanMsg 网页会话密码学。算法与 crates/lanmsg-core/src/web/crypto.rs 逐字节一致。
// 浏览器：全局 LanMsgCrypto（需先加载 vendor/nacl-fast.min.js 与 vendor/scrypt.js）。
// Node：module.exports（测试用）。
(function (root) {
  "use strict";
  var isNode = typeof module !== "undefined" && module.exports;
  var nacl = isNode ? require("./vendor/nacl-fast.min.js") : root.nacl;
  var scryptLib = isNode ? require("./vendor/scrypt.js") : root.scrypt;

  var NONCE_LEN = 24;
  var TAG_LEN = 16;
  var enc = new TextEncoder();

  function concat() {
    var n = 0, i;
    for (i = 0; i < arguments.length; i++) n += arguments[i].length;
    var out = new Uint8Array(n), o = 0;
    for (i = 0; i < arguments.length; i++) {
      out.set(arguments[i], o);
      o += arguments[i].length;
    }
    return out;
  }

  function b64uEncode(u8) {
    var s = "";
    for (var i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }

  function b64uDecode(str) {
    var s = str.replace(/-/g, "+").replace(/_/g, "/");
    while (s.length % 4) s += "=";
    var bin = atob(s);
    var out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function sha512(u8) {
    return nacl.hash(u8);
  }

  // HMAC-SHA512，块长 128 字节。
  function hmacSha512(key, msg) {
    var block = 128;
    if (key.length > block) key = sha512(key);
    var k = new Uint8Array(block);
    k.set(key);
    var ipad = new Uint8Array(block), opad = new Uint8Array(block);
    for (var i = 0; i < block; i++) {
      ipad[i] = k[i] ^ 0x36;
      opad[i] = k[i] ^ 0x5c;
    }
    return sha512(concat(opad, sha512(concat(ipad, msg))));
  }

  // RFC 5869，HKDF-SHA512。
  function hkdfSha512(ikm, salt, info, len) {
    var prk = hmacSha512(salt, ikm);
    var out = new Uint8Array(len), t = new Uint8Array(0), pos = 0, ctr = 1;
    while (pos < len) {
      t = hmacSha512(prk, concat(t, info, new Uint8Array([ctr++])));
      var n = Math.min(t.length, len - pos);
      out.set(t.subarray(0, n), pos);
      pos += n;
    }
    return out;
  }

  // 配对码规范化：仅 ASCII 大写并去除空白（与 Rust normalize_code 一致）。
  function normalizeCode(code) {
    return code.replace(/\s+/g, "").replace(/[a-z]/g, function (c) {
      return c.toUpperCase();
    });
  }

  // scrypt(N=16384, r=8, p=1, dkLen=32)。
  function scryptCode(code, salt) {
    return scryptLib.scrypt(enc.encode(normalizeCode(code)), salt, 16384, 8, 1, 32);
  }

  function newHandshake() {
    var sk = nacl.randomBytes(32);
    return { sk: sk, pk: nacl.scalarMult.base(sk), nonce: nacl.randomBytes(16) };
  }

  function isZero(u8) {
    var acc = 0;
    for (var i = 0; i < u8.length; i++) acc |= u8[i];
    return acc === 0;
  }

  // mode: "key" | "code"。共享密钥全零（低阶公钥）时返回 null。
  function derive(mode, auth, mySk, theirPk, pagePk, hostPk, pageNonce, hostNonce, session) {
    var modeByte = mode === "key" ? 1 : 2;
    var th = sha512(concat(
      enc.encode("lanmsg-web-v1"),
      new Uint8Array([modeByte]),
      pagePk, hostPk, pageNonce, hostNonce,
      enc.encode(session)
    ));
    var shared = nacl.scalarMult(mySk, theirPk);
    if (isZero(shared)) return null;
    var okm = hkdfSha512(shared, auth, th, 128);
    return {
      kPage: okm.slice(0, 32),
      kHost: okm.slice(32, 64),
      kEncP2h: okm.slice(64, 96),
      kEncH2p: okm.slice(96, 128),
      th: th
    };
  }

  function mac(key, label, th) {
    return hmacSha512(key, concat(enc.encode(label), th)).slice(0, 32);
  }

  function pageMac(keys) {
    return mac(keys.kPage, "page", keys.th);
  }

  function hostMac(keys) {
    return mac(keys.kHost, "host", keys.th);
  }

  // 常数时间比较。
  function checkHostMac(keys, got) {
    return got.length === 32 && nacl.verify(hostMac(keys), got);
  }

  // nonce(24) || secretbox。
  function sealWithNonce(key, pt, nonce) {
    return concat(nonce, nacl.secretbox(pt, nonce, key));
  }

  function seal(key, pt) {
    return sealWithNonce(key, pt, nacl.randomBytes(NONCE_LEN));
  }

  function open(key, ct) {
    if (ct.length < NONCE_LEN + TAG_LEN) return null;
    return nacl.secretbox.open(ct.subarray(NONCE_LEN), ct.subarray(0, NONCE_LEN), key);
  }

  // 分块明文：seq(u64 BE) || fidLen(u8) || fid(ASCII) || idx(u32 BE) || data。
  // seq、fid、idx 在 secretbox 认证范围内，防重放与分块换位。
  function frameChunk(seq, fid, idx, data) {
    var f = enc.encode(fid);
    var head = new Uint8Array(8 + 1 + f.length + 4);
    var dv = new DataView(head.buffer);
    dv.setUint32(0, Math.floor(seq / 4294967296));
    dv.setUint32(4, seq >>> 0);
    head[8] = f.length;
    head.set(f, 9);
    dv.setUint32(9 + f.length, idx);
    return concat(head, data);
  }

  function parseChunk(plain) {
    if (plain.length < 14) return null;
    var dv = new DataView(plain.buffer, plain.byteOffset, plain.length);
    var flen = plain[8];
    if (plain.length < 13 + flen) return null;
    return {
      seq: dv.getUint32(0) * 4294967296 + dv.getUint32(4),
      fid: new TextDecoder().decode(plain.subarray(9, 9 + flen)),
      idx: dv.getUint32(9 + flen),
      data: plain.subarray(13 + flen)
    };
  }

  var api = {
    b64uEncode: b64uEncode, b64uDecode: b64uDecode, sha512: sha512,
    hmacSha512: hmacSha512, hkdfSha512: hkdfSha512, normalizeCode: normalizeCode,
    scryptCode: scryptCode, newHandshake: newHandshake, derive: derive,
    pageMac: pageMac, hostMac: hostMac, checkHostMac: checkHostMac,
    seal: seal, sealWithNonce: sealWithNonce, open: open,
    frameChunk: frameChunk, parseChunk: parseChunk, concat: concat,
    randomBytes: nacl.randomBytes
  };
  if (isNode) module.exports = api;
  else root.LanMsgCrypto = api;
})(typeof window !== "undefined" ? window : this);
