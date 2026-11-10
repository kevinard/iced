//! Optional, focus-scoped keyboard shortcut inhibition for ordinary windows.
use crate::futures::futures::{
    channel::mpsc,
    stream::{self, BoxStream},
    StreamExt,
};
use std::{
    fmt,
    sync::{Arc, Mutex},
};

#[path = "keyboard_shortcuts/recording.rs"]
pub mod recording;
pub use recording::{
    CancelReason as RecordingCancelReason, Driver as RecordingDriver, Recording,
    State as RecordingState,
};

/// Observed native permission and focus. A successful request is not Active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No keyboard seat currently focuses this window.
    Unfocused,
    /// A focused seat has not yet acknowledged inhibition.
    Pending,
    /// Every focused keyboard seat acknowledges inhibition.
    Active,
    /// The compositor disabled inhibition on a focused seat.
    Inactive,
    /// The window or native owner retired; this state cannot revive.
    Closed,
}
/// Platform implementation, owned until the final handle retires.
pub trait Driver: Send + Sync {
    /// Read the latest actual state without consuming a notification.
    fn status(&self) -> Status;
    /// Arm an optional source-owned recording attempt under actual protection.
    fn record(&self) -> Option<Recording> {
        None
    }
}
struct Inner {
    driver: Box<dyn Driver>,
    updates: Mutex<Option<mpsc::Receiver<()>>>,
}
/// Keeps a native request alive; window closure revokes retained clones too.
#[derive(Clone)]
pub struct Inhibitor(Arc<Inner>);
impl fmt::Debug for Inhibitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Keyboard shortcut inhibitor")
            .field(&self.status())
            .finish()
    }
}
impl Inhibitor {
    /// Construct from a native adapter and coalesced, bounded wake-ups.
    pub fn new(driver: impl Driver + 'static, updates: mpsc::Receiver<()>) -> Self {
        Self(Arc::new(Inner {
            driver: Box::new(driver),
            updates: Mutex::new(Some(updates)),
        }))
    }
    /// Inspect actual focus and compositor permission.
    pub fn status(&self) -> Status {
        self.0.driver.status()
    }
    /// Arm one native recording attempt. A successful inhibition request alone
    /// does not establish permission; an unavailable or busy source returns None.
    pub fn record(&self) -> Option<Recording> {
        self.0.driver.record()
    }
    /// Take the single change stream, starting with the current state.
    pub fn changes(&self) -> Option<BoxStream<'static, Status>> {
        let updates = self.0.updates.lock().unwrap().take()?;
        Some(
            stream::unfold(Some((self.clone(), updates, true)), |state| async move {
                let (inhibitor, mut updates, initial) = state?;
                if !initial {
                    updates.next().await?;
                }
                let status = inhibitor.status();
                let next = if status == Status::Closed {
                    None
                } else {
                    Some((inhibitor, updates, false))
                };
                Some((status, next))
            })
            .boxed(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::futures::futures::FutureExt;
    struct Fake(Arc<Mutex<Status>>);
    impl Driver for Fake {
        fn status(&self) -> Status {
            *self.0.lock().unwrap()
        }
    }
    #[test]
    fn coalesced_changes_report_latest_state_and_closed_stream_cannot_revive() {
        let state = Arc::new(Mutex::new(Status::Unfocused));
        let (mut send, receive) = mpsc::channel(0);
        let inhibitor = Inhibitor::new(Fake(state.clone()), receive);
        let mut changes = inhibitor.changes().unwrap();
        assert!(inhibitor.changes().is_none());
        assert_eq!(changes.next().now_or_never(), Some(Some(Status::Unfocused)));
        *state.lock().unwrap() = Status::Pending;
        send.try_send(()).unwrap();
        *state.lock().unwrap() = Status::Active;
        let _ = send.try_send(());
        assert_eq!(changes.next().now_or_never(), Some(Some(Status::Active)));
        *state.lock().unwrap() = Status::Closed;
        send.try_send(()).unwrap();
        assert_eq!(changes.next().now_or_never(), Some(Some(Status::Closed)));
        *state.lock().unwrap() = Status::Active;
        let _ = send.try_send(());
        assert_eq!(changes.next().now_or_never(), Some(None));
    }
}
