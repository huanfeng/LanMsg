// FFI 层集成测试：经 LanMsgCore 驱动真实 Service（外部钥匙库 / 平台均为内存假实现）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lanmsg_api::{ErrorCode, Event, InitMode, Request, Response};
use lanmsg_ffi::{EventListener, FfiError, FfiKeyStore, FfiPlatform, LanMsgCore, ThumbnailResult};

#[derive(Default)]
struct MemKs(Mutex<HashMap<String, Vec<u8>>>);

impl FfiKeyStore for MemKs {
    fn load(&self, name: String) -> Result<Option<Vec<u8>>, FfiError> {
        Ok(self.0.lock().unwrap().get(&name).cloned())
    }
    fn save(&self, name: String, bytes: Vec<u8>) -> Result<(), FfiError> {
        self.0.lock().unwrap().insert(name, bytes);
        Ok(())
    }
    fn delete(&self, name: String) -> Result<(), FfiError> {
        self.0.lock().unwrap().remove(&name);
        Ok(())
    }
}

struct FakePlatform;

impl FfiPlatform for FakePlatform {
    fn is_metered(&self) -> bool {
        false
    }
    fn is_wifi(&self) -> bool {
        true
    }
    fn thumbnail(&self, _path: String, _mime: String, _max_edge: u32) -> Option<ThumbnailResult> {
        None
    }
}

#[derive(Default)]
struct Collect(Mutex<Vec<Event>>, AtomicUsize);

impl EventListener for Collect {
    fn on_event(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
    fn on_lagged(&self) {
        self.1.fetch_add(1, Ordering::SeqCst);
    }
}

fn new_core(dir: &std::path::Path, ks: Arc<MemKs>) -> Arc<LanMsgCore> {
    LanMsgCore::new(dir.to_string_lossy().into(), ks, Arc::new(FakePlatform)).unwrap()
}

fn init_req() -> Request {
    Request::Init {
        display_name: "小明".into(),
        device_name: "手机".into(),
        mode: InitMode::FirstDevice,
    }
}

fn self_info(rt: &tokio::runtime::Runtime, core: &LanMsgCore) -> lanmsg_api::SelfInfo {
    match rt.block_on(core.request(Request::GetSelf)).unwrap() {
        Response::SelfInfo { value } => value,
        other => panic!("意外响应: {other:?}"),
    }
}

#[test]
fn new_core_reports_uninitialized_then_initializes() {
    let dir = tempfile::tempdir().unwrap();
    let core = new_core(dir.path(), Arc::new(MemKs::default()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    assert!(!self_info(&rt, &core).initialized);
    rt.block_on(core.request(init_req())).unwrap();
    assert!(self_info(&rt, &core).initialized);
    core.shutdown();
}

#[test]
fn keys_are_saved_through_foreign_keystore() {
    let dir = tempfile::tempdir().unwrap();
    let ks = Arc::new(MemKs::default());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let core = new_core(dir.path(), ks.clone());
    rt.block_on(core.request(init_req())).unwrap();
    let first = self_info(&rt, &core).device_id;
    core.shutdown();
    drop(core);
    {
        let m = ks.0.lock().unwrap();
        assert!(m.contains_key("device.key"));
        assert!(m.contains_key("user.key"));
    }
    let core2 = new_core(dir.path(), ks);
    assert_eq!(self_info(&rt, &core2).device_id, first);
    core2.shutdown();
}

#[test]
fn request_after_shutdown_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let core = new_core(dir.path(), Arc::new(MemKs::default()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    core.shutdown();
    let r = rt.block_on(core.request(Request::GetSelf));
    assert!(matches!(
        r,
        Err(FfiError::Api {
            code: ErrorCode::Internal,
            ..
        })
    ));
    core.shutdown();
}

#[test]
fn event_listener_receives_message_added() {
    let dir = tempfile::tempdir().unwrap();
    let core = new_core(dir.path(), Arc::new(MemKs::default()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(core.request(init_req())).unwrap();
    let me = self_info(&rt, &core);
    let user = lanmsg_proto::ids::UserId::from_hex(&me.user_id.unwrap()).unwrap();
    let conv = lanmsg_proto::ids::ConvId::self_devices(&user).to_hex();
    let collect = Arc::new(Collect::default());
    core.set_event_listener(Some(collect.clone()));
    rt.block_on(core.request(Request::SendText {
        conv,
        text: "x".into(),
    }))
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let got = collect
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::MessageAdded { .. }));
        if got {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "5 秒内未收到 MessageAdded"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    core.shutdown();
}
