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
                        .debug_selector(|| "notification".into())
                        .relative()
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
                            blur_radius: px(20.),
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
                        .child(crate::outlined_button(
                            div()
                                .id("dismiss-notification")
                                .debug_selector(|| "dismiss-notification".into())
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
                            &self.utility_focus.notification,
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

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{Client, focus_visible, ui_tests::press};
    use gpui::{Modifiers, TestAppContext, size};

    #[gpui::test]
    fn notice_dismissal_keeps_geometry_and_works_from_pointer_and_keyboard(
        cx: &mut TestAppContext,
    ) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(directory.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.open_notification_fixture(true);
                app
            });
            cx.simulate_resize(size(px(1250.), px(900.)));
            cx.run_until_parked();
            let bounds = cx.debug_bounds("dismiss-notification").unwrap();
            view.update_in(cx, |app, window, _| {
                app.utility_focus.notification.focus(window)
            });
            cx.run_until_parked();
            assert_eq!(bounds, cx.debug_bounds("dismiss-notification").unwrap());
            press(cx, "space");
            assert!(view.read_with(cx, |app, _| app.notifications.current.is_none()));
            view.update_in(cx, |app, window, cx| {
                app.open_notification_fixture(false);
                app.brand_focus.focus(window);
                cx.notify();
            });
            cx.run_until_parked();
            let bounds = cx.debug_bounds("dismiss-notification").unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();
            assert!(view.read_with(cx, |app, _| app.notifications.current.is_none()));
            view.update_in(cx, |app, window, cx| {
                app.open_notification_fixture(false);
                cx.notify();
                assert!(!focus_visible::indicated(
                    &app.utility_focus.notification,
                    window,
                    cx
                ));
            });
            cx.run_until_parked();
            assert_eq!(bounds, cx.debug_bounds("dismiss-notification").unwrap());
        }
    }
}
