//! What a server publishes for its world generation at each Parts install (world open and
//! `/reload`): its resolved Part table, which the worldgen Part methods read on the workers and on
//! the server thread alike, and the block-state index their states convert through, with the
//! registry that index was built from.
use super::api_values::block_state::Index;
use std::sync::{Arc, PoisonError, RwLock};
use sunrise_level::api_context::WorldgenReader;
use sunrise_world::registry::BlockRegistry;

/// The published index and the registry it converts against.
pub type Published = (Arc<Index>, &'static BlockRegistry);

static INDEX: RwLock<Option<Published>> = RwLock::new(None);

/// Held by each test that publishes: the published tables are process-wide.
#[cfg(any(test, feature = "test-support"))]
pub static PUBLISHING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Publish this server's Part table, resolved against its tags, and its block registry's index
/// (the worker resource's own) for the features generated from now on.
pub(in crate::game) fn publish(
    parts: Option<&mod_api::Parts>,
    index: &Arc<Index>,
    registry: &'static BlockRegistry,
) {
    *INDEX.write().unwrap_or_else(PoisonError::into_inner) = Some((index.clone(), registry));
    mod_api::__private::publish_worldgen_parts(parts.cloned());
}

/// The index the server published last; None before a server published one.
pub fn index() -> Option<Arc<Index>> {
    published().map(|(index, _)| index)
}

fn published() -> Option<Published> {
    INDEX.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Runs `f` with the server's worldgen context as a mod reads it: its registries, settings and
/// `state_with_property` and `state_property`, which answer None before the server published its
/// index. None before a server published its context.
pub(in crate::game) fn with_host<R>(
    f: impl FnOnce(&mod_api::OwningWorldgen<'_>) -> R,
) -> Option<R> {
    let context = sunrise_world::native_worldgen_hooks::context()?;
    let reader = WorldgenReader(&context);
    let published = published();
    let state_with_property = |state: mod_api::BlockState, key: &str, value: &str| {
        let (index, registry) = published.as_ref()?;
        let engine = index.to_engine(state).ok()?;
        let set = super::native_hooks::with_property(registry, engine, key, value)?;
        index.to_api(set).ok()
    };
    let state_property = |state: mod_api::BlockState, key: &str| {
        let (index, registry) = published.as_ref()?;
        let engine = index.to_engine(state).ok()?;
        registry
            .state(engine)?
            .properties
            .get(key)
            .map(|v| v.to_string())
    };
    Some(f(&mod_api::OwningWorldgen::new(
        &reader,
        &state_with_property,
        &state_property,
    )))
}

/// The engine's feature callback: one feature's Part lookups read the tables published when it
/// started.
pub fn feature(place: &mut dyn FnMut()) {
    mod_api::__private::worldgen_feature(place);
}
