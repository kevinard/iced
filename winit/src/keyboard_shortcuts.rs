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
    struct RecordingDriver(native::Recording);
    fn reason(reason: native::RecordingCancelReason) -> shared::RecordingCancelReason {
        use native::RecordingCancelReason as N;
        use shared::RecordingCancelReason as S;
        match reason {
            N::User => S::User,
            N::ProtectionChanged => S::ProtectionChanged,
            N::KeymapChanged => S::KeymapChanged,
            N::AmbiguousSeat => S::AmbiguousSeat,
            N::Unrepresentable => S::Unrepresentable,
            N::InputLimit => S::InputLimit,
            N::Dropped => S::Dropped,
        }
    }
    impl shared::RecordingDriver for RecordingDriver {
        fn state(&self) -> shared::RecordingState {
            use native::RecordingState as N;
            use shared::RecordingState as S;
            match self.0.state() {
                N::Preparing => S::Preparing,
                N::WaitingForRelease => S::WaitingForRelease,
                N::Ready => S::Ready,
                N::Collecting => S::Collecting,
                N::Recorded(value) => S::Recorded(value),
                N::Cancelled(value) => S::Cancelled(reason(value)),
                N::Closed => S::Closed,
            }
        }
        fn cancel(&self) {
            self.0.cancel();
        }
    }
    impl shared::Driver for Driver {
        fn record(&self) -> Option<shared::Recording> {
            let (changed, receiver) = channel();
            Some(shared::Recording::new(
                RecordingDriver(self.0.record(changed)?),
                receiver,
            ))
        }
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
