// 校验 web/crypto.js 与 Rust 生成的 vectors.json 逐项一致。运行：node --test web/test/
import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import nodeCrypto from "node:crypto";

const require = createRequire(import.meta.url);
const C = require("../crypto.js");
const V = JSON.parse(readFileSync(new URL("./vectors.json", import.meta.url), "utf8"));
const I = V.inputs;
const d = (s) => C.b64uDecode(s);
const e = (u) => C.b64uEncode(u);
const text = new TextEncoder();

async function authFor(mode) {
  return mode === "key" ? d(I.url_key) : await C.scryptCode(I.code, d(I.scrypt_salt));
}

function deriveFor(mode, auth, side) {
  const mySk = side === "page" ? d(I.page_sk) : d(I.host_sk);
  const theirPk = side === "page" ? d(I.host_pk) : d(I.page_pk);
  return C.derive(mode, auth, mySk, theirPk, d(I.page_pk), d(I.host_pk),
    d(I.page_nonce), d(I.host_nonce), I.session);
}

for (const mode of ["key", "code"]) {
  test(`vectors match (${mode})`, async () => {
    const m = V.modes[mode];
    const auth = await authFor(mode);
    assert.equal(e(auth), m.auth);
    for (const side of ["page", "host"]) {
      const k = deriveFor(mode, auth, side);
      assert.equal(e(k.th), m.th);
      assert.equal(e(k.kPage), m.k_page);
      assert.equal(e(k.kHost), m.k_host);
      assert.equal(e(k.kEncP2h), m.k_enc_p2h);
      assert.equal(e(k.kEncH2p), m.k_enc_h2p);
      assert.equal(e(C.pageMac(k)), m.page_mac);
      assert.equal(e(C.hostMac(k)), m.host_mac);
      assert.ok(C.checkHostMac(k, d(m.host_mac)));
    }
    const k = deriveFor(mode, auth, "page");
    const nonce = d(I.seal_nonce);
    const pt = text.encode(I.plaintext);
    assert.equal(e(C.sealWithNonce(k.kEncP2h, pt, nonce)), m.sealed_p2h);
    assert.equal(e(C.sealWithNonce(k.kEncH2p, pt, nonce)), m.sealed_h2p);
    assert.deepEqual(C.open(k.kEncP2h, d(m.sealed_p2h)), pt);
    assert.deepEqual(C.open(k.kEncH2p, d(m.sealed_h2p)), pt);
  });
}

test("public keys match private keys", () => {
  assert.equal(e(require("../vendor/nacl-fast.min.js").scalarMult.base(d(I.page_sk))), I.page_pk);
  assert.equal(e(require("../vendor/nacl-fast.min.js").scalarMult.base(d(I.host_sk))), I.host_pk);
});

test("wrong host mac rejected", async () => {
  const k = deriveFor("key", await authFor("key"), "page");
  const bad = d(V.modes.key.host_mac);
  bad[0] ^= 1;
  assert.equal(C.checkHostMac(k, bad), false);
  assert.equal(C.checkHostMac(k, bad.subarray(0, 5)), false);
});

test("wrong auth gives different keys and mac", async () => {
  const good = deriveFor("key", await authFor("key"), "page");
  const bad = deriveFor("key", new Uint8Array(32), "page");
  assert.notEqual(e(C.pageMac(bad)), e(C.pageMac(good)));
});

test("tampered or short ciphertext fails to open", async () => {
  const k = deriveFor("key", await authFor("key"), "page");
  const sealed = d(V.modes.key.sealed_p2h);
  sealed[sealed.length - 1] ^= 1;
  assert.equal(C.open(k.kEncP2h, sealed), null);
  assert.equal(C.open(k.kEncP2h, sealed.subarray(0, 30)), null);
});

test("directions are not interchangeable", async () => {
  const k = deriveFor("key", await authFor("key"), "page");
  assert.equal(C.open(k.kEncH2p, d(V.modes.key.sealed_p2h)), null);
});

test("zero public key rejected", async () => {
  const auth = await authFor("key");
  const k = C.derive("key", auth, d(I.page_sk), new Uint8Array(32), d(I.page_pk),
    d(I.host_pk), d(I.page_nonce), d(I.host_nonce), I.session);
  assert.equal(k, null);
});

test("code normalisation: lowercase and spaces give the same auth", async () => {
  const a = await C.scryptCode(" abcd 2345 ", d(I.scrypt_salt));
  assert.equal(e(a), V.modes.code.auth);
  assert.equal(C.normalizeCode("ab cd\t23"), "ABCD23");
});

test("hkdf and hmac agree with node:crypto", () => {
  const ikm = Uint8Array.from({ length: 22 }, (_, i) => i);
  const salt = Uint8Array.from({ length: 13 }, (_, i) => 100 + i);
  const info = Uint8Array.from([1, 2, 3]);
  const want = new Uint8Array(nodeCrypto.hkdfSync("sha512", ikm, salt, info, 200));
  assert.deepEqual(C.hkdfSha512(ikm, salt, info, 200), want);
  const longKey = new Uint8Array(200).fill(7);
  const mac = nodeCrypto.createHmac("sha512", longKey).update(info).digest();
  assert.deepEqual(Buffer.from(C.hmacSha512(longKey, info)), mac);
});

test("chunk frame roundtrip binds seq, fid and index", () => {
  const f = C.frameChunk(5000000000, "ab12", 7, new Uint8Array([1, 2, 3]));
  const p = C.parseChunk(f);
  assert.equal(p.seq, 5000000000);
  assert.equal(p.fid, "ab12");
  assert.equal(p.idx, 7);
  assert.deepEqual([...p.data], [1, 2, 3]);
  assert.equal(C.parseChunk(new Uint8Array(5)), null);
});

test("fresh handshake derives matching keys on both sides", async () => {
  const page = C.newHandshake(), host = C.newHandshake();
  const auth = new Uint8Array(32).fill(9);
  const a = C.derive("key", auth, page.sk, host.pk, page.pk, host.pk, page.nonce, host.nonce, "s");
  const b = C.derive("key", auth, host.sk, page.pk, page.pk, host.pk, page.nonce, host.nonce, "s");
  assert.equal(e(a.kEncH2p), e(b.kEncH2p));
  const sealed = C.seal(b.kEncH2p, text.encode("x"));
  assert.deepEqual([...C.open(a.kEncH2p, sealed)], [120]);
});

test("seq window rejects replays, stale and invalid values", () => {
  const w = C.newSeqWindow(4);
  assert.equal(w.accept(1), true);
  assert.equal(w.accept(1), false);
  assert.equal(w.accept(3), true);
  assert.equal(w.accept(2), true);
  assert.equal(w.accept(10), true);
  assert.equal(w.accept(7), true);
  assert.equal(w.accept(6), false);
  assert.equal(w.accept(5), false);
  assert.equal(w.accept(0), false);
  assert.equal(w.accept(-1), false);
  assert.equal(w.accept(1.5), false);
  assert.equal(w.accept(undefined), false);
});

test("seq window keeps rejecting old values after pruning", () => {
  const w = C.newSeqWindow(4);
  for (let i = 1; i <= 100; i++) assert.equal(w.accept(i), true);
  for (let i = 1; i <= 100; i++) assert.equal(w.accept(i), false);
});
