use std::{os::unix::prelude::RawFd, sync::Arc};

use smithay::{
    backend::{
        input::{InputEvent, KeyboardKeyEvent},
        renderer::{
            utils::{draw_surface_tree, on_commit_buffer_handler},
            Frame, Renderer,
        },
        winit::{self, WinitEvent},
    },
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
    reexports::wayland_server::Display,
    utils::{Rectangle, Transform},
    wayland::{
        buffer::{Buffer, BufferHandler},
        compositor::{
            with_surface_tree_downward, CompositorHandler, CompositorState, SurfaceAttributes,
            TraversalAction,
        },
        data_device::{ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler},
        seat::{FilterResult, Seat, SeatHandler, SeatState},
        shell::xdg::{XdgRequest, XdgShellHandler, XdgShellState},
        shm::ShmState,
    },
};
use wayland_protocols::xdg::shell::server::xdg_toplevel;
use wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::wl_surface::{self, WlSurface},
    socket::ListeningSocket,
    DisplayHandle,
};

impl BufferHandler for App {
    fn buffer_destroyed(&mut self, _buffer: &Buffer) {}
}

impl XdgShellHandler for App {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn request(&mut self, dh: &DisplayHandle, request: XdgRequest) {
        dbg!(&request);

        match request {
            XdgRequest::NewToplevel { surface } => {
                surface.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Activated);
                });
                surface.send_configure(dh);
            }
            XdgRequest::Move { .. } => {
                //
            }
            _ => {}
        }
    }
}

impl DataDeviceHandler for App {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }

    fn send_selection(&mut self, _mime_type: String, _fd: RawFd) {}
}

impl ClientDndGrabHandler for App {}
impl ServerDndGrabHandler for App {
    fn send(&mut self, _mime_type: String, _fd: RawFd) {}
}

impl CompositorHandler for App {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn commit(&mut self, dh: &DisplayHandle, surface: &WlSurface) {
        on_commit_buffer_handler(dh, surface);
    }
}

impl AsRef<ShmState> for App {
    fn as_ref(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

struct App {
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,

    seat: Seat<Self>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_winit()
}

fn log() -> ::slog::Logger {
    use slog::Drain;
    ::slog::Logger::root(::slog_stdlog::StdLog.fuse(), slog::o!())
}

pub fn run_winit() -> Result<(), Box<dyn std::error::Error>> {
    let log = log();

    let mut display: Display<App> = Display::new()?;

    let seat_state = SeatState::new();
    let seat = Seat::new(&mut display, "winit", None);

    let mut state = {
        App {
            compositor_state: CompositorState::new(&mut display, None),
            xdg_shell_state: XdgShellState::new(&mut display, None).0,
            shm_state: ShmState::new(&mut display, vec![], None),
            seat_state,
            data_device_state: DataDeviceState::new(&mut display, None),
            seat,
        }
    };

    let listener = ListeningSocket::bind("wayland-5").unwrap();
    let mut clients = Vec::new();

    let (mut backend, mut winit) = winit::init(None)?;

    let start_time = std::time::Instant::now();

    let keyboard = state
        .seat
        .add_keyboard(&mut display.handle(), Default::default(), 200, 200, |_, _| {})
        .unwrap();

    std::env::set_var("WAYLAND_DISPLAY", "wayland-5");
    std::process::Command::new("weston-terminal").spawn().ok();

    loop {
        winit.dispatch_new_events(|event| match event {
            WinitEvent::Resized { .. } => {}
            WinitEvent::Input(event) => match event {
                InputEvent::Keyboard { event } => {
                    let dh = &mut display.handle();
                    keyboard.input::<(), _>(dh, event.key_code(), event.state(), 0.into(), 0, |_, _| {
                        //
                        FilterResult::Forward
                    });
                }
                InputEvent::PointerMotionAbsolute { .. } => {
                    let dh = &mut display.handle();
                    state.xdg_shell_state.toplevel_surfaces(|surfaces| {
                        for surface in surfaces {
                            let surface = surface.wl_surface();
                            keyboard.set_focus(dh, Some(surface), 0.into());
                            break;
                        }
                    });
                }
                _ => {}
            },
            _ => (),
        })?;

        backend.bind().unwrap();

        let size = backend.window_size().physical_size;
        let damage = Rectangle::from_loc_and_size((0, 0), size);

        backend
            .renderer()
            .render(size, Transform::Flipped180, |renderer, frame| {
                frame.clear([0.1, 0.0, 0.0, 1.0], &[damage.to_f64()]).unwrap();

                state.xdg_shell_state.toplevel_surfaces(|surfaces| {
                    for surface in surfaces {
                        let dh = &mut display.handle();
                        let surface = surface.wl_surface();
                        draw_surface_tree(
                            dh,
                            renderer,
                            frame,
                            surface,
                            1.0,
                            (0, 0).into(),
                            &[damage.to_logical(1)],
                            &log,
                        )
                        .unwrap();

                        send_frames_surface_tree(dh, surface, start_time.elapsed().as_millis() as u32);
                    }
                });
            })?;

        if let Some(stream) = listener.accept()? {
            println!("Got a client: {:?}", stream);

            let client = display
                .handle()
                .insert_client(stream, Arc::new(ClientState))
                .unwrap();
            clients.push(client);
        }

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;

        // It is important that all events on the display have been dispatched and flushed to clients before
        // swapping buffers because this operation may block.
        backend.submit(Some(&[damage.to_logical(1)]), 1.0).unwrap();
    }
}

pub fn send_frames_surface_tree(dh: &DisplayHandle, surface: &wl_surface::WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_surf, states, &()| {
            // the surface may not have any user_data if it is a subsurface and has not
            // yet been commited
            for callback in states
                .cached_state
                .current::<SurfaceAttributes>()
                .frame_callbacks
                .drain(..)
            {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}

struct ClientState;
impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {
        println!("initialized");
    }

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        println!("disconnected");
    }
}

// Macros used to delegate protocol handling to types in the app state.
delegate_xdg_shell!(App);
delegate_compositor!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_data_device!(App);
