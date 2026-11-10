//! One source-owned keyboard chord, without granting application authority.
use super::{mpsc, stream, Arc, BoxStream, Mutex, StreamExt};
use std::fmt;

/// Why an attempt ended without replacing the previous trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// Explicit cancellation, including plain Escape.
    User,
    /// Focus, permission or keyboard-seat continuity changed.
    ProtectionChanged,
    /// The keyboard map or layout changed.
    KeymapChanged,
    /// Input from different seats would mix a chord.
    AmbiguousSeat,
    /// The symbol or effective modifiers cannot be expressed as a trigger.
    Unrepresentable,
    /// Native input or attempt counters exceeded their bounds.
    InputLimit,
    /// The final owning handle was dropped.
    Dropped,
}
/// The latest state of one native recording attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Draining input queued before the explicit arm request.
    Preparing,
    /// Release keys which were already held.
    WaitingForRelease,
    /// Ready for a fresh key press.
    Ready,
    /// Collecting a chord; release its keys to finish.
    Collecting,
    /// A canonical XDG trigger, still subject to separate explicit approval.
    Recorded(String),
    /// The old trigger remains unchanged.
    Cancelled(CancelReason),
    /// The native inhibitor, window or event loop retired.
    Closed,
}
impl State {
    /// Whether this attempt can no longer accept input.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Recorded(_) | Self::Cancelled(_) | Self::Closed)
    }
}
/// Native implementation; it must revoke at source before notifying observers.
pub trait Driver: Send + Sync {
    /// Read the latest coherent source-owned state.
    fn state(&self) -> State;
    /// Revoke the attempt even while other handles or the stream remain alive.
    fn cancel(&self);
}
struct Inner {
    driver: Box<dyn Driver>,
    updates: Mutex<Option<mpsc::Receiver<()>>>,
}
/// Keeps one attempt alive. Window/inhibitor retirement revokes all clones.
#[derive(Clone)]
pub struct Recording(Arc<Inner>);
impl fmt::Debug for Recording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Shortcut recording")
            .field(&self.state())
            .finish()
    }
}
impl Recording {
    /// Construct from an actual native owner and bounded coalesced wake-ups.
    pub fn new(driver: impl Driver + 'static, updates: mpsc::Receiver<()>) -> Self {
        Self(Arc::new(Inner {
            driver: Box::new(driver),
            updates: Mutex::new(Some(updates)),
        }))
    }
    /// Read the current state without consuming its notification.
    pub fn state(&self) -> State {
        self.0.driver.state()
    }
    /// Explicitly stop recording, including when a stream retains another clone.
    pub fn cancel(&self) {
        self.0.driver.cancel();
    }
    /// Take one bounded change stream. It owns a clone until its terminal result;
    /// use `cancel` when abandoning the attempt before that result arrives.
    pub fn changes(&self) -> Option<BoxStream<'static, State>> {
        let updates = self.0.updates.lock().unwrap().take()?;
        Some(
            stream::unfold(Some((self.clone(), updates, true)), |state| async move {
                let (recording, mut updates, initial) = state?;
                if !initial {
                    updates.next().await?;
                }
                let state = recording.state();
                let next = if state.is_terminal() {
                    None
                } else {
                    Some((recording, updates, false))
                };
                Some((state, next))
            })
            .boxed(),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::futures::futures::FutureExt;
    struct Fake {
        state: Arc<Mutex<State>>,
        wake: Arc<Mutex<mpsc::Sender<()>>>,
    }
    impl Driver for Fake {
        fn state(&self) -> State {
            self.state.lock().unwrap().clone()
        }
        fn cancel(&self) {
            *self.state.lock().unwrap() = State::Cancelled(CancelReason::User);
            let _ = self.wake.lock().unwrap().try_send(());
        }
    }
    fn fake() -> (Recording, Arc<Mutex<State>>, Arc<Mutex<mpsc::Sender<()>>>) {
        let state = Arc::new(Mutex::new(State::Preparing));
        let (send, receive) = mpsc::channel(0);
        let wake = Arc::new(Mutex::new(send));
        (
            Recording::new(
                Fake {
                    state: state.clone(),
                    wake: wake.clone(),
                },
                receive,
            ),
            state,
            wake,
        )
    }
    #[test]
    fn cancel_revokes_while_a_stream_and_retained_clone_own_the_attempt() {
        let (recording, _, _) = fake();
        let retained = recording.clone();
        let mut changes = recording.changes().unwrap();
        assert!(retained.changes().is_none());
        assert_eq!(changes.next().now_or_never(), Some(Some(State::Preparing)));
        recording.cancel();
        drop(recording);
        assert_eq!(retained.state(), State::Cancelled(CancelReason::User));
        assert_eq!(
            changes.next().now_or_never(),
            Some(Some(State::Cancelled(CancelReason::User)))
        );
        assert_eq!(changes.next().now_or_never(), Some(None));
    }
    #[test]
    fn coalesced_terminal_results_are_readable_and_the_stream_never_revives() {
        let (recording, state, wake) = fake();
        let mut changes = recording.changes().unwrap();
        assert_eq!(changes.next().now_or_never(), Some(Some(State::Preparing)));
        *state.lock().unwrap() = State::Ready;
        wake.lock().unwrap().try_send(()).unwrap();
        *state.lock().unwrap() = State::Recorded("CTRL+a".into());
        let _ = wake.lock().unwrap().try_send(());
        assert_eq!(
            changes.next().now_or_never(),
            Some(Some(State::Recorded("CTRL+a".into())))
        );
        *state.lock().unwrap() = State::Ready;
        let _ = wake.lock().unwrap().try_send(());
        assert_eq!(changes.next().now_or_never(), Some(None));
    }
}
