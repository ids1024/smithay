use once_cell::sync::Lazy;
use reis::eis;
use std::collections::HashMap;

use super::{ReceiverState, SenderState};

static SERVER_INTERFACES: Lazy<HashMap<&'static str, u32>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("ei_callback", 1);
    m.insert("ei_connection", 1);
    m.insert("ei_seat", 1);
    m.insert("ei_device", 1);
    m.insert("ei_pingpong", 1);
    m.insert("ei_keyboard", 1);
    m.insert("ei_pointer", 1);
    m.insert("ei_pointer_absolute", 1);
    m.insert("ei_button", 1);
    m.insert("ei_scroll", 1);
    m.insert("ei_touchscreen", 1);
    m
});

pub(super) enum HandshakeResult {
    Continue,
    Disconnect,
    Sender(SenderState),
    Receiver(ReceiverState),
}

pub struct HandshakeState {
    handshake: eis::Handshake,
    context_type: Option<eis::handshake::ContextType>,
    name: Option<String>,
    negotiated_interfaces: HashMap<&'static str, u32>,
}

impl HandshakeState {
    pub fn new(context: &eis::Context) -> Self {
        let handshake = context.handshake();
        handshake.handshake_version(1);
        context.flush();
        Self {
            handshake,
            context_type: None,
            name: None,
            negotiated_interfaces: HashMap::new(),
        }
    }

    fn handle_request(&mut self, request: eis::handshake::Request) -> HandshakeResult {
        match request {
            eis::handshake::Request::HandshakeVersion { version } => {}
            eis::handshake::Request::ContextType { context_type } => {
                if self.context_type.is_some() {
                    return HandshakeResult::Disconnect;
                }
                self.context_type = Some(context_type);
            }
            eis::handshake::Request::Name { name } => {
                if self.name.is_some() {
                    return HandshakeResult::Disconnect;
                }
                self.name = Some(name);
            }
            eis::handshake::Request::InterfaceVersion { name, version } => {
                if let Some((interface, server_version)) = SERVER_INTERFACES.get_key_value(name.as_str()) {
                    self.negotiated_interfaces
                        .insert(interface, version.min(*server_version));
                }
            }
            eis::handshake::Request::Finish => {
                // May prompt user here whether to allow this

                for (interface, version) in self.negotiated_interfaces.iter() {
                    self.handshake.interface_version(interface, *version);
                }

                if ["ei_connection", "ei_pingpong", "ei_callback"]
                    .iter()
                    .any(|i| !self.negotiated_interfaces.contains_key(i))
                {
                    return HandshakeResult::Disconnect;
                }

                let connection = self.handshake.connection(0, 1);

                return match self.context_type {
                    Some(eis::handshake::ContextType::Sender) => {
                        HandshakeResult::Sender(SenderState::new(self.name.clone(), connection))
                    }
                    Some(eis::handshake::ContextType::Receiver) => todo!(),
                    None => HandshakeResult::Disconnect,
                };
            }
            _ => {}
        }
        HandshakeResult::Continue
    }
}
