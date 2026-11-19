//! Existing CSS spinner behavior, including the OS reduced-motion preference.
use gpui::{
    Animation, AnimationExt, AnyElement, ElementId, IntoElement, Transformation, percentage,
};
use std::time::Duration;

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
    let icon = crate::icon("loader", size, color);
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
