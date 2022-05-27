use std::{
    os::unix::io::RawFd,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use smithay::{
    delegate_compositor, delegate_data_device, delegate_layer_shell, delegate_output, delegate_seat,
    delegate_shm, delegate_xdg_activation, delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{PopupManager, Space},
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_protocols::xdg::decoration::{
            self as xdg_decoration, zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
        },
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_data_source::WlDataSource, wl_surface::WlSurface},
            Display, DisplayHandle, Resource,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::CompositorState,
        data_device::{
            set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
            ServerDndGrabHandler,
        },
        output::{Output, OutputManagerState},
        seat::{CursorImageStatus, Seat, SeatHandler, SeatState, XkbConfig},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{
                decoration::{XdgDecorationHandler, XdgDecorationManager},
                ToplevelSurface, XdgShellState,
            },
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
    },
};

#[cfg(feature = "xwayland")]
use smithay::xwayland::{XWayland, XWaylandEvent};

pub struct CalloopData<BackendData: 'static> {
    pub state: AnvilState<BackendData>,
    pub display: Display<AnvilState<BackendData>>,
}

struct ClientState;
impl ClientData for ClientState {
    /// Notification that a client was initialized
    fn initialized(&self, client_id: ClientId) {}
    /// Notification that a client is disconnected
    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {}
}

#[derive(Debug)]
pub struct AnvilState<BackendData: 'static> {
    pub backend_data: BackendData,
    pub socket_name: Option<String>,
    pub running: Arc<AtomicBool>,
    pub handle: LoopHandle<'static, CalloopData<BackendData>>,

    // desktop
    pub space: Space,
    pub popups: PopupManager,

    // smithay state
    pub compositor_state: CompositorState,
    pub data_device_state: DataDeviceState,
    pub layer_shell_state: WlrLayerShellState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<AnvilState<BackendData>>,
    pub shm_state: ShmState,
    pub xdg_activation_state: XdgActivationState,
    pub xdg_decoration_state: XdgDecorationManager,
    pub xdg_shell_state: XdgShellState,

    pub dnd_icon: Option<WlSurface>,
    pub log: slog::Logger,

    // input-related fields
    pub suppressed_keys: Vec<u32>,
    pub pointer_location: Point<f64, Logical>,
    pub cursor_status: Arc<Mutex<CursorImageStatus>>,
    pub seat_name: String,
    pub seat: Seat<AnvilState<BackendData>>,
    pub start_time: std::time::Instant,

    // things we must keep alive
    #[cfg(feature = "xwayland")]
    pub xwayland: XWayland<AnvilState<BackendData>>,
}

delegate_compositor!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData> DataDeviceHandler for AnvilState<BackendData> {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
    fn send_selection(&mut self, mime_type: String, fd: RawFd) {
        unreachable!("Anvil doesn't do server-side selections");
    }
}
impl<BackendData> ClientDndGrabHandler for AnvilState<BackendData> {
    fn started(&mut self, source: Option<WlDataSource>, icon: Option<WlSurface>, seat: Seat<Self>) {
        self.dnd_icon = icon;
    }
    fn dropped(&mut self, seat: Seat<Self>) {
        self.dnd_icon = None;
    }
}
impl<BackendData> ServerDndGrabHandler for AnvilState<BackendData> {
    fn send(&mut self, mime_type: String, fd: RawFd) {
        unreachable!("Anvil doesn't do server-side grabs");
    }
}
delegate_data_device!(@<BackendData: 'static> AnvilState<BackendData>);
delegate_output!(@<BackendData: 'static> AnvilState<BackendData>);

impl<BackendData> AsRef<ShmState> for AnvilState<BackendData> {
    fn as_ref(&self) -> &ShmState {
        &self.shm_state
    }
}
delegate_shm!(@<BackendData: 'static> AnvilState<BackendData>);

impl<BackendData> SeatHandler for AnvilState<BackendData> {
    fn seat_state(&mut self) -> &mut SeatState<AnvilState<BackendData>> {
        &mut self.seat_state
    }
}
delegate_seat!(@<BackendData: 'static> AnvilState<BackendData>);

impl<BackendData> XdgActivationHandler for AnvilState<BackendData> {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed().as_secs() < 10 {
            // Just grant the wish
            let w = self.space.window_for_surface(&surface).cloned();
            if let Some(window) = w {
                self.space.raise_window(&window, true);
            }
        } else {
            // Discard the request
            self.xdg_activation_state.remove_request(&token);
        }
    }

    fn destroy_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        // The request is cancelled
    }
}
delegate_xdg_activation!(@<BackendData: 'static> AnvilState<BackendData>);

impl<BackendData> XdgDecorationHandler for AnvilState<BackendData> {
    fn new_decoration(&mut self, dh: &DisplayHandle, toplevel: ToplevelSurface) {
        use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        toplevel.send_configure();
    }
    fn request_mode(&mut self, _dh: &DisplayHandle, _toplevel: ToplevelSurface, _mode: DecorationMode) {}
    fn unset_mode(&mut self, _dh: &DisplayHandle, _toplevel: ToplevelSurface) {}
}
delegate_xdg_decoration!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_xdg_shell!(@<BackendData: Backend + 'static> AnvilState<BackendData>);
delegate_layer_shell!(@<BackendData: 'static> AnvilState<BackendData>);

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    pub fn init(
        mut display: &mut Display<AnvilState<BackendData>>,
        handle: LoopHandle<'static, CalloopData<BackendData>>,
        backend_data: BackendData,
        log: slog::Logger,
        listen_on_socket: bool,
    ) -> AnvilState<BackendData> {
        // init wayland clients
        let socket_name = if listen_on_socket {
            let source = ListeningSocketSource::new_auto(log.clone()).unwrap();
            let socket_name = source.socket_name().to_string_lossy().into_owned();
            handle
                .insert_source(source, |client_stream, _, data| {
                    use std::os::unix::io::AsRawFd;

                    data.state
                        .handle
                        .insert_source(
                            Generic::new(client_stream.as_raw_fd(), Interest::READ, Mode::Level),
                            |_, _, data| {
                                data.display.dispatch_clients(&mut data.state).unwrap();
                                Ok(PostAction::Continue)
                            },
                        )
                        .unwrap();
                    data.display
                        .handle()
                        .insert_client(client_stream, Arc::new(ClientState));
                    data.display.dispatch_clients(&mut data.state).unwrap();
                })
                .expect("Failed to init wayland socket source");
            info!(log, "Listening on wayland socket"; "name" => socket_name.clone());
            ::std::env::set_var("WAYLAND_DISPLAY", &socket_name);
            Some(socket_name)
        } else {
            None
        };

        // init globals
        let compositor_state = CompositorState::new(display, log.clone());
        let data_device_state = DataDeviceState::new(display, log.clone());
        let layer_shell_state = WlrLayerShellState::new(display, log.clone());
        let output_manager_state = OutputManagerState::new();
        let seat_state = SeatState::new();
        let shm_state = ShmState::new(display, vec![], log.clone());
        let xdg_activation_state = XdgActivationState::new(display, log.clone());
        let xdg_decoration_state = XdgDecorationManager::new(display, log.clone()).0;
        let xdg_shell_state = XdgShellState::new(display, log.clone()).0;

        // init input
        let seat_name = backend_data.seat_name();
        let mut seat = Seat::new(&mut display, seat_name.clone(), log.clone());

        let cursor_status = Arc::new(Mutex::new(CursorImageStatus::Default));
        let cursor_status2 = cursor_status.clone();
        seat.add_pointer(move |new_status| *cursor_status2.lock().unwrap() = new_status);

        let dh = display.handle();
        seat.add_keyboard(XkbConfig::default(), 200, 25, move |seat, focus| {
            let focus = focus.and_then(|s| dh.get_client(s.id()).ok());
            set_data_device_focus(&dh, seat, focus)
        })
        .expect("Failed to initialize the keyboard");

        /*
        init_tablet_manager_global(&mut display.borrow_mut());

        let cursor_status3 = cursor_status.clone();
        seat.tablet_seat().on_cursor_surface(move |_tool, new_status| {
            // TODO: tablet tools should have their own cursors
            *cursor_status3.lock().unwrap() = new_status;
        });
        */

        #[cfg(feature = "xwayland")]
        let xwayland = {
            let (xwayland, channel) = XWayland::new(handle.clone(), display.clone(), log.clone());
            let ret = handle.insert_source(channel, |event, _, anvil_state| match event {
                XWaylandEvent::Ready { connection, client } => anvil_state.xwayland_ready(connection, client),
                XWaylandEvent::Exited => anvil_state.xwayland_exited(),
            });
            if let Err(e) = ret {
                error!(
                    log,
                    "Failed to insert the XWaylandSource into the event loop: {}", e
                );
            }
            xwayland
        };

        AnvilState {
            backend_data,
            socket_name,
            running: Arc::new(AtomicBool::new(true)),
            handle,
            space: Space::new(log.clone()),
            popups: PopupManager::new(log.clone()),
            compositor_state,
            data_device_state,
            layer_shell_state,
            output_manager_state,
            seat_state,
            shm_state,
            xdg_activation_state,
            xdg_decoration_state,
            xdg_shell_state,
            dnd_icon: None,
            log,
            suppressed_keys: Vec::new(),
            pointer_location: (0.0, 0.0).into(),
            cursor_status,
            seat_name,
            seat,
            start_time: std::time::Instant::now(),
            #[cfg(feature = "xwayland")]
            xwayland,
        }
    }
}

pub trait Backend {
    fn seat_name(&self) -> String;
    fn reset_buffers(&mut self, output: &Output);
    fn early_import(&mut self, surface: &WlSurface);
}
