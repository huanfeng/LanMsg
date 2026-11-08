//! LocalSend HTTPS 用的自签证书（与设备身份证书无关）。

use std::fs;
use std::path::Path;

use rcgen::{date_time_ymd, CertificateParams, DistinguishedName, DnType, KeyPair};
use sha2::{Digest, Sha256};

use crate::LsError;

const CERT_FILE: &str = "localsend.crt.der";
const KEY_FILE: &str = "localsend.key.der";

/// DER 编码的证书与 PKCS#8 私钥。
#[derive(Debug, Clone)]
pub struct LsCert {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
}

/// 读取目录下已有的证书，不存在则生成（ECDSA P-256，CN `LanMsg`，有效期 100 年）并落盘。
pub fn load_or_create(dir: &Path) -> Result<LsCert, LsError> {
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);
    if let (Ok(cert_der), Ok(key_der)) = (fs::read(&cert_path), fs::read(&key_path)) {
        if !cert_der.is_empty() && !key_der.is_empty() {
            return Ok(LsCert { cert_der, key_der });
        }
    }

    let key_pair =
        KeyPair::generate().map_err(|e| LsError::Tls(format!("key generation failed: {e}")))?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "LanMsg");
    params.distinguished_name = dn;
    params.not_before = date_time_ymd(2020, 1, 1);
    params.not_after = date_time_ymd(2120, 1, 1);
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| LsError::Tls(format!("certificate generation failed: {e}")))?;

    let out = LsCert {
        cert_der: cert.der().to_vec(),
        key_der: key_pair.serialize_der(),
    };
    fs::create_dir_all(dir)?;
    fs::write(&cert_path, &out.cert_der)?;
    fs::write(&key_path, &out.key_der)?;
    Ok(out)
}

/// 证书 DER 的 SHA-256，大写十六进制。
pub fn fingerprint(cert_der: &[u8]) -> String {
    Sha256::digest(cert_der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_uppercase_sha256_hex() {
        let f = fingerprint(b"abc");
        assert_eq!(f.len(), 64);
        assert!(f.chars().all(|c| matches!(c, '0'..='9' | 'A'..='F')));
        assert_eq!(
            f,
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
    }

    #[test]
    fn load_or_create_persists_and_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path()).unwrap();
        assert!(!a.cert_der.is_empty() && !a.key_der.is_empty());
        let b = load_or_create(dir.path()).unwrap();
        assert_eq!(a.cert_der, b.cert_der);
        assert_eq!(a.key_der, b.key_der);
        assert!(dir.path().join("localsend.crt.der").exists());
    }
}
