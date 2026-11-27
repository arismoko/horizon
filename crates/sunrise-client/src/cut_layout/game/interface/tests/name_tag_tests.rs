//! Naming with a name tag: `Mob.interact` (through
//! `checkAndHandleImportantInteractions`) and `Player.interactOn` post
//! `GameEvent.ENTITY_INTERACT` at the named entity from the player once
//! `NameTagItem` consumed the tag, and an armour stand's name follows
//! `ArmorStandRenderer.shouldShowName`, `isCustomNameVisible` alone: no team
//! visibility and no F1 (`LivingEntityRenderer` asks both, the override
//! neither).
//!
//! The two ends of a name tag: `NameTagItem.interactLivingEntity` shrinks the
//! stack it names with -- and `Player.interactOn` puts a creative player's
//! count straight back -- and `EntityAttachment.NAME_TAG` says where the tag
//! hangs, which `EntityTypes.SNIFFER` moves to 2.05 above the feet.
use crate::game::load_resources;
use std::sync::Arc;
use sunrise_client::audio::Audio;
use sunrise_client::hud::HudBatch;
use sunrise_client::settings::Settings;
use sunrise_client_gameplay::game::client_side::names::name_tag_at;
use sunrise_client_gameplay::game::client_side::resource_tables_client::UiResources;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::ChunkPos;
use sunrise_game_data::scoreboard as sb;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::InteractTarget;
use sunrise_gameplay_common::game::Options;
use sunrise_gameplay_common::game::Screen;
use sunrise_gameplay_common::game::creatures::render_fixture::mob;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::entities::ArmorStand;
use sunrise_sim::mobs::{Mob, Species, Who};
use sunrise_sim::motion::PlayerMotion;
use sunrise_sim::save::uuid_text;

/// `EntityTypes.SNIFFER`: `.sized(1.9F, 1.75F).nameTagOffset(2.05F)`. The tag
/// hangs 2.05 above the feet where every other type hangs it at its box
/// height, and `SubmitNodeCollector.submitNameTag` lifts both 0.5 more.
#[test]
fn a_sniffers_tag_hangs_from_its_name_tag_attachment() {
    let tag_height = |m: &Mob| name_tag_at(m, 1.0).y - m.position.y;
    let sniffer = mob(1, "minecraft:sniffer", Species::Sniffer, 1.9, 1.75);
    assert!(
        (tag_height(&sniffer) - 2.55).abs() < 1e-6,
        "2.05 + 0.5, got {}",
        tag_height(&sniffer)
    );
    // A cow attaches nothing of its own: `Fallback.AT_HEIGHT`, 1.4 + 0.5.
    let cow = mob(2, "minecraft:cow", Species::Cow, 0.9, 1.4);
    assert!(
        (tag_height(&cow) - 1.9).abs() < 1e-6,
        "got {}",
        tag_height(&cow)
    );
    // `EntityDimensions.scale`: a baby's attachment shrinks with its box.
    let mut baby = mob(3, "minecraft:sniffer", Species::Sniffer, 1.9, 1.75);
    baby.extra.age = -1;
    assert!(
        (tag_height(&baby) - 1.525).abs() < 1e-6,
        "2.05 * 0.5 + 0.5, got {}",
        tag_height(&baby)
    );
    // `Sniffer.DIGGING_DIMENSIONS` is built without the attachment, so a
    // digging sniffer's tag falls back to its lower box: 1.35 + 0.5.
    let mut digging = mob(4, "minecraft:sniffer", Species::Sniffer, 1.9, 1.75);
    digging.extra.variant = sunrise_sim::mobs::sniffer::DIGGING;
    assert!(
        (tag_height(&digging) - 1.85).abs() < 1e-6,
        "got {}",
        tag_height(&digging)
    );
}

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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
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
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.selected = 0;
    game.set_mode_id(0);
    game
}

fn client_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_client_gameplay::game::ClientGame<'a> {
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
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
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
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.client.screen = Screen::Playing;
    game.selected = 0;
    game
}

fn spawn(
    game: &mut sunrise_gameplay_common::game::CommonGame<'_>,
    kind: &str,
    id: u32,
    at: [f64; 3],
) -> usize {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    game.mobs
        .push(Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0));
    game.mobs.len() - 1
}

fn target(id: u32) -> Option<InteractTarget> {
    Some(InteractTarget {
        id: id as i32,
        location: [0.0, 0.0, 0.0],
    })
}

/// A name tag with `name` as its `minecraft:custom_name`.
fn named_tag(name: &str) -> ItemStack {
    let mut stack = ItemStack::new("minecraft:name_tag", 1);
    stack.data_mut().set_other(
        "minecraft:custom_name",
        Some(sunrise_core::nbt::Tag::String(name.into())),
    );
    stack
}

fn take_interacts() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    TEST_POSTED_EVENTS.with(|events| {
        std::mem::take(events.borrow_mut().as_mut().unwrap())
            .into_iter()
            .filter(|event| event.0 == "minecraft:entity_interact")
            .collect()
    })
}

#[test]
#[ignore = "daily"]
fn naming_posts_entity_interact_at_the_named_entity() {
    let mut resources = None;
    let mut game = game(&mut resources);
    TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
    // A mob names itself in `Mob.interact`: the event goes out at the mob,
    // from the player who used the tag.
    game.inventory[0] = Some(named_tag("Bessie"));
    let cow = spawn(&mut game, "minecraft:cow", 7, [2.5, 64.0, 2.5]);
    assert!(
        game.interact_mob(target(7)).unwrap(),
        "the tag names the cow"
    );
    assert_eq!(game.mobs[cow].custom_name.as_deref(), Some("Bessie"));
    assert_eq!(
        take_interacts(),
        vec![(
            "minecraft:entity_interact",
            [2.5, 64.0, 2.5],
            Some(Who::Player)
        )]
    );
    // An armour stand is no mob: `Player.interactOn` posts the same event for
    // any item that named a living entity.
    game.inventory[0] = Some(named_tag("Holo"));
    game.armor_stands
        .push(ArmorStand::new(21, Vec3::new(3.0, 64.0, 4.0), 0.0));
    assert!(game.interact_armor_stand(target(21)), "the tag names it");
    assert_eq!(
        game.armor_stands[0].custom_name.as_ref().map(Text::plain),
        Some("Holo".to_owned())
    );
    assert_eq!(
        take_interacts(),
        vec![(
            "minecraft:entity_interact",
            [3.0, 64.0, 4.0],
            Some(Who::Player)
        )]
    );
    TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = None);
}

#[test]
#[ignore = "daily"]
fn an_armour_stands_name_ignores_f1_and_team_visibility() {
    let mut resources = None;
    let mut game = client_game(&mut resources);
    let mut stand = ArmorStand::new(42, Vec3::new(0.0, 64.0, 4.0), 0.0);
    stand.custom_name = Some(Text::literal("Holo"));
    stand.name_visible = true;
    game.armor_stands.push(stand);
    let camera = glam::DVec3::new(0.5, 65.0, 0.5);
    assert_eq!(game.shown_name_tags(camera), vec!["Holo".to_owned()]);
    // F1 hides the HUD: `ArmorStandRenderer.shouldShowName` never asks.
    game.client.chat_ui.hide_gui = true;
    assert_eq!(
        game.shown_name_tags(camera),
        vec!["Holo".to_owned()],
        "F1 keeps the stand's name"
    );
    // Nor does a team the stand is on, whose name tags it hides from everyone.
    let stand_uuid = [0, 0, 0, 42];
    game.persist.uuids.insert(42, stand_uuid);
    game.score.board.add_team("hidden").nametag_visibility = sb::Visibility::Never;
    assert!(
        game.score
            .board
            .add_to_team(&uuid_text(stand_uuid), "hidden")
    );
    // A mob's name still follows `LivingEntityRenderer`: the same team hides it.
    let cow = spawn(&mut game, "minecraft:cow", 9, [1.0, 64.0, 4.0]);
    game.mobs[cow].custom_name = Some("Moo".into());
    game.mobs[cow].name_visible = true;
    let cow_uuid = [0, 0, 0, 9];
    game.persist.uuids.insert(9, cow_uuid);
    assert!(game.score.board.add_to_team(&uuid_text(cow_uuid), "hidden"));
    assert_eq!(
        game.shown_name_tags(camera),
        vec!["Holo".to_owned()],
        "the team hides the cow's name, not the armour stand's"
    );
    // The same stand with its name hidden (`CustomNameVisible` off) shows none.
    game.armor_stands[0].name_visible = false;
    assert!(game.shown_name_tags(camera).is_empty());
}

#[test]
#[ignore = "daily"]
fn a_trades_title_draws_the_names_style() {
    let mut resources = None;
    let mut game = client_game(&mut resources);
    // This test draws the merchant screen; load full UI resources for glyph metrics.
    game.client.gui = UiResources::load_with_limit(
        &game.pack.source,
        &game.options.resources,
        sunrise_level::pack_image::MAX_ATLAS_SIZE,
    )
    .unwrap();
    // `MerchantScreen.extractLabels` draws the menu's title, the merchant's
    // `getDisplayName()`: a name with a colour keeps it, instead of coming out
    // in the screen's grey (-12566464).
    let session = sunrise_sim::merchant::Session::new(Vec::new(), true);
    game.open_remote_merchant(session, Some(Text::colored("Bob", 0xFF_0000)));
    let mut batch = HudBatch::new([854, 480], game.client.settings.gui_scale_inputs());
    assert!(game.draw_merchant(&mut batch));
    let (_, oy) = game.inventory_origin(&batch);
    // The title's row: glyphs only, its own line (`oy + 6`, 8 px tall).
    let title: Vec<u32> = batch
        .vertices
        .iter()
        .filter(|v| v.text == 1 && v.pos[1] >= oy + 6.0 && v.pos[1] <= oy + 14.0)
        .map(|v| v.color)
        .collect();
    assert!(!title.is_empty(), "the title draws glyphs");
    // rgba(255, 0, 0, 255): the name's red, not GREY.
    assert!(
        title.contains(&0xFF_0000FF),
        "the title keeps the name's colour: {title:x?}"
    );
}

/// `Player.interactOn`: `NameTagItem` shrinks the stack it named with, and a
/// creative player's count comes straight back -- one tag names as many stands
/// as they click.
#[test]
#[ignore = "daily"]
fn a_creative_player_keeps_the_tag_it_names_a_stand_with() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.armor_stands
        .push(ArmorStand::new(9, Vec3::new(0.5, 64.0, 0.5), 0.0));
    let mut tag = ItemStack::new("minecraft:name_tag", 2);
    tag.data_mut().custom_name = Some(Tag::compound([("text", Tag::String("Bob".into()))]));
    game.inventory[0] = Some(tag);
    let target = || {
        Some(InteractTarget {
            id: 9,
            location: [0.0, 0.0, 0.0],
        })
    };
    game.set_mode_id(1);
    assert!(game.interact_armor_stand(target()));
    assert_eq!(
        game.inventory[0].as_ref().map(|s| s.count),
        Some(2),
        "a creative player keeps the tag"
    );
    let named = game.armor_stands[0]
        .custom_name
        .as_ref()
        .map(sunrise_game_data::text::Text::plain);
    assert_eq!(named.as_deref(), Some("Bob"));
    // Survival spends one.
    game.set_mode_id(0);
    assert!(game.interact_armor_stand(target()));
    assert_eq!(
        game.inventory[0].as_ref().map(|s| s.count),
        Some(1),
        "a survival player spends the tag"
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
