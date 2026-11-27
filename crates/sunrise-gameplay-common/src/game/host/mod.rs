//! The game's side of the host traits: `sunrise-mod-api/src/host/<area>.rs` declares each
//! area's methods and access levels. Build-time generated impls forward to the inherent
//! `Game` functions with the same names. `Host`, `ServerHost` and `ClientHost` are the unions of those traits, so
//! a mod reaches every area through them and the handles.
// The generated impls convert error types that are already equal; TODO(#2273): make the generator skip it.
#![allow(clippy::useless_conversion, clippy::unit_arg)]

pub(in crate::game) mod phase_contexts;
pub(crate) mod services;
include!(concat!(env!("OUT_DIR"), "/game_host_generated.rs"));
