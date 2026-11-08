// 托盘、通知、拉起界面（P3 Task 9）。
// 主线程跑 windui 零窗口常驻模式；tokio 在 DaemonRuntime 的后台线程。
// 后台线程（事件转发、外部关闭）经 `App::channel` 回到 UI 线程。
// 注释中文，标识符英文。

use std::path::PathBuf;

use lanmsg_api::{Event, Request, Response, StatusDto};
use windui::platform::{resident_available, tray_supported};
use windui::prelude::*;

use crate::launch::{conv_from_tag, open_gui, open_gui_with};
use crate::notify::{notice_for, Notice, TitleCache};
use crate::runtime::DaemonRuntime;
use crate::strings;

/// 托盘图标（32x32 PNG，编译期内嵌）。
const TRAY_ICON_PNG: &[u8] = include_bytes!("../assets/tray-32.png");

/// 后台线程送回 UI 线程的消息。
enum UiMsg {
    /// 核心事件。
    Event(Event),
    /// 服务已被外部关闭（IPC Shutdown / SIGTERM），托盘需退出。
    Quit,
}

/// 解码托盘图标为 RGBA；失败返回 None（托盘用系统默认图标）。
fn tray_icon_rgba() -> Option<(u32, u32, Vec<u8>)> {
    match image::load_from_memory(TRAY_ICON_PNG) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            Some((rgba.width(), rgba.height(), rgba.into_raw()))
        }
        Err(e) => {
            tracing::warn!("failed to decode tray icon: {e}");
            None
        }
    }
}

/// 在界面线程分离启动界面进程，失败只记日志。
fn open_gui_logged(data_dir: &std::path::Path, conv: Option<&str>) {
    if let Err(e) = open_gui(data_dir, conv) {
        tracing::warn!("failed to launch UI: {e}");
    }
}

/// 一个互斥的状态勾选项：点击时经 Service 设置状态，成功后更新 `status` 信号。
fn status_item(
    label: &str,
    target: StatusDto,
    status: Signal<StatusDto>,
    rt: &DaemonRuntime,
) -> TrayMenuItem {
    let checked = status.map({
        let target = target.clone();
        move |s| *s == target
    });
    let service = rt.service.clone();
    let handle = rt.handle();
    TrayMenuItem::check(label, checked, move |_ctx| {
        let req = Request::SetStatus {
            status: target.clone(),
        };
        match handle.block_on(service.handle(req)) {
            Ok(_) => status.set(target.clone()),
            Err(e) => tracing::warn!("failed to set status: {e:?}"),
        }
    })
}

/// 弹通知前同步最新在线状态：仅 `MessageAdded` / `IncomingRequest` 才取（其余事件不会弹通知）。
/// 取到则覆盖 `status`（CLI/界面/Android 改状态时托盘本地值会过期），取不到沿用旧值。
fn notice_with_fresh_status(
    ev: &Event,
    focus: Option<&str>,
    status: &mut StatusDto,
    titles: &TitleCache,
    fetch_status: impl Fn() -> Option<StatusDto>,
) -> Option<Notice> {
    if matches!(
        ev,
        Event::MessageAdded { .. } | Event::IncomingRequest { .. }
    ) {
        match fetch_status() {
            Some(fresh) => *status = fresh,
            None => tracing::debug!("failed to get current status, keeping local tray state"),
        }
    }
    notice_for(ev, focus, status, titles)
}

/// 运行托盘（主线程阻塞）。显示服务不可用时转为无头继续服务，不退出进程。
pub fn run_tray(rt: DaemonRuntime, data_dir: PathBuf) {
    let handle = rt.handle();

    // 初始状态与本机设备 id 来自 GetSelf。
    let (initial, self_device) = match handle.block_on(rt.service.handle(Request::GetSelf)) {
        Ok(Response::SelfInfo { value }) => (value.status, value.device_id),
        _ => (StatusDto::Online, String::new()),
    };
    let status = signal(initial);

    // 托盘配置始终传入：Linux 上没有 Watcher 时 windui 自行降级，通知照常发出。
    let supported = tray_supported();
    if !supported {
        // 没有显示服务：拉起界面也会失败，直接走无头。
        if !resident_available() {
            tracing::warn!("display server unavailable, switching to headless mode");
            rt.block_until_shutdown();
            return;
        }
        // 此刻没有托盘入口：开一次界面，让用户看得到界面。
        tracing::info!("tray host unavailable, opening the UI directly");
        open_gui_logged(&data_dir, None);
    }

    let mut tray = Tray::new().tooltip("LanMsg");
    if let Some((w, h, rgba)) = tray_icon_rgba() {
        tray = tray.icon_rgba(w, h, &rgba);
    }
    let dir_left = data_dir.clone();
    let dir_menu = data_dir.clone();
    let dir_web = data_dir.clone();
    let dir_click = data_dir.clone();
    let service_quit = rt.service.clone();
    let handle_quit = handle.clone();
    let dir_quit = data_dir.clone();
    let tray = tray
        .on_left_click(move |_ctx| open_gui_logged(&dir_left, None))
        .on_notification_click(move |_ctx, tag| {
            // tag 为空 / None 时只激活界面；否则尽力定位会话（Windows 的 tag 可能不对应被点的那条）。
            open_gui_logged(&dir_click, conv_from_tag(tag).as_deref());
        })
        .on_availability_changed(|_ctx, up| {
            tracing::debug!("tray availability changed: {up}");
        })
        .menu(vec![
            TrayMenuItem::item(strings::TRAY_OPEN, move |_ctx| {
                open_gui_logged(&dir_menu, None)
            }),
            TrayMenuItem::item(strings::TRAY_WEB, move |_ctx| {
                if let Err(e) = open_gui_with(&dir_web, &["--web"]) {
                    tracing::warn!("failed to launch UI for web transfer: {e}");
                }
            }),
            TrayMenuItem::separator(),
            status_item(strings::TRAY_ONLINE, StatusDto::Online, status, &rt),
            status_item(strings::TRAY_INVISIBLE, StatusDto::Invisible, status, &rt),
            status_item(strings::TRAY_BUSY, StatusDto::Busy, status, &rt),
            TrayMenuItem::separator(),
            TrayMenuItem::item(strings::TRAY_QUIT, move |ctx| {
                if let Err(e) = handle_quit.block_on(service_quit.handle(Request::Shutdown)) {
                    tracing::warn!("quit request failed: {e:?}");
                }
                crate::runtime::write_quit_marker(&dir_quit);
                ctx.quit();
            }),
        ]);

    let mut app = App::resident("LanMsg").tray(tray);
    let tray_handle = app.tray_handle();
    let service = rt.service.clone();
    let handle_ev = handle.clone();
    let mut titles = TitleCache::default();
    titles.set_self_device(self_device);
    let tx = app.channel::<UiMsg>(move |_ctx, msg| match msg {
        UiMsg::Quit => tray_handle.quit(),
        UiMsg::Event(ev) => {
            titles.observe(&ev);
            let focus = service.ui_focus().map(|c| c.to_hex());
            let mut current = status.get();
            // 回调跑在 UI 线程（非 tokio 线程），可 block_on。
            let fetch = || match handle_ev.block_on(service.handle(Request::GetSelf)) {
                Ok(Response::SelfInfo { value }) => Some(value.status),
                _ => None,
            };
            let n = notice_with_fresh_status(&ev, focus.as_deref(), &mut current, &titles, fetch);
            if current != status.get() {
                status.set(current);
            }
            if let Some(n) = n {
                tray_handle.notify_tagged(n.title, n.body, n.conv.unwrap_or_default());
            }
        }
    });

    let ev_tx = tx.clone();
    rt.spawn_event_forwarder(move |ev| {
        let _ = ev_tx.send(UiMsg::Event(ev));
    });
    rt.on_shutdown(move || {
        let _ = tx.send(UiMsg::Quit);
    });

    if app.run_resident() {
        // 用户点退出或服务被外部关闭：正常收尾（Drop 清理 IPC socket）。
        rt.shutdown();
    } else {
        tracing::warn!("display server unavailable, switching to headless mode");
        rt.block_until_shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_api::{MessageBodyView, MessageState, MessageView};

    fn added() -> Event {
        Event::MessageAdded {
            msg: MessageView {
                id: "m1".into(),
                conv: "c1".into(),
                author_user: "u".into(),
                author_device: "d".into(),
                author_device_name: "手机".into(),
                is_mine: false,
                hlc_millis: 0,
                body: MessageBodyView::Text { text: "hi".into() },
                state: MessageState::Delivered,
            },
        }
    }

    #[test]
    fn fresh_busy_suppresses_notice_and_updates_local_status() {
        let mut status = StatusDto::Online;
        let n =
            notice_with_fresh_status(&added(), None, &mut status, &TitleCache::default(), || {
                Some(StatusDto::Busy)
            });
        assert_eq!(n, None);
        assert_eq!(status, StatusDto::Busy);
    }

    #[test]
    fn fetch_failure_keeps_old_status() {
        let mut status = StatusDto::Busy;
        let n =
            notice_with_fresh_status(&added(), None, &mut status, &TitleCache::default(), || None);
        assert_eq!(n, None);
        assert_eq!(status, StatusDto::Busy);
        let mut status = StatusDto::Online;
        let n =
            notice_with_fresh_status(&added(), None, &mut status, &TitleCache::default(), || None);
        assert!(n.is_some());
    }

    #[test]
    fn other_events_do_not_fetch_status() {
        let mut status = StatusDto::Online;
        let n = notice_with_fresh_status(
            &Event::PeerOffline { device: "d".into() },
            None,
            &mut status,
            &TitleCache::default(),
            || panic!("不应取状态"),
        );
        assert_eq!(n, None);
    }

    #[test]
    fn embedded_tray_icon_decodes_to_32x32_rgba() {
        let (w, h, rgba) = tray_icon_rgba().expect("内嵌图标应可解码");
        assert_eq!((w, h), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4);
        // 圆角：左上角像素透明，中心不透明。
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[(16 * 32 + 16) * 4 + 3], 255);
    }
}
