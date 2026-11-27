//! The adventure-mode gates on a right click: `TntBlock.prime` refuses to
//! light TNT for a player whose held stack's `can_break` does not name it,
//! and `ItemStack.useOn`'s `can_place_on` is what lets a placement through.
use sunrise_command::args::GameMode;
use sunrise_core::nbt::Tag;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Who;
use {sunrise_gameplay::game::game_tests::tests::server_game, sunrise_gameplay_common::game::Hit};

type Posted = (&'static str, [f64; 3], Option<Who>);

/// A server's game with nobody playing, or `None` without prepared resources.
fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> Option<sunrise_gameplay::game::ServerGame<'a>> {
    std::env::var_os("SUNRISE_TEST_RESOURCES")?;
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.player = None;
    Some(game)
}

/// A client's game with nobody playing, or `None` without prepared resources.
#[cfg(feature = "client")]
fn client_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> Option<sunrise_client_gameplay::game::ClientGame<'a>> {
    use {
        crate::game::load_resources, crate::settings::Settings, sunrise_client_lib::audio::Audio,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role,
    };
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::Client;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        res,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root,
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
    );
    game.player = None;
    Some(game)
}

/// `stack` with a `can_break` or `can_place_on` component naming `block`.
fn stack_with(name: &str, component: &str, block: &str) -> ItemStack {
    let mut stack = ItemStack::new(name, 1);
    stack.data_mut().set_other(
        component,
        Some(Tag::compound([(
            "blocks",
            Tag::List(vec![Tag::String(block.into())]),
        )])),
    );
    stack
}

/// A click on the block at `pos` from above, at the centre of its top face.
fn hit(pos: [i32; 3]) -> Hit {
    Hit {
        pos,
        face: [0, 1, 0],
        point: [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ],
        inside: false,
    }
}

/// An adventure player right-clicks the block at `pos` with what is in hand,
/// as `Minecraft.startUseItem` does; whether the click consumed the action and
/// the game events the click posted.
fn click(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) -> (bool, Vec<Posted>) {
    let hit = hit(pos);
    game.server.forced_hit = Some(hit);
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| *events.borrow_mut() = Some(Vec::new()));
    let consumed = game.with_player_actor(|g| g.use_hand(Some(&hit))).unwrap();
    game.server.forced_hit = None;
    let events = sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|events| events.borrow_mut().take().unwrap());
    (consumed, events)
}

/// `TntBlock.prime`: an adventure player lights TNT only with a stack whose
/// `can_break` names it. A refused click is still `TntBlock.useItemOn`'s
/// SUCCESS, so nothing but the swing happens: no fuse, no sound, no wear.
#[test]
fn an_adventure_click_primes_tnt_only_with_a_can_break_that_names_it() {
    let mut resources = None;
    let Some(mut game) = game(&mut resources) else {
        return;
    };
    assert!(
        game.ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
    );
    let pos = [2, 4, 2];
    let tnt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:tnt")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], tnt).unwrap();
    game.set_mode(GameMode::Adventure);
    let air = game.res.facts().air;

    // No `can_break`: the block stays, the fuse is not lit and the flint and
    // steel is not worn.
    *game.held_mut() = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    let (consumed, events) = click(&mut game, pos);
    assert!(
        consumed,
        "TntBlock.useItemOn's SUCCESS still ends the click"
    );
    assert!(
        game.tnts.is_empty(),
        "an adventure player without can_break primes nothing"
    );
    assert_eq!(game.world.block(pos[0], pos[1], pos[2]), tnt);
    assert_eq!(game.held().map(|s| s.damage), Some(0));
    assert!(events.is_empty(), "no game event, got {events:?}");

    // A `can_break` naming TNT lets the same click prime it: the block goes,
    // the fuse event names the player, and the flint and steel wears by one.
    *game.held_mut() = Some(stack_with(
        "minecraft:flint_and_steel",
        "minecraft:can_break",
        "minecraft:tnt",
    ));
    let events = click(&mut game, pos).1;
    assert_eq!(game.tnts.len(), 1);
    assert_eq!(game.world.block(pos[0], pos[1], pos[2]), air);
    assert_eq!(game.held().map(|s| s.damage), Some(1));
    assert_eq!(
        events,
        vec![("minecraft:prime_fuse", [2.5, 4.5, 2.5], Some(Who::Player))]
    );
}

/// `ItemStack.useOn` passes in adventure mode unless the stack's
/// `can_place_on` names the clicked block, so only then does the client run
/// `BlockItem.place` and predict where the block goes.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn an_adventure_placement_is_predicted_when_can_place_on_names_the_clicked_block() {
    let mut resources = None;
    let Some(mut game) = client_game(&mut resources) else {
        return;
    };
    let pos = [2, 4, 2];
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    game.common
        .world
        .set(pos[0], pos[1] + 1, pos[2], game.common.res.facts().air)
        .unwrap();
    game.mode = GameMode::Adventure;
    let hit = hit(pos);
    // No `can_place_on`, and one that names another block: nothing to predict.
    for stack in [
        ItemStack::new("minecraft:stone", 1),
        stack_with(
            "minecraft:stone",
            "minecraft:can_place_on",
            "minecraft:dirt",
        ),
    ] {
        *game.held_mut() = Some(stack.clone());
        assert!(game.remote_can_place(&hit, &stack).is_none());
    }
    // One that names the clicked stone: the placement on top of it is the
    // client's own.
    let allowed = stack_with(
        "minecraft:stone",
        "minecraft:can_place_on",
        "minecraft:stone",
    );
    *game.held_mut() = Some(allowed.clone());
    let preview = game
        .remote_can_place(&hit, &allowed)
        .expect("an allowed placement is predicted");
    assert_eq!([preview.pos.x, preview.pos.y, preview.pos.z], [2, 5, 2]);
    assert_eq!(
        game.world.block(pos[0], pos[1] + 1, pos[2]),
        game.res.facts().air,
        "the prediction is dry"
    );
    // `ItemStack.useOn` tests the stack the use names, so the other hand's
    // component neither allows nor refuses it.
    *game.held_mut() = Some(ItemStack::new("minecraft:stone", 1));
    assert!(game.remote_can_place(&hit, &allowed).is_some());
    assert!(
        game.remote_can_place(&hit, &ItemStack::new("minecraft:stone", 1))
            .is_none()
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
