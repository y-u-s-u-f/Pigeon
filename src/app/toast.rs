//! The note at the bottom of the window (ME-056): what just happened, an
//! Undo for it, or what failed and how to fix it.

use std::{rc::Rc, time::Duration};

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
    px,
};

use super::PigeonApp;
use crate::{actions::Undo, assets::Icon, backend::UNDO_WINDOW, theme::Theme, ui, ui::style::*};

/// How long a toast stays: as long as the backend holds an undoable op back.
const TOAST_DURATION: Duration = UNDO_WINDOW;
/// While the pointer rests on a toast, how often it checks again.
const HOVER_RECHECK: Duration = Duration::from_millis(500);
/// The widest a toast gets; longer messages wrap.
const TOAST_MAX: gpui::Pixels = px(560.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// Something happened: a check.
    Done,
    /// Something to know, nothing done ("Already signed in").
    Info,
    /// Under way ("Sending…").
    Working,
    /// Something failed: stays until dismissed.
    Error,
}

/// A toast button's fix.
type Fix = Rc<dyn Fn(&mut PigeonApp, &mut Window, &mut Context<PigeonApp>)>;

/// What a toast's button does.
#[derive(Clone)]
pub enum ToastAction {
    /// Undo the stack entry with this id, while it's still the newest.
    Undo(u64),
    /// A fix-it ("Try Again", "Open Drafts").
    Run { label: SharedString, run: Fix },
}

impl ToastAction {
    pub fn run(
        label: impl Into<SharedString>,
        run: impl Fn(&mut PigeonApp, &mut Window, &mut Context<PigeonApp>) + 'static,
    ) -> Self {
        ToastAction::Run {
            label: label.into(),
            run: Rc::new(run),
        }
    }
}

pub struct Toast {
    pub kind: ToastKind,
    pub message: SharedString,
    pub action: Option<ToastAction>,
    hovered: bool,
    /// Counts toasts shown back to back, so each one animates in.
    serial: usize,
    _timer: Option<Task<()>>,
}

impl PigeonApp {
    /// Show a toast, replacing the one showing. Undo entries live on the
    /// stack, so a new toast never loses one.
    pub(crate) fn show_toast(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        action: Option<ToastAction>,
        cx: &mut Context<Self>,
    ) {
        let timer = (kind != ToastKind::Error).then(|| {
            cx.spawn(async |this, cx| {
                cx.background_executor().timer(TOAST_DURATION).await;
                // Held while the pointer is on it.
                while this
                    .read_with(cx, |this, _| this.toast.as_ref().is_some_and(|t| t.hovered))
                    .unwrap_or(false)
                {
                    cx.background_executor().timer(HOVER_RECHECK).await;
                }
                this.update(cx, |this, cx| {
                    this.toast = None;
                    cx.notify();
                })
                .ok();
            })
        });
        let serial = self.toast.as_ref().map_or(0, |t| t.serial + 1);
        self.toast = Some(Toast {
            kind,
            message: message.into(),
            action,
            hovered: false,
            serial,
            _timer: timer,
        });
        cx.notify();
    }

    /// A failure (STA-2, MA-050): in plain words, with a fix if there is
    /// one. It stays until dismissed.
    pub(crate) fn show_error_toast(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.show_toast(ToastKind::Error, message, None, cx);
    }

    pub(crate) fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        if self.toast.take().is_some() {
            cx.notify();
        }
    }

    /// Escape takes an error toast away first, unless an overlay is up.
    pub(super) fn dismiss_error_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let error = self
            .toast
            .as_ref()
            .is_some_and(|t| t.kind == ToastKind::Error);
        let dismiss = error && !self.overlay_open();
        if dismiss {
            self.dismiss_toast(cx);
        }
        dismiss
    }

    pub(super) fn render_toast(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let toast = self.toast.as_ref()?;
        let theme = Theme::of(cx);
        let (glyph, color) = match toast.kind {
            ToastKind::Done => (Some(Icon::Check), theme.text_success),
            ToastKind::Info => (None, theme.text_muted),
            ToastKind::Working => (Some(Icon::Clock), theme.text_muted),
            ToastKind::Error => (Some(Icon::OctagonAlert), theme.text_danger),
        };
        // An Undo only while its entry is the newest.
        let action = toast.action.clone().filter(|a| match a {
            ToastAction::Undo(id) => self.undo_stack.newest_id() == Some(*id),
            ToastAction::Run { .. } => true,
        });
        let button = action.map(|action| {
            let (label, keycap) = match &action {
                ToastAction::Undo(_) => (SharedString::from("Undo"), Some("cmd-z")),
                ToastAction::Run { label, .. } => (label.clone(), None),
            };
            focusable_control(div().id("toast-action"), theme)
                .h(CONTROL_MD)
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.accent)
                .hover(|s| s.bg(theme.hover_bg))
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .child(label)
                .children(keycap.map(|k| ui::keys_caps(k, theme)))
                .on_click(cx.listener(move |this, _, window, cx| match &action {
                    ToastAction::Undo(_) => window.dispatch_action(Box::new(Undo), cx),
                    ToastAction::Run { run, .. } => {
                        this.dismiss_toast(cx);
                        run(this, window, cx);
                    }
                }))
        });
        let has_button = button.is_some();
        let error = toast.kind == ToastKind::Error;
        Some(
            div()
                .absolute()
                .bottom(SPACE_XL)
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(enter(
                    ("toast-enter", toast.serial),
                    div()
                        .id("toast")
                        .occlude()
                        .min_h(CONTROL_LG)
                        .max_w(TOAST_MAX)
                        .py(SPACE_XS)
                        .pl(SPACE_LG)
                        .pr(SPACE_SM)
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .rounded(RADIUS_MD)
                        .bg(theme.overlay_bg)
                        .border_1()
                        .border_color(theme.overlay_border)
                        .shadow(ui::overlay_shadow(theme))
                        .text_size(TEXT_BASE)
                        .role(if error {
                            gpui::Role::Alert
                        } else {
                            gpui::Role::Status
                        })
                        .aria_label(toast.message.clone())
                        .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                            if let Some(toast) = &mut this.toast {
                                toast.hovered = *hovered;
                            }
                        }))
                        .children(glyph.map(|g| ui::icon(g, ICON_MD, color)))
                        .child(
                            ui::live_text(
                                "toast-message",
                                toast.message.clone(),
                                if error {
                                    gpui::accesskit::Live::Assertive
                                } else {
                                    gpui::accesskit::Live::Polite
                                },
                            )
                            .min_w_0(),
                        )
                        .children(button)
                        .when(error, |this| {
                            this.child(
                                focusable_control(div().id("toast-dismiss"), theme)
                                    .size(CONTROL_SM)
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .rounded(RADIUS_SM)
                                    .hover(|s| s.bg(theme.hover_bg))
                                    .role(gpui::Role::Button)
                                    .aria_label("Dismiss")
                                    .child(ui::icon(Icon::X, ICON_SM, theme.text_muted))
                                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_toast(cx))),
                            )
                        })
                        .when(!has_button && !error, |this| this.pr(SPACE_LG)),
                ))
                .into_any_element(),
        )
    }
}

/// A raw error in plain words with a next step (MA-050). The raw text goes
/// to the log.
pub(crate) fn plain_error(what: &str, error: &dyn std::fmt::Display) -> String {
    let raw = error.to_string();
    log::warn!("{what}: {raw}");
    let lower = raw.to_lowercase();
    let why = if lower.contains("permission denied") || lower.contains("os error 13") {
        "Pigeon isn't allowed to write there. Choose another folder."
    } else if lower.contains("no space left") || lower.contains("os error 28") {
        "The disk is full. Free some space and try again."
    } else if lower.contains("read-only file system") || lower.contains("os error 30") {
        "That disk is read-only. Choose another folder."
    } else if lower.contains("couldn't reach")
        || lower.contains("network")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("dns")
    {
        "Couldn't reach Gmail. Check your connection and try again."
    } else if lower.contains("not found") || lower.contains("404") {
        "It's no longer on the server."
    } else {
        "Try again in a moment."
    };
    format!("{what}. {why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_error_maps_common_causes() {
        let denied = std::io::Error::from_raw_os_error(13);
        assert_eq!(
            plain_error("Couldn't save “a.pdf”", &denied),
            "Couldn't save “a.pdf”. Pigeon isn't allowed to write there. Choose another folder."
        );
        assert!(
            plain_error("Couldn't get “a.pdf”", &"network error: reset").contains("connection")
        );
        assert!(plain_error("Couldn't get it", &"weird").ends_with("Try again in a moment."));
    }
}
