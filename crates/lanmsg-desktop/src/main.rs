// LanMsg 界面进程入口。
// 参数：`--data-dir <PATH>`、`--open-conv <CONV>`、`--demo <页>`（离线演示，不连后台服务），
// 以及 windui 自带的 `--screenshot <PNG>` 等截图参数（由 windui 自行解析，这里忽略）。
// 注释中文，标识符英文。

// Windows 发布版不带控制台窗口；日志写 stderr 失败（无控制台时句柄无效）不会 panic。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
mod bridge;
mod debounce;
mod decoders;
mod fixtures;
mod model;
mod strings;
mod style;
mod tz;
mod ui_state;
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
                eprintln!("未知的演示页面: {name}");
                std::process::exit(2);
            }
        },
    };
    decoders::register_media_decoders();
    let data_dir = cli
        .data_dir
        .clone()
        .unwrap_or_else(lanmsg_core::paths::default_data_dir);

    let dark = windui::event::system_prefers_dark();
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
    if demo.is_none() {
        // 清理上次遗留的未确认粘贴图片；后台线程，不阻塞界面
        let (pending, _) = views::composer::paste_dirs(&data_dir);
        std::thread::spawn(move || {
            views::composer::cleanup_pending(&pending, views::composer::PASTE_PENDING_MAX_AGE)
        });
        if let Some(conv) = cli.open_conv {
            ui.request_open_conv(conv);
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
            ui_tick.tick_files();
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
            if let Some(conv) = parse_cli(&argv).open_conv {
                let _ = open_tx.send(conv);
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
