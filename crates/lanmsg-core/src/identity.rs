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

    /// 加入方采纳配对包中的用户身份（P2 Task 7）。
    ///
    /// 写 KeyStore user.key、Store self_cert，并更新内存中的 user/cert。
    /// 已初始化 → AlreadyInitialized。注意：只写身份本身，users/devices/
    /// 会话/联系人等数据行由调用方（Pairing）负责。
    pub fn adopt_user(
        &mut self,
        ks: &dyn KeyStore,
        store: &Store,
        user_secret: [u8; 32],
        cert: DeviceCert,
    ) -> Result<(), CoreError> {
        if self.user.is_some() {
            return Err(CoreError::AlreadyInitialized);
        }
        ks.save(KEY_USER, &user_secret)?;
        store.set_setting(
            SETTING_SELF_CERT,
            &hex_encode(&lanmsg_proto::to_cbor(&cert)),
        )?;
        self.user = Some(UserKey::from_bytes(&user_secret));
        self.cert = Some(cert);
        Ok(())
    }

    pub fn user_id(&self) -> Option<UserId> {
        self.user.as_ref().map(|u| u.user_id())
    }

    /// 本机被吊销（P2 Task 8）：清空内存中的用户密钥与证书（设备密钥保留），
    /// 此后按未初始化语义处理（is_initialized == false、请求返回 NotInitialized）。
    /// 持久化的 user.key 与 self_cert 不删：重启后由对端的吊销检查拒绝其接入
    /// （身份的彻底清除属后续阶段语义）。
    pub fn mark_revoked_self(&mut self) {
        self.user = None;
        self.cert = None;
    }

    /// 设备名变更时用用户密钥重签本设备证书。
    ///
    /// 写 setting "self_cert"，更新 users/devices 表中自己的行
    /// （users.display_name 同步为新设备名，与 init_first_device 的初值一致），
    /// 返回新证书。未初始化 → `NotInitialized`。
    pub fn reissue_cert(
        &mut self,
        store: &Store,
        device_name: &str,
        now: i64,
    ) -> Result<DeviceCert, CoreError> {
        let user = self.user.as_ref().ok_or(CoreError::NotInitialized)?;
        let device = self.device.device_id();
        let device_type = match &self.cert {
            Some(c) => c.device_type,
            None => return Err(CoreError::NotInitialized),
        };

        let cert = DeviceCert::issue(user, device, device_name.to_string(), device_type, now);
        let cert_bytes = lanmsg_proto::to_cbor(&cert);
        store.set_setting(SETTING_SELF_CERT, &hex_encode(&cert_bytes))?;

        // users 表：display_name 用新设备名，is_self 保持（自己行恒为 true）。
        let is_self = store
            .get_user(&user.user_id())
            .ok()
            .flatten()
            .map(|r| r.is_self)
            .unwrap_or(true);
        store.upsert_user(&crate::store::UserRow {
            user: user.user_id(),
            display_name: device_name.to_string(),
            is_self,
            updated_at: now,
        })?;

        // devices 表：证书与设备名换新。
        store.upsert_device(&crate::store::DeviceRow {
            device,
            user: user.user_id(),
            cert: cert.clone(),
            name: device_name.to_string(),
            device_type,
            last_addr: None,
            last_path: None,
            last_seen: None,
            revoked: false,
        })?;

        self.cert = Some(cert.clone());
        Ok(cert)
    }

    pub fn device_id(&self) -> DeviceId {
        self.device.device_id()
    }

    pub fn is_initialized(&self) -> bool {
        self.user.is_some()
    }
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub(crate) fn hex_decode(s: &str) -> Result<Vec<u8>, CoreError> {
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

    /// KeyStore 出错（如系统密钥库未解锁）时必须向上返回错误，不得生成新密钥。
    #[test]
    fn identity_does_not_regenerate_on_keystore_error() {
        use crate::error::KeyStoreError;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct ErrKs {
            saves: AtomicUsize,
        }
        impl KeyStore for ErrKs {
            fn load(&self, _: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
                Err(KeyStoreError::Io(std::io::Error::other("模拟：未解锁")))
            }
            fn save(&self, _: &str, _: &[u8]) -> Result<(), KeyStoreError> {
                self.saves.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            fn delete(&self, _: &str) -> Result<(), KeyStoreError> {
                Ok(())
            }
        }
        let ks = ErrKs {
            saves: AtomicUsize::new(0),
        };
        let res = Identity::load_or_create(&ks, &temp_store());
        assert!(matches!(res, Err(CoreError::KeyStore(_))));
        assert_eq!(ks.saves.load(Ordering::Relaxed), 0);
    }
}
