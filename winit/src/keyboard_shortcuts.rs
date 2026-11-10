//! Optional ordinary-window protocol bridge; notifications are coalesced.
use crate::runtime::window::keyboard_shortcuts as shared;
#[cfg(not(all(target_os = "linux", feature = "wayland")))]
pub(crate) fn enable(_: &winit::window::Window) -> Option<shared::Inhibitor> {
    None
}
#[cfg(all(target_os = "linux", feature = "wayland"))]
pub(crate) fn enable(window: &winit::window::Window) -> Option<shared::Inhibitor> {
    use crate::futures::futures::channel::mpsc;
    use std::sync::{Arc, Mutex};
    use winit::platform::wayland::{keyboard_shortcuts as native, WindowExtWayland};
    fn channel() -> (native::Changed, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel(0);
        let sender = Arc::new(Mutex::new(sender));
        let changed: native::Changed = Arc::new(move || {
            let _ = sender
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .try_send(());
        });
        (changed, receiver)
    }
    let (changed, receiver) = channel();
    let endpoint = window.inhibit_keyboard_shortcuts(changed)?;
    struct Driver(native::KeyboardShortcutsInhibitor);
    impl shared::Driver for Driver {
        fn status(&self) -> shared::Status {
            match self.0.status() {
                native::Status::Unfocused => shared::Status::Unfocused,
                native::Status::Pending => shared::Status::Pending,
                native::Status::Active => shared::Status::Active,
                native::Status::Inactive => shared::Status::Inactive,
                native::Status::Closed => shared::Status::Closed,
            }
        }
    }
    Some(shared::Inhibitor::new(Driver(endpoint), receiver))
}
