//! Block parts answer the enchanting power an enchanting table counts (`enchant_power_bonus`)
//! and the colour a beacon beam takes (`beacon_color_multiplier`).
use mod_api::{
    BlockPart, EnchantPowerBonusEvent, Hooks, Host, LevelReadError, ModEntry, Mods, Registrar,
    RegistrationContext,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::beacon_common::BeaconCommonGameExt1;
use sunrise_level::chunk::Chunk;
use {
    crate::game::server_side::native_entity_refs::tests::with_game_role_client,
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
    sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

const TABLE: [i32; 3] = [8, 64, 8];
const PROVIDER: [i32; 3] = [12, 64, 8];
const UNRELATED_TABLE: [i32; 3] = [9, 64, 8];
const POWER_NAN: f32 = f32::from_bits(0x7fc0_1234);

#[derive(Clone, Default)]
struct QueryProbe {
    calls: Arc<AtomicUsize>,
    cycles: bool,
}

#[mod_api::sunrise_part]
impl BlockPart for QueryProbe {
    fn enchant_power_bonus(&self, host: &dyn Host, event: &mut EnchantPowerBonusEvent) {
        let Some(level) = event.level else { return };
        self.calls.fetch_add(1, Ordering::Relaxed);
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let table = [event.table_pos.x, event.table_pos.y, event.table_pos.z];
        let bookshelf = host.block_state_id("minecraft:bookshelf").unwrap();
        if event.state == bookshelf {
            assert_eq!(event.power, 1.0);
            event.power = if table == TABLE {
                f32::INFINITY
            } else {
                POWER_NAN
            };
            return;
        }
        let stone = host.block_state_id("minecraft:stone").unwrap();
        let contextual_power = if level.block_state(host, pos) == Some(stone) {
            -0.5
        } else {
            1.25
        };
        let material = host.block_state_id("minecraft:gold_block").unwrap();
        if event.state == material {
            event.power = contextual_power;
            return;
        }
        assert_eq!(pos, PROVIDER);
        assert_eq!(table, UNRELATED_TABLE);
        assert_eq!(event.state, host.block_state_id("minecraft:glass").unwrap());
        assert_eq!(event.power, 0.0);
        event.power = if self.cycles {
            assert_eq!(
                level.get_enchant_power_bonus(
                    host,
                    [pos[0] + 1, pos[1], pos[2]],
                    event.state,
                    TABLE
                ),
                Err(LevelReadError::Unavailable)
            );
            level
                .get_enchant_power_bonus(host, pos, material, table)
                .unwrap()
        } else {
            contextual_power
        };
    }
}

#[mod_api::__subscriptions]
impl Hooks for QueryProbe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for name in ["minecraft:glass", "minecraft:gold_block"] {
            let key = r.block_key(name);
            r.block_part(key, self.clone());
        }
        if self.cycles {
            let bookshelf = r.block_key("minecraft:bookshelf");
            r.block_part(bookshelf, self.clone());
        }
    }
}

fn with_query_probe(
    name: &str,
    role: ExecutionRole,
    cycles: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &AtomicUsize),
) {
    if run_fresh_cases(name, "SUNRISE_ENCHANT_QUERY_CHILD", &["query"]) {
        return;
    }
    let probe = QueryProbe {
        cycles,
        ..Default::default()
    };
    let calls = probe.calls.clone();
    let mods: Mods = vec![ModEntry::new("power_query", Box::new(probe))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game_role(fixture_root(), role, |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game, &calls);
        });
    });
}

fn with_query_probe_client(
    name: &str,
    role: ExecutionRole,
    cycles: bool,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &AtomicUsize),
) {
    if run_fresh_cases(name, "SUNRISE_ENCHANT_QUERY_CHILD", &["query"]) {
        return;
    }
    let probe = QueryProbe {
        cycles,
        ..Default::default()
    };
    let calls = probe.calls.clone();
    let mods: Mods = vec![ModEntry::new("power_query", Box::new(probe))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game_role_client(fixture_root(), role, |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game, &calls);
        });
    });
}

#[test]
#[ignore = "daily"]
fn enchant_power_query_uses_the_supplied_states_tag_default() {
    with_query_probe(
        "cut_layout::game::client_side::enchant_power_beam_tests::enchant_power_query_uses_the_supplied_states_tag_default",
        ExecutionRole::IntegratedServer,
        false,
        |game, calls| {
            place(game, [4, 0, 0], "minecraft:gold_block");
            let level = game.level("minecraft:overworld").unwrap();
            let host: &dyn Host = game;
            for (name, expected) in [("minecraft:bookshelf", 1.0), ("minecraft:stone", 0.0)] {
                let candidate = host.block_state_id(name).unwrap();
                assert_eq!(
                    level.get_enchant_power_bonus(host, PROVIDER, candidate, TABLE),
                    Ok(expected)
                );
            }
            assert_eq!(calls.load(Ordering::Relaxed), 0);
        },
    );
}

#[test]
#[ignore = "daily"]
fn enchant_power_query_reads_the_named_server_level_and_refuses_cycles() {
    with_query_probe(
        "cut_layout::game::client_side::enchant_power_beam_tests::enchant_power_query_reads_the_named_server_level_and_refuses_cycles",
        ExecutionRole::IntegratedServer,
        true,
        |game, calls| {
            place(game, [4, 0, 0], "minecraft:stone");
            place(game, [2, 0, 0], "minecraft:stone");
            let overworld = game.level("minecraft:overworld").unwrap();
            assert_eq!(game.enter_command_level(Dim::Nether), Some(Dim::Overworld));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            place(game, [4, 0, 0], "minecraft:gold_block");
            let nether = game.level("minecraft:the_nether").unwrap();
            let host: &dyn Host = game;
            let glass = host.block_state_id("minecraft:glass").unwrap();
            assert_eq!(
                overworld.get_enchant_power_bonus(host, PROVIDER, glass, UNRELATED_TABLE),
                Ok(-0.5)
            );
            assert_eq!(
                nether.get_enchant_power_bonus(host, PROVIDER, glass, UNRELATED_TABLE),
                Ok(1.25)
            );
            assert_eq!(
                calls.load(Ordering::Relaxed),
                4,
                "each glass provider delegated to the material exactly once"
            );
            let bookshelf = host.block_state_id("minecraft:bookshelf").unwrap();
            assert_eq!(
                nether
                    .get_enchant_power_bonus(host, PROVIDER, bookshelf, UNRELATED_TABLE)
                    .unwrap()
                    .to_bits(),
                POWER_NAN.to_bits()
            );
            assert_eq!(
                nether.get_enchant_power_bonus(host, PROVIDER, bookshelf, TABLE),
                Ok(f32::INFINITY)
            );
            place(game, [2, 0, 0], "minecraft:bookshelf");
            assert_eq!(
                game.count_bookshelves(TABLE),
                1,
                "offers keep the tag default"
            );
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            assert_eq!(
                nether.get_enchant_power_bonus(game, PROVIDER, glass, UNRELATED_TABLE),
                Err(LevelReadError::Stale)
            );
            assert_eq!(calls.load(Ordering::Relaxed), 7);
        },
    );
}

#[test]
#[ignore = "daily"]
fn enchant_power_query_reads_the_logged_in_client_level() {
    with_query_probe_client(
        "cut_layout::game::client_side::enchant_power_beam_tests::enchant_power_query_reads_the_logged_in_client_level",
        ExecutionRole::Client,
        false,
        |game, calls| {
            let _server = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
            game.native_lifecycle.begin_open(None);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            place(game, [4, 0, 0], "minecraft:stone");
            let level = game.level("minecraft:overworld").unwrap();
            let host: &dyn Host = game;
            let glass = host.block_state_id("minecraft:glass").unwrap();
            assert_eq!(
                level.get_enchant_power_bonus(host, PROVIDER, glass, UNRELATED_TABLE),
                Ok(-0.5)
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        },
    );
}

pub(crate) fn place<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &mut G,
    offset: [i32; 3],
    block: &str,
) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let [x, y, z] = [0, 1, 2].map(|i| TABLE[i] + offset[i]);
    game.world.set(x, y, z, state).unwrap();
}

#[test]
#[ignore = "daily"]
fn block_parts_answer_the_enchanting_power_and_the_beam_colour() {
    if run_fresh_cases(
        "cut_layout::game::client_side::enchant_power_beam_tests::block_parts_answer_the_enchanting_power_and_the_beam_colour",
        "SUNRISE_ENCHANT_OFFER_CHILD",
        &["offers"],
    ) {
        return;
    }
    // The fixture's parts: glass is worth two bookshelves and tints a beam red, and a
    // bookshelf is worth nothing.
    mod_platform::__private::part_tests::enchant_power_and_beam_game_thread();
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        // A bookshelf whose part answers 0 is not counted.
        place(game, [-2, 0, -2], "minecraft:bookshelf");
        assert_eq!(game.count_bookshelves(TABLE), 0);
        // Glass answering 2 counts as two bookshelves.
        place(game, [-2, 0, -1], "minecraft:glass");
        assert_eq!(game.count_bookshelves(TABLE), 2);
        // Unless the block between it and the table does not transmit.
        place(game, [-1, 0, 0], "minecraft:stone");
        assert_eq!(game.count_bookshelves(TABLE), 0);

        let registry = game.pack.assets.registry;
        let color = |game: &sunrise_gameplay::game::ServerGame<'_>, block: &str| {
            let state = registry.default_state(block).unwrap();
            game.beacon_color(state, block, [8, 65, 8], [8, 64, 8])
        };
        // Glass gives no colour of its own; its part tints the beam red, alpha dropped.
        assert_eq!(color(game, "minecraft:glass"), Some(0xFF_0000));
        // Stained glass without a part keeps its dye colour, and stone none.
        assert_eq!(color(game, "minecraft:red_stained_glass"), Some(0xB0_2E26));
        assert_eq!(color(game, "minecraft:stone"), None);
    });
}

/// Drive the public worker entry, observing real particle requests rather than cache bits.
pub(crate) fn enchant_particles(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> usize {
    use sunrise_client::{
        animate_tick::{AnimateContext, animate_tick},
        particles::ParticleEngine,
    };
    use sunrise_level::particle_options::ParticleType;
    let mut engine = ParticleEngine::new(
        &sunrise_assets::resource_source::ResourceSource::directory(game.options.resources.clone()),
        &Default::default(),
        &Default::default(),
    );
    engine.seed_randoms(5, 9, 0);
    engine.camera = [8.5, 65.0, 8.5];
    engine.created_requests = Some(Vec::new());
    let ctx = AnimateContext {
        enchant_power: &game.client.ambience.enchant_power,
        world: &game.world,
        facts: game.res.facts(),
        collision: game.res.collision(),
        biomes: game.res.biomes(),
        registry: game.pack.assets.registry,
        raining_at: &|_| false,
        height_no_leaves: &|_| 0,
        firefly_bush_sounds: false,
        creaking_active: false,
        sky_darken: 0,
        marker: None,
        ambient_particles: None,
        model_offset: &|_, _, _| None,
        gateway_particles: &|_, _| 0,
        thundering: false,
        ultrawarm: false,
        block_parts: None,
    };
    for _ in 0..4096 {
        animate_tick(game.client_res().animate(), &mut engine, &ctx, TABLE);
    }
    engine
        .created_requests
        .take()
        .unwrap()
        .iter()
        .filter(|(options, ..)| options.ty == ParticleType::Enchant)
        .count()
}

#[test]
#[ignore = "daily"]
fn enchant_particles_share_native_power_and_rebuild_only_on_parts_reload() {
    use std::sync::atomic::AtomicU32;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;

    const CHILD: &str = "SUNRISE_ENCHANT_PARTICLES_CHILD";
    if run_fresh_cases(
        "cut_layout::game::client_side::enchant_power_beam_tests::enchant_particles_share_native_power_and_rebuild_only_on_parts_reload",
        CHILD,
        &["vanilla", "parts"],
    ) {
        return;
    }
    const CUSTOM: &str = "power_test:provider";
    static POWER: AtomicU32 = AtomicU32::new(2.0f32.to_bits());
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Provider;
    #[mod_api::sunrise_part]
    impl BlockPart for Provider {
        fn enchant_power_bonus(&self, _: &dyn Host, event: &mut EnchantPowerBonusEvent) {
            if event.pos == BlockPos::new(0, 0, 0) {
                assert_eq!(event.table_pos, BlockPos::new(0, 0, 0));
                assert_eq!(event.level, None);
            } else {
                assert_eq!(
                    event.level.unwrap().dimension(),
                    mod_api::DimensionKey::OVERWORLD
                );
                assert_eq!(event.table_pos, BlockPos::new(TABLE[0], TABLE[1], TABLE[2]));
            }
            CALLS.fetch_add(1, Ordering::Relaxed);
            event.power = f32::from_bits(POWER.load(Ordering::Relaxed));
        }
    }
    struct Suppressed;
    #[mod_api::sunrise_part]
    impl BlockPart for Suppressed {
        fn enchant_power_bonus(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::EnchantPowerBonusEvent,
        ) {
            event.power = 0.0;
        }
    }
    struct PowerMod;
    #[mod_api::__subscriptions]
    impl Hooks for PowerMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let provider = r.block_key(CUSTOM);
            r.block_part(provider, Provider);
            let bookshelf = r.block_key("minecraft:bookshelf");
            r.block_part(bookshelf, Suppressed);
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .register(mod_api::Block::new(CUSTOM, "minecraft:glass"))
                .unwrap();
        }
    }
    let parts = std::env::var(CHILD).unwrap() == "parts";
    let mods: Mods = if parts {
        vec![ModEntry::new("power_test", Box::new(PowerMod))].into()
    } else {
        Vec::new().into()
    };
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    crate::entry::install_client_hooks();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_role_client(
        root,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        |game| {
            let _server = logged_client(game);
            game.native_lifecycle.begin_open(None);
            game.resolve_parts().unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            place(game, [0, 0, 0], "minecraft:enchanting_table");
            place(game, [-2, 0, -1], "minecraft:bookshelf");
            let shelves = enchant_particles(game);
            assert_eq!(shelves > 0, !parts, "bookshelf default or explicit zero");
            assert_eq!(game.count_bookshelves(TABLE), i32::from(!parts));
            if !parts {
                place(game, [-1, 0, 0], "minecraft:stone");
                assert_eq!(enchant_particles(game), 0);
                assert_eq!(game.count_bookshelves(TABLE), 0);
                return;
            }
            place(game, [-2, 0, -1], CUSTOM);
            assert_eq!(game.count_bookshelves(TABLE), 2);
            let calls = CALLS.load(Ordering::Relaxed);
            assert!(enchant_particles(game) > 0, "custom non-tagged provider");
            assert_eq!(
                CALLS.load(Ordering::Relaxed),
                calls,
                "no per-frame dispatch"
            );
            place(game, [-1, 0, 0], "minecraft:stone");
            assert_eq!(enchant_particles(game), 0);
            assert_eq!(game.count_bookshelves(TABLE), 0);
            place(game, [-1, 0, 0], "minecraft:air");
            POWER.store(0.0f32.to_bits(), Ordering::Relaxed);
            assert!(
                enchant_particles(game) > 0,
                "old snapshot until parts reload"
            );
            assert_eq!(CALLS.load(Ordering::Relaxed), calls);
            game.resolve_parts().unwrap();
            assert_eq!(
                CALLS.load(Ordering::Relaxed),
                calls + 1,
                "one query per custom state"
            );
            assert_eq!(
                enchant_particles(game),
                0,
                "reloaded answer suppresses particles"
            );
            assert_eq!(CALLS.load(Ordering::Relaxed), calls + 1);
            POWER.store((-0.5f32).to_bits(), Ordering::Relaxed);
            game.resolve_parts().unwrap();
            assert!(
                enchant_particles(game) > 0,
                "any nonzero power, not only positive"
            );
        },
    );
}

/// A mod's scanned offsets and its `is_valid_book_shelf` answers: a bookshelf on its ring three
/// blocks out counts, a repeat of one of vanilla's offsets counts once, a `false` drops a shelf
/// vanilla counts, and a `true` counts a shelf behind a blocked midpoint.
#[test]
#[ignore = "daily"]
fn mods_add_scanned_offsets_and_decide_which_shelves_count() {
    if run_fresh_cases(
        "cut_layout::game::client_side::enchant_power_beam_tests::mods_add_scanned_offsets_and_decide_which_shelves_count",
        "SUNRISE_ENCHANT_SHELF_CHILD",
        &["shelves"],
    ) {
        return;
    }
    struct FarShelves;
    #[mod_api::__subscriptions]
    impl Hooks for FarShelves {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.add_enchanting_table_offsets(&[[3, 0, 0], [-2, 0, -2]]);
        }
        fn is_valid_book_shelf(&self, _: &dyn Host, event: &mut mod_api::IsValidBookShelfEvent) {
            match [event.offset.x, event.offset.y, event.offset.z] {
                [2, 0, 0] => event.valid = false,
                [0, 0, -2] => event.valid = true,
                _ => {}
            }
        }
    }
    let mods: Mods = vec![ModEntry::new("far_shelves", Box::new(FarShelves))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game_role(fixture_root(), ExecutionRole::IntegratedServer, |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // Beyond vanilla's ring; the block halfway, (1, 0, 0), is air.
            place(game, [3, 0, 0], "minecraft:bookshelf");
            assert_eq!(game.count_bookshelves(TABLE), 1);
            place(game, [-2, 0, -2], "minecraft:bookshelf");
            assert_eq!(game.count_bookshelves(TABLE), 2);
            place(game, [2, 0, 0], "minecraft:bookshelf");
            assert_eq!(game.count_bookshelves(TABLE), 2);
            // Vanilla refuses a shelf whose midpoint doesn't transmit; the listener accepts it.
            place(game, [0, 0, -1], "minecraft:stone");
            place(game, [0, 0, -2], "minecraft:bookshelf");
            assert_eq!(game.count_bookshelves(TABLE), 3);
        });
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::bookshelves_common::BookshelvesCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
