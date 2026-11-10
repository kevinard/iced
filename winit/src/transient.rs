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

pub(crate) fn dialog_attributes(attributes: WindowAttributes, parent: Option<(String, bool, Option<String>)>) -> WindowAttributes {
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    if let Some((handle, modal, token)) = parent {
        use winit::platform::wayland::WindowAttributesExtWayland;
        use winit::platform::startup_notify::WindowAttributesExtStartupNotify;
        let attributes = attributes.with_foreign_parent(handle, modal);
        return if let Some(token) = token {
            attributes.with_activation_token(winit::window::ActivationToken::from_raw(token))
        } else { attributes };
    }
    #[cfg(not(all(target_os = "linux", feature = "wayland")))]
    let _ = parent;
    attributes
}
