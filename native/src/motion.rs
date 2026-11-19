//! Existing CSS spinner behavior, including the OS reduced-motion preference.
use crate::Astral;
use gpui::Context;
use gpui::{
    Animation, AnimationExt, AnyElement, ElementId, IntoElement, Transformation, percentage,
};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct State {
    pub reduced: bool,
    observing: bool,
}

#[cfg(target_os = "macos")]
pub fn reduced_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(target_os = "windows")]
pub fn reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: windows_sys::core::BOOL = 1;
    // GET only: pvParam receives one BOOL. This never changes an OS preference.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut windows_sys::core::BOOL).cast(),
            0,
        )
    };
    success != 0 && enabled == 0
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn reduced_motion() -> bool {
    false
}

pub fn spinner(id: impl Into<ElementId>, size: f32, color: u32, reduced: bool) -> AnyElement {
    rotating_icon(id, "loader", size, color, reduced)
}

pub fn rotating_icon(
    id: impl Into<ElementId>,
    path: &'static str,
    size: f32,
    color: u32,
    reduced: bool,
) -> AnyElement {
    let icon = crate::icon(path, size, color);
    if reduced {
        icon.into_any_element()
    } else {
        icon.with_animation(
            id,
            Animation::new(Duration::from_secs(1)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
    }
}

impl Astral {
    fn busy_motion_visible(&self) -> bool {
        matches!(self.page, crate::Page::Library | crate::Page::Discover)
            && self.addon_controls_busy()
    }

    pub(crate) fn observe_busy_motion(&mut self, cx: &mut Context<Self>) {
        if self.motion.observing || !self.busy_motion_visible() {
            return;
        }
        self.motion.reduced = reduced_motion();
        self.motion.observing = true;
        let executor = cx.background_executor().clone();
        let mut last_progress = self
            .single_update
            .as_ref()
            .and_then(crate::single_update::State::percentage);
        cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(50)).await;
                let active = this.update(cx, |app, cx| {
                    if !app.busy_motion_visible() {
                        app.motion.observing = false;
                        return false;
                    }
                    let reduced = reduced_motion();
                    let progress = app
                        .single_update
                        .as_ref()
                        .and_then(crate::single_update::State::percentage);
                    if reduced != app.motion.reduced || progress != last_progress {
                        app.motion.reduced = reduced;
                        last_progress = progress;
                        cx.notify();
                    }
                    true
                });
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }
}
