// 界面状态出口：Model + Bridge + 变更计数信号。
// 视图只经 `rev`（派生信号）读取、经 `act`/`dispatch` 写入，Model 本身不暴露给 windui。
// 注释中文，标识符英文。

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use windui::prelude::{signal, Signal};

use crate::autostart;
use crate::bridge::{Bridge, ReplyTag, UiMsg};
use crate::debounce::Debouncer;
use crate::model::{ConnState, Effect, Model, Page, PairUi};
use crate::strings;
use crate::views::composer::PastedImage;
use crate::views::files::FilesUi;
use lanmsg_api::types::{PairStateDto, Response};

/// 当前输入框的登记：(会话, 文本信号)。
type ComposerSlot = (String, Signal<String>);

/// 界面共享状态句柄（可克隆，仅限 UI 线程使用）。
#[derive(Clone)]
pub struct UiState {
    model: Rc<RefCell<Model>>,
    /// 每次 Model 变化自增；视图用它派生出需要的信号。
    pub rev: Signal<u64>,
    bridge: Bridge,
    /// `--open-conv` 带来的待定位会话：会话列表到达后才能判断是否存在。
    pending_open: Rc<RefCell<Option<String>>>,
    /// 会话列表搜索框的文本；停止输入后由 `tick_search` 防抖触发搜索。
    pub search_text: Signal<String>,
    /// 转发对话框请求：(第几次打开, 要转发的文件哈希)，哈希为空表示关闭。
    /// 带序号是为了让对话框被 ESC 隐藏后，对同一批文件再次转发也能重建。
    pub forward: Signal<(u64, Vec<String>)>,
    /// 等待确认发送的粘贴图片；None 表示没有确认小窗。
    pub paste: Signal<Option<PastedImage>>,
    /// 回到列表底部的请求序号：消息列表按它重建，首次布局即滚到底。
    pub bottom_epoch: Signal<u64>,
    /// 文件页筛选控件的状态。
    pub files_ui: FilesUi,
    /// 当前输入框：(会话, 文本信号)；发送失败的草稿经它回填。
    composer: Rc<RefCell<Option<ComposerSlot>>>,
    /// 配对对话框的显隐信号：ESC 只隐藏、不改 Model，由 `tick_pair` 发现后取消配对。
    pair_show: Rc<RefCell<Option<Signal<bool>>>>,
    /// 配对完成（Done）后开始计时的时刻，用于 1.5 秒后自动关闭。
    pair_done_since: Rc<RefCell<Option<Instant>>>,
    /// 选择界面上次刷新候选设备的时刻；离开选择界面即清空。
    pair_refresh_at: Rc<RefCell<Option<Instant>>>,
    debouncer: Rc<RefCell<Debouncer>>,
    started: Instant,
    /// 数据目录：登记开机自启时决定是否带 `--data-dir`。
    data_dir: PathBuf,
}

/// 配对完成后多久自动关闭对话框（毫秒）。
const PAIR_DONE_CLOSE_MS: u128 = 1500;
/// 选择界面周期刷新候选设备的间隔。
const PAIR_REFRESH_MS: u128 = 3000;

/// 退出前同步清焦点的最长等待。
const FOCUS_CLEAR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// 文件页筛选控件只有文件页可见时才需要同步（纯判定，便于测试）。
fn files_tick_needed(page: Page) -> bool {
    page == Page::Files
}

/// 搜索框停止输入多久后发起搜索（毫秒）。
const SEARCH_DEBOUNCE_MS: u64 = 300;

impl UiState {
    pub fn new(model: Model, bridge: Bridge) -> UiState {
        let files_ui = FilesUi::new(&model.file_filter, &model.convs);
        UiState {
            model: Rc::new(RefCell::new(model)),
            rev: signal(0u64),
            bridge,
            pending_open: Rc::new(RefCell::new(None)),
            search_text: signal(String::new()),
            forward: signal((0u64, Vec::new())),
            paste: signal(None),
            bottom_epoch: signal(0u64),
            files_ui,
            composer: Rc::new(RefCell::new(None)),
            pair_show: Rc::new(RefCell::new(None)),
            pair_done_since: Rc::new(RefCell::new(None)),
            pair_refresh_at: Rc::new(RefCell::new(None)),
            debouncer: Rc::new(RefCell::new(Debouncer::new(SEARCH_DEBOUNCE_MS))),
            started: Instant::now(),
            data_dir: lanmsg_core::paths::default_data_dir(),
        }
    }

    /// 数据目录（粘贴图片的临时目录等位于其下）。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 让消息列表回到底部（发送消息、点击“有新消息”提示条时）。
    pub fn jump_to_bottom(&self) {
        self.bottom_epoch.update(|v| *v += 1);
    }

    /// 指定实际使用的数据目录（非演示模式）。
    pub fn with_data_dir(mut self, dir: PathBuf) -> UiState {
        self.data_dir = dir;
        self
    }

    /// 处理桥消息：更新 Model、执行副作用、通知视图。
    pub fn dispatch(&self, msg: UiMsg) {
        // 断线时丢弃此前的待定位请求，避免重连很久之后突然跳转；
        // 断线期间新发起的请求保留到重连后的首次会话列表回复
        if matches!(msg, UiMsg::Disconnected(_)) {
            self.pending_open.borrow_mut().take();
        }
        let is_convs_reply = matches!(
            &msg,
            UiMsg::Reply {
                tag: ReplyTag::Conversations,
                result: Ok(Response::Conversations { .. }),
            }
        );
        // 先放开 borrow 再执行副作用，避免回调重入时 RefCell 冲突
        let fx = self.model.borrow_mut().apply(msg);
        self.run(fx);
        self.restore_draft();
        if is_convs_reply {
            self.resolve_pending_open();
        }
    }

    /// 在 Model 上执行一个动作（用户操作入口）。
    pub fn act(&self, f: impl FnOnce(&mut Model) -> Vec<Effect>) {
        let fx = f(&mut self.model.borrow_mut());
        self.run(fx);
    }

    /// 只读访问 Model。
    pub fn read<R>(&self, f: impl FnOnce(&Model) -> R) -> R {
        f(&self.model.borrow())
    }

    /// 从 Model 派生一个随 `rev` 刷新的信号。
    pub fn derive<T: 'static>(&self, f: impl Fn(&Model) -> T + 'static) -> Signal<T> {
        let model = self.model.clone();
        self.rev.map(move |_| f(&model.borrow()))
    }

    /// 从 Model 派生一个“内容摘要”信号：所取内容没变，摘要就不变。
    /// 供 `switcher` 作重建依据（api 类型没有 `Eq + Hash`）；视图在闭包里再用 `read` 取内容。
    pub fn digest<T: std::fmt::Debug>(&self, f: impl Fn(&Model) -> T + 'static) -> Signal<u64> {
        use std::hash::{Hash, Hasher};
        self.derive(move |m| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", f(m)).hash(&mut h);
            h.finish()
        })
    }

    /// 取走待显示的提示文字（由持有 `EventCtx` 的调用方弹出）。
    pub fn take_toast(&self) -> Option<String> {
        self.model.borrow_mut().toast.take()
    }

    /// 尽力定位到某个会话（通知点击 / 单实例转发）。
    ///
    /// 会话不存在（例如 Windows 通知点击给出的 conv 不准确）时静默忽略。
    pub fn request_open_conv(&self, conv: String) {
        // 向导未完成时不跳转
        if self.read(|m| m.page == Page::Wizard) {
            return;
        }
        *self.pending_open.borrow_mut() = Some(conv);
        // 仅在已连接时立即刷新会话列表；否则等重连后的全量刷新，
        // 回复到达后在 resolve_pending_open 里判断存在与否
        if self.read(|m| m.conn == ConnState::Connected) {
            self.act(|m| m.set_page(Page::Chats));
        }
    }

    fn resolve_pending_open(&self) {
        let Some(conv) = self.pending_open.borrow_mut().take() else {
            return;
        };
        if self.read(|m| m.convs.iter().any(|c| c.conv == conv)) {
            self.act(|m| m.open_conv(conv));
        }
    }

    fn run(&self, effects: Vec<Effect>) {
        for fx in effects {
            match fx {
                Effect::Call(req, tag) => self.bridge.call(req, tag),
                Effect::EnableAutostart => {
                    if let Err(e) = autostart::set_enabled(true, &self.data_dir) {
                        self.model.borrow_mut().toast =
                            Some(format!("{}{}", strings::AUTOSTART_FAILED, e));
                    }
                }
            }
        }
        self.prune_composer();
        self.rev.update(|v| *v += 1);
    }

    /// 登记当前会话的输入框（`composer` 每次随会话重建时调用），并补回此前失败的草稿。
    pub fn register_composer(&self, conv: String, text: Signal<String>) {
        *self.composer.borrow_mut() = Some((conv, text));
        self.restore_draft();
    }

    /// 输入框所属会话已不在前台（离开会话页或切换了会话）时注销登记，
    /// 避免往已失效的 Signal 写入并白白消费草稿。
    fn prune_composer(&self) {
        let stale = match self.composer.borrow().as_ref() {
            Some((conv, _)) => self.read(|m| {
                m.page != Page::Chats || m.current_conv.as_deref() != Some(conv.as_str())
            }),
            None => false,
        };
        if stale {
            self.composer.borrow_mut().take();
        }
    }

    /// 发送失败的草稿属于当前输入框所在会话时回填并清除；输入框已有新内容则草稿放前面。
    fn restore_draft(&self) {
        let Some((conv, text)) = self.composer.borrow().clone() else {
            return;
        };
        let draft = self.model.borrow_mut().draft_restore.remove(&conv);
        if let Some(d) = draft {
            text.set(crate::views::composer::merge_draft(&d, &text.get()));
        }
    }

    /// 周期调用：粘贴确认小窗被 ESC 隐藏后，删除临时文件并清除状态。
    pub fn tick_paste(&self) {
        if let Some(p) = self.paste.get() {
            if !p.show.get() {
                let _ = std::fs::remove_file(p.path());
                self.paste.set(None);
            }
        }
    }

    /// 配对对话框建立时登记它的显隐信号。
    pub fn register_pair_show(&self, show: Signal<bool>) {
        *self.pair_show.borrow_mut() = Some(show);
    }

    /// 周期调用：配对对话框被 ESC 隐藏时取消配对；配对完成 1.5 秒后自动关闭。
    pub fn tick_pair(&self) {
        let (open, done) = self.read(|m| {
            (
                m.pair.is_some(),
                matches!(
                    &m.pair,
                    Some(PairUi::Showing {
                        state: PairStateDto::Done,
                        ..
                    })
                ),
            )
        });
        if !open {
            self.pair_show.borrow_mut().take();
            self.pair_done_since.borrow_mut().take();
            self.pair_refresh_at.borrow_mut().take();
            return;
        }
        let hidden = self
            .pair_show
            .borrow()
            .as_ref()
            .is_some_and(|show| !show.get());
        if hidden {
            self.act(|m| m.pair_cancel());
            return;
        }
        if self.read(|m| m.pair == Some(PairUi::Picking)) {
            let mut at = self.pair_refresh_at.borrow_mut();
            if at.is_some_and(|t| t.elapsed().as_millis() < PAIR_REFRESH_MS) {
                return;
            }
            *at = Some(Instant::now());
            drop(at);
            self.act(|m| m.pair_refresh_peers());
            return;
        }
        self.pair_refresh_at.borrow_mut().take();
        if !done {
            self.pair_done_since.borrow_mut().take();
            return;
        }
        let since = *self
            .pair_done_since
            .borrow_mut()
            .get_or_insert_with(Instant::now);
        if since.elapsed().as_millis() >= PAIR_DONE_CLOSE_MS {
            self.act(|m| m.pair_close());
        }
    }

    /// 周期调用：搜索框文本稳定后发起搜索。
    pub fn tick_search(&self) {
        let text = self.search_text.get();
        if self.debouncer.borrow().is_idle(&text) {
            return;
        }
        let now = self.started.elapsed().as_millis() as u64;
        let fired = self.debouncer.borrow_mut().feed(&text, now);
        if let Some(q) = fired {
            self.act(|m| m.search(q));
        }
    }

    /// 周期调用：把文件页筛选控件上的变化同步给 Model。
    pub fn tick_files(&self) {
        if !files_tick_needed(self.read(|m| m.page)) {
            return;
        }
        let now = self.started.elapsed().as_millis() as u64;
        self.files_ui.tick(self, now);
    }

    /// 点击搜索结果：清空搜索框并打开会话。
    pub fn open_from_search(&self, conv: String) {
        let now = self.started.elapsed().as_millis() as u64;
        self.debouncer.borrow_mut().reset_to("", now);
        self.search_text.set(String::new());
        self.act(|m| m.open_from_search(conv));
    }

    /// 打开“转发到”对话框。
    pub fn open_forward(&self, hashes: Vec<String>) {
        self.forward.update(|f| *f = (f.0 + 1, hashes));
    }

    /// 关闭“转发到”对话框。
    pub fn close_forward(&self) {
        self.forward.update(|f| f.1.clear());
    }

    /// 连接状态是否为断开（顶部提示条用）。
    pub fn disconnected(&self) -> Signal<bool> {
        self.derive(|m| m.conn.is_down())
    }

    /// 顶部提示条文案：普通重连 / 后台服务已退出 / 拉起失败原因。
    pub fn banner_text(&self) -> Signal<String> {
        self.derive(|m| match &m.conn {
            ConnState::DaemonQuit => strings::DAEMON_QUIT_BANNER.to_string(),
            ConnState::LaunchFailed(why) => why.clone(),
            ConnState::Disconnected(r) if r == strings::DAEMON_STARTING => r.clone(),
            _ => strings::DISCONNECTED_BANNER.to_string(),
        })
    }

    /// 后台服务已主动退出（提示条上显示“启动后台服务”按钮）。
    pub fn daemon_quit(&self) -> Signal<bool> {
        self.derive(|m| m.conn == ConnState::DaemonQuit)
    }

    /// “启动后台服务”按钮：清除退出标记并拉起 lanmsgd。
    pub fn start_daemon(&self) {
        self.bridge.start_daemon();
    }

    /// 窗口关闭前同步清除界面焦点（最多等 1 秒）；core 在连接断开时也会清，这里是双保险。
    pub fn clear_focus_on_exit(&self) {
        self.bridge.call_sync(
            lanmsg_api::types::Request::SetUiFocus { conv: None },
            FOCUS_CLEAR_TIMEOUT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanmsg_api::types::{ConvKind, ConversationSummary};

    fn conv(id: &str) -> ConversationSummary {
        ConversationSummary {
            conv: id.into(),
            kind: ConvKind::Dm,
            title: id.into(),
            last_text: None,
            last_kind: None,
            last_hlc_millis: Some(1),
            unread: 0,
            peer_online: true,
        }
    }

    fn convs_reply(ids: &[&str]) -> UiMsg {
        UiMsg::Reply {
            tag: ReplyTag::Conversations,
            result: Ok(Response::Conversations {
                value: ids.iter().map(|i| conv(i)).collect(),
            }),
        }
    }

    #[test]
    fn files_tick_only_on_files_page() {
        assert!(files_tick_needed(Page::Files));
        for p in [Page::Chats, Page::Contacts, Page::Settings, Page::Wizard] {
            assert!(!files_tick_needed(p));
        }
    }

    #[test]
    fn files_filter_change_is_not_applied_off_the_files_page() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.act(|m| m.set_page(Page::Chats));
        let before = ui.read(|m| m.file_filter.clone());
        ui.files_ui.kind.set(1);
        ui.tick_files();
        assert_eq!(
            ui.read(|m| m.file_filter.clone()),
            before,
            "非文件页不应重建"
        );
        ui.act(|m| m.set_page(Page::Files));
        ui.tick_files();
        assert_ne!(ui.read(|m| m.file_filter.clone()), before);
    }

    #[test]
    fn open_conv_request_resolves_when_conv_list_arrives() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.dispatch(UiMsg::Connected);
        ui.request_open_conv("c2".into());
        assert_eq!(
            ui.read(|m| m.current_conv.clone()),
            None,
            "列表未到先不打开"
        );
        ui.dispatch(convs_reply(&["c1", "c2"]));
        assert_eq!(ui.read(|m| m.current_conv.clone()).as_deref(), Some("c2"));
    }

    #[test]
    fn pending_open_is_dropped_on_disconnect() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.dispatch(UiMsg::Connected);
        ui.request_open_conv("c2".into());
        ui.dispatch(UiMsg::Disconnected("断开".into()));
        ui.dispatch(UiMsg::Connected);
        ui.dispatch(convs_reply(&["c1", "c2"]));
        assert_eq!(ui.read(|m| m.current_conv.clone()), None);
    }

    #[test]
    fn pending_open_while_disconnected_resolves_after_reconnect() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.dispatch(UiMsg::Disconnected("断开".into()));
        ui.request_open_conv("c2".into());
        assert_eq!(ui.read(|m| m.page), Page::Chats, "不应被改页");
        ui.dispatch(UiMsg::Connected);
        ui.dispatch(convs_reply(&["c1", "c2"]));
        assert_eq!(ui.read(|m| m.current_conv.clone()).as_deref(), Some("c2"));
    }

    #[test]
    fn open_conv_request_for_unknown_conv_is_ignored() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.request_open_conv("ghost".into());
        ui.dispatch(convs_reply(&["c1"]));
        assert_eq!(ui.read(|m| m.current_conv.clone()), None);
        // 之后新出现同名会话也不会被追溯打开
        ui.dispatch(convs_reply(&["c1", "ghost"]));
        assert_eq!(ui.read(|m| m.current_conv.clone()), None);
    }

    fn send_failed(conv: &str, draft: &str) -> UiMsg {
        UiMsg::Reply {
            tag: ReplyTag::Sent {
                conv: conv.into(),
                draft: Some(draft.into()),
            },
            result: Err(lanmsg_api::types::ApiError {
                code: lanmsg_api::types::ErrorCode::PeerOffline,
                message: "对方离线".into(),
            }),
        }
    }

    #[test]
    fn failed_draft_is_restored_into_live_composer() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.act(|m| m.open_conv("c1".into()));
        let text = signal(String::new());
        ui.register_composer("c1".into(), text);
        ui.dispatch(send_failed("c1", "草稿"));
        assert_eq!(text.get(), "草稿");
        assert!(ui.read(|m| m.draft_restore.is_empty()));
    }

    #[test]
    fn composer_is_unregistered_when_leaving_page_or_switching_conv() {
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.act(|m| m.open_conv("c1".into()));
        let text = signal(String::new());
        ui.register_composer("c1".into(), text);
        ui.act(|m| m.set_page(Page::Files));
        ui.dispatch(send_failed("c1", "草稿"));
        assert_eq!(text.get(), "", "已离开会话页，不应写入失效的输入框");
        assert_eq!(
            ui.read(|m| m.draft_restore.get("c1").cloned()).as_deref(),
            Some("草稿")
        );

        // 切换会话同样注销；回到 c1 重建输入框时补回草稿
        let ui = UiState::new(Model::new(), Bridge::detached());
        ui.act(|m| m.open_conv("c1".into()));
        let old = signal(String::new());
        ui.register_composer("c1".into(), old);
        ui.act(|m| m.open_conv("c2".into()));
        ui.dispatch(send_failed("c1", "草稿"));
        assert_eq!(old.get(), "");
        ui.act(|m| m.open_conv("c1".into()));
        let new = signal(String::new());
        ui.register_composer("c1".into(), new);
        assert_eq!(new.get(), "草稿");
    }
}
