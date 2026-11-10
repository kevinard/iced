//! Ordinary-window transport adapter; byte I/O is bounded and off dispatch.
use crate::runtime::window::drag_drop as exchange;

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
pub(crate) fn enable(_: &winit::window::Window) -> Option<exchange::DragDrop> {
    None
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub(crate) fn enable(window: &winit::window::Window) -> Option<exchange::DragDrop> {
    wayland::enable(window)
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland {
    use super::*;
    use crate::futures::futures::{
        channel::{mpsc, oneshot},
        future::BoxFuture,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use winit::platform::wayland::{WindowExtWayland, drag_drop as native};

    fn actions(actions: exchange::Actions) -> native::DragActions {
        native::DragActions {
            copy: actions.copy,
            move_files: actions.move_files,
        }
    }
    fn action(action: exchange::Action) -> native::DragAction {
        match action {
            exchange::Action::Copy => native::DragAction::Copy,
            exchange::Action::Move => native::DragAction::Move,
        }
    }
    fn selected(action: native::DragAction) -> exchange::Action {
        match action {
            native::DragAction::Copy => exchange::Action::Copy,
            native::DragAction::Move => exchange::Action::Move,
        }
    }

    pub(super) fn enable(window: &winit::window::Window) -> Option<exchange::DragDrop> {
        let (sender, receiver) = mpsc::channel(0);
        let sender = Arc::new(Mutex::new(sender));
        let changed: native::Changed = Arc::new(move || {
            let _ = sender
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .try_send(());
        });
        let endpoint = window.drag_drop(changed.clone())?;
        Some(exchange::DragDrop::new(
            Driver { endpoint, changed },
            receiver,
        ))
    }

    struct Driver {
        endpoint: native::DragDrop,
        changed: native::Changed,
    }
    impl exchange::Driver for Driver {
        fn capture_origin(&self) -> Option<exchange::Origin> {
            self.endpoint.capture_origin().map(exchange::Origin::new)
        }
        fn offers(&self) -> Vec<exchange::Offer> {
            let mut offers = self.endpoint.offers();
            offers.sort_by_key(|offer| offer.snapshot().id);
            offers
                .into_iter()
                .map(|offer| exchange::Offer::new(Offer(offer)))
                .collect()
        }

        fn start(
            &self,
            origin: exchange::Origin,
            contents: Vec<exchange::Content>,
            supported: exchange::Actions,
        ) -> Result<exchange::Source, String> {
            if contents.is_empty()
                || contents.len() > 32
                || contents
                    .iter()
                    .try_fold(0usize, |size, content| {
                        size.checked_add(content.bytes.len())
                    })
                    .is_none_or(|size| size > 16 * 1024 * 1024)
            {
                return Err("The dragged data is too large or empty".into());
            }
            let formats = contents
                .iter()
                .map(|content| content.mime.clone())
                .collect();
            let shared = Arc::new(Sending {
                source: Mutex::new(None),
                writes: Mutex::new(Vec::new()),
                error: Mutex::new(None),
            });
            let sent = shared.clone();
            let changed = self.changed.clone();
            let send: native::SendData = Arc::new(move |mime, fd| {
                let Some(content) = contents.iter().find(|content| content.mime == mime) else {
                    return;
                };
                let result_state = sent.clone();
                let changed = changed.clone();
                let result = native::write_pipe(
                    fd,
                    content.bytes.clone(),
                    Duration::from_secs(5),
                    move |result| {
                        if let Err(error) = result {
                            *result_state.error.lock().unwrap() = Some(error.to_string());
                            let source = result_state.source.lock().unwrap().clone();
                            if let Some(source) = source {
                                source.cancel();
                            }
                            changed();
                        }
                    },
                );
                match result {
                    Ok(task) => {
                        let mut writes = sent.writes.lock().unwrap();
                        writes.retain(|write| !write.is_finished());
                        writes.push(task);
                    }
                    Err(error) => {
                        *sent.error.lock().unwrap() = Some(error.to_string());
                        let source = sent.source.lock().unwrap().clone();
                        if let Some(source) = source {
                            source.cancel();
                        }
                    }
                }
            });
            let origin = origin
                .downcast_ref::<native::Origin>()
                .ok_or_else(|| "The drag origin belongs to another backend".to_owned())?;
            let source = self
                .endpoint
                .start(origin, formats, actions(supported), send)
                .map_err(|e| e.to_string())?;
            *shared.source.lock().unwrap() = Some(source.clone());
            Ok(exchange::Source::new(Source { source, shared }))
        }
    }

    struct Sending {
        source: Mutex<Option<native::Source>>,
        writes: Mutex<Vec<native::PipeTransfer>>,
        error: Mutex<Option<String>>,
    }
    impl Drop for Sending {
        fn drop(&mut self) {
            self.writes.get_mut().unwrap().clear();
        }
    }
    struct Source {
        source: native::Source,
        shared: Arc<Sending>,
    }
    impl exchange::SourceDriver for Source {
        fn snapshot(&self) -> exchange::SourceSnapshot {
            let state = self.source.snapshot();
            if state.finished {
                self.shared.writes.lock().unwrap().clear();
            }
            exchange::SourceSnapshot {
                id: state.id,
                selected: state.selected.map(selected),
                dropped: state.dropped,
                finished: state.finished,
                cancelled: state.cancelled,
                error: self.shared.error.lock().unwrap().clone(),
            }
        }
        fn cancel(&self) {
            self.source.cancel();
            self.shared.writes.lock().unwrap().clear();
        }
    }
    impl Drop for Source {
        fn drop(&mut self) {
            self.source.cancel();
        }
    }

    struct Offer(native::Offer);
    impl exchange::OfferDriver for Offer {
        fn snapshot(&self) -> exchange::OfferSnapshot {
            let state = self.0.snapshot();
            exchange::OfferSnapshot {
                id: state.id,
                position: state.position,
                actions: exchange::Actions {
                    copy: state.actions.copy,
                    move_files: state.actions.move_files,
                },
                selected: state.selected.map(selected),
                dropped: state.dropped,
                left: state.left,
                finished: state.finished,
                local_source: state.local_source,
            }
        }
        fn has_format(&self, mime: &str) -> bool {
            self.0.has_format(mime)
        }
        fn accept(
            &self,
            mime: Option<String>,
            supported: exchange::Actions,
            preferred: Option<exchange::Action>,
        ) -> Result<(), String> {
            self.0
                .accept(mime, actions(supported), preferred.map(action))
                .map_err(|e| e.to_string())
        }
        fn read(
            &self,
            mime: String,
            limit: usize,
            deadline: Duration,
        ) -> BoxFuture<'static, Result<Vec<u8>, String>> {
            let offer = self.0.clone();
            Box::pin(async move {
                let (send, receive) = oneshot::channel();
                let fd = offer.receive(mime).map_err(|e| e.to_string())?;
                let task = native::read_pipe(fd, limit, deadline, move |result| {
                    let _ = send.send(result.map_err(|e| e.to_string()));
                })
                .map_err(|e| e.to_string())?;
                let result = receive
                    .await
                    .map_err(|_| "The drag data reader stopped".to_owned())?;
                drop(task);
                result
            })
        }
        fn finish(&self, success: bool) -> Result<(), String> {
            self.0.finish(success).map_err(|e| e.to_string())
        }
    }
}
