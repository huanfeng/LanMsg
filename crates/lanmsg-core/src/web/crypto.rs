//! Web 会话密码学。算法定义见 P5 计划 Task 12，JS 侧（web/crypto.js）必须逐字节一致。
//!
//! 流程：auth（url_key 或 scrypt(code)）作 HKDF 盐，X25519 共享密钥作 IKM，
//! transcript 哈希作 info，派生 k_page / k_host / k_enc_p2h / k_enc_h2p（按方向分离）；双向 HMAC 确认后用
//! XSalsa20-Poly1305（NaCl secretbox）加密载荷。

use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Key, Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha512};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::CoreError;

type HmacSha512 = Hmac<Sha512>;

/// 配对码字母表（32 个字符，每字符 5 比特）。
pub const CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
/// 配对码长度。
pub const CODE_LEN: usize = 8;
/// 连续确认失败多少次后作废配对码。
pub const MAX_CODE_FAILURES: u32 = 5;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const TRANSCRIPT_PREFIX: &[u8] = b"lanmsg-web-v1";

/// 认证模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebMode {
    /// 二维码 / 链接：口令在 URL 片段里。
    Key,
    /// 手动输入 8 位配对码。
    Code,
}

impl WebMode {
    fn byte(self) -> u8 {
        match self {
            WebMode::Key => 0x01,
            WebMode::Code => 0x02,
        }
    }
}

/// 宿主每次开启网页传输生成的秘密材料。
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct WebSecrets {
    pub url_key: [u8; 32],
    pub code: String,
    pub scrypt_salt: [u8; 16],
    code_auth: [u8; 32],
}

/// 配对码规范化：仅 ASCII 大写并去除空白。
pub fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// scrypt(code 大写, salt, N=16384, r=8, p=1, dkLen=32)。
pub fn scrypt_code(code: &str, salt: &[u8; 16]) -> [u8; 32] {
    let mut out = [0u8; 32];
    // 参数为常量且合法，不会失败。
    let params = scrypt::Params::new(14, 8, 1, 32).expect("constant scrypt params are valid");
    scrypt::scrypt(normalize_code(code).as_bytes(), salt, &params, &mut out)
        .expect("output length matches params");
    out
}

/// 生成 8 位配对码。
pub(crate) fn generate_code() -> String {
    let mut raw = [0u8; CODE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    // 256 是 32 的整数倍，取低 5 位无偏差。
    raw.iter()
        .map(|b| CODE_ALPHABET[(*b & 0x1f) as usize] as char)
        .collect()
}

impl WebSecrets {
    pub fn generate() -> WebSecrets {
        let mut rng = rand::rngs::OsRng;
        let mut url_key = [0u8; 32];
        rng.fill_bytes(&mut url_key);
        let mut scrypt_salt = [0u8; 16];
        rng.fill_bytes(&mut scrypt_salt);
        let code = generate_code();
        let code_auth = scrypt_code(&code, &scrypt_salt);
        WebSecrets {
            url_key,
            code,
            scrypt_salt,
            code_auth,
        }
    }

    /// 轮换配对码：同时重生成 code 与 scrypt_salt 并重算 code_auth。
    pub fn rotate_code(&mut self) {
        let mut salt = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        self.code.zeroize();
        self.code = generate_code();
        self.scrypt_salt = salt;
        self.code_auth = scrypt_code(&self.code, &self.scrypt_salt);
    }

    /// 指定模式下的认证密钥。
    pub fn auth_key(&self, mode: WebMode) -> [u8; 32] {
        match mode {
            WebMode::Key => self.url_key,
            WebMode::Code => self.code_auth,
        }
    }
}

/// 会话密钥与 transcript 哈希。
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionKeys {
    pub k_page: [u8; 32],
    pub k_host: [u8; 32],
    /// 页面 -> 宿主方向的加密密钥。
    pub k_enc_p2h: [u8; 32],
    /// 宿主 -> 页面方向的加密密钥。
    pub k_enc_h2p: [u8; 32],
    pub th: [u8; 64],
}

/// 宿主对页面握手请求的应答。
pub struct HostHandshake {
    pub session: String,
    pub host_pk: [u8; 32],
    pub host_nonce: [u8; 16],
    keys: SessionKeys,
}

impl HostHandshake {
    pub fn keys(&self) -> &SessionKeys {
        &self.keys
    }

    pub fn into_keys(self) -> SessionKeys {
        self.keys
    }
}

/// 宿主侧：生成临时密钥、nonce、session，并推导会话密钥。
pub fn host_respond(
    mode: WebMode,
    auth: &[u8; 32],
    page_pk: &[u8; 32],
    page_nonce: &[u8; 16],
) -> Result<HostHandshake, CoreError> {
    let mut rng = rand::rngs::OsRng;
    let host_sk = StaticSecret::random_from_rng(rng);
    let host_pk = PublicKey::from(&host_sk).to_bytes();
    let mut host_nonce = [0u8; 16];
    rng.fill_bytes(&mut host_nonce);
    let mut sid = [0u8; 16];
    rng.fill_bytes(&mut sid);
    let session = data_encoding::HEXLOWER.encode(&sid);
    let mut sk_bytes = host_sk.to_bytes();
    let keys = derive(
        mode,
        auth,
        &sk_bytes,
        page_pk,
        page_pk,
        &host_pk,
        page_nonce,
        &host_nonce,
        &session,
    );
    sk_bytes.zeroize();
    Ok(HostHandshake {
        session,
        host_pk,
        host_nonce,
        keys: keys?,
    })
}

/// 双方共用的密钥推导（`my_sk` / `their_pk` 因侧而异，其余输入两侧一致）。
#[allow(clippy::too_many_arguments)]
pub fn derive(
    mode: WebMode,
    auth: &[u8; 32],
    my_sk: &[u8; 32],
    their_pk: &[u8; 32],
    page_pk: &[u8; 32],
    host_pk: &[u8; 32],
    page_nonce: &[u8; 16],
    host_nonce: &[u8; 16],
    session: &str,
) -> Result<SessionKeys, CoreError> {
    let mut hasher = Sha512::new();
    hasher.update(TRANSCRIPT_PREFIX);
    hasher.update([mode.byte()]);
    hasher.update(page_pk);
    hasher.update(host_pk);
    hasher.update(page_nonce);
    hasher.update(host_nonce);
    hasher.update(session.as_bytes());
    let th: [u8; 64] = hasher.finalize().into();

    let shared = StaticSecret::from(*my_sk).diffie_hellman(&PublicKey::from(*their_pk));
    if !shared.was_contributory() {
        return Err(CoreError::InvalidArgument("invalid peer public key".into()));
    }
    let hk = Hkdf::<Sha512>::new(Some(auth), shared.as_bytes());
    let mut okm = [0u8; 128];
    // 96 字节远小于 HKDF-SHA512 上限，不会失败。
    hk.expand(&th, &mut okm)
        .expect("128-byte okm is within HKDF limit");
    let mut keys = SessionKeys {
        k_page: [0; 32],
        k_host: [0; 32],
        k_enc_p2h: [0; 32],
        k_enc_h2p: [0; 32],
        th,
    };
    keys.k_page.copy_from_slice(&okm[0..32]);
    keys.k_host.copy_from_slice(&okm[32..64]);
    keys.k_enc_p2h.copy_from_slice(&okm[64..96]);
    keys.k_enc_h2p.copy_from_slice(&okm[96..128]);
    okm.zeroize();
    Ok(keys)
}

fn mac_of(key: &[u8; 32], label: &[u8], th: &[u8; 64]) -> HmacSha512 {
    // HMAC 接受任意长度密钥，不会失败。
    let mut mac = <HmacSha512 as Mac>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(label);
    mac.update(th);
    mac
}

impl SessionKeys {
    /// 校验页面的确认 MAC（常数时间）。
    pub fn verify_page_mac(&self, mac: &[u8]) -> bool {
        if mac.len() != 32 {
            return false;
        }
        mac_of(&self.k_page, b"page", &self.th)
            .verify_truncated_left(mac)
            .is_ok()
    }

    /// 页面确认 MAC（测试与向量用）。
    pub fn page_mac(&self) -> [u8; 32] {
        truncate_mac(mac_of(&self.k_page, b"page", &self.th))
    }

    /// 宿主确认 MAC，发给页面校验。
    pub fn host_mac(&self) -> [u8; 32] {
        truncate_mac(mac_of(&self.k_host, b"host", &self.th))
    }

    /// 宿主 -> 页面：`nonce(24) || secretbox`，使用 h2p 密钥。
    pub fn seal_to_page(&self, pt: &[u8]) -> Vec<u8> {
        seal_with_key(&self.k_enc_h2p, pt, &random_nonce())
    }

    /// 页面 -> 宿主，使用 p2h 密钥（页面侧与测试用）。
    pub fn seal_to_host(&self, pt: &[u8]) -> Vec<u8> {
        seal_with_key(&self.k_enc_p2h, pt, &random_nonce())
    }

    #[cfg(test)]
    pub(crate) fn seal_to_page_with_nonce(&self, pt: &[u8], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
        seal_with_key(&self.k_enc_h2p, pt, nonce)
    }

    #[cfg(test)]
    pub(crate) fn seal_to_host_with_nonce(&self, pt: &[u8], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
        seal_with_key(&self.k_enc_p2h, pt, nonce)
    }

    /// 宿主解密页面发来的密文（p2h）。
    pub fn open_from_page(&self, ct: &[u8]) -> Result<Vec<u8>, CoreError> {
        open_with_key(&self.k_enc_p2h, ct)
    }

    /// 页面解密宿主发来的密文（h2p，页面侧与测试用）。
    pub fn open_from_host(&self, ct: &[u8]) -> Result<Vec<u8>, CoreError> {
        open_with_key(&self.k_enc_h2p, ct)
    }
}

fn random_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

fn seal_with_key(key: &[u8; 32], pt: &[u8], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let cipher = XSalsa20Poly1305::new(Key::from_slice(key));
    // 加密仅在明文长度溢出时失败，不会发生。
    let ct = cipher
        .encrypt(Nonce::from_slice(nonce), pt)
        .expect("secretbox encryption cannot fail");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

fn open_with_key(key: &[u8; 32], ct: &[u8]) -> Result<Vec<u8>, CoreError> {
    if ct.len() < NONCE_LEN + TAG_LEN {
        return Err(CoreError::InvalidArgument("ciphertext too short".into()));
    }
    let (nonce, body) = ct.split_at(NONCE_LEN);
    XSalsa20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), body)
        .map_err(|_| CoreError::InvalidArgument("ciphertext authentication failed".into()))
}

fn truncate_mac(mac: HmacSha512) -> [u8; 32] {
    let full = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&full[..32]);
    out
}

/// 配对码连续失败计数。
#[derive(Debug, Default)]
pub struct CodeGuard {
    failures: u32,
}

impl CodeGuard {
    /// 确认成功后清零计数。
    pub fn record_success(&mut self) {
        self.failures = 0;
    }

    /// 记录一次失败；返回 `true` 表示已达上限，调用方必须轮换 code 与 scrypt_salt。
    /// 达到上限时计数清零。
    pub fn record_failure(&mut self) -> bool {
        self.failures += 1;
        if self.failures >= MAX_CODE_FAILURES {
            self.failures = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn b64u(b: &[u8]) -> String {
        data_encoding::BASE64URL_NOPAD.encode(b)
    }

    fn pk_of(sk: &[u8; 32]) -> [u8; 32] {
        PublicKey::from(&StaticSecret::from(*sk)).to_bytes()
    }

    /// 页面侧推导，返回 (页面密钥, 宿主应答)。
    fn handshake(
        mode: WebMode,
        page_auth: &[u8; 32],
        host_auth: &[u8; 32],
    ) -> (SessionKeys, HostHandshake) {
        let page_sk = [5u8; 32];
        let page_pk = pk_of(&page_sk);
        let page_nonce = [6u8; 16];
        let host = host_respond(mode, host_auth, &page_pk, &page_nonce).unwrap();
        let page = derive(
            mode,
            page_auth,
            &page_sk,
            &host.host_pk,
            &page_pk,
            &host.host_pk,
            &page_nonce,
            &host.host_nonce,
            &host.session,
        )
        .unwrap();
        (page, host)
    }

    #[test]
    fn page_and_host_derive_same_keys() {
        let s = WebSecrets::generate();
        for mode in [WebMode::Key, WebMode::Code] {
            let auth = s.auth_key(mode);
            let (page, host) = handshake(mode, &auth, &auth);
            let h = host.keys();
            assert_eq!(page.k_page, h.k_page);
            assert_eq!(page.k_host, h.k_host);
            assert_eq!(page.k_enc_p2h, h.k_enc_p2h);
            assert_eq!(page.k_enc_h2p, h.k_enc_h2p);
            assert_ne!(h.k_enc_p2h, h.k_enc_h2p);
            assert_eq!(page.th, h.th);
            assert!(h.verify_page_mac(&page.page_mac()));
            assert_eq!(page.host_mac(), h.host_mac());
        }
    }

    #[test]
    fn wrong_code_fails_page_mac() {
        let s = WebSecrets::generate();
        let wrong = scrypt_code("22222222", &s.scrypt_salt);
        let (page, host) = handshake(WebMode::Code, &wrong, &s.auth_key(WebMode::Code));
        assert!(!host.keys().verify_page_mac(&page.page_mac()));
        assert!(!host.keys().verify_page_mac(&[0u8; 5]));
    }

    #[test]
    fn seal_open_roundtrip_and_tamper_detected() {
        let s = WebSecrets::generate();
        let auth = s.auth_key(WebMode::Key);
        let (page, host) = handshake(WebMode::Key, &auth, &auth);
        let sealed = page.seal_to_host(b"hello");
        assert_eq!(host.keys().open_from_page(&sealed).unwrap(), b"hello");
        let mut bad = sealed.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(host.keys().open_from_page(&bad).is_err());
        assert!(host.keys().open_from_page(&sealed[..30]).is_err());
        let back = host.keys().seal_to_page(b"world");
        assert_eq!(page.open_from_host(&back).unwrap(), b"world");
    }

    #[test]
    fn directions_are_not_interchangeable() {
        let s = WebSecrets::generate();
        let auth = s.auth_key(WebMode::Key);
        let (page, host) = handshake(WebMode::Key, &auth, &auth);
        // 宿主发出的密文（h2p）不能被宿主当作页面发来的密文（p2h）接受，即反射攻击无效。
        let reflected = host.keys().seal_to_page(b"x");
        assert!(host.keys().open_from_page(&reflected).is_err());
        let reflected = page.seal_to_host(b"x");
        assert!(page.open_from_host(&reflected).is_err());
    }

    #[test]
    fn zero_public_key_rejected() {
        let s = WebSecrets::generate();
        let auth = s.auth_key(WebMode::Key);
        assert!(host_respond(WebMode::Key, &auth, &[0u8; 32], &[1u8; 16]).is_err());
    }

    #[test]
    fn code_alphabet_and_length() {
        for _ in 0..100 {
            let c = generate_code();
            assert_eq!(c.len(), CODE_LEN);
            assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        }
    }

    #[test]
    fn normalize_code_uppercases_and_strips_whitespace() {
        assert_eq!(normalize_code(" ab cd\t23 "), "ABCD23");
    }

    #[test]
    fn rotate_code_changes_code_salt_and_auth() {
        let mut s = WebSecrets::generate();
        let (code, salt, auth) = (s.code.clone(), s.scrypt_salt, s.auth_key(WebMode::Code));
        s.rotate_code();
        assert_ne!(s.code, code);
        assert_ne!(s.scrypt_salt, salt);
        assert_ne!(s.auth_key(WebMode::Code), auth);
        assert_eq!(
            s.auth_key(WebMode::Code),
            scrypt_code(&s.code, &s.scrypt_salt)
        );
    }

    #[test]
    fn code_mode_rotates_after_five_failures() {
        let mut g = CodeGuard::default();
        for _ in 0..4 {
            assert!(!g.record_failure());
        }
        assert!(g.record_failure());
        assert!(!g.record_failure());
    }

    #[test]
    fn success_resets_failure_count() {
        let mut g = CodeGuard::default();
        for _ in 0..4 {
            assert!(!g.record_failure());
        }
        g.record_success();
        for _ in 0..4 {
            assert!(!g.record_failure());
        }
        assert!(g.record_failure());
    }

    /// 固定输入生成的向量，供 web/test/crypto.test.mjs 比对。
    /// 向量变化时：删除 web/test/vectors.json 后重跑
    /// `cargo test -p lanmsg-core --lib web::crypto::tests::write_test_vectors`。
    #[test]
    fn write_test_vectors() {
        let page_sk = [0x11u8; 32];
        let host_sk = [0x22u8; 32];
        let page_nonce = [0x33u8; 16];
        let host_nonce = [0x44u8; 16];
        let session = "000102030405060708090a0b0c0d0e0f";
        let url_key = [0x55u8; 32];
        let code = "ABCD2345";
        let salt = [0x66u8; 16];
        let seal_nonce = [0x77u8; 24];
        let plaintext = "你好 LanMsg";
        let page_pk = pk_of(&page_sk);
        let host_pk = pk_of(&host_sk);

        let mut modes = serde_json::Map::new();
        for (name, mode, auth) in [
            ("key", WebMode::Key, url_key),
            ("code", WebMode::Code, scrypt_code(code, &salt)),
        ] {
            let page = derive(
                mode,
                &auth,
                &page_sk,
                &host_pk,
                &page_pk,
                &host_pk,
                &page_nonce,
                &host_nonce,
                session,
            )
            .unwrap();
            let host = derive(
                mode,
                &auth,
                &host_sk,
                &page_pk,
                &page_pk,
                &host_pk,
                &page_nonce,
                &host_nonce,
                session,
            )
            .unwrap();
            assert_eq!(page.k_enc_p2h, host.k_enc_p2h);
            assert_eq!(page.k_enc_h2p, host.k_enc_h2p);
            modes.insert(
                name.into(),
                json!({
                    "auth": b64u(&auth),
                    "th": b64u(&page.th),
                    "k_page": b64u(&page.k_page),
                    "k_host": b64u(&page.k_host),
                    "k_enc_p2h": b64u(&page.k_enc_p2h),
                    "k_enc_h2p": b64u(&page.k_enc_h2p),
                    "page_mac": b64u(&page.page_mac()),
                    "host_mac": b64u(&page.host_mac()),
                    "sealed_p2h": b64u(&page.seal_to_host_with_nonce(plaintext.as_bytes(), &seal_nonce)),
                    "sealed_h2p": b64u(&host.seal_to_page_with_nonce(plaintext.as_bytes(), &seal_nonce)),
                }),
            );
        }
        let doc = json!({
            "inputs": {
                "page_sk": b64u(&page_sk),
                "host_sk": b64u(&host_sk),
                "page_pk": b64u(&page_pk),
                "host_pk": b64u(&host_pk),
                "page_nonce": b64u(&page_nonce),
                "host_nonce": b64u(&host_nonce),
                "session": session,
                "url_key": b64u(&url_key),
                "code": code,
                "scrypt_salt": b64u(&salt),
                "seal_nonce": b64u(&seal_nonce),
                "plaintext": plaintext,
            },
            "modes": modes,
        });
        let text = serde_json::to_string_pretty(&doc).unwrap() + "\n";
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/test/vectors.json");
        match std::fs::read_to_string(&path) {
            Ok(old) => assert_eq!(
                old, text,
                "vectors changed; delete web/test/vectors.json and rerun to regenerate"
            ),
            Err(_) => {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, text).unwrap();
            }
        }
    }
}
