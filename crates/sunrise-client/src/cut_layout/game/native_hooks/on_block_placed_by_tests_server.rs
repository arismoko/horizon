use std::sync::{Arc, atomic::Ordering};
use sunrise_core::{
    geometry::{Direction, Vec3},
    nbt::Tag,
};
use sunrise_gameplay::game::native_hooks::on_block_placed_by_tests::CRATE;
use sunrise_gameplay::game::native_hooks::on_block_placed_by_tests::Notice;
use sunrise_gameplay::game::native_hooks::on_block_placed_by_tests::Probe;
use sunrise_gameplay::game::native_hooks::on_block_placed_by_tests::SKULL;
use sunrise_gameplay::game::native_hooks::on_block_placed_by_tests::crate_stack;
use sunrise_gameplay::game::native_hooks::on_block_placed_by_tests::shulker_stack;
use sunrise_items::items::ItemStack;
use sunrise_level::{chunk::Chunk, containers::BlockEntity};
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};
use {sunrise_gameplay_common::game::GuestUse, sunrise_gameplay_common::game::Hit};

#[test]
#[ignore = "daily"]
fn block_items_notify_after_setup_and_before_consumption() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_hooks_server::on_block_placed_by_tests::block_items_notify_after_setup_and_before_consumption",
        "SUNRISE_BLOCK_ITEM_PLACED_CHILD",
        &["game"],
    ) {
        return;
    }
    let probe = Probe::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placed_probe",
        Box::new(probe.clone()),
    )]
    .into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let air = game.res.facts().air;
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
        bind_local_player(game, 20, 123);
        game.server.lan_acting_name = Some("Placer".into());
        game.server.lan_acting_permission = Some(0);
        game.creative = false;
        game.yaw = 0.0;

        // The global veto runs before placement; neither the block nor its Part's write exists.
        probe.cancel.store(true, Ordering::SeqCst);
        place(game, [2, 64, 2], crate_stack());
        assert!(probe.notices.lock().unwrap().is_empty());
        assert_eq!(game.world.block(2, 64, 2), air);
        assert!(game.world.block_entities.get(&(2, 64, 2)).is_none());
        assert_eq!(game.held().unwrap().count, 3);

        probe.cancel.store(false, Ordering::SeqCst);
        place(game, [2, 64, 2], crate_stack());
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(2, 64, 2)) else {
            panic!("placed mod entity missing");
        };
        assert_eq!(
            entity.nbt.get(&"CustomName".into()),
            Some(&Tag::String("Crate name".into()))
        );
        assert_eq!(entity.nbt.get(&"PlacedByPart".into()), Some(&Tag::Byte(1)));
        assert_eq!(game.held().unwrap().count, 2);

        // A refused bed (blocked head) never notifies, while a successful bed notifies only its foot.
        game.world.set(8, 64, 3, stone).unwrap();
        place(game, [8, 64, 2], ItemStack::new("minecraft:red_bed", 2));
        assert_eq!(game.world.block(8, 64, 2), air);
        assert_eq!(game.held().unwrap().count, 2);
        assert_eq!(probe.notices.lock().unwrap().len(), 1);
        place(game, [6, 64, 2], ItemStack::new("minecraft:red_bed", 2));
        assert_eq!(game.held().unwrap().count, 1);

        // The dispenser enters the same BlockItem placement path with no player and sees the
        // component-adjusted state, loaded block entity and full original stack before shrink.
        let left = game
            .dispense_item((9, 64, 6), Direction::East, shulker_stack())
            .unwrap()
            .continue_value()
            .unwrap()
            .unwrap();
        assert_eq!(left.count, 3);
        let notices = probe.notices.lock().unwrap();
        assert_eq!(
            *notices,
            [
                Notice {
                    item: CRATE.into(),
                    count: 3,
                    placer: Some(123),
                    pos: [2, 64, 2],
                    face: mod_api::BlockFace::Up,
                    click: [2.25, 64.0, 2.75]
                },
                Notice {
                    item: "minecraft:red_bed".into(),
                    count: 2,
                    placer: Some(123),
                    pos: [6, 64, 2],
                    face: mod_api::BlockFace::Up,
                    click: [6.25, 64.0, 2.75]
                },
                Notice {
                    item: "minecraft:shulker_box".into(),
                    count: 4,
                    placer: None,
                    pos: [10, 64, 6],
                    face: mod_api::BlockFace::Up,
                    click: [10.5, 64.0, 6.5]
                },
            ]
        );
        drop(notices);

        probe.cancel.store(true, Ordering::SeqCst);
        let left = game
            .dispense_item((9, 64, 8), Direction::East, shulker_stack())
            .unwrap()
            .continue_value()
            .unwrap()
            .unwrap();
        assert_eq!(left.count, 4);
        assert_eq!(game.world.block(10, 64, 8), air);
        probe.cancel.store(false, Ordering::SeqCst);
        game.world.set(10, 64, 10, stone).unwrap();
        let left = game
            .dispense_item((9, 64, 10), Direction::East, shulker_stack())
            .unwrap()
            .continue_value()
            .unwrap()
            .unwrap();
        assert_eq!(left.count, 4);
        assert_eq!(game.world.block(10, 64, 10), stone);
        // An unrelated item still places normally without dispatching any Part.
        place(game, [4, 64, 2], ItemStack::new("minecraft:stone", 2));
        assert_eq!(game.world.block(4, 64, 2), stone);
        assert_eq!(game.held().unwrap().count, 1);
        assert_eq!(probe.notices.lock().unwrap().len(), 3);

        // Completing a wither clears its pattern in setPlacedBy. Ordinary
        // placement effects must not drain with those callback effects.
        game.ui.difficulty = 2;
        let skull = game.pack.assets.registry.default_state(SKULL).unwrap();
        let soul = game
            .pack
            .assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        for (x, y, state) in [
            (9, 66, skull),
            (11, 66, skull),
            (9, 65, soul),
            (10, 65, soul),
            (11, 65, soul),
            (10, 64, soul),
        ] {
            game.world.set(x, y, 12, state).unwrap();
        }
        let before = game.mobs.len();
        sunrise_gameplay::game::dispense_fixture::start_recording();
        place(game, [10, 66, 12], ItemStack::new(SKULL, 1));
        let effects = sunrise_gameplay::game::dispense_fixture::stop_recording();
        let cleared = effects
            .iter()
            .rposition(|line| line.starts_with("L 2001 "))
            .unwrap();
        let notice = effects
            .iter()
            .position(|line| line == "item notice")
            .unwrap();
        let placed = effects
            .iter()
            .position(|line| line == "G 10,66,12 minecraft:block_place")
            .unwrap();
        assert!(cleared < notice && notice < placed, "{effects:?}");
        assert_eq!(game.mobs.len(), before + 1);
        assert_eq!(game.held(), None);
    });
}

pub(crate) fn place(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: [i32; 3],
    stack: ItemStack,
) {
    *game.held_mut() = Some(stack);
    game.server.guest_use = Some(GuestUse::on_block(mod_api::Hand::Main));
    let [x, y, z] = pos;
    let hit = Hit {
        pos: [x, y - 1, z],
        face: [0, 1, 0],
        point: [f64::from(x) + 0.25, f64::from(y), f64::from(z) + 0.75],
        inside: false,
    };
    assert!(game.use_hand(Some(&hit)).unwrap());
}
