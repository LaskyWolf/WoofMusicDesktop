//! The window's background: the desktop blurred behind a dark tint where the
//! system offers a blur (KWin's blur on Wayland, Mica on Windows 11), plain
//! black elsewhere. `WOOF_BLUR=0` keeps it black.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::{Color, Task};

/// Set once the system blurs what is behind the window.
static BLURRED: AtomicBool = AtomicBool::new(false);

fn allowed() -> bool {
    std::env::var("WOOF_BLUR").map_or(true, |v| v != "0")
}

/// The window is made transparent (decided before it opens).
pub fn transparent() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(|| {
        let answer = allowed() && (cfg!(windows) || compositor_blurs());
        log::info!(
            "Window background: {}",
            if answer { "blur offered" } else { "black" }
        );
        answer
    })
}

/// The colour under everything: see-through over a blur, else black.
pub fn color() -> Color {
    if BLURRED.load(Ordering::Relaxed) {
        // Two thirds: exact even where the window's format keeps 2 bits of
        // alpha (Rgb10a2 on Vulkan).
        Color::from_rgba(0.0, 0.0, 0.0, 2.0 / 3.0)
    } else {
        crate::theme::BACKGROUND
    }
}

/// Wayland: the compositor offers `ext-background-effect-v1` with its blur
/// (KDE Plasma 6.7+, GNOME 51, niri 26.04). winit speaks only KWin's older
/// blur protocol, which Plasma 6.8 no longer lists, so the app asks itself.
#[cfg(target_os = "linux")]
mod wayland {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
    use wayland_protocols::ext::background_effect::v1::client::{
        ext_background_effect_manager_v1::{self as manager, ExtBackgroundEffectManagerV1},
        ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
    };

    /// Whether the compositor said it can blur.
    #[derive(Default)]
    struct State {
        blur: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &ExtBackgroundEffectManagerV1,
            event: manager::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let manager::Event::Capabilities {
                flags: WEnum::Value(flags),
            } = event
            {
                state.blur = flags.contains(manager::Capability::Blur);
            }
        }
    }

    delegate_noop!(State: ignore ExtBackgroundEffectSurfaceV1);
    delegate_noop!(State: ignore wl_compositor::WlCompositor);
    delegate_noop!(State: ignore wl_region::WlRegion);
    delegate_noop!(State: ignore wl_surface::WlSurface);

    /// Binds the manager on `connection` and waits for its capabilities.
    fn manager(
        connection: &Connection,
    ) -> Option<(
        ExtBackgroundEffectManagerV1,
        wayland_client::globals::GlobalList,
        wayland_client::EventQueue<State>,
        State,
    )> {
        let (globals, mut queue) = registry_queue_init::<State>(connection).ok()?;
        let manager = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        let mut state = State::default();
        queue.roundtrip(&mut state).ok()?;
        state.blur.then_some((manager, globals, queue, state))
    }

    /// A separate connection to the compositor, before the window opens.
    pub fn offers_blur() -> bool {
        Connection::connect_to_env()
            .ok()
            .and_then(|connection| manager(&connection))
            .is_some()
    }

    /// Asks for the blur behind the whole of the window's surface, on
    /// winit's own connection. The window's next frame applies it.
    pub fn blur(window: &dyn iced::window::Window) -> bool {
        let (Ok(display), Ok(handle)) = (window.display_handle(), window.window_handle()) else {
            return false;
        };
        let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle)) =
            (display.as_raw(), handle.as_raw())
        else {
            return false;
        };
        // Safety: both pointers come from winit, which keeps the display and
        // the surface alive as long as the window.
        let connection = Connection::from_backend(unsafe {
            Backend::from_foreign_display(display.display.as_ptr().cast())
        });
        let Ok(id) = (unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                handle.surface.as_ptr().cast(),
            )
        }) else {
            return false;
        };
        let Ok(surface) = wl_surface::WlSurface::from_id(&connection, id) else {
            return false;
        };
        let Some((manager, globals, queue, _)) = manager(&connection) else {
            return false;
        };
        let handle = queue.handle();
        let Ok(compositor) = globals.bind::<wl_compositor::WlCompositor, _, _>(&handle, 1..=6, ())
        else {
            return false;
        };
        // The region is clipped to the surface, so one this big covers any window size.
        let region = compositor.create_region(&handle, ());
        region.add(0, 0, i32::MAX, i32::MAX);
        manager
            .get_background_effect(&surface, &handle, ())
            .set_blur_region(Some(&region));
        region.destroy();
        let flushed = connection.flush().is_ok();
        // ponytail: the queue stays alive with the window (the compositor
        // may still send capabilities); it is never read again.
        std::mem::forget(queue);
        flushed
    }
}

#[cfg(target_os = "linux")]
fn compositor_blurs() -> bool {
    wayland::offers_blur()
}

#[cfg(not(target_os = "linux"))]
fn compositor_blurs() -> bool {
    false
}

/// Asks for the blur once the window exists: the compositor on Wayland, DWM
/// (Mica) on Windows. Where it is refused, the background stays black.
pub fn apply<T: Send + 'static>() -> Task<T> {
    #[cfg(target_os = "linux")]
    if transparent() {
        return iced::window::latest()
            .and_then(|id| {
                iced::window::run(id, |window| {
                    let applied = wayland::blur(window);
                    log::info!("Window blur applied: {applied}");
                    BLURRED.store(applied, Ordering::Relaxed);
                })
            })
            .discard();
    }
    #[cfg(windows)]
    if transparent() {
        return iced::window::latest()
            .and_then(|id| {
                iced::window::run(id, |window| {
                    let applied = window_vibrancy::apply_mica(window, Some(true)).is_ok();
                    BLURRED.store(applied, Ordering::Relaxed);
                })
            })
            .discard();
    }
    Task::none()
}
