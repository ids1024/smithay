// Have some way to set if socket can be used for reciver, sender, or both?
// - restrict what devices it can use?
// For emulation:
// - create a seat for each seat
// - create a device for pointer, touch, keyboard, if the seat has that
// - direct emulated input on these to the relevant handles
// For reciever context:
// - do we need to pass the application any requests from the client?

use crate::backend::input::{self, Axis, AxisRelativeDirection, AxisSource, InputBackend, InputEvent};

use calloop::generic::Generic;
use calloop::{EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory};
use once_cell::sync::Lazy;
use reis::{calloop::EisRequestSourceEvent, eis, request::{self, ReisRequest}, PendingRequestResult};
use std::{
    collections::HashMap,
    io,
    os::unix::{
        io::{AsRawFd, RawFd},
        net::UnixStream,
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

mod handshake;
use handshake::{HandshakeResult, HandshakeState};

// TODO re-export listener source?

struct SenderState {
    name: Option<String>,
    connection: eis::Connection,
    seat: eis::Seat,
    last_serial: u32,
}

impl SenderState {
    fn new(name: Option<String>, connection: eis::Connection) -> Self {
        // TODO create seat, etc.
        // check protocol versions
        let seat = connection.seat(1);
        seat.name("default");
        seat.capability(0x2, "ei_pointer");
        seat.capability(0x4, "ei_pointer_absolute");
        seat.capability(0x8, "ei_button");
        seat.capability(0x10, "ei_scroll");
        seat.capability(0x20, "ei_keyboard");
        seat.capability(0x40, "ei_touchscreen");
        seat.done();
        Self {
            name,
            connection,
            seat,
            last_serial: 0,
        }
    }
}

struct ReceiverState {}

enum ContextState {
    Handshake(HandshakeState),
    Sender { connection: eis::Connection },
    Receiver { connection: eis::Connection },
}

// TODO how to indicate device/seat?
enum EmulatedInput {
    // XXX implied frame?
    // - our send frames with Vec of events?
    MotionRelative { x: f32, y: f32 },
    MotionAbsolute { x: f32, y: f32 },
    Scroll { x: f32, y: f32 },
    // XXX high res scrolling?
    ScrollDiscrete { x: u32, y: u32 },
}

// TODO have receiver and sender types for each device type?
struct Reciever {}

struct EiInput {
    source: reis::calloop::EisRequestSource,
}

impl InputBackend for EiInput {
    type Device = request::Device;
    type KeyboardKeyEvent = request::KeyboardKey;
    type PointerAxisEvent = request::ScrollDelta; // XXX?
    type PointerButtonEvent = request::Button;
    type PointerMotionEvent = request::PointerMotion;
    type PointerMotionAbsoluteEvent = request::PointerMotionAbsolute;

    type GestureSwipeBeginEvent = input::UnusedEvent;
    type GestureSwipeUpdateEvent = input::UnusedEvent;
    type GestureSwipeEndEvent = input::UnusedEvent;
    type GesturePinchBeginEvent = input::UnusedEvent;
    type GesturePinchUpdateEvent = input::UnusedEvent;
    type GesturePinchEndEvent = input::UnusedEvent;
    type GestureHoldBeginEvent = input::UnusedEvent;
    type GestureHoldEndEvent = input::UnusedEvent;

    // TODO
    // type TouchDownEvent = request::TouchDown;
    // type TouchUpEvent = request::TouchUp;
    // type TouchMotionEvent = request::TouchMotion;
    type TouchDownEvent = input::UnusedEvent;
    type TouchUpEvent = input::UnusedEvent;
    type TouchMotionEvent = input::UnusedEvent;
    type TouchCancelEvent = input::UnusedEvent; // XXX?
    type TouchFrameEvent = input::UnusedEvent; // XXX

    type TabletToolAxisEvent = input::UnusedEvent;
    type TabletToolProximityEvent = input::UnusedEvent;
    type TabletToolTipEvent = input::UnusedEvent;
    type TabletToolButtonEvent = input::UnusedEvent;

    type SwitchToggleEvent = input::UnusedEvent;

    type SpecialEvent = input::UnusedEvent;
}

impl input::Device for request::Device {
    fn id(&self) -> String {
        self.name().unwrap_or("").to_string()
    }

    fn name(&self) -> String {
        self.name().unwrap_or("").to_string()
    }

    fn has_capability(&self, capability: input::DeviceCapability) -> bool {
        todo!()
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

impl<T: request::DeviceEvent + request::EventTime> input::Event<EiInput> for T {
    fn time(&self) -> u64 {
        request::EventTime::time(self)
    }

    fn device(&self) -> request::Device {
        request::DeviceEvent::device(self).clone()
    }
}

impl input::KeyboardKeyEvent<EiInput> for request::KeyboardKey {
    fn key_code(&self) -> u32 {
        self.key
    }

    fn state(&self) -> input::KeyState {
        match self.state {
            eis::keyboard::KeyState::Released => input::KeyState::Released,
            eis::keyboard::KeyState::Press => input::KeyState::Pressed,
        }
    }

    fn count(&self) -> u32 {
        1
    }
}

impl input::PointerAxisEvent<EiInput> for request::ScrollDelta {
    fn amount(&self, _axis: input::Axis) -> Option<f64> {
        todo!()
    }

    fn amount_v120(&self, axis: input::Axis) -> Option<f64> {
        todo!()
    }

    fn source(&self) -> input::AxisSource {
        todo!()
    }

    fn relative_direction(&self, _axis: input::Axis) -> input::AxisRelativeDirection {
        todo!()
    }
}

impl input::PointerButtonEvent<EiInput> for request::Button {
    fn button_code(&self) -> u32 {
        self.button
    }

    fn state(&self) -> input::ButtonState {
        match self.state {
            eis::button::ButtonState::Press => input::ButtonState::Pressed,
            eis::button::ButtonState::Released => input::ButtonState::Released,
        }
    }
}

impl input::PointerMotionEvent<EiInput> for request::PointerMotion {
    fn delta_x(&self) -> f64 {
        todo!()
    }

    fn delta_y(&self) -> f64 {
        todo!()
    }

    fn delta_x_unaccel(&self) -> f64 {
        todo!()
    }

    fn delta_y_unaccel(&self) -> f64 {
        todo!()
    }
}

impl input::PointerMotionAbsoluteEvent<EiInput> for request::PointerMotionAbsolute {}
impl input::AbsolutePositionEvent<EiInput> for request::PointerMotionAbsolute {
    fn x(&self) -> f64 {
        todo!()
    }

    fn y(&self) -> f64 {
        todo!()
    }

    fn x_transformed(&self, width: i32) -> f64 {
        todo!()
    }

    fn y_transformed(&self, height: i32) -> f64 {
        todo!()
    }
}

// Want event source producing (among others?) InputEvent<LibinputInputBackend>

impl EventSource for EiInput {
    type Event = InputEvent<EiInput>;
    type Metadata = ();
    type Ret = ();
    type Error = io::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        cb: F,
    ) -> Result<PostAction, <Self as EventSource>::Error>
    where
        F: FnMut(InputEvent<EiInput>, &mut ()) -> (),
    {
        self.source.process_events(readiness, token, |event, _| {
            // TODO unwrap
            match event.unwrap() {
                 EisRequestSourceEvent::Request(request) => {
                     match request {
                        ReisRequest::KeyboardKey(evt) => {
                        }
                        _ => {}
                     }
                 }
                 // TODO
                 _ => {}
            }
            // TODO
            Ok(PostAction::Continue)
        })
    }

    fn register(&mut self, poll: &mut calloop::Poll, token_factory: &mut TokenFactory) -> Result<(), calloop::Error> {
        self.source.register(poll, token_factory)
    }

    fn reregister(&mut self, poll: &mut calloop::Poll, token_factory: &mut TokenFactory) -> Result<(), calloop::Error> {
        self.source.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> Result<(), calloop::Error> {
        self.source.unregister(poll)
    }
}
