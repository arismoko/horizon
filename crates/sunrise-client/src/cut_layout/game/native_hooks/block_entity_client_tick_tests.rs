//! `block_entity_client_tick` (#3611), through a client's real frames (`client_tick_steps` after
//! the frame's `update_world` drain): a client's copies of a type that declared client ticks are
//! told once per client tick each, between `client_tick_pre`, the level's pre and its post, and
//! `client_tick_post`, in the ticker list's order, with the data the frame's drain merged; not
//! while paused or frozen; a type that didn't declare them never; a copy removed or whose column
//! the server forgets not again; and a server, the world's own ticks, never. A spawner between
//! them in the ticker list ticks between them, from the same list.
use mod_api::{
    BlockEntityClientTickEvent, BlockEntityDataReceivedEvent, BlockEntityPart, BlockPart,
    BlocksEvent, ClientHost, ClientTickPostEvent, ClientTickPreEvent, Hooks, HostSide,
    LevelTickPostEvent, LevelTickPreEvent, Registrar, RegistrationContext,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_gameplay::game::world_loading::WorldRequest;
use sunrise_level::containers::BlockEntity;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

pub(crate) const HUM: &str = "client_tick_probe:hum";
pub(crate) const QUIET: &str = "client_tick_probe:quiet";
/// A mod block named as vanilla's shulker boxes are: its ticker is its own type's.
pub(crate) const CRATE: &str = "client_tick_probe:crate_shulker_box";
/// Chunk (100, 100): far from the spawn.
pub(crate) const CHUNK: (i32, i32) = (100, 100);
pub(crate) const HUMS: [[i32; 3]; 2] = [[1610, 70, 1613], [1601, 64, 1602]];
pub(crate) const QUIET_AT: [i32; 3] = [1605, 64, 1605];
/// A hum in the next column, which stays.
#[cfg(feature = "client")]
pub(crate) const OTHER: [i32; 3] = [1620, 64, 1600];
#[cfg(feature = "client")]
pub(crate) const CRATE_AT: [i32; 3] = [1621, 64, 1600];
/// A spawner that arrives after the second hum and before the first.
#[cfg(feature = "client")]
pub(crate) const SPAWNER_AT: [i32; 3] = [1603, 64, 1608];

/// What the probe heard, in order: the client tick's and level's brackets, each client tick
/// (`tick`, its position and `rpm`) and each synced data notice (`data`).
pub(crate) static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// How many particles the game had requested at each client tick told
/// (`sunrise_client::particles::requested_count`), in order.
pub(crate) static TICK_PARTICLES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

pub(crate) fn note(event: String) {
    EVENTS.lock().unwrap().push(event);
}

pub(crate) fn take_events() -> Vec<String> {
    std::mem::take(&mut *EVENTS.lock().unwrap())
}

pub(crate) fn rpm_of(nbt: &Compound) -> i32 {
    match nbt.get(&"rpm".into()) {
        Some(Tag::Int(rpm)) => *rpm,
        _ => -1,
    }
}

/// A client tick's event, once its scoped view is checked to read its position and its state to
/// be its block's.
pub(crate) fn record(event: &mut BlockEntityClientTickEvent<'_>) {
    let (handle, pos, block, id, rpm) = event
        .subject
        .with_view(|view| {
            (
                view.handle(),
                view.position(),
                view.state().block(),
                view.id().to_owned(),
                rpm_of(view.nbt()),
            )
        })
        .expect("live resident client block entity");
    let host = event.subject.host();
    assert_eq!(handle.position(host), Some(pos));
    // The block and its block entity type share the name here.
    assert_eq!(block.id(host), Some(id.as_str()));
    TICK_PARTICLES
        .lock()
        .unwrap()
        .push(sunrise_client_render::particles::requested_count());
    note(format!("tick {pos:?} {rpm}"));
}

/// Declares client ticks.
pub(crate) struct Hum;
#[mod_api::sunrise_part]
impl BlockPart for Hum {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Hum {
    fn block_entity_client_tick(&self, event: &mut BlockEntityClientTickEvent<'_>) {
        record(event);
    }
    fn block_entity_data_received(&self, event: &mut BlockEntityDataReceivedEvent<'_>) {
        let (pos, rpm) = event
            .subject
            .with_view(|view| (view.position(), rpm_of(view.nbt())))
            .expect("live resident client block entity");
        note(format!("data {pos:?} {rpm}"));
    }
}

/// Has a client tick, but the selector rejects its client states.
pub(crate) struct Quiet;
#[mod_api::sunrise_part]
impl BlockPart for Quiet {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Quiet {
    fn block_entity_client_tick(&self, event: &mut BlockEntityClientTickEvent<'_>) {
        record(event);
    }
}

pub(crate) struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let hum = r.block_entity_type("hum").blocks(&[HUM]).build();
        r.block_entity_part(&hum, Hum).unwrap();
        let quiet = r
            .block_entity_type("quiet")
            .blocks(&[QUIET])
            .ticker_when(|_, _, side| side == mod_api::LogicalSide::Server)
            .build();
        r.block_entity_part(&quiet, Quiet).unwrap();
        let boxed = r
            .block_entity_type("crate_shulker_box")
            .blocks(&[CRATE])
            .build();
        r.block_entity_part(&boxed, Hum).unwrap();
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        for block in [HUM, QUIET, CRATE] {
            event
                .register(mod_api::Block::new(block, "minecraft:stone"))
                .unwrap();
        }
    }
    fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        note("client_pre".into());
    }
    fn client_tick_post(&self, _: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        note("client_post".into());
    }
    fn level_tick_pre(&self, host: HostSide<'_>, _: &mut LevelTickPreEvent) {
        if let HostSide::Client(_) = host {
            note("level_pre".into());
        }
    }
    fn level_tick_post(&self, host: HostSide<'_>, _: &mut LevelTickPostEvent) {
        if let HostSide::Client(_) = host {
            note("level_post".into());
        }
    }
}

pub(crate) fn install() {
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("client_tick_probe", Box::new(Probe))].into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
}

/// The prepared resources with the installed mods' content.
pub(crate) fn prepared_root() -> PathBuf {
    let base = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf()
}

#[cfg(feature = "client")]
pub(crate) fn rpm(rpm: i32) -> Compound {
    let mut nbt = Compound::new();
    nbt.insert("rpm".into(), Tag::Int(rpm));
    nbt
}

/// One frame: what the server sent is handled (`update_world`, which a frame runs before its
/// ticks), then `steps` client ticks. Returns what the probe heard.
#[cfg(feature = "client")]
pub(crate) fn frame(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    steps: u32,
) -> Vec<String> {
    game.side_update_world();
    game.client_tick_steps(steps, None).unwrap();
    take_events()
}

/// The events of one client tick that ticks the level, its client ticks being `ticks`.
#[cfg(feature = "client")]
pub(crate) fn ticked(ticks: &[([i32; 3], i32)]) -> Vec<String> {
    let mut events = vec!["client_pre".to_owned(), "level_pre".to_owned()];
    events.extend(ticks.iter().map(|(pos, rpm)| format!("tick {pos:?} {rpm}")));
    events.extend(["level_post".to_owned(), "client_post".to_owned()]);
    events
}

pub(crate) fn request(world: &Path) -> WorldRequest {
    let rules = std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
    let options = sunrise_sim::world_options::WorldOptions {
        world_type: sunrise_sim::save::WorldPreset::Flat,
        rules: sunrise_game_data::gamerules::GameRules::with_registry(rules.clone(), &[]),
        ..Default::default()
    };
    let mut request = WorldRequest::new(
        "client_tick",
        Some("client_tick"),
        Some(1),
        true,
        Some(&options),
        rules,
    )
    .unwrap();
    request.save_root = world.to_owned();
    request
}

/// A world's own ticks (a server's) never tell a client tick, a declared type's included.
#[test]
#[ignore = "daily"]
fn a_server_never_ticks_a_client_ticker() {
    if run_fresh_cases(
        "cut_layout::game::native_hooks::block_entity_client_tick_tests::a_server_never_ticks_a_client_ticker",
        "SUNRISE_BLOCK_ENTITY_CLIENT_TICK_SERVER_CHILD",
        &["game"],
    ) {
        return;
    }
    install();
    let saves = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("block-entity-client-tick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);
    with_game_at(prepared_root(), |game| {
        game.load_world(request(&saves)).unwrap();
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 1600 1600"));
        let deadline = Instant::now() + Duration::from_secs(120);
        while !game.world.chunks.contains_key(&CHUNK) || game.server.streaming.busy() {
            assert!(Instant::now() < deadline, "chunk {CHUNK:?} did not load");
            game.server.streaming.wait(Duration::from_millis(5));
            game.side_update_world();
        }
        game.tick_world(true).unwrap();
        for ([x, y, z], block) in [(HUMS[0], HUM), (QUIET_AT, QUIET)] {
            assert!(game.run_command(&format!("setblock {x} {y} {z} {block}")));
        }
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let [x, y, z] = HUMS[0];
        assert!(
            matches!(
                game.world.block_entities.get(&(x, y, z)),
                Some(BlockEntity::Mod(_))
            ),
            "the hum was placed"
        );
        assert!(
            !take_events().iter().any(|e| e.starts_with("tick")),
            "a server told a client tick"
        );
    });
    let _ = std::fs::remove_dir_all(&saves);
}

use sunrise_gameplay_common::game::side_ops::events::EventsSideGameExt as _;
