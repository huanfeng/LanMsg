// P1 核心骨架：身份管理（Task 6 补全）。
// Identity 持设备密钥、用户密钥、设备证书，持久化到 KeyStore 与 Store。
// 注释中文，标识符英文。

use crate::error::CoreError;
use crate::keystore::KeyStore;
use crate::store::{ConvKindRow, Store};
use lanmsg_proto::ids::{ConvId, DeviceId, UserId};
use lanmsg_proto::sign::{DeviceKey, UserKey};
use lanmsg_proto::types::{DeviceCert, DeviceType};

pub const KEY_DEVICE: &str = "device.key";
pub const KEY_USER: &str = "user.key";
const SETTING_SELF_CERT: &str = "self_cert";
const SETTING_DISPLAY_NAME: &str = "display_name";
const SETTING_DEVICE_NAME: &str = "device_name";

/// 身份：设备密钥、可选用户密钥、可选设备证书。
pub struct Identity {
    pub device: DeviceKey,
    pub user: Option<UserKey>,
    pub cert: Option<DeviceCert>,
}

impl Identity {
    /// 加载或创建身份。
    ///
    /// - 设备密钥不存在则随机生成并保存到 KeyStore。
    /// - 用户密钥存在则加载，不存在则为 None（未初始化）。
    /// - 证书从 store settings["self_cert"] 读取 hex 编码的 CBOR 后反序列化。
    pub fn load_or_create(ks: &dyn KeyStore, store: &Store) -> Result<Self, CoreError> {
        let device = match ks.load(KEY_DEVICE)? {
            Some(bytes) => {
                let arr: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| CoreError::InvalidArgument("设备密钥长度非法".into()))?;
                DeviceKey::from_bytes(&arr)
            }
            None => {
                let device = DeviceKey::generate();
                ks.save(KEY_DEVICE, &device.to_bytes())?;
                device
            }
        };

        let user = match ks.load(KEY_USER)? {
            Some(bytes) => {
                let arr: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| CoreError::InvalidArgument("用户密钥长度非法".into()))?;
                Some(UserKey::from_bytes(&arr))
            }
            None => None,
        };

        let cert = match store.get_setting(SETTING_SELF_CERT)? {
            Some(hex) => {
                let bytes = hex_decode(&hex)?;
                Some(lanmsg_proto::from_cbor(&bytes).map_err(|e| {
                    CoreError::Store(crate::store::StoreError::Codec(e.to_string()))
                })?)
            }
            None => None,
        };

        Ok(Identity { device, user, cert })
    }

    /// 初始化首台设备。已有用户密钥则返回 AlreadyInitialized。
    ///
    /// 已知限制：初始化跨 KeyStore 与 Store 两次写入，非原子；中途失败会留下不可恢复的半初始化状态（P2 配对流程重审）。
    pub fn init_first_device(
        &mut self,
        ks: &dyn KeyStore,
        store: &Store,
        device_name: &str,
        device_type: DeviceType,
        now: i64,
    ) -> Result<(), CoreError> {
        if self.user.is_some() {
            return Err(CoreError::AlreadyInitialized);
        }

        let user = UserKey::generate();
        let user_id = user.user_id();
        ks.save(KEY_USER, &user.to_bytes())?;

        // 设备证书，由用户密钥签发。
        let cert = DeviceCert::issue(
            &user,
            self.device.device_id(),
            device_name.to_string(),
            device_type,
            now,
        );
        let cert_bytes = lanmsg_proto::to_cbor(&cert);
        store.set_setting(SETTING_SELF_CERT, &hex_encode(&cert_bytes))?;

        // 用户行。
        store.upsert_user(&crate::store::UserRow {
            user: user_id,
            display_name: device_name.to_string(),
            is_self: true,
            updated_at: now,
        })?;

        // 设置。
        store.set_setting(SETTING_DISPLAY_NAME, device_name)?;
        store.set_setting(SETTING_DEVICE_NAME, device_name)?;

        // 设备行。
        store.upsert_device(&crate::store::DeviceRow {
            device: self.device.device_id(),
            user: user_id,
            cert: cert.clone(),
            name: device_name.to_string(),
            device_type,
            last_addr: None,
            last_path: None,
            last_seen: None,
            revoked: false,
        })?;

        // 自身设备会话。
        store.ensure_conversation(&crate::store::ConvRow {
            conv: ConvId::self_devices(&user_id),
            kind: ConvKindRow::SelfDevices,
            peer_user: None,
            title: None,
            created_at: now,
        })?;

        // announce 与 HelloInfo：P1 阶段可留空，Task 11 实现。

        self.user = Some(user);
        self.cert = Some(cert);
        Ok(())
    }

    pub fn user_id(&self) -> Option<UserId> {
        self.user.as_ref().map(|u| u.user_id())
    }

    pub fn device_id(&self) -> DeviceId {
        self.device.device_id()
    }

    pub fn is_initialized(&self) -> bool {
        self.user.is_some()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, CoreError> {
    if !s.len().is_multiple_of(2) {
        return Err(CoreError::InvalidArgument("hex 长度必须为偶数".into()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(c: u8) -> Result<u8, CoreError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(CoreError::InvalidArgument("非法 hex 字符".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::FileKeyStore;
    use crate::store::Store;

    /// 构造临时 KeyStore；TempDir 必须由调用方持有，否则目录被提前删除。
    fn temp_ks() -> (tempfile::TempDir, FileKeyStore) {
        let dir = tempfile::tempdir().unwrap();
        let ks = FileKeyStore::new(dir.path().join("keys"));
        (dir, ks)
    }

    fn temp_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn identity_persists_device_key_and_initializes_once() {
        let (_dir, ks) = temp_ks();
        let store = temp_store();

        // 首次：不存在设备密钥，会生成并持久化。
        let identity = Identity::load_or_create(&ks, &store).unwrap();
        assert!(!identity.is_initialized());
        assert_eq!(ks.load(KEY_DEVICE).unwrap().unwrap().len(), 32);
        assert_eq!(ks.load(KEY_USER).unwrap(), None);

        // 再次 load：设备密钥应相同（已持久化）。
        let identity2 = Identity::load_or_create(&ks, &store).unwrap();
        assert_eq!(identity.device_id(), identity2.device_id());

        // init_first_device。
        let mut identity = identity;
        identity
            .init_first_device(
                &ks,
                &store,
                "我的设备",
                DeviceType::Desktop,
                1_700_000_000_000,
            )
            .unwrap();
        assert!(identity.is_initialized());
        assert!(identity.user_id().is_some());

        // 重新 load：cert 应仍在（不验签，只断言存在）。
        let identity3 = Identity::load_or_create(&ks, &store).unwrap();
        assert!(identity3.cert.is_some());

        // 重复初始化应失败。
        let err = identity.init_first_device(
            &ks,
            &store,
            "我的设备",
            DeviceType::Desktop,
            1_700_000_000_000,
        );
        assert!(matches!(err, Err(CoreError::AlreadyInitialized)));
    }
}
