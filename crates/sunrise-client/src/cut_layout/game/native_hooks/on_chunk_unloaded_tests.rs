//! `on_chunk_unloaded` through a real game (#3145): a chunk with two mod block entities and a
//! vanilla chest leaves the world when its `/forceload` ticket goes, through the game's own
//! streaming, save and unload; the chunk comes back from its saved data; then the world closes.
use mod_api::{
    BlockEntityPart, BlockPart, BlocksEvent, Hooks, HostSide, OnChunkUnloadedEvent, Registrar,
    RegistrationContext,
};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_gameplay::game::world_loading::WorldRequest;
#[cfg(feature = "client")]
use sunrise_level::containers::BlockEntity;
#[cfg(feature = "client")]
use {
    crate::game::server_side::native_entity_refs::tests::with_game_role_client,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

pub(crate) const POT: &str = "unload_probe:pot";
/// Chunk (100, 100): far from the spawn, so only its ticket keeps it.
pub(crate) const CHUNK: (i32, i32) = (100, 100);
pub(crate) const POTS: [[i32; 3]; 2] = [[1601, 100, 1602], [1610, 120, 1613]];
pub(crate) const CHEST: [i32; 3] = [1605, 100, 1605];

/// What each call saw: the side, the position its handle names, its entity's id, and
/// whether the merge the part tried inside it was refused.
pub(crate) static CALLS: Mutex<Vec<(&'static str, [i32; 3], String, bool)>> =
    Mutex::new(Vec::new());

pub(crate) struct Pot;
#[mod_api::sunrise_part]
impl BlockPart for Pot {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Pot {
    fn on_chunk_unloaded(&self, host: HostSide<'_>, event: &mut OnChunkUnloadedEvent<'_>) {
        let HostSide::Server(host) = host else {
            assert_eq!(
                event.entity.nbt().get(&"unload_witness".into()),
                Some(&Tag::Int(event.pos[0]))
            );
            assert_eq!(event.entity.item_slot(0).unwrap().count(), 5);
            assert!(event.entity.copy_item_slot(0).is_none());
            CALLS
                .lock()
                .unwrap()
                .push(("client", event.pos, event.entity.id().to_owned(), true));
            return;
        };
        let mut written = Compound::new();
        written.insert("written_on_unload".into(), Tag::Byte(1));
        let refused = event.block_entity.merge_data(host, written) != Ok(true);
        CALLS
            .lock()
            .unwrap()
            .push(("server", event.pos, event.entity.id().to_owned(), refused));
    }
}

pub(crate) struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let kind = r.block_entity_type("pot").blocks(&[POT]).build();
        r.block_entity_part(&kind, Pot).unwrap();
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(POT, "minecraft:stone"))
            .unwrap();
    }
}

pub(crate) fn request(world: &Path, create: bool) -> WorldRequest {
    let rules = std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
    let options = sunrise_sim::world_options::WorldOptions {
        world_type: sunrise_sim::save::WorldPreset::Flat,
        rules: sunrise_game_data::gamerules::GameRules::with_registry(rules.clone(), &[]),
        ..Default::default()
    };
    let mut request = WorldRequest::new(
        "unload",
        Some("unload"),
        Some(1),
        true,
        create.then_some(&options),
        rules,
    )
    .unwrap();
    request.save_root = world.to_owned();
    request
}

/// The prepared resources with the installed mods' content.
pub(crate) fn prepared_root() -> std::path::PathBuf {
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf()
}

/// Runs the game's streaming until chunk `CHUNK` is resident (`true`) or gone, and
/// nothing is left on its way (a ticket removed mid-generation is another matter).
pub(crate) fn pump_until(game: &mut sunrise_gameplay::game::ServerGame<'_>, resident: bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while game.world.chunks.contains_key(&CHUNK) != resident || game.server.streaming.busy() {
        assert!(Instant::now() < deadline, "chunk {CHUNK:?} did not change");
        game.server.streaming.wait(Duration::from_millis(5));
        game.side_update_world();
    }
}

/// A client's copies: a column the server forgets tells its mod block entities, while
/// the client level going (a respawn, a dimension change, leaving) tells none.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_tells_a_forgotten_column_but_not_its_level_going() {
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_items::items::ItemStack;
    use sunrise_level::{
        chunk::Chunk,
        containers::{ModBlockEntity, ModBlockEntityFields},
    };
    use sunrise_protocol::connection::Pipe;
    if run_fresh_cases(
        "cut_layout::game::native_hooks::on_chunk_unloaded_tests::a_client_tells_a_forgotten_column_but_not_its_level_going",
        "SUNRISE_ON_CHUNK_UNLOADED_CLIENT_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("unload_probe", Box::new(Probe))].into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    with_game_role_client(prepared_root(), ExecutionRole::Client, |game| {
        game.options.dedicated = None;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "UnloadProbe").unwrap();
        game.client.remote.as_mut().unwrap().logged_in = true;
        let other = [1620, 64, 1600];
        for (x, z) in [CHUNK, (101, 100)] {
            let (common, client) =
                sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
            client
                .streaming
                .insert_remote(&mut common.world, Chunk::empty(x, z));
        }
        for [x, y, z] in POTS.into_iter().chain([other]) {
            let pot = ModBlockEntity::new(
                POT.into(),
                ModBlockEntityFields {
                    nbt: Compound::from([("unload_witness".into(), Tag::Int(x))]),
                    items: vec![Some(ItemStack::new("unload_probe:unregistered", 5))],
                    ..Default::default()
                },
            );
            game.world
                .block_entities
                .insert((x, y, z), BlockEntity::Mod(Box::new(pot)));
        }
        let [x, y, z] = CHEST;
        game.world
            .block_entities
            .insert((x, y, z), BlockEntity::Chest(vec![]));

        game.forget_remote_columns(&mut [CHUNK].into_iter().collect(), true);
        let mut calls = std::mem::take(&mut *CALLS.lock().unwrap());
        calls.sort();
        assert_eq!(
            calls,
            POTS.map(|pos| ("client", pos, POT.to_owned(), true))
                .to_vec()
        );

        game.forget_all_remote_columns();
        assert!(game.world.block_entities.is_empty());
        assert!(CALLS.lock().unwrap().is_empty(), "told when the level went");
    });
}

use sunrise_gameplay_common::game::side_ops::events::EventsSideGameExt as _;
