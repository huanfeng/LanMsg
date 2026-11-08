//! LocalSend HTTPS 用的自签证书（与设备身份证书无关）。

use std::fs;
use std::io::Write;
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

/// 读取目录下已有的证书，不存在或损坏则生成（ECDSA P-256，CN `LanMsg`，有效期 100 年）并落盘。
pub fn load_or_create(dir: &Path) -> Result<LsCert, LsError> {
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);
    if let (Ok(cert_der), Ok(key_der)) = (fs::read(&cert_path), fs::read(&key_path)) {
        if is_valid(&cert_der, &key_der) {
            return Ok(LsCert { cert_der, key_der });
        }
        tracing::warn!("localsend certificate files are invalid, regenerating");
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
    // 先写临时文件，两个都写成功后再改名，避免只落盘一半
    let cert_tmp = dir.join(format!("{CERT_FILE}.tmp"));
    let key_tmp = dir.join(format!("{KEY_FILE}.tmp"));
    write_file(&key_tmp, &out.key_der, true)?;
    write_file(&cert_tmp, &out.cert_der, false)?;
    fs::rename(&key_tmp, &key_path)?;
    fs::rename(&cert_tmp, &cert_path)?;
    Ok(out)
}

/// 私钥必须能解析；证书只做 DER 外层 SEQUENCE 长度校验（不引入 X.509 解析依赖）。
fn is_valid(cert_der: &[u8], key_der: &[u8]) -> bool {
    KeyPair::try_from(key_der).is_ok() && der_sequence_is_complete(cert_der)
}

fn der_sequence_is_complete(der: &[u8]) -> bool {
    if der.len() < 2 || der[0] != 0x30 {
        return false;
    }
    let (len, header) = match der[1] {
        n if n < 0x80 => (n as usize, 2),
        n @ 0x81..=0x84 => {
            let cnt = (n & 0x7f) as usize;
            if der.len() < 2 + cnt {
                return false;
            }
            let len = der[2..2 + cnt]
                .iter()
                .fold(0usize, |a, b| (a << 8) | *b as usize);
            (len, 2 + cnt)
        }
        _ => return false,
    };
    header + len == der.len()
}

fn write_file(path: &Path, data: &[u8], private: bool) -> Result<(), LsError> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    opts.open(path)?.write_all(data)?;
    Ok(())
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

    #[test]
    fn corrupted_files_trigger_regeneration() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path()).unwrap();
        fs::write(dir.path().join("localsend.key.der"), b"garbage").unwrap();
        let b = load_or_create(dir.path()).unwrap();
        assert_ne!(a.cert_der, b.cert_der);
        let c = load_or_create(dir.path()).unwrap();
        assert_eq!(b.key_der, c.key_der);

        let mut truncated = c.cert_der.clone();
        truncated.truncate(truncated.len() - 5);
        fs::write(dir.path().join("localsend.crt.der"), truncated).unwrap();
        let d = load_or_create(dir.path()).unwrap();
        assert_ne!(c.cert_der, d.cert_der);
    }

    #[cfg(unix)]
    #[test]
    fn private_key_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path()).unwrap();
        let m = fs::metadata(dir.path().join("localsend.key.der")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
    }
}
