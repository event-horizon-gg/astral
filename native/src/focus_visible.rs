//! Button focus indication follows Chromium's last user focus source, rather
//! than treating every programmatic `focus()` call as keyboard navigation.
use gpui::{App, FocusHandle, IntoElement, KeyDownEvent, Window, WindowId, prelude::*};
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Copy, Default)]
struct Mode {
    mouse_focus: bool,
    keyboard_event: bool,
}

#[derive(Default)]
pub(crate) struct State(RefCell<HashMap<WindowId, Mode>>);
impl gpui::Global for State {}

fn change(window: &mut Window, cx: &App, update: impl FnOnce(&mut Mode)) {
    if let Some(state) = cx.try_global::<State>() {
        update(
            state
                .0
                .borrow_mut()
                .entry(window.window_handle().window_id())
                .or_default(),
        );
        window.refresh();
    }
}

pub(crate) fn indicated(focus: &FocusHandle, window: &Window, cx: &App) -> bool {
    let mode = cx
        .try_global::<State>()
        .and_then(|state| {
            state
                .0
                .borrow()
                .get(&window.window_handle().window_id())
                .copied()
        })
        .unwrap_or_default();
    focus.is_focused(window) && (!mode.mouse_focus || mode.keyboard_event)
}

pub(crate) fn keyboard(window: &mut Window, cx: &App) {
    change(window, cx, |mode| mode.keyboard_event = true);
}

pub(crate) fn key_down(event: &KeyDownEvent, window: &mut Window, cx: &App) {
    let modifiers = event.keystroke.modifiers;
    if !modifiers.control && !modifiers.alt && !modifiers.platform {
        keyboard(window, cx);
    }
}

pub(crate) fn native_tab(window: &mut Window, cx: &App) {
    change(window, cx, |mode| mode.mouse_focus = false);
}

// useDialog lets the browser perform interior Tab moves, and calls focus()
// itself only for boundary wrapping or focus outside the dialog.
pub(crate) fn dialog_tab<'a>(
    controls: impl IntoIterator<Item = &'a FocusHandle>,
    backwards: bool,
    window: &mut Window,
    cx: &App,
) {
    let controls: Vec<_> = controls.into_iter().collect();
    if let Some(index) = controls.iter().position(|focus| focus.is_focused(window))
        && ((!backwards && index + 1 < controls.len()) || (backwards && index > 0))
    {
        native_tab(window, cx);
    }
}

pub(crate) fn pointer_events() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        |_, (), window, _| {
            // Window-level capture also sees deferred/occluding headers and dialogs.
            // This canvas creates no hitbox and does not consume the pointer event.
            window.on_mouse_event(|_: &gpui::MouseDownEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                change(window, cx, |mode| mode.keyboard_event = false);
                let previous = window.focused(cx);
                window.defer(cx, move |window, cx| {
                    let current = window.focused(cx);
                    if current.is_some() && current != previous {
                        change(window, cx, |mode| mode.mouse_focus = true);
                    }
                });
            });
        },
    )
    .absolute()
    .inset_0()
}
