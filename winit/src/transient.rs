//! Native auxiliary-window attributes, applied before initial configuration.
use winit::window::Window;

pub(crate) fn activation_token(window: &Window) -> Option<winit::event_loop::AsyncRequestSerial> {
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    {
        use winit::platform::wayland::WindowExtWayland;
        window.request_activation_token_for_input().ok()
    }
    #[cfg(not(all(target_os = "linux", feature = "wayland")))]
    { let _ = window; None }
}
