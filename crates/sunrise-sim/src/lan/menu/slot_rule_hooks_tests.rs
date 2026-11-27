//! `menu::SlotRules` in a live server menu: a `/reload` that changes a mod's
//! setting moves the placement and stacking answers of `lan::menu::Menu`, and
//! the loop hooks read the same load context. Moved out of `menu.rs` so the
//! menu model does not have to name the server side of the house.
use super::{Menu, Slots, asks_still_valid, tests::guest};
use crate::items::ItemStack;
use crate::menu::{Input, Kind, tests::bare_env_in};
use std::sync::Arc;
use sunrise_world::native_load_context::{
    DataLoadOrigin, LoadContext, LoadInputs, LoopContext, LoopSide,
};

#[test]
fn installed_slot_rules_change_mod_menu_placement() {
    use crate::native_client_hooks::test_hooks::{JUMPS, STILL_VALID_ASKED};
    crate::native_client_hooks::test_hooks::install();
    JUMPS.set(0);
    STILL_VALID_ASKED.set(0);
    let kind = Kind::Mod(Box::leak(Box::new(crate::containers::ModMenuType {
        id: "test:rules",
        owner: "test",
        slots: 1,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![],
        inputs: Vec::new(),
        links: None,
        crafting: None,
    })));
    let mut menu = crate::menu::Menu::new(kind);
    menu.set_registry_id(Some("test:rules"));
    let stack = ItemStack::new("minecraft:stone", 1);
    // A `/reload` builds the next context from the settings then on disk; the
    // next call sees it, and the one before it keeps its own.
    let settings = std::env::temp_dir().join(format!("sunrise-rules-{}", std::process::id()));
    let file = settings.join("rules_mod/stack");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let inputs = Arc::new(LoadInputs::new(Default::default(), settings.clone()));
    let load = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, []);
    let before = load.loop_context(LoopSide::Server, []);
    assert_eq!(before.setting("rules_mod", "stack"), None);
    bare_env_in(&before, |env| {
        assert_eq!(menu.slot_rules(env, 0, &stack), (true, true, 7));
    });
    std::fs::write(&file, "3").unwrap();
    let after = load.loop_context(LoopSide::Server, ["rules_mod"]);
    bare_env_in(&after, |env| {
        assert_eq!(menu.slot_rules(env, 0, &stack), (true, true, 3));
        // Slot 28 is hotbar 0 in this one-slot mod menu; both indices
        // refer to inventory cell 0 and the rule caps that cell at three.
        menu.set_player_item(0, Some(ItemStack::new("minecraft:stone", 8)));
        menu.click(env, 28, 0, Input::Swap).unwrap();
        assert_eq!(menu.player_item(0).unwrap().count, 8);
        assert_eq!(
            (0..menu.slot_count())
                .filter_map(|i| menu.slot(i))
                .map(|s| s.count)
                .sum::<u32>(),
            8
        );
        // The container's index 0 is distinct, despite the same index.
        menu.load_slot(0, Some(ItemStack::new("minecraft:dirt", 2)));
        menu.click(env, 0, 0, Input::Swap).unwrap();
        assert_eq!(menu.slot(0).unwrap().count, 3);
        assert_eq!(menu.player_item(0).unwrap().count, 5);
        assert_eq!(
            (0..menu.slot_count())
                .filter_map(|i| menu.slot(i))
                .map(|s| s.count)
                .sum::<u32>(),
            10
        );
    });
    bare_env_in(&before, |env| {
        assert_eq!(menu.slot_rules(env, 0, &stack), (true, true, 7));
    });
    // The other loop hooks read the same context: before the reload the
    // setting is missing, after it every one sees it.
    let name = |context: &LoopContext| {
        let mut displayname = crate::text::Text::literal("Alex");
        crate::lan::hub::native_player_name_format(
            context,
            &crate::text::Text::literal("Alex"),
            &mut displayname,
        );
        displayname.plain()
    };
    let tab_list = |context: &LoopContext| {
        let mut display_name = None;
        crate::lan::hub::native_player_tab_list_name_format(context, 1, "Alex", &mut display_name);
        display_name.map(|text| text.plain())
    };
    assert_eq!(name(&before), "Alex");
    assert_eq!(tab_list(&before), None);
    assert_eq!(name(&after), "3");
    assert_eq!(tab_list(&after), Some("3".to_owned()));
    // The motion value hooks read it too, and change nothing without it.
    let cow = crate::mobs::Mob::new(
        1,
        crate::mobs::sensor_scan_tests::kind(),
        sunrise_core::geometry::Vec3::ZERO,
        0.0,
    );
    let values = |context: &LoopContext| {
        let (mut jump, mut speed, mut flying) = (0.5, 0.1, 0.02);
        let cow = crate::health::LivingLend::mob(&cow, &|| None);
        crate::motion::native_living_jump_power(context, cow, &mut jump);
        crate::motion::native_living_speed(context, cow, &mut speed);
        crate::motion::native_living_flying_speed(context, cow, &mut flying);
        (jump, speed, flying)
    };
    assert_eq!(values(&before), (0.5, 0.1, 0.02));
    assert_eq!(values(&after), (1.5, 2.1, 3.02));
    // The block factors a part edits read it too, and are as the block's own without it.
    let factors = |context: &LoopContext| {
        use sunrise_world::movement_properties::BlockFactorEdits;
        let edits = crate::motion::ModBlockFactors(context);
        let state = sunrise_world::registry::BlockStateId(0);
        (edits.jump(state, 0.5), edits.speed(state, 0.4))
    };
    assert_eq!(factors(&before), (0.5, 0.4));
    assert_eq!(factors(&after), (4.5, 5.4));
    // The ladder test of a block reads it too.
    let ladder = |context: &LoopContext| {
        let mut ladder = false;
        crate::motion::native_is_ladder(
            context,
            1,
            sunrise_core::position::BlockPos::new(0, 0, 0),
            sunrise_world::registry::BlockStateId(0),
            &mut ladder,
        );
        ladder
    };
    assert!(!ladder(&before));
    assert!(ladder(&after));
    // So does the block under an open trapdoor.
    let trapdoor_ladder = |context: &LoopContext| {
        let mut climbable = false;
        crate::motion::ladder::native_makes_open_trapdoor_above_climbable(
            context,
            sunrise_world::registry::BlockStateId(0),
            sunrise_core::position::BlockPos::new(0, 0, 0),
            sunrise_world::registry::BlockStateId(0),
            &mut climbable,
        );
        climbable
    };
    assert!(!trapdoor_ladder(&before));
    assert!(trapdoor_ladder(&after));
    // A block's friction reads it too; an answer that is NaN keeps the block's own
    // and a finite one is used as given.
    let friction = |context: &LoopContext, kind: &str| {
        use sunrise_world::registry::BlockStateId;
        let world = crate::world::World::new(BlockStateId(0), 0);
        crate::motion::read_friction(
            crate::motion::FrictionLevel {
                context,
                world: &world,
                registry: crate::test_fixtures::air_registry(),
                side: crate::motion::dry_side(),
                coefficient: &|_| Some(0.6),
            },
            Some((1, kind)),
            sunrise_core::position::BlockPos::new(0, 0, 0),
            BlockStateId(0),
            0.6,
        )
        .unwrap()
    };
    assert_eq!(friction(&before, "minecraft:player"), 0.6);
    assert_eq!(friction(&after, "minecraft:player"), 0.98);
    assert_eq!(friction(&after, "minecraft:cow"), 0.6);
    assert_eq!(friction(&after, "minecraft:item"), 3.0);
    // The edge backoff and the fluid current read it too.
    let movement = |context: &LoopContext| {
        let (mut staying, mut flow) = (false, [0.25, 0.0, 0.0]);
        crate::motion::native_player_stays_on_ground_surface(context, 1, &mut staying);
        crate::motion::native_fluid_flow(
            context,
            "minecraft:water",
            sunrise_core::position::BlockPos::new(0, 0, 0),
            &mut flow,
        );
        (staying, flow[0])
    };
    assert_eq!(movement(&before), (false, 0.25));
    assert_eq!(movement(&after), (true, 1.25));
    // A block's sound type reads it too, with the entity and the cause it was asked for.
    let key = |name| mod_api::SoundEventKey::parse(name).unwrap();
    let stone = mod_api::BlockSoundType {
        identity: mod_api::BlockSoundTypeIdentity::Other,
        volume: 1.0,
        pitch: 1.0,
        break_sound: key("block.stone.break"),
        step: key("block.stone.step"),
        place: key("block.stone.place"),
        hit: key("block.stone.hit"),
        fall: key("block.stone.fall"),
    };
    let step = |context: &LoopContext, kind: &str, cause| {
        let world = crate::world::World::new(sunrise_world::registry::BlockStateId(0), 0);
        let answer = crate::block_sounds::native_sound_type(
            crate::motion::FrictionLevel {
                context,
                world: &world,
                registry: crate::test_fixtures::air_registry(),
                side: crate::motion::dry_side(),
                coefficient: &|_| None,
            },
            sunrise_core::position::BlockPos::new(0, 0, 0),
            sunrise_world::registry::BlockStateId(0),
            Some((1, kind)),
            cause,
            &stone,
        );
        let sound = answer.as_ref().unwrap_or(&stone);
        (sound.step.as_str().to_owned(), sound.volume)
    };
    let (own, answered) = (
        ("minecraft:block.stone.step".to_owned(), 1.0),
        ("rules_mod:block.rules.step".to_owned(), 2.0),
    );
    use mod_api::BlockSoundCause::{Place, Step};
    assert_eq!(step(&before, "minecraft:player", Step), own);
    assert_eq!(step(&after, "minecraft:player", Step), answered);
    assert_eq!(step(&after, "minecraft:cow", Step), own);
    assert_eq!(step(&after, "minecraft:player", Place), own);
    // Fall, jump and the ender teleport read it as well.
    let landing = |context: &LoopContext| {
        let (mut distance, mut multiplier, mut cancel) = (3.0, 1.0, false);
        crate::landing::native_living_fall(
            context,
            1,
            "minecraft:cow",
            &mut distance,
            &mut multiplier,
            &mut cancel,
        );
        let (mut target, mut teleport) = ([1.0, 2.0, 3.0], false);
        crate::mobs::enderman::native_ender_entity_teleport(
            context,
            1,
            "minecraft:enderman",
            [0.0; 3],
            &mut target,
            &mut teleport,
        );
        crate::motion::native_living_jump(context, 1, "minecraft:cow", [0.0, 0.42, 0.0]);
        (distance, multiplier, cancel, target, teleport)
    };
    assert_eq!(landing(&before), (3.0, 1.0, false, [1.0, 2.0, 3.0], false));
    assert_eq!(landing(&after), (4.0, 2.0, false, [1.0, 3.0, 3.0], true));
    assert_eq!(JUMPS.get(), 2);
    // Heal and drown read it as well.
    let health = |context: &LoopContext| {
        let (mut amount, mut cancel) = (2.0, false);
        let healed_mob = crate::mobs::Mob::new(
            1,
            crate::mobs::sensor_scan_tests::kind(),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        let healed = crate::health::LivingLend::mob(&healed_mob, &|| None);
        crate::hunger::native_living_heal(context, healed, &mut amount, &mut cancel);
        let (mut drowning, mut damage, mut cancel_drown) = (false, 2.0, false);
        crate::breathing::native_living_drown(
            context,
            1,
            "minecraft:zombie",
            &mut drowning,
            &mut damage,
            &mut 8,
            &mut cancel_drown,
        );
        (amount, cancel, drowning, damage)
    };
    assert_eq!(health(&before), (2.0, false, false, 2.0));
    assert_eq!(health(&after), (3.0, true, true, 3.0));
    // A listener that sets `bubble_count` to 0: a drowning player takes the
    // damage and shows no bubbles (`drown_bubbles` sends nothing for 0).
    let gasp = crate::breathing::Breath {
        can_breathe: false,
        consume: 1,
        refill: 0,
    };
    let mut air = -19;
    let drowned =
        crate::breathing::drown(&after, 3, "minecraft:player", &mut air, 300, gasp, true).unwrap();
    assert_eq!((drowned.damage, drowned.bubbles, air), (Some(3.0), 0, 0));
    // A listener's `consume` of `i32::MAX` saturates instead of overflowing;
    // water animals at or below -20 air drown and reset their air to zero.
    assert_eq!(
        crate::breathing::apply(
            -19,
            300,
            crate::breathing::Breath {
                can_breathe: false,
                consume: i32::MAX,
                refill: 0,
            },
        ),
        (0, true)
    );
    // Water animals out of water: the mods' damage comes back and the air is zero.
    let out_of_water = |kind: &str, max: i32| {
        let mut air = -19;
        let step = crate::breathing::AirStep::own(air, max, false);
        let hurt = step
            .finish(&after, 4, kind, &mut air, step.vanilla)
            .and_then(|drowning| drowning.damage);
        (hurt, air)
    };
    assert_eq!(out_of_water("minecraft:salmon", 300), (Some(3.0), 0));
    assert_eq!(out_of_water("minecraft:dolphin", 4800), (Some(3.0), 0));
    // A server's block menu: the game's `stillValid` answer arrives first and
    // the parts on its mod menu type may change it; a vanilla menu type never
    // asks them, and a menu whose block entity is gone closes without asking.
    let mod_type: &'static crate::containers::ModMenuType =
        Box::leak(Box::new(crate::containers::ModMenuType {
            id: "test:rules",
            owner: "test",
            slots: 27,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![],
            inputs: Vec::new(),
            links: None,
            crafting: None,
        }));
    let still_valid = |context: &LoopContext,
                       mod_type: Option<&'static crate::containers::ModMenuType>,
                       registry_id: &str,
                       k: (i32, i32, i32),
                       entity: bool| {
        let mut world = crate::world::World::new(sunrise_world::registry::BlockStateId(0), 0);
        if entity {
            world
                .block_entities
                .insert(k, crate::containers::BlockEntity::Chest(vec![None; 27]));
        }
        let guest = guest();
        let menu = Menu {
            window: 1,
            kind: 2,
            pos: Some(k),
            slots: Slots::Block(k),
            size: 27,
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            reported_unsent_data_len: None,
            blockless_data: Vec::new(),
            mod_type,
            registry_id: registry_id.into(),
            craft: None,
            links: Vec::new(),
            asks_still_valid: asks_still_valid(mod_type, registry_id),
            opening_item: None,
            opener_block: None,
            removed: false,
            slot_changed_seen: None,
        };
        menu.still_valid(context, &world, &guest)
    };
    let asked = || STILL_VALID_ASKED.get();
    // The guest's eyes are at (0, 65.62, 0), its reach 4.5 plus 4: a chest 3
    // blocks away stays open, one 20 blocks away closes, and a vanilla menu
    // type asks no part either way, though one answers for it.
    assert!(asks_still_valid(Some(mod_type), "minecraft:generic_9x3"));
    let (near, far) = ((0, 64, 3), (0, 64, 20));
    assert!(still_valid(
        &after,
        None,
        "minecraft:generic_9x3",
        near,
        true
    ));
    assert!(!still_valid(
        &after,
        None,
        "minecraft:generic_9x3",
        far,
        true
    ));
    assert_eq!(asked(), 0);
    // The part keeps the far menu open, or closes the near one.
    assert!(still_valid(&after, Some(mod_type), "test:rules", far, true));
    assert!(!still_valid(
        &before,
        Some(mod_type),
        "test:rules",
        near,
        true
    ));
    assert_eq!(asked(), 2);
    // Gone: closed whatever the part would answer, and the part is not asked.
    assert!(!still_valid(
        &after,
        Some(mod_type),
        "test:rules",
        far,
        false
    ));
    assert_eq!(asked(), 2);
    // A mod menu with no backing block (`Slots::Native`) is asked too, with no position even
    // though its opener supplied a screen position, and the part's answer decides.
    let blockless = |context: &LoopContext| {
        let world = crate::world::World::new(sunrise_world::registry::BlockStateId(0), 0);
        let guest = guest();
        let menu = Menu {
            window: 1,
            kind: 2,
            pos: Some(far),
            slots: Slots::Native,
            size: 27,
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            reported_unsent_data_len: None,
            blockless_data: Vec::new(),
            mod_type: Some(mod_type),
            registry_id: "test:rules".into(),
            craft: None,
            links: Vec::new(),
            asks_still_valid: asks_still_valid(Some(mod_type), "test:rules"),
            opening_item: None,
            opener_block: None,
            removed: false,
            slot_changed_seen: None,
        };
        menu.still_valid(context, &world, &guest)
    };
    assert!(blockless(&after));
    assert!(!blockless(&before));
    assert_eq!(asked(), 4);
    assert_eq!(
        crate::native_client_hooks::test_hooks::STILL_VALID_POS.get(),
        None
    );
    std::fs::remove_dir_all(&settings).unwrap();
}
