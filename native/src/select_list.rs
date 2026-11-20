//! Browser-style list popup content. Native Windows ownership/placement is
//! supplied by the popup host; tests render this on GPUI's private TestPlatform.
use gpui::{Context, FocusHandle, Font, Pixels, ScrollHandle, Window, div, prelude::*, px, rgb};

#[derive(Clone)]
pub(crate) struct Style {
    pub font: Font,
    pub font_size: Pixels,
    pub row_height: Pixels,
    pub padding_start: Pixels,
    pub background: u32,
    pub text: u32,
    pub border: u32,
    pub highlight: u32,
    pub highlight_text: u32,
    pub disabled_text: u32,
}

pub(crate) struct SelectList {
    labels: Vec<String>,
    disabled: Vec<usize>,
    selected: Option<usize>,
    search: crate::type_ahead::TypeAhead,
    style: Style,
    scroll: ScrollHandle,
    pub focus: FocusHandle,
    result: Option<async_channel::Sender<Option<usize>>>,
    activation: Option<gpui::Subscription>,
    activated: bool,
    bar_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<Pixels>>>>,
    drag: Option<f32>,
    owner: Option<gpui::AnyWindowHandle>,
}

impl SelectList {
    pub fn new(
        labels: Vec<String>,
        disabled: Vec<usize>,
        selected: usize,
        style: Style,
        result: async_channel::Sender<Option<usize>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroll = ScrollHandle::new();
        let selected = (selected < labels.len()).then_some(selected);
        if let Some(index) = selected {
            scroll.scroll_to_item(index);
        }
        Self {
            labels,
            disabled,
            selected,
            search: Default::default(),
            style,
            scroll,
            focus: cx.focus_handle(),
            result: Some(result),
            activation: None,
            activated: false,
            bar_bounds: Default::default(),
            drag: None,
            owner: None,
        }
    }

    fn watch_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activated = window.is_window_active();
        self.activation = Some(cx.observe_window_activation(window, |view, window, _| {
            if window.is_window_active() {
                view.activated = true;
            } else if view.activated {
                view.finish(None, window);
            }
        }));
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |view, _| {
                if window.is_window_active() {
                    view.activated = true;
                } else {
                    view.finish(None, window);
                }
            });
        });
    }

    fn scroll_metrics(&self, viewport: f32) -> Option<crate::scrollbar::Metrics> {
        crate::scrollbar::Metrics::new(
            viewport,
            self.labels.len() as f32 * f32::from(self.style.row_height) - viewport,
            f32::from(self.scroll.offset().y),
        )
    }

    fn scrollbar_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.bar_bounds.get() else {
            return;
        };
        let Some(metrics) = self.scroll_metrics(f32::from(bounds.size.height)) else {
            return;
        };
        let y = f32::from(event.position.y - bounds.top());
        if metrics.contains_thumb(y) {
            self.drag = Some(y - metrics.thumb_top);
        } else {
            self.scroll.set_offset(gpui::point(
                px(0.),
                px(metrics.page_offset(y, f32::from(self.scroll.offset().y))),
            ));
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn scrollbar_drag(
        &mut self,
        event: &gpui::MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(grab), Some(bounds)) = (self.drag, self.bar_bounds.get()) else {
            return;
        };
        if let Some(metrics) = self.scroll_metrics(f32::from(bounds.size.height)) {
            self.scroll.set_offset(gpui::point(
                px(0.),
                px(metrics.offset_for_top(f32::from(event.position.y - bounds.top()) - grab)),
            ));
            cx.notify();
        }
    }

    fn enabled(&self, index: usize) -> bool {
        index < self.labels.len() && !self.disabled.contains(&index)
    }

    fn highlight(&mut self, index: usize, keyboard: bool, cx: &mut Context<Self>) {
        if !self.enabled(index) {
            return;
        }
        self.selected = Some(index);
        if keyboard {
            self.scroll.scroll_to_item(index);
        }
        cx.notify();
    }

    fn finish(&mut self, selection: Option<usize>, window: &mut Window) {
        let selection = selection.filter(|index| self.enabled(*index));
        if let Some(result) = self.result.take() {
            let _ = result.try_send(selection);
        }
        window.remove_window();
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" => self.finish(None, window),
            "enter" | "tab" => self.finish(self.selected, window),
            "up" | "down" if key.modifiers.alt => self.finish(None, window),
            "f4" if !key.modifiers.alt && !key.modifiers.control => self.finish(None, window),
            "up" | "down" | "home" | "end" | "pageup" | "pagedown" if !key.modifiers.modified() => {
                let page = ((f32::from(window.viewport_size().height - px(2.))
                    / f32::from(self.style.row_height))
                .floor() as usize)
                    .saturating_sub(1)
                    .max(1);
                let current = self.selected.map(|index| index as isize).unwrap_or(-1);
                let (start, direction, mut remaining) = match key.key.as_str() {
                    "home" => (-1, 1, 1),
                    "end" => (self.labels.len() as isize, -1, 1),
                    "up" => (current, -1, 1),
                    "down" => (current, 1, 1),
                    "pageup" => (current, -1, page as isize),
                    _ => (current, 1, page as isize),
                };
                let mut candidate = start;
                let mut last = None;
                loop {
                    candidate += direction;
                    if candidate < 0 || candidate >= self.labels.len() as isize {
                        break;
                    }
                    remaining -= 1;
                    if self.enabled(candidate as usize) {
                        last = Some(candidate as usize);
                        if remaining <= 0 {
                            break;
                        }
                    }
                }
                if let Some(index) = last {
                    self.highlight(index, true, cx);
                }
            }
            _ => {
                let text = if key.key == "space"
                    && !key.modifiers.control
                    && !key.modifiers.alt
                    && !key.modifiers.platform
                    && !key.modifiers.function
                {
                    Some(" ")
                } else {
                    crate::type_ahead::printable(key)
                };
                if let Some(text) = text {
                    let labels: Vec<_> = self
                        .labels
                        .iter()
                        .enumerate()
                        .map(|(index, label)| {
                            if self.enabled(index) {
                                label.as_str()
                            } else {
                                ""
                            }
                        })
                        .collect();
                    if let Some(index) = self.search.find(
                        text,
                        &labels,
                        self.selected.unwrap_or(0),
                        std::time::Instant::now(),
                    ) {
                        self.highlight(index, true, cx);
                    }
                } else {
                    return;
                }
            }
        }
        if let Some(owner) = self.owner {
            let _ = owner.update(cx, |_, window, cx| {
                crate::focus_visible::key_down(event, window, cx)
            });
        }
        window.prevent_default();
        cx.stop_propagation();
    }
}

impl gpui::Render for SelectList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style.clone();
        let view = cx.entity().downgrade();
        let drag_events = gpui::canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                let view = view.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase.capture() {
                        let _ = view.update(cx, |view, cx| view.scrollbar_drag(event, window, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        let metrics = self.scroll_metrics(f32::from(window.viewport_size().height - px(2.)));
        div()
            .id("select-list")
            .relative()
            .debug_selector(|| "select-list".into())
            .track_focus(&self.focus)
            .child(drag_events)
            .h_full()
            .w_full()
            .flex()
            .border_1()
            .border_color(rgb(style.border))
            .bg(rgb(style.background))
            .text_color(rgb(style.text))
            .font_family(style.font.family.clone())
            .font_weight(style.font.weight)
            .text_size(style.font_size)
            .line_height(style.row_height)
            .on_key_down(cx.listener(Self::key))
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.drag = None),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.drag = None),
            )
            .child(
                div()
                    .id("select-list-scroll")
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(self.labels.iter().enumerate().map(|(index, label)| {
                        let enabled = self.enabled(index);
                        let selected = self.selected == Some(index) && enabled;
                        div()
                            .id(("select-option", index))
                            .debug_selector(move || format!("select-option-{index}"))
                            .h(style.row_height)
                            .flex_shrink_0()
                            .w_full()
                            .pl(style.padding_start)
                            .pr(px(2.))
                            .whitespace_nowrap()
                            .cursor_default()
                            .when(!enabled, |row| row.text_color(rgb(style.disabled_text)))
                            .when(selected, |row| {
                                row.bg(rgb(style.highlight))
                                    .text_color(rgb(style.highlight_text))
                            })
                            .child(label.clone())
                            .when(enabled, |row| {
                                row.on_mouse_move(cx.listener(move |view, _, _, cx| {
                                    if view.drag.is_none() {
                                        view.highlight(index, false, cx);
                                    }
                                }))
                                .on_mouse_up(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |view, _, window, _| {
                                        if view.drag.is_none() {
                                            view.finish(Some(index), window);
                                        }
                                    }),
                                )
                            })
                    })),
            )
            .when_some(metrics, |view, metrics| {
                view.child(
                    div()
                        .id("select-list-scrollbar")
                        .debug_selector(|| "select-list-scrollbar".into())
                        .relative()
                        .w(px(9.))
                        .h_full()
                        .flex_shrink_0()
                        .bg(rgb(0x111516))
                        .cursor_default()
                        .child(crate::remember_bounds(self.bar_bounds.clone()))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::scrollbar_down))
                        .child(
                            div()
                                .absolute()
                                .top(px(metrics.thumb_top))
                                .left_0()
                                .w(px(9.))
                                .h(px(metrics.thumb_height))
                                .rounded(px(5.))
                                .border_2()
                                .border_color(rgb(0x111516))
                                .bg(rgb(0x33473b)),
                        ),
                )
            })
    }
}

/// Runs outside the parent entity update. The caller supplies the platform
/// owner attachment, allowing the same lifecycle to be exercised without HWNDs.
pub(crate) fn open(
    labels: Vec<String>,
    disabled: Vec<usize>,
    selected: usize,
    style: Style,
    options: gpui::WindowOptions,
    owner: gpui::AnyWindowHandle,
    attach: impl FnOnce(&Window) -> Result<(), String> + 'static,
    cx: &gpui::App,
) -> gpui::Task<Result<Option<usize>, String>> {
    let mut app = cx.to_async();
    cx.foreground_executor().spawn(async move {
        let (send, receive) = async_channel::bounded(1);
        let popup = app
            .open_window(options, move |window, cx| {
                let view =
                    cx.new(|cx| SelectList::new(labels, disabled, selected, style, send, cx));
                view.update(cx, |view, _| {
                    view.owner = Some(owner);
                    view.focus.focus(window);
                });
                view
            })
            .map_err(|error| format!("Couldn't open the selection popup. {error}"))?;
        let attached = popup.update(&mut app, |view, window, cx| {
            if let Err(error) = attach(window) {
                window.remove_window();
                return Err(error);
            }
            window.activate_window();
            view.watch_activation(window, cx);
            Ok(())
        });
        match attached {
            Ok(Ok(())) => Ok(receive.recv().await.unwrap_or(None)),
            Ok(Err(error)) => Err(error),
            Err(error) => {
                let _ = popup.update(&mut app, |_, window, _| window.remove_window());
                Err(format!("The selection popup closed while opening. {error}"))
            }
        }
    })
}

/// Work-area placement without overlapping the field. Values are all logical
/// pixels on the owner's monitor, including monitors with negative origins.
pub(crate) fn placement(
    anchor: gpui::Bounds<Pixels>,
    desired: gpui::Size<Pixels>,
    work: gpui::Bounds<Pixels>,
) -> gpui::Bounds<Pixels> {
    let width = desired.width.max(anchor.size.width).min(work.size.width);
    let below = (work.bottom() - anchor.bottom()).max(px(0.));
    let above = (anchor.top() - work.top()).max(px(0.));
    let use_below = below >= desired.height || below >= above;
    let height = desired
        .height
        .min(if use_below { below } else { above })
        .max(px(1.));
    let x = anchor.left().max(work.left()).min(work.right() - width);
    let y = if use_below {
        anchor.bottom()
    } else {
        anchor.top() - height
    };
    gpui::Bounds::new(
        gpui::point(x, y.max(work.top()).min(work.bottom() - height)),
        gpui::size(width, height),
    )
}

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::ui_tests::press;
    use gpui::{Modifiers, TestAppContext, size};

    fn style() -> Style {
        Style {
            font: gpui::font("Arial"),
            font_size: px(12.),
            row_height: px(18.),
            padding_start: px(9.),
            background: 0x191b20,
            text: 0xf2f1ed,
            border: 0x777777,
            highlight: 0x0078d7,
            highlight_text: 0xffffff,
            disabled_text: 0x808080,
        }
    }

    #[gpui::test]
    fn popup_open_failure_returns_error_and_removes_the_child_window(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (_, cx) = cx.add_window_view(|_, cx| {
            crate::Astral::new(
                [None, None],
                Some(directory.path()),
                Some(crate::Client::Retail),
                false,
                cx,
            )
        });
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let saved = result.clone();
        cx.update(|window, cx| {
            let task = open(
                vec!["Alpha".into()],
                vec![],
                0,
                style(),
                gpui::WindowOptions::default(),
                window.window_handle(),
                |_| Err("fixture owner attachment failed".into()),
                cx,
            );
            cx.foreground_executor()
                .spawn(async move {
                    *saved.borrow_mut() = Some(task.await);
                })
                .detach();
        });
        cx.run_until_parked();
        assert_eq!(
            *result.borrow(),
            Some(Err("fixture owner attachment failed".into()))
        );
        assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    }

    #[gpui::test]
    fn popup_focus_loss_cancels_and_removes_the_child_window(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (_, cx) = cx.add_window_view(|_, cx| {
            crate::Astral::new(
                [None, None],
                Some(directory.path()),
                Some(crate::Client::Retail),
                false,
                cx,
            )
        });
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let saved = result.clone();
        cx.update(|window, cx| {
            let task = open(
                vec!["Alpha".into()],
                vec![],
                0,
                style(),
                gpui::WindowOptions::default(),
                window.window_handle(),
                |_| Ok(()),
                cx,
            );
            cx.foreground_executor()
                .spawn(async move {
                    *saved.borrow_mut() = Some(task.await);
                })
                .detach();
        });
        cx.run_until_parked();
        assert!(result.borrow().is_none());
        assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(*result.borrow(), Some(Ok(None)));
        assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    }

    #[test]
    fn popup_placement_respects_work_area_edges_and_negative_monitors() {
        use gpui::{Bounds, point};
        let work = Bounds::new(point(px(0.), px(0.)), size(px(1024.), px(768.)));
        let anchor = Bounds::new(point(px(900.), px(450.)), size(px(120.), px(20.)));
        assert_eq!(
            placement(anchor, size(px(300.), px(100.)), work),
            Bounds::new(point(px(724.), px(470.)), size(px(300.), px(100.)))
        );
        let anchor = Bounds::new(point(px(900.), px(730.)), size(px(120.), px(20.)));
        assert_eq!(
            placement(anchor, size(px(300.), px(100.)), work).origin.y,
            px(630.)
        );
        let anchor = Bounds::new(point(px(400.), px(400.)), size(px(120.), px(20.)));
        assert_eq!(
            placement(anchor, size(px(3000.), px(2000.)), work),
            Bounds::new(point(px(0.), px(0.)), size(px(1024.), px(400.)))
        );
        let work = Bounds::new(point(px(-1920.), px(0.)), size(px(1920.), px(1080.)));
        let anchor = Bounds::new(point(px(-100.), px(50.)), size(px(90.), px(20.)));
        assert_eq!(
            placement(anchor, size(px(400.), px(100.)), work),
            Bounds::new(point(px(-400.), px(70.)), size(px(400.), px(100.)))
        );
    }

    #[gpui::test]
    fn dragging_the_scrollbar_outside_then_releasing_over_a_row_does_not_select(
        cx: &mut TestAppContext,
    ) {
        let (send, receive) = async_channel::bounded(1);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = SelectList::new(
                (0..30).map(|index| format!("Choice {index:02}")).collect(),
                vec![],
                0,
                style(),
                send,
                cx,
            );
            view.focus.focus(window);
            view
        });
        cx.simulate_resize(size(px(180.), px(92.)));
        cx.run_until_parked();
        let track = cx.debug_bounds("select-list-scrollbar").unwrap();
        cx.simulate_event(gpui::MouseDownEvent {
            button: gpui::MouseButton::Left,
            position: gpui::point(track.center().x, track.top() + px(5.)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseMoveEvent {
            position: gpui::point(px(300.), px(300.)),
            pressed_button: Some(gpui::MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| assert!(view.scroll.offset().y <= px(-449.)));
        let row = cx.debug_bounds("select-option-29").unwrap().center();
        cx.simulate_event(gpui::MouseMoveEvent {
            position: row,
            pressed_button: Some(gpui::MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        cx.simulate_event(gpui::MouseUpEvent {
            button: gpui::MouseButton::Left,
            position: row,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        cx.run_until_parked();
        assert!(receive.try_recv().is_err());
        view.read_with(cx, |view, _| {
            assert_eq!(view.selected, Some(0));
            assert!(view.drag.is_none());
        });
    }

    #[gpui::test]
    fn a_long_popup_scrolls_to_the_keyboard_choice_and_track_clicks_do_not_commit(
        cx: &mut TestAppContext,
    ) {
        let (send, receive) = async_channel::bounded(1);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = SelectList::new(
                (0..30).map(|index| format!("Choice {index:02}")).collect(),
                vec![],
                0,
                style(),
                send,
                cx,
            );
            view.focus.focus(window);
            view
        });
        cx.simulate_resize(size(px(180.), px(92.)));
        press(cx, "end");
        view.read_with(cx, |view, _| {
            assert_eq!(view.selected, Some(29));
            assert!(view.scroll.offset().y < px(0.));
        });
        let track = cx.debug_bounds("select-list-scrollbar").unwrap();
        cx.simulate_click(
            gpui::point(track.center().x, track.top() + px(3.)),
            Modifiers::default(),
        );
        assert!(receive.try_recv().is_err());
        press(cx, "end");
        cx.simulate_keystrokes("enter");
        assert_eq!(receive.try_recv().unwrap(), Some(29));
    }

    #[gpui::test]
    fn keyboard_highlight_is_provisional_and_escape_cancels(cx: &mut TestAppContext) {
        let (send, receive) = async_channel::bounded(1);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = SelectList::new(
                vec!["Unavailable".into(), "Alpha".into(), "Beta".into()],
                vec![0],
                0,
                style(),
                send,
                cx,
            );
            view.focus.focus(window);
            view
        });
        cx.simulate_resize(size(px(180.), px(56.)));
        press(cx, "down");
        assert_eq!(view.read_with(cx, |view, _| view.selected), Some(1));
        press(cx, "b");
        assert_eq!(view.read_with(cx, |view, _| view.selected), Some(2));
        assert!(receive.try_recv().is_err());
        // Closing is a key-down action. Its physical key-up belongs to the
        // owner window, because the popup no longer exists.
        cx.simulate_keystrokes("escape");
        assert_eq!(receive.try_recv().unwrap(), None);
    }

    #[gpui::test]
    fn keyboard_commit_uses_the_highlight_and_skips_disabled_rows(cx: &mut TestAppContext) {
        let (send, receive) = async_channel::bounded(1);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let view = SelectList::new(
                vec!["Alpha".into(), "Disabled".into(), "Gamma".into()],
                vec![1],
                0,
                style(),
                send,
                cx,
            );
            view.focus.focus(window);
            view
        });
        cx.simulate_resize(size(px(180.), px(56.)));
        press(cx, "down");
        assert_eq!(view.read_with(cx, |view, _| view.selected), Some(2));
        cx.simulate_keystrokes("tab");
        assert_eq!(receive.try_recv().unwrap(), Some(2));
    }

    #[gpui::test]
    fn pointer_release_commits_enabled_rows_and_disabled_rows_are_inert(cx: &mut TestAppContext) {
        let (send, receive) = async_channel::bounded(1);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = SelectList::new(
                vec!["Unavailable".into(), "Beta".into()],
                vec![0],
                0,
                style(),
                send,
                cx,
            );
            view.focus.focus(window);
            view
        });
        cx.simulate_resize(size(px(180.), px(38.)));
        cx.run_until_parked();
        let disabled = cx.debug_bounds("select-option-0").unwrap().center();
        cx.simulate_click(disabled, Modifiers::default());
        assert!(receive.try_recv().is_err());
        let enabled = cx.debug_bounds("select-option-1").unwrap().center();
        cx.simulate_click(enabled, Modifiers::default());
        assert_eq!(receive.try_recv().unwrap(), Some(1));
    }
}
