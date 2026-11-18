//! The tray and Windows menu bar share muda's single process-wide callback.
//! Fan events out so installing either surface cannot replace the other.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn subscribe() -> async_channel::Receiver<String> {
    use std::sync::{Mutex, Once, OnceLock};
    use tray_icon::menu::MenuEvent;
    static INIT: Once = Once::new();
    static SUBSCRIBERS: OnceLock<Mutex<Vec<async_channel::Sender<String>>>> = OnceLock::new();
    let (sender, receiver) = async_channel::unbounded();
    SUBSCRIBERS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .push(sender);
    INIT.call_once(|| {
        MenuEvent::set_event_handler(Some(|event: MenuEvent| {
            let mut subscribers = SUBSCRIBERS.get().unwrap().lock().unwrap();
            broadcast(&mut subscribers, event.id.as_ref());
        }));
    });
    receiver
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn broadcast(subscribers: &mut Vec<async_channel::Sender<String>>, id: &str) {
    subscribers.retain(|sender| sender.try_send(id.to_owned()).is_ok());
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn tray_and_window_menu_subscribers_both_receive_events_and_closed_receivers_are_removed() {
        // No native callback, window, tray or application is initialized here.
        let (tray, tray_events) = async_channel::unbounded();
        let (window, window_events) = async_channel::unbounded();
        let (closed, closed_events) = async_channel::unbounded();
        drop(closed_events);
        let mut subscribers = vec![tray, window, closed];
        for id in ["astral-open", "astral-app-menu-1", "astral-quit"] {
            broadcast(&mut subscribers, id);
            assert_eq!(tray_events.try_recv().unwrap(), id);
            assert_eq!(window_events.try_recv().unwrap(), id);
            assert!(tray_events.try_recv().is_err());
            assert!(window_events.try_recv().is_err());
        }
        assert_eq!(subscribers.len(), 2);
    }
}
