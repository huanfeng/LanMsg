// 二次确认对话框：屏蔽请求、移除设备等不可轻易撤销的操作共用。
// 注释中文，标识符英文。

use windui::prelude::*;

use crate::strings;
use crate::style;

/// 一次待确认的操作；`id` 由调用方在确认时取回（请求 id、设备 id…）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Confirm {
    pub id: String,
    pub title: &'static str,
    pub message: String,
    /// 确认按钮文字，与动作同词（“屏蔽”“移除”）。
    pub action: &'static str,
}

/// 待确认状态：`pending` 决定对话框内容，`show` 决定是否可见（ESC 只隐藏、不清状态）。
#[derive(Clone, Copy)]
pub struct ConfirmState {
    pub pending: Signal<Option<Confirm>>,
    pub show: Signal<bool>,
}

impl ConfirmState {
    pub fn new() -> ConfirmState {
        ConfirmState {
            pending: signal(None),
            show: signal(false),
        }
    }

    /// 弹出确认框。
    pub fn open(&self, c: Confirm) {
        self.pending.set(Some(c));
        self.show.set(true);
    }

    fn close(&self) {
        self.show.set(false);
        self.pending.set(None);
    }
}

impl Default for ConfirmState {
    fn default() -> Self {
        ConfirmState::new()
    }
}

/// 对话框宿主，放在页面 stack 的最上层；确认时以 `Confirm::id` 调用 `on_ok`。
pub fn confirm_host(state: ConfirmState, on_ok: impl Fn(String) + 'static) -> Element {
    let on_ok = std::rc::Rc::new(on_ok);
    Element::switcher(state.pending, move |c| match c {
        None => Element::leaf().size(0, 0),
        Some(c) => {
            let body = Element::label(c.message.clone())
                .font_size(style::FONT_BODY)
                .fg_role(Role::TextMuted)
                .width_match();
            let (id, ok) = (c.id.clone(), on_ok.clone());
            let footer = Element::row()
                .width_match()
                .spacing(style::SPACE_SM)
                .child(Element::flex_spacer())
                .child(
                    Element::button(strings::CONFIRM_CANCEL)
                        .outline()
                        .on_click(move |_| state.close()),
                )
                .child(
                    Element::button(c.action)
                        .intent(Intent::Danger)
                        .on_click(move |_| {
                            state.close();
                            ok(id.clone());
                        }),
                );
            Element::dialog_panel(
                state.show,
                c.title,
                360,
                move |_| state.close(),
                body,
                footer,
            )
        }
    })
}
