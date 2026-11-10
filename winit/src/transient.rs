//! Native auxiliary-window attributes, applied before initial configuration.
use std::sync::Arc;
use winit::window::{Window, WindowAttributes};

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub(crate) fn supported(parent: &Window) -> bool {
    use winit::platform::wayland::WindowExtWayland;
    parent.xdg_toplevel().is_some()
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
pub(crate) fn supported(_parent: &Window) -> bool {
    false
}

pub(crate) fn attributes(
    attributes: WindowAttributes,
    parent: Option<Arc<Window>>,
) -> WindowAttributes {
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    if let Some(parent) = parent {
        use winit::platform::wayland::WindowAttributesExtWayland;
        return attributes.with_transient_parent(parent);
    }
    #[cfg(not(all(target_os = "linux", feature = "wayland")))]
    let _ = parent;
    attributes
}
