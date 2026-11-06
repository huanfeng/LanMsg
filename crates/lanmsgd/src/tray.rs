// 托盘、通知、拉起界面（P3 Task 9）。
// 主线程跑 windui 零窗口常驻模式；tokio 在 DaemonRuntime 的后台线程。
// 后台线程（事件转发、外部关闭）经 `App::channel` 回到 UI 线程。
// 注释中文，标识符英文。

use std::path::PathBuf;

use lanmsg_api::{Event, Request, Response, StatusDto};
use windui::platform::tray_supported;
use windui::prelude::*;

use crate::launch::{conv_from_tag, open_gui};
use crate::notify::{notice_for, TitleCache};
use crate::runtime::DaemonRuntime;

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
            tracing::warn!("托盘图标解码失败: {e}");
            None
        }
    }
}

/// 在界面线程分离启动界面进程，失败只记日志。
fn open_gui_logged(data_dir: &std::path::Path, conv: Option<&str>) {
    if let Err(e) = open_gui(data_dir, conv) {
        tracing::warn!("拉起界面失败: {e}");
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
            Err(e) => tracing::warn!("设置状态失败: {e:?}"),
        }
    })
}

/// 运行托盘（主线程阻塞）。显示服务不可用时转为无头继续服务，不退出进程。
pub fn run_tray(rt: DaemonRuntime, data_dir: PathBuf) {
    let handle = rt.handle();

    // 初始状态来自 GetSelf。
    let initial = match handle.block_on(rt.service.handle(Request::GetSelf)) {
        Ok(Response::SelfInfo { value }) => value.status,
        _ => StatusDto::Online,
    };
    let status = signal(initial);

    // 托盘配置始终传入：Linux 上没有 Watcher 时 windui 自行降级，通知照常发出。
    let supported = tray_supported();
    if !supported {
        // 此刻没有托盘入口：开一次界面，让用户看得到界面。
        tracing::info!("托盘宿主不可用，直接打开界面");
        open_gui_logged(&data_dir, None);
    }

    let mut tray = Tray::new().tooltip("LanMsg");
    if let Some((w, h, rgba)) = tray_icon_rgba() {
        tray = tray.icon_rgba(w, h, &rgba);
    }
    let dir_left = data_dir.clone();
    let dir_menu = data_dir.clone();
    let dir_click = data_dir.clone();
    let service_quit = rt.service.clone();
    let handle_quit = handle.clone();
    let tray = tray
        .on_left_click(move |_ctx| open_gui_logged(&dir_left, None))
        .on_notification_click(move |_ctx, tag| {
            // tag 为空 / None 时只激活界面；否则尽力定位会话（Windows 的 tag 可能不对应被点的那条）。
            open_gui_logged(&dir_click, conv_from_tag(tag).as_deref());
        })
        .on_availability_changed(|_ctx, up| {
            tracing::debug!("托盘可用性变化: {up}");
        })
        .menu(vec![
            TrayMenuItem::item("打开 LanMsg", move |_ctx| {
                open_gui_logged(&dir_menu, None)
            }),
            TrayMenuItem::separator(),
            status_item("在线", StatusDto::Online, status, &rt),
            status_item("隐身", StatusDto::Invisible, status, &rt),
            status_item("忙碌", StatusDto::Busy, status, &rt),
            TrayMenuItem::separator(),
            TrayMenuItem::item("退出", move |ctx| {
                if let Err(e) = handle_quit.block_on(service_quit.handle(Request::Shutdown)) {
                    tracing::warn!("退出请求失败: {e:?}");
                }
                ctx.quit();
            }),
        ]);

    let mut app = App::resident("LanMsg").tray(tray);
    let tray_handle = app.tray_handle();
    let service = rt.service.clone();
    let mut titles = TitleCache::default();
    let tx = app.channel::<UiMsg>(move |_ctx, msg| match msg {
        UiMsg::Quit => tray_handle.quit(),
        UiMsg::Event(ev) => {
            titles.observe(&ev);
            let focus = service.ui_focus().map(|c| c.to_hex());
            if let Some(n) = notice_for(&ev, focus.as_deref(), &status.get(), &titles) {
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
        tracing::warn!("显示服务不可用，转为无头模式");
        rt.block_until_shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
