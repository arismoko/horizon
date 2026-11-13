// game-module-visibility: pub(crate)
//! What a server publishes for its world generation at each Parts install (world open and
//! `/reload`): its resolved Part table, which the worldgen Part methods read on the workers and on
//! the server thread alike, and the block-state index their states convert through.
use super::api_values::block_state::Index;
use std::sync::{Arc, PoisonError, RwLock};

static INDEX: RwLock<Option<Arc<Index>>> = RwLock::new(None);

/// Publish this server's Part table, resolved against its tags, and its block registry's index
/// (the worker resource's own) for the features generated from now on.
pub(in crate::game) fn publish(parts: Option<&mod_api::Parts>, index: &Arc<Index>) {
    *INDEX.write().unwrap_or_else(PoisonError::into_inner) = Some(index.clone());
    mod_api::__private::publish_worldgen_parts(parts.cloned());
}

/// The index the server published last; None before a server published one.
pub(in crate::game) fn index() -> Option<Arc<Index>> {
    INDEX.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The engine's feature callback: one feature's Part lookups read the tables published when it
/// started.
pub(crate) fn feature(place: &mut dyn FnMut()) {
    mod_api::__private::worldgen_feature(place);
}
