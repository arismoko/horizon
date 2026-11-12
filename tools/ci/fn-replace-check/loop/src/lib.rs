//! Two `[[native.loop]]` slots in `block_entities.tick` (`src/loops/`), compiled into the launcher's
//! copy of the loop, and a HEAD mixin on the loop's entry (`src/mixins/`).
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

/// Called by the helpers: a helper may use its mod's crate.
pub fn note(what: &str) {
    eprintln!("fnr: loop {what}");
}

static ENTRY_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The API listener at the loop's entry, `[[inject]] listen`.
pub fn entry_listener(_host: mod_api::HostSide<'_>, _args: &mut mod_points::TickBlockEntitiesArgs) {
    let calls = ENTRY_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if calls % 200 == 1 {
        note(&format!(
            "api head listener at tick_block_entities, call {calls}"
        ));
    }
}
