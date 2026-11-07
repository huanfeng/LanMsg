// P1 核心骨架：TLS 配置（Task 8）。
// 注释中文，标识符英文。

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, IsCa, KeyPair, PKCS_ED25519};
use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified as RustlsServerCertVerified,
    ServerCertVerifier as ClientServerCertVerifier,
};
use rustls::crypto::{verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{
    ClientCertVerified as ServerClientCertVerified, ClientCertVerifier as ServerClientCertVerifier,
};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use time::OffsetDateTime;
use x509_parser::parse_x509_certificate;

use crate::error::CoreError;
use lanmsg_proto::ids::DeviceId;
use lanmsg_proto::sign::DeviceKey;

/// SNI 固定值。
pub const TLS_SNI: &str = "lanmsg";

/// 生成设备自签证书：CN = 设备 hex 前 16 位，有效期 100 年。
fn build_self_signed_cert(device: &DeviceKey) -> Result<(Vec<u8>, Vec<u8>), CoreError> {
    let device_hex = device.device_id().to_hex();
    let cn = &device_hex[..16];

    let pkcs8 = device.to_pkcs8_der();
    let key_pair = KeyPair::from_pkcs8_der_and_sign_algo(
        &rustls::pki_types::PrivatePkcs8KeyDer::from(pkcs8.clone()),
        &PKCS_ED25519,
    )
    .map_err(|e| CoreError::Tls(format!("从设备密钥构建 KeyPair 失败: {e}")))?;

    let mut params = CertificateParams::default();
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(rcgen::DnType::CommonName, cn.to_string());
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    let hundred_years = time::Duration::days(365 * 100);
    params.not_after = now + hundred_years;

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| CoreError::Tls(format!("生成证书失败: {e}")))?;

    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();

    Ok((cert_der, key_der))
}

/// 接受任意客户端证书，但用 verify_tls13_signature 校验握手签名。
#[derive(Debug)]
struct AnyClientCertVerifier;

impl ServerClientCertVerifier for AnyClientCertVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ServerClientCertVerified, TlsError> {
        if end_entity.is_empty() {
            return Err(TlsError::General("客户端未提供证书".into()));
        }
        Ok(ServerClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 不允许".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        let provider = CryptoProvider::get_default()
            .ok_or_else(|| TlsError::General("无签名验证提供程序".into()))?;
        verify_tls13_signature(
            message,
            cert,
            dss,
            &provider.signature_verification_algorithms,
        )
        .map_err(|e| TlsError::General(format!("TLS 1.3 签名验证失败: {e}")))
    }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// 接受任意服务器证书（设备自签、身份即公钥），但用 verify_tls13_signature 校验握手签名。
/// `expect` 有值时，服务端证书里的设备公钥必须等于它，否则握手失败。
///
/// TLS 1.3 里客户端在校验完服务端的证书与签名之后才发送自己的证书，所以这里拒绝时
/// 客户端证书（即本机设备公钥）一个字节都没有发出去。
#[derive(Debug)]
struct AnyServerCertVerifier {
    expect: Option<DeviceId>,
}

impl ClientServerCertVerifier for AnyServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<RustlsServerCertVerified, TlsError> {
        if let Some(expect) = self.expect {
            let actual = device_id_from_cert(end_entity)
                .map_err(|e| TlsError::General(format!("cannot read server device id: {e}")))?;
            if actual != expect {
                return Err(TlsError::General(
                    "server device does not match the expected device".into(),
                ));
            }
        }
        Ok(RustlsServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 不允许".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        let provider = CryptoProvider::get_default()
            .ok_or_else(|| TlsError::General("无签名验证提供程序".into()))?;
        verify_tls13_signature(
            message,
            cert,
            dss,
            &provider.signature_verification_algorithms,
        )
        .map_err(|e| TlsError::General(format!("TLS 1.3 签名验证失败: {e}")))
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// 为设备生成服务端与客户端 TLS 配置。
pub fn make_tls_configs(
    device: &DeviceKey,
) -> Result<(Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>), CoreError> {
    make_tls_configs_expecting(device, None)
}

/// 同 [`make_tls_configs`]，但客户端配置要求服务端设备为 `expect`（在客户端证书发出前校验）。
pub fn make_tls_configs_expecting(
    device: &DeviceKey,
    expect: Option<DeviceId>,
) -> Result<(Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>), CoreError> {
    let (cert_der, key_der) = build_self_signed_cert(device)?;

    let server_config =
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_client_cert_verifier(Arc::new(AnyClientCertVerifier))
            .with_single_cert(
                vec![CertificateDer::from(cert_der.clone())],
                PrivateKeyDer::try_from(key_der.clone())
                    .map_err(|e| CoreError::Tls(format!("私钥格式错误: {e}")))?,
            )
            .map_err(|e| CoreError::Tls(format!("构建服务端配置失败: {e}")))?;

    let client_config =
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyServerCertVerifier { expect }))
            .with_client_auth_cert(
                vec![CertificateDer::from(cert_der)],
                PrivateKeyDer::try_from(key_der)
                    .map_err(|e| CoreError::Tls(format!("私钥格式错误: {e}")))?,
            )
            .map_err(|e| CoreError::Tls(format!("构建客户端配置失败: {e}")))?;

    Ok((Arc::new(server_config), Arc::new(client_config)))
}

/// 从 TLS 连接的对端证书中提取设备 ID。
pub fn peer_device_id(conn: &rustls::CommonState) -> Result<DeviceId, CoreError> {
    let certs = conn
        .peer_certificates()
        .ok_or_else(|| CoreError::Tls("对端未提供证书".into()))?;

    device_id_from_cert(&certs[0])
}

/// 从设备自签证书（DER）中取 Ed25519 公钥，即设备 ID。
fn device_id_from_cert(end_entity: &[u8]) -> Result<DeviceId, CoreError> {
    let (_remain, x509_cert) = parse_x509_certificate(end_entity)
        .map_err(|e| CoreError::Tls(format!("解析对端证书失败: {e}")))?;

    let pki = &x509_cert.tbs_certificate.subject_pki;
    let alg_oid = &pki.algorithm.algorithm;

    // 检查算法 OID 是否为 Ed25519 (1.3.101.112)
    if alg_oid.to_string() != "1.3.101.112" {
        return Err(CoreError::Tls(format!(
            "对端证书非 Ed25519 算法: {alg_oid}"
        )));
    }

    let spki_data = &pki.subject_public_key.data;
    if spki_data.len() != 32 {
        return Err(CoreError::Tls("Ed25519 公钥长度非 32 字节".into()));
    }

    let mut pk = [0u8; 32];
    pk.copy_from_slice(spki_data);
    Ok(DeviceId(pk))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_devices() -> (DeviceKey, DeviceKey) {
        let server_device = DeviceKey::generate();
        let client_device = DeviceKey::generate();
        (server_device, client_device)
    }

    #[tokio::test]
    async fn mutual_tls_exposes_both_device_ids() {
        let (server_device, client_device) = setup_devices().await;

        let (server_config, _) = make_tls_configs(&server_device).unwrap();
        let (_, client_config) = make_tls_configs(&client_device).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = tokio_rustls::TlsAcceptor::from(server_config);
            let conn = acceptor.accept(stream).await.unwrap();
            let (_, server_conn) = conn.get_ref();
            let id = peer_device_id(server_conn).unwrap();
            assert_eq!(id, client_device.device_id());
        });

        let connector = tokio_rustls::TlsConnector::from(client_config);
        let domain = ServerName::try_from(TLS_SNI.as_bytes()).unwrap();
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let conn = connector.connect(domain, stream).await.unwrap();
        let (_, client_conn) = conn.get_ref();
        let id = peer_device_id(client_conn).unwrap();
        assert_eq!(id, server_device.device_id());

        server_handle.await.unwrap();
    }

    /// 记录是否收到过客户端证书的服务端校验器。
    #[derive(Debug)]
    struct RecordingClientVerifier {
        got_client_cert: Arc<std::sync::atomic::AtomicBool>,
    }

    impl ServerClientCertVerifier for RecordingClientVerifier {
        fn offer_client_auth(&self) -> bool {
            true
        }
        fn client_auth_mandatory(&self) -> bool {
            true
        }
        fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
            &[]
        }
        fn verify_client_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            intermediates: &[CertificateDer<'_>],
            now: UnixTime,
        ) -> Result<ServerClientCertVerified, TlsError> {
            self.got_client_cert
                .store(true, std::sync::atomic::Ordering::SeqCst);
            AnyClientCertVerifier.verify_client_cert(end_entity, intermediates, now)
        }
        fn verify_tls12_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            AnyClientCertVerifier.verify_tls12_signature(m, c, d)
        }
        fn verify_tls13_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            AnyClientCertVerifier.verify_tls13_signature(m, c, d)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ED25519]
        }
    }

    /// 用记录校验器起一个服务端，客户端以 `expect` 拨号；返回 (客户端握手是否成功, 服务端是否收到过客户端证书)。
    async fn handshake_with_expectation(
        server_device: &DeviceKey,
        client_device: &DeviceKey,
        expect: Option<DeviceId>,
    ) -> (bool, bool) {
        let (cert_der, key_der) = build_self_signed_cert(server_device).unwrap();
        let got = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_config =
            rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_client_cert_verifier(Arc::new(RecordingClientVerifier {
                    got_client_cert: got.clone(),
                }))
                .with_single_cert(
                    vec![CertificateDer::from(cert_der)],
                    PrivateKeyDer::try_from(key_der).unwrap(),
                )
                .unwrap();
        let (_, client_config) = make_tls_configs_expecting(client_device, expect).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
                .accept(stream)
                .await;
        });
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let ok = tokio_rustls::TlsConnector::from(client_config)
            .connect(ServerName::try_from(TLS_SNI.as_bytes()).unwrap(), stream)
            .await
            .is_ok();
        server.await.unwrap();
        (ok, got.load(std::sync::atomic::Ordering::SeqCst))
    }

    #[tokio::test]
    async fn wrong_expected_server_fails_before_the_client_certificate_is_sent() {
        let (server, client) = (DeviceKey::generate(), DeviceKey::generate());
        let other = DeviceKey::generate().device_id();
        let (ok, server_saw_client_cert) =
            handshake_with_expectation(&server, &client, Some(other)).await;
        assert!(!ok, "期望设备不符时握手必须失败");
        assert!(!server_saw_client_cert, "服务端不应收到客户端证书");
    }

    #[tokio::test]
    async fn expected_server_completes_the_handshake_and_sends_the_certificate() {
        let (server, client) = (DeviceKey::generate(), DeviceKey::generate());
        let (ok, server_saw_client_cert) =
            handshake_with_expectation(&server, &client, Some(server.device_id())).await;
        assert!(ok);
        assert!(server_saw_client_cert);
    }
}
