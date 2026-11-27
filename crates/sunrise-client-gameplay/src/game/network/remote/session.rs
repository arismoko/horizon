use std::sync::Arc;
use sunrise_client_lib::net::Session;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_protocol::{registry::Builtin, text::Language};

/// Connection already started; its eventual packets still belong to the next listener.
pub(in crate::game) struct RemoteOpening {
    pub(in crate::game) session: Session,
    pub(in crate::game) builtin: Arc<Builtin>,
    pub(in crate::game) lang: Arc<Language>,
    pub(in crate::game) info: ClientInformation,
    pub(in crate::game) user: (String, u128),
    pub(in crate::game) local: bool,
    pub(in crate::game) list_name: Option<String>,
}
