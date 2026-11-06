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
use crate::model::{ConnState, Effect, Model, Page};
use crate::strings;
use crate::views::composer::PastedImage;
use lanmsg_api::types::Response;

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
    debouncer: Rc<RefCell<Debouncer>>,
    started: Instant,
    /// 数据目录：登记开机自启时决定是否带 `--data-dir`。
    data_dir: PathBuf,
}

/// 搜索框停止输入多久后发起搜索（毫秒）。
const SEARCH_DEBOUNCE_MS: u64 = 300;

impl UiState {
    pub fn new(model: Model, bridge: Bridge) -> UiState {
        UiState {
            model: Rc::new(RefCell::new(model)),
            rev: signal(0u64),
            bridge,
            pending_open: Rc::new(RefCell::new(None)),
            search_text: signal(String::new()),
            forward: signal((0u64, Vec::new())),
            paste: signal(None),
            bottom_epoch: signal(0u64),
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
        self.rev.update(|v| *v += 1);
    }

    /// 周期调用：搜索框文本稳定后发起搜索。
    pub fn tick_search(&self) {
        let now = self.started.elapsed().as_millis() as u64;
        let fired = self
            .debouncer
            .borrow_mut()
            .feed(&self.search_text.get(), now);
        if let Some(q) = fired {
            self.act(|m| m.search(q));
        }
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
        self.derive(|m| matches!(m.conn, ConnState::Disconnected(_)))
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
}
