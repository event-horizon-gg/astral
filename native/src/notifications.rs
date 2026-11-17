use crate::{Astral, Palette};
use gpui::{Context, Window, div, prelude::*, px, rgb, rgba};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct State {
    sequence: u64,
    current: Option<Notice>,
}

struct Notice {
    sequence: u64,
    message: String,
    error: bool,
}

impl State {
    fn show(&mut self, message: String, error: bool) -> u64 {
        self.sequence = self.sequence.wrapping_add(1);
        self.current = Some(Notice {
            sequence: self.sequence,
            message,
            error,
        });
        self.sequence
    }

    fn expire(&mut self, sequence: u64) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|notice| notice.sequence == sequence)
        {
            self.current = None;
            true
        } else {
            false
        }
    }
}

impl Astral {
    pub(crate) fn open_notification_fixture(&mut self, error: bool) {
        if self.fixture {
            // Keep only comparison fixtures visible until dismissed; real notices expire.
            self.notifications.show(
                if error {
                    "The cloud backup changed. Sync again to review it."
                } else {
                    "Cloud backup restored. Local backup saved."
                }
                .into(),
                error,
            );
        }
    }

    pub(crate) fn show_notification(
        &mut self,
        message: impl Into<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        let sequence = self.notifications.show(message.into(), error);
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(6500)).await;
            let _ = this.update(cx, |app, cx| {
                if app.notifications.expire(sequence) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn notification_view(
        &self,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<gpui::Div> {
        let notice = self.notifications.current.as_ref()?;
        let error = notice.error;
        let error_color = if self.preferences.light_theme {
            0xb42318
        } else {
            0xf18e8e
        };
        let color = if error { error_color } else { palette.accent };
        Some(
            div()
                .absolute()
                .bottom(px(26.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("notification")
                        .relative()
                        .overflow_hidden()
                        .occlude()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .max_w((window.viewport_size().width - px(40.)).min(px(640.)))
                        .min_w_0()
                        .py(px(13.))
                        .px(px(16.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette.line)
                        .when(error, |toast| toast.border_l(px(3.)))
                        .bg(rgb(palette.panel))
                        .text_color(rgb(palette.text))
                        .text_size(px(13.))
                        .shadow(vec![gpui::BoxShadow {
                            color: rgba(0x00000033).into(),
                            offset: gpui::point(px(0.), px(10.)),
                            blur_radius: px(40.),
                            spread_radius: px(0.),
                        }])
                        .when(error, |toast| {
                            toast.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(3.))
                                    .bg(rgb(error_color)),
                            )
                        })
                        .child(crate::icon(
                            if error { "alert" } else { "check-circle" },
                            17.,
                            color,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .line_height(px(20.8))
                                .child(notice.message.clone()),
                        )
                        .child(crate::keyboard_button(
                            div()
                                .id("dismiss-notification")
                                .size(px(30.))
                                .ml(px(7.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(gpui::transparent_black())
                                .cursor_pointer()
                                .hover(move |style| style.bg(palette.hover))
                                .child(crate::icon("close", 14., palette.muted)),
                            palette,
                            cx.listener(|app, _: &(), _, cx| {
                                app.notifications.current = None;
                                cx.notify();
                            }),
                        )),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_old_timeout_cannot_dismiss_a_new_notification() {
        let mut state = State::default();
        let first = state.show("Saved".into(), false);
        let second = state.show("Restore failed".into(), true);
        assert!(!state.expire(first));
        assert_eq!(state.current.as_ref().unwrap().message, "Restore failed");
        assert!(state.current.as_ref().unwrap().error);
        assert!(state.expire(second));
        assert!(state.current.is_none());
    }

    #[test]
    fn dismissal_remains_dismissed_when_the_timeout_arrives() {
        let mut state = State::default();
        let sequence = state.show("Saved".into(), false);
        state.current = None;
        assert!(!state.expire(sequence));
    }
}
