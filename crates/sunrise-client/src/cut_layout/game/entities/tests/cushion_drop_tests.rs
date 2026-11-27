//! `CushionItem.useOn`'s `createDefaultStackConfig` (`Entity.applyImplicitComponents`
//! copies the stack's `CUSTOM_NAME`), `Cushion.dropItem`'s suppression for a
//! player with infinite materials, and `BlockAttachedEntity.tick`'s
//! `discard()` when the support goes away (no `entity_die` game event).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::ChunkPos;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::Hit;
use sunrise_gameplay_common::game::Options;
use sunrise_gameplay_common::game::Progress;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::Who;

/// The cushion's position: `Vec3.atCenterOfWithY(4 199 4, 200.0)`.
const AT: [f64; 3] = [4.5, 200.0, 4.5];

fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    // `Cushion.dropItem` only drops under `ENTITY_DROPS`, and the blast below
    // only reaches the cushion under `MOB_GRIEFING` (both default on).
    assert!(
        game.ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
            && game
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
    );
    game
}

/// The top face of the stone block a cushion goes on.
fn top_hit() -> Hit {
    Hit {
        pos: [4, 199, 4],
        face: [0, 1, 0],
        point: AT,
        inside: false,
    }
}

/// `/give @s minecraft:white_cushion[minecraft:custom_name='"Seat"']`.
fn named_stack() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:white_cushion", 1);
    stack.data_mut().custom_name = Some(Tag::String("Seat".into()));
    stack
}

fn capture() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| *events.borrow_mut() = Some(Vec::new()));
}

fn events() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| events.borrow_mut().take().unwrap())
}

/// `CushionItem.useOn` builds the entity with `EntityType.createDefaultStackConfig`,
/// whose `applyComponentsFromItemStack` copies `CUSTOM_NAME` onto the cushion.
#[test]
#[ignore = "daily"]
fn a_placed_cushion_keeps_its_stacks_custom_name() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.creative = true;
    assert!(game.place_cushion(&top_hit(), &named_stack()));
    assert_eq!(game.cushions.len(), 1);
    assert_eq!(game.cushions[0].position, Vec3::new(AT[0], AT[1], AT[2]));
    assert_eq!(
        game.cushions[0].custom_name,
        Some(Tag::String("Seat".into())),
        "the placed cushion is named after its stack"
    );
    assert_eq!(
        game.cushions[0]
            .item_stack_with_data()
            .custom_name()
            .as_deref(),
        Some("Seat"),
        "and `getCushionItemStackWithData` hands the name back"
    );
    // An unnamed stack names nothing.
    let plain = ItemStack::new("minecraft:white_cushion", 1);
    game.cushions.clear();
    assert!(game.place_cushion(&top_hit(), &plain));
    assert_eq!(game.cushions[0].custom_name, None);
}

/// `Cushion.dropItem` drops nothing when the cause is a player with infinite
/// materials (`hasInfiniteMaterials`, the TNT's owner for a blast), and the
/// item otherwise. `BlockAttachedEntity.hurtServer` passes the damage source's
/// causing entity.
#[test]
#[ignore = "daily"]
fn a_creative_blast_leaves_no_cushion_item() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // (creative, the TNT's owner is a player, the guest behind it, drops)
    for (creative, owner_player, guest, dropped) in [
        // A creative player's TNT: no item.
        (true, true, None, 0),
        // A survival player's: one.
        (false, true, None, 1),
        // A mob's TNT (`PrimedTnt.getOwner`): not a player, so one, whatever
        // the player in place holds.
        (true, false, None, 1),
        // A creative guest's, on the guest's own turn (the hub is out of
        // `self.lan` while it acts): no item.
        (true, true, Some(7), 0),
    ] {
        game.creative = creative;
        game.cushions.clear();
        game.dropped.clear();
        assert!(game.place_cushion(&top_hit(), &named_stack()));
        // `Explosion.getIndirectSourceEntity` of a player's TNT is the player.
        game.server.combat.0 = Default::default();
        game.server.combat.0.blast_owner = Some(if owner_player {
            (1, Text::literal("Steve"))
        } else {
            (700, Text::literal("Zombie"))
        });
        game.server.combat.0.blast_owner_player = owner_player;
        game.server.combat.0.blast_guest = guest;
        game.server.lan_acting = guest;
        game.server_explode(
            [4.5, 200.5, 4.5],
            2.0,
            sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
            false,
        )
        .unwrap();
        game.server.lan_acting = None;
        assert!(game.cushions.is_empty(), "the blast breaks the cushion");
        assert_eq!(
            game.dropped.len(),
            dropped,
            "creative={creative} owner_player={owner_player} guest={guest:?}: \
             the blast's cause decides the drop"
        );
        if dropped == 1 {
            assert_eq!(
                game.dropped[0].stack.custom_name().as_deref(),
                Some("Seat"),
                "a blast that drops the cushion drops it named"
            );
        }
    }
}

/// `BlockAttachedEntity.tick`: a cushion whose support is gone is `discard()`ed
/// and then dropped with `causedBy = null`, which posts no `entity_die`;
/// `kill` (a player's break) still does.
#[test]
#[ignore = "daily"]
fn a_cushion_that_loses_its_support_is_discarded_silently() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.creative = true;
    assert!(game.place_cushion(&top_hit(), &named_stack()));
    capture();
    game.break_cushion(0, true);
    assert_eq!(
        events(),
        vec![("minecraft:entity_die", AT, Some(Who::Player))],
        "a player's break kills the cushion"
    );
    assert!(game.dropped.is_empty(), "a creative breaker gets no item");

    assert!(game.place_cushion(&top_hit(), &named_stack()));
    assert!(game.run_command("setblock 4 199 4 air strict"));
    // The next check (100 ticks) finds the anchor gone.
    game.cushions[0].check_interval = 100;
    capture();
    game.tick_cushions();
    assert!(game.cushions.is_empty(), "the cushion is gone");
    assert_eq!(events(), [], "a discarded cushion posts no game event");
    assert_eq!(game.dropped.len(), 1, "it still drops its item");
    assert_eq!(game.dropped[0].stack.custom_name().as_deref(), Some("Seat"));
}

fn doubles(v: [f64; 3]) -> Tag {
    Tag::List(v.map(|d| Tag::Double(d.to_bits())).to_vec())
}

fn floats(v: [f32; 2]) -> Tag {
    Tag::List(v.map(|f| Tag::Float(f.to_bits())).to_vec())
}

/// The cushion's entity root as the save path writes it.
fn saved_cushion(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> Tag {
    let mut chunks = game.entity_chunks(&std::collections::HashSet::from([(0, 0)]), false);
    assert_eq!(chunks.len(), 1);
    let (_, mut roots) = chunks.pop().unwrap();
    assert_eq!(roots.len(), 1);
    roots.pop().unwrap()
}

/// #2681: a supported cushion loaded with Motion and Rotation keeps them
/// through a tick and a save, and again through a reload; a Motion component
/// above 10 in magnitude loads as 0, and a freshly placed cushion saves zeros with its own yaw.
#[test]
#[ignore = "daily"]
fn a_loaded_cushion_keeps_its_motion_and_pitch_through_tick_and_reload() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let loaded = Tag::compound([
        ("id", Tag::String("minecraft:cushion".into())),
        ("Pos", doubles(AT)),
        ("block_pos", Tag::IntArray(vec![4, 199, 4])),
        ("Motion", doubles([0.5, 20.0, -0.25])),
        ("Rotation", floats([90.0, 10.0])),
    ]);
    game.load_cushion(loaded, Some([1, 2, 3, 4]));
    assert_eq!(game.cushions.len(), 1);
    // The check at 100 ticks finds the stone under it.
    for _ in 0..102 {
        game.tick_cushions();
    }
    assert_eq!(game.cushions.len(), 1, "a supported cushion stays");
    let saved = saved_cushion(&mut game);
    assert_eq!(saved.get("Motion"), Some(&doubles([0.5, 0.0, -0.25])));
    assert_eq!(saved.get("Rotation"), Some(&floats([90.0, 10.0])));

    game.cushions.clear();
    game.load_cushion(saved, Some([1, 2, 3, 4]));
    assert_eq!(game.cushions.len(), 1);
    let again = saved_cushion(&mut game);
    assert_eq!(again.get("Motion"), Some(&doubles([0.5, 0.0, -0.25])));
    assert_eq!(again.get("Rotation"), Some(&floats([90.0, 10.0])));

    game.cushions.clear();
    game.creative = true;
    assert!(game.place_cushion(&top_hit(), &named_stack()));
    let placed = saved_cushion(&mut game);
    assert_eq!(placed.get("Motion"), Some(&doubles([0.0, 0.0, 0.0])));
    assert_eq!(
        placed.get("Rotation"),
        Some(&floats([game.cushions[0].yaw, 0.0]))
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
