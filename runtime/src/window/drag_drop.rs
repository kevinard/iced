//! Optional native drag transport. No filesystem or application policy lives here.
use crate::futures::futures::{channel::mpsc, future::BoxFuture};
use crate::{
    core::window::Id,
    task::{self, Task},
};
use std::{
    any::Any,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A single action selected for a native drop.
pub enum Action {
    /// The destination copies the referenced data.
    Copy,
    /// The destination moves the referenced data; source cleanup is not implied.
    Move,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// Supported native actions. An empty set rejects negotiation.
pub struct Actions {
    /// Copy is available.
    pub copy: bool,
    /// Move is available.
    pub move_files: bool,
}

#[derive(Clone)]
/// Prepared native payload; Debug shows its size without logging its bytes.
pub struct Content {
    /// The advertised MIME type.
    pub mime: String,
    /// Immutable bytes served on a background pipe worker.
    pub bytes: Arc<[u8]>,
}
impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Drag content")
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Debug)]
/// Current state of an incoming offer. It grants no filesystem authority.
pub struct OfferSnapshot {
    /// Identity within the native connection, stable through this offer's lifetime.
    pub id: u64,
    /// Surface-local logical coordinates. Divide by application scale for widgets.
    pub position: (f64, f64),
    /// Actions advertised by the source.
    pub actions: Actions,
    /// Latest compositor choice, which must be honored once dropped.
    pub selected: Option<Action>,
    /// Drop has occurred; this remains true through a later Leave.
    pub dropped: bool,
    /// The pointer has left the receiving surface.
    pub left: bool,
    /// The offer has ended or was explicitly finished.
    pub finished: bool,
    /// Actual source on this connection and seat, independently of public MIME data.
    pub local_source: Option<u64>,
}
#[derive(Clone, Debug)]
/// Latched outgoing-source lifecycle, observable after its origin closes.
pub struct SourceSnapshot {
    /// Identity within the native connection.
    pub id: u64,
    /// Latest negotiated native action.
    pub selected: Option<Action>,
    /// The implicit grab ended in a drop, while completion may still be pending.
    pub dropped: bool,
    /// The transport ended locally or received native completion/cancellation.
    pub finished: bool,
    /// The source ended without a successful native completion.
    pub cancelled: bool,
    /// A payload worker failure, if any.
    pub error: Option<String>,
}

/// Platform adapters implement only native transport and bounded byte I/O.
pub trait Driver: Send + Sync {
    /// Retained offers, including dropped offers still awaiting completion.
    fn offers(&self) -> Vec<Offer>;
    /// Consume one origin record for each client left-button press, in input
    /// dispatch order, even when that press does not start a drag.
    fn capture_origin(&self) -> Option<Origin>;
    /// Start from the origin's current implicit grab; return an explicit refusal.
    fn start(
        &self,
        origin: Origin,
        contents: Vec<Content>,
        actions: Actions,
    ) -> Result<Source, String>;
}

/// Exact native input capability, invalidated by release or a newer press.
/// Capture before asynchronous preparation; it cannot borrow a later gesture.
#[derive(Clone)]
pub struct Origin(Arc<dyn Any + Send + Sync>);
impl Origin {
    /// Construct a capability from a platform adapter's opaque origin.
    pub fn new(value: impl Any + Send + Sync) -> Self {
        Self(Arc::new(value))
    }
    /// Inspect an adapter's own origin representation without exposing native handles.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}
impl fmt::Debug for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Native drag origin")
    }
}
/// Platform implementation of one negotiated incoming offer.
pub trait OfferDriver: Send + Sync {
    /// Fetch latched native state without consuming an event.
    fn snapshot(&self) -> OfferSnapshot;
    /// Inspect advertised formats without starting a transfer.
    fn has_format(&self, mime: &str) -> bool;
    /// Set supported/preferred actions before release; None rejects the format.
    fn accept(
        &self,
        mime: Option<String>,
        actions: Actions,
        preferred: Option<Action>,
    ) -> Result<(), String>;
    /// Read bounded bytes off dispatch; dropping the future cancels its worker.
    fn read(
        &self,
        mime: String,
        limit: usize,
        deadline: Duration,
    ) -> BoxFuture<'static, Result<Vec<u8>, String>>;
    /// Finish a claimed drop, reporting success only after application work commits.
    fn finish(&self, success: bool) -> Result<(), String>;
}
/// Platform implementation of an outgoing source; it never deletes referenced files.
pub trait SourceDriver: Send + Sync {
    /// Inspect current lifecycle and payload failures.
    fn snapshot(&self) -> SourceSnapshot;
    /// Stop serving and request native cancellation, independently of accepted work.
    /// Local termination is not an acknowledgement from a foreign destination.
    fn cancel(&self);
}

/// Offers retain their native identity and final action across UI rebuilding.
#[derive(Clone)]
pub struct Offer(Arc<dyn OfferDriver>);
impl Offer {
    /// Construct an offer from a platform adapter.
    pub fn new(driver: impl OfferDriver + 'static) -> Self {
        Self(Arc::new(driver))
    }
    /// Fetch the current native lifecycle and final action.
    pub fn snapshot(&self) -> OfferSnapshot {
        self.0.snapshot()
    }
    /// Check whether the source advertised this MIME type.
    pub fn has_format(&self, mime: &str) -> bool {
        self.0.has_format(mime)
    }
    /// Negotiate hover acceptance. A dropped offer can no longer change actions.
    pub fn accept(
        &self,
        mime: Option<String>,
        actions: Actions,
        preferred: Option<Action>,
    ) -> Result<(), String> {
        self.0.accept(mime, actions, preferred)
    }
    /// Read bounded bytes on a worker. Dropping this future cancels that read.
    pub fn read(
        &self,
        mime: String,
        limit: usize,
        deadline: Duration,
    ) -> BoxFuture<'static, Result<Vec<u8>, String>> {
        self.0.read(mime, limit, deadline)
    }
    /// End a dropped offer; only report success after all accepted work succeeds.
    pub fn finish(&self, success: bool) -> Result<(), String> {
        self.0.finish(success)
    }
}
impl fmt::Debug for Offer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.snapshot().fmt(f)
    }
}

#[derive(Clone)]
/// Outgoing native offering. Keep it and its endpoint notices until completion.
pub struct Source(Arc<dyn SourceDriver>);
impl Source {
    /// Construct a source from a platform adapter.
    pub fn new(driver: impl SourceDriver + 'static) -> Self {
        Self(Arc::new(driver))
    }
    /// Inspect latched completion, cancellation and payload failures.
    pub fn snapshot(&self) -> SourceSnapshot {
        self.0.snapshot()
    }
    /// Stop offering data. Receivers own their already accepted application work.
    pub fn cancel(&self) {
        self.0.cancel();
    }
}
impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.snapshot().fmt(f)
    }
}

/// Notices coalesce. Inspect every retained offer and owned source after waking;
/// Drop/Finish are latched states, never transient messages that motion can erase.
#[derive(Clone)]
pub struct DragDrop {
    driver: Arc<dyn Driver>,
    notices: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
}
impl DragDrop {
    /// Construct an endpoint and its coalesced notice receiver for a platform adapter.
    pub fn new(driver: impl Driver + 'static, notices: mpsc::Receiver<()>) -> Self {
        Self {
            driver: Arc::new(driver),
            notices: Arc::new(Mutex::new(Some(notices))),
        }
    }
    /// Inspect retained incoming offers without consuming their lifecycle.
    pub fn offers(&self) -> Vec<Offer> {
        self.driver.offers()
    }
    /// Consume the origin record for this client left press, including ordinary
    /// clicks. None is an expired/overflowed record, never a replacement grab.
    pub fn capture_origin(&self) -> Option<Origin> {
        self.driver.capture_origin()
    }
    /// Start on the currently held origin grab using already prepared payloads.
    pub fn start(
        &self,
        origin: Origin,
        contents: Vec<Content>,
        actions: Actions,
    ) -> Result<Source, String> {
        self.driver.start(origin, contents, actions)
    }
    /// Take the endpoint's single coalesced notice stream. Poll state after each wake.
    pub fn take_notices(&self) -> Option<mpsc::Receiver<()>> {
        self.notices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}
impl fmt::Debug for DragDrop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Native drag endpoint")
    }
}

/// None means an unsupported backend, closed window, or duplicate endpoint.
pub fn enable(id: Id) -> Task<Option<DragDrop>> {
    task::oneshot(move |reply| crate::Action::Window(super::Action::EnableDragDrop(id, reply)))
}
