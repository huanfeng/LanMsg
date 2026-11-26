// LanMsg 界面进程入口。
// 参数：`--data-dir <PATH>`、`--open-conv <CONV>`、`--web`（连上后开启网页传输）、`--demo <页>`（离线演示，不连后台服务）、
// `--station-put <PATH>...`（不开窗口，放入本机中转站）、`--theme dark|light`，
// 以及 windui 自带的 `--screenshot <PNG>` 等截图参数（由 windui 自行解析，这里忽略）。
// 注释中文，标识符英文。

// Windows 发布版不带控制台窗口；日志写 stderr 失败（无控制台时句柄无效）不会 panic。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
mod bridge;
mod debounce;
mod decoders;
mod error_text;
mod event_text;
mod fixtures;
mod model;
mod station_put;
mod strings;
mod style;
mod tz;
mod ui_state;
mod update;
mod views;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windui::prelude::*;

use bridge::{Bridge, UiMsg};
use fixtures::{demo_model, DemoPage};
use model::Model;
use ui_state::UiState;

/// 窗口默认与最小尺寸（逻辑像素）。
const WINDOW_SIZE: (i32, i32) = (1000, 680);
const WINDOW_MIN: (i32, i32) = (760, 520);

/// 周期任务间隔；各 tick 自带“无事可做就立即返回”的判定。搜索防抖 300ms 仍可满足。
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// 自有命令行参数；windui 的截图参数不在此解析。
#[derive(Debug, Default, PartialEq)]
struct Cli {
    data_dir: Option<PathBuf>,
    open_conv: Option<String>,
    demo: Option<String>,
    /// `--station-put <PATH>...`：把这些路径放进本机中转站后退出（不开窗口）。
    station_put: Option<Vec<PathBuf>>,
    /// `--theme dark|light`：强制明暗主题（截图与排查用），缺省跟随系统。
    theme_dark: Option<bool>,
    /// `--web`：连上后台服务后开启网页传输（托盘“开启网页传输”拉起界面时带上）。
    web: bool,
}

/// 从 argv 里挑出自有参数，其余原样忽略（单实例转发的 argv 同样走这里）。
fn parse_cli(args: &[String]) -> Cli {
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data-dir" => cli.data_dir = it.next().map(PathBuf::from),
            "--open-conv" => cli.open_conv = it.next().cloned(),
            "--demo" => cli.demo = it.next().cloned(),
            "--web" => cli.web = true,
            "--theme" => {
                cli.theme_dark = match it.next().map(String::as_str) {
                    Some("dark") => Some(true),
                    Some("light") => Some(false),
                    _ => None,
                }
            }
            // 后面直到下一个以 -- 开头的参数都是路径
            "--station-put" => {
                let mut paths = Vec::new();
                while let Some(next) = it.clone().next() {
                    if next.starts_with("--") {
                        break;
                    }
                    paths.push(PathBuf::from(next));
                    it.next();
                }
                cli.station_put = Some(paths);
            }
            _ => {}
        }
    }
    cli
}

/// 单实例标识：同一数据目录只允许一个界面。
fn instance_id(data_dir: &Path) -> String {
    let canonical = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let hex = blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex();
    format!("lanmsg-gui-{}", &hex.as_str()[..8])
}

/// Linux 上是否明显没有图形显示：`DISPLAY` 与 `WAYLAND_DISPLAY` 都为空，且不是 demo、不是离屏截图。
///
/// 窗口要到 `run()` 才会真正连显示服务器，而 `Bridge::start` 在那之前就会在后台线程里拉起 `lanmsgd`；
/// 没有显示时窗口随后报错退出，就会留下一个按默认数据目录运行的孤儿 daemon。所以在拉起之前先预检。
/// 只拦“明显没有显示”这一种最确定的情形，连不连得上显示服务器仍由 windui 自己报告。
/// 离屏截图（`--screenshot`）与 demo 不需要显示，不能被拦住。
fn missing_display(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    demo: bool,
    is_linux: bool,
) -> bool {
    if !is_linux || demo || args.iter().any(|a| a == "--screenshot") {
        return false;
    }
    let unset = |k: &str| env(k).is_none_or(|v| v.is_empty());
    unset("DISPLAY") && unset("WAYLAND_DISPLAY")
}

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_cli(&args);
    let demo = match cli.demo.as_deref() {
        None => None,
        Some(name) => match DemoPage::parse(name) {
            Some(p) => Some(p),
            None => {
                eprintln!("unknown demo page: {name}");
                std::process::exit(2);
            }
        },
    };
    let data_dir = cli
        .data_dir
        .clone()
        .unwrap_or_else(lanmsg_core::paths::default_data_dir);
    // “发送到”入口：不创建窗口、不注册单实例，放完即退出。
    if let Some(paths) = cli.station_put.clone() {
        if let Err(e) = station_put::run_station_put(&data_dir, paths) {
            tracing::error!("station-put failed: {e}");
            station_put::log_failure(&data_dir, &e);
            std::process::exit(1);
        }
        return;
    }
    // 没有显示就别拉起后台服务（见 missing_display）。
    if missing_display(
        &args,
        &|k| std::env::var(k).ok(),
        demo.is_some(),
        cfg!(target_os = "linux"),
    ) {
        eprintln!(
            "[lanmsg] no display: neither DISPLAY nor WAYLAND_DISPLAY is set; \
             run lanmsg from a graphical session (use lanmsg-cli or lanmsgd --headless without one)"
        );
        std::process::exit(1);
    }
    decoders::register_media_decoders();

    let dark = cli
        .theme_dark
        .unwrap_or_else(windui::event::system_prefers_dark);
    let mut app = App::new(strings::APP_TITLE, WINDOW_SIZE.0, WINDOW_SIZE.1)
        .min_size(WINDOW_MIN.0, WINDOW_MIN.1)
        .icon(style::window_icon())
        .theme(style::theme(dark))
        .screenshot_from_args();
    let theme = app.theme_handle();

    // 通道回调先于 UiState 创建，经此槽位取用
    let slot: Rc<RefCell<Option<UiState>>> = Rc::new(RefCell::new(None));
    let slot_msg = slot.clone();
    let tx = app.channel::<UiMsg>(move |ctx, msg| {
        let ui = slot_msg.borrow().clone();
        if let Some(ui) = ui {
            ui.dispatch(msg);
            if let Some(t) = ui.take_toast() {
                ctx.toast(t);
            }
        }
    });
    let slot_open = slot.clone();
    let slot_web = slot.clone();
    let web_tx = app.channel::<()>(move |_ctx, ()| {
        let ui = slot_web.borrow().clone();
        if let Some(ui) = ui {
            ui.request_web();
        }
    });
    let open_tx = app.channel::<String>(move |_ctx, conv| {
        let ui = slot_open.borrow().clone();
        if let Some(ui) = ui {
            ui.request_open_conv(conv);
        }
    });

    let ui = match demo {
        Some(p) => UiState::new(demo_model(p), Bridge::detached()),
        None => {
            let bridge = Bridge::start(
                lanmsg_api::ipc::endpoint_name(&data_dir),
                data_dir.clone(),
                tx,
            );
            UiState::new(Model::new(), bridge).with_data_dir(data_dir.clone())
        }
    };
    *slot.borrow_mut() = Some(ui.clone());
    if let Some((path, name, kind)) = demo.and_then(fixtures::demo_preview) {
        ui.preview.open_blocking(path, name, kind);
    }
    if demo.is_none() {
        // 清理上次遗留的未确认粘贴图片；后台线程，不阻塞界面
        let (pending, _) = views::composer::paste_dirs(&data_dir);
        std::thread::spawn(move || {
            views::composer::cleanup_pending(&pending, views::composer::PASTE_PENDING_MAX_AGE)
        });
        if let Some(conv) = cli.open_conv {
            ui.request_open_conv(conv);
        }
        if cli.web {
            ui.request_web();
        }
    }

    let ui_focus = ui.clone();
    let ui_close = ui.clone();
    let ui_tick = ui.clone();
    app = app
        .on_close_request(move |_ctx| {
            // 关窗即退出：先同步清掉界面焦点，再放行关闭
            ui_close.clear_focus_on_exit();
            true
        })
        .on_interval(TICK_INTERVAL, move |_ctx| {
            ui_tick.tick_search();
            ui_tick.tick_paste();
            ui_tick.tick_pair();
            ui_tick.tick_web();
            ui_tick.tick_scan();
            ui_tick.tick_firewall();
            ui_tick.tick_files();
            ui_tick.preview.tick();
        })
        .on_window_activated(move |_ctx, active| {
            ui_focus.act(|m| m.set_window_focused(active));
        })
        .on_system_theme_changed(move |_ctx, is_dark| {
            // 只换色板，保留其它主题设置；色值映射见 style::palette
            theme.update(|t| t.palette = style::palette(is_dark));
        });
    // 演示模式不注册单实例
    if demo.is_none() {
        app = app.single_instance(instance_id(&data_dir), move |argv| {
            // 第二个实例把 --open-conv 带过来；解析不到就只是激活窗口
            let cli = parse_cli(&argv);
            if let Some(conv) = cli.open_conv {
                let _ = open_tx.send(conv);
            }
            if cli.web {
                let _ = web_tx.send(());
            }
        });
    }
    app.content(views::shell::shell(&ui)).run();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn missing_display_only_blocks_a_plain_linux_launch_without_any_display_env() {
        let none = |_: &str| None;
        let empty = |_: &str| Some(String::new());
        let x11 = |k: &str| (k == "DISPLAY").then(|| ":0".to_string());
        let wayland = |k: &str| (k == "WAYLAND_DISPLAY").then(|| "wayland-0".to_string());
        let plain = argv(&["lanmsg"]);
        // 明显没有显示：拦截（变量缺失或为空都算）。
        assert!(missing_display(&plain, &none, false, true));
        assert!(missing_display(&plain, &empty, false, true));
        // 任一显示变量存在：放行，连不连得上交给 windui 报告。
        assert!(!missing_display(&plain, &x11, false, true));
        assert!(!missing_display(&plain, &wayland, false, true));
        // 离屏截图与 demo 不需要显示：不能被拦（截图测试与 CI 靠它）。
        let shot = argv(&["lanmsg", "--demo", "chats", "--screenshot", "/tmp/a.png"]);
        assert!(!missing_display(&shot, &none, false, true));
        assert!(!missing_display(&plain, &none, true, true));
        // 非 Linux：Windows/macOS 恒有图形会话。
        assert!(!missing_display(&plain, &none, false, false));
    }

    #[test]
    fn parse_cli_picks_own_args_and_ignores_windui_ones() {
        let cli = parse_cli(&argv(&[
            "lanmsg",
            "--screenshot",
            "/tmp/a.png",
            "--size",
            "1000",
            "680",
            "--data-dir",
            "/d",
            "--open-conv",
            "abc",
            "--demo",
            "chat",
        ]));
        assert_eq!(cli.data_dir, Some(PathBuf::from("/d")));
        assert_eq!(cli.open_conv.as_deref(), Some("abc"));
        assert_eq!(cli.demo.as_deref(), Some("chat"));
    }

    #[test]
    fn station_put_arg_parses_paths() {
        let cli = parse_cli(&argv(&[
            "lanmsg",
            "--station-put",
            "a",
            "b c",
            "--data-dir",
            "/d",
        ]));
        assert_eq!(
            cli.station_put,
            Some(vec![PathBuf::from("a"), PathBuf::from("b c")])
        );
        assert_eq!(cli.data_dir, Some(PathBuf::from("/d")));
        // 没有路径时是空列表（由入口报错），不是 None
        assert_eq!(
            parse_cli(&argv(&["lanmsg", "--station-put"])).station_put,
            Some(Vec::new())
        );
    }

    #[test]
    fn theme_arg_forces_dark_or_light() {
        assert_eq!(
            parse_cli(&argv(&["lanmsg", "--theme", "dark"])).theme_dark,
            Some(true)
        );
        assert_eq!(
            parse_cli(&argv(&["lanmsg", "--theme", "light"])).theme_dark,
            Some(false)
        );
        assert_eq!(parse_cli(&argv(&["lanmsg"])).theme_dark, None);
    }

    #[test]
    fn web_arg_starts_web_on_connect() {
        assert!(parse_cli(&argv(&["lanmsg", "--web"])).web);
        assert!(!parse_cli(&argv(&["lanmsg", "--data-dir", "/d"])).web);
    }

    #[test]
    fn parse_cli_defaults_to_none() {
        assert_eq!(parse_cli(&argv(&["lanmsg"])), Cli::default());
    }

    #[test]
    fn instance_id_is_stable_and_differs_per_dir() {
        let a = instance_id(Path::new("/nonexistent/a"));
        assert_eq!(a, instance_id(Path::new("/nonexistent/a")));
        assert_ne!(a, instance_id(Path::new("/nonexistent/b")));
        assert!(a.starts_with("lanmsg-gui-") && a.len() == "lanmsg-gui-".len() + 8);
    }
}
