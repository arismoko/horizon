//! Actual entity removal, commands and chunk-save assembly, with no intervening tick.
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::commands_common as commands;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::Mob;
use {
    crate::game::*,
    sunrise_gameplay_common::game::{
        Arc, HashMap, Instant, Options, PathBuf, PlayerMotion, Progress, Vec3, World,
        execution_role,
    },
};

#[test]
#[ignore = "daily"]
fn discarded_mob_leaves_selectors_and_saves_while_frozen() {
    run_discarded_mob_leaves_selectors_and_saves_while_frozen();
}

pub(crate) fn run_discarded_mob_leaves_selectors_and_saves_while_frozen() {
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.native_lifecycle
        .begin_open(Some(mod_api::Authority::IntegratedServer));
    game.world = World::new(game.res.facts().air, 1);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("tick freeze"));
    assert!(game.clock.frozen());
    let kind = game.res.mob_types().get("minecraft:blaze").unwrap().clone();
    game.mobs
        .push(Mob::new(42, kind.clone(), Vec3::new(0.5, 64.0, 2.5), 0.0));
    game.mobs
        .push(Mob::new(43, kind, Vec3::new(1.5, 64.0, 2.5), 0.0));
    // The survivor is dying, but not removed. Keep existing selection semantics:
    // generic entity lookup/UUID still finds it; @e keeps its death_time filter.
    game.mobs[1].health = 0.0;
    game.mobs[1].death_time = 1;
    let removed_uuid = game.uuid_for(42);
    let dying_uuid = game.uuid_for(43);
    let selector = |game: &sunrise_gameplay::game::ServerGame<'_>, text: &str| {
        sunrise_command::selector::parse(
            &mut sunrise_command::Reader::new(text),
            &game.ui.registries,
            true,
        )
        .unwrap()
    };
    let all = selector(&game, "@e[type=minecraft:blaze]");
    assert_eq!(game.select(&all), [commands::Target::Mob(42)]);
    let before_id = game.level_entity(42).is_some();
    let before_uuid = game.target_with_uuid(removed_uuid).is_some();
    assert!(before_id && before_uuid);
    assert_eq!(
        game.target_with_uuid(dying_uuid),
        Some(commands::Target::Mob(43))
    );
    let keys = [(0, 0)].into();
    let mut saved = HashMap::new();
    let mut failed = HashMap::new();
    game.save_every_entity(&keys, &mut saved, &mut failed, false);
    assert!(failed.is_empty());
    // A dying but not removed entity still saves until its death animation
    // completes. Both roots are savable before the explicit discard.
    assert_eq!(saved.values().map(Vec::len).sum::<usize>(), 2);
    let before_save = saved.values().any(|tags| !tags.is_empty());
    let health = game.mobs[0].health;
    let world_time = game.world.game_time;
    assert!(!game.entity_discard(mod_api::EntityRef::new(
        mod_api::WorldEpoch::new(u64::MAX),
        42
    )));
    assert!(game.entity_discard(mod_api::EntityRef::new(
        game.native_lifecycle.epoch().unwrap(),
        42
    )));
    assert!(!game.entity_discard(mod_api::EntityRef::new(
        game.native_lifecycle.epoch().unwrap(),
        42
    )));
    assert_eq!(game.world.game_time, world_time);
    assert_eq!(game.mobs[0].health, health, "discard is not lethal damage");
    assert_eq!(game.mobs[0].death_time, 0);
    assert!(game.dropped.is_empty(), "discard must not roll death loot");
    assert!(game.orbs.is_empty(), "discard must not award death XP");
    let after_enumeration = !game.select(&all).is_empty();
    assert!(!after_enumeration);
    assert_eq!(game.level_entity(42), None);
    assert_eq!(game.target_with_uuid(removed_uuid), None);
    assert_eq!(
        game.target_with_uuid(dying_uuid),
        Some(commands::Target::Mob(43))
    );
    assert_eq!(game.level_entity(43), Some(commands::Target::Mob(43)));
    assert!(game.run_command("execute unless entity @e[type=minecraft:blaze] run data merge storage fixture:discard {gone:1b}"));
    assert_eq!(
        game.score.storage.get("fixture:discard").get("gone"),
        Some(&Tag::Byte(1))
    );
    // UUID-backed commands also bypass the entity-list enumeration.
    let uuid = sunrise_sim::lan::playerdata::ints_uuid(removed_uuid);
    let uuid = sunrise_protocol::crypto::uuid_string(uuid);
    assert!(game.run_command(&format!(
        "execute unless entity {uuid} run data merge storage fixture:discard {{uuid_gone:1b}}"
    )));
    assert_eq!(
        game.score.storage.get("fixture:discard").get("uuid_gone"),
        Some(&Tag::Byte(1))
    );
    saved.clear();
    failed.clear();
    game.save_every_entity(&keys, &mut saved, &mut failed, false);
    assert!(failed.is_empty());
    assert_eq!(
        saved
            .values()
            .flatten()
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>(),
        [43],
        "discard removes only its target; the dying survivor still saves"
    );
    let report = serde_json::json!({
        "before_id": before_id, "before_uuid": before_uuid, "before_save": before_save,
        "after_id": game.level_entity(42).is_some(),
        "after_uuid": game.target_with_uuid(removed_uuid).is_some(),
        "after_enumeration": after_enumeration,
        "after_save": saved.values().flatten().any(|(id, _, _)| *id == 42),
        "removed": game.mobs[0].removed,
    });
    if let Some(path) = std::env::var_os("SUNRISE_DISCARD_REPORT") {
        std::fs::write(path, serde_json::to_vec(&report).unwrap()).unwrap();
    }
    // Removing another mob during its death animation also hides UUID lookup
    // immediately. No dead/alive predicate was added to general lookup.
    assert!(game.entity_discard(mod_api::EntityRef::new(
        game.native_lifecycle.epoch().unwrap(),
        43
    )));
    assert_eq!(game.target_with_uuid(dying_uuid), None);
    // The old UUID entry can remain in EntityMemory until its normal sweep.
    // It must not shadow a newly joined entity with the same UUID.
    let kind = game.res.mob_types().get("minecraft:blaze").unwrap().clone();
    game.mobs
        .push(Mob::new(44, kind, Vec3::new(0.5, 64.0, 2.5), 0.0));
    game.persist.uuids.insert(44, removed_uuid);
    assert_eq!(
        game.target_with_uuid(removed_uuid),
        Some(commands::Target::Mob(44))
    );
    // Root save suppression alone is insufficient: a live but discarded
    // passenger must not reappear under an otherwise savable vehicle.
    let kind = game.res.mob_types().get("minecraft:blaze").unwrap().clone();
    game.mobs
        .push(Mob::new(45, kind, Vec3::new(0.5, 65.0, 2.5), 0.0));
    game.mobs[3].extra.mount = Some(44);
    saved.clear();
    failed.clear();
    game.save_every_entity(&keys, &mut saved, &mut failed, false);
    assert!(failed.is_empty());
    let root = &saved[&(0, 0)][0].2;
    assert!(matches!(root.get("Passengers"), Some(Tag::List(list)) if list.len() == 1));
    assert!(game.entity_discard(mod_api::EntityRef::new(
        game.native_lifecycle.epoch().unwrap(),
        45
    )));
    saved.clear();
    failed.clear();
    game.save_every_entity(&keys, &mut saved, &mut failed, false);
    assert!(failed.is_empty());
    assert_eq!(saved[&(0, 0)].len(), 1);
    assert!(saved[&(0, 0)][0].2.get("Passengers").is_none());
    // EntitySelectorParser gives @e and @n `Entity::isAlive`: a nearer mob at
    // no health is skipped before its death timer starts (a /kill in the
    // same tick), though its UUID still resolves.
    let kind = game.res.mob_types().get("minecraft:blaze").unwrap().clone();
    let mut killed = Mob::new(46, kind, Vec3::new(0.5, 64.0, 1.5), 0.0);
    killed.health = 0.0;
    game.mobs.push(killed);
    let killed_uuid = game.uuid_for(46);
    assert_eq!(game.select(&all), [commands::Target::Mob(44)]);
    let nearest = selector(&game, "@n[type=minecraft:blaze]");
    assert_eq!(game.select(&nearest), [commands::Target::Mob(44)]);
    assert_eq!(
        game.target_with_uuid(killed_uuid),
        Some(commands::Target::Mob(46))
    );
}

/// A registered selector type keeps every candidate it is offered.
struct Fallen;
impl mod_api::EntitySelectorType for Fallen {
    fn base(&self) -> mod_api::SelectorBase {
        mod_api::SelectorBase::new(false, None, mod_api::SelectorSort::Arbitrary, "test.fallen")
    }
    fn matches(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mod_api::CommandSource,
        _: mod_api::EntityRef,
    ) -> bool {
        true
    }
}
struct FallenMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for FallenMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_entity_selector("fallen", Fallen).unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn dead_guest_leaves_at_e_but_reaches_a_registered_selector() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "fallen_fixture",
        Box::new(FallenMod),
    )]
    .into();
    mod_api::testing::rebuild_parts(|| {
        sunrise_gameplay_common::game::api_values::install_parts(&mods)
    })
    .unwrap();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.health = 0.0;
                    guest.dead = true;
                })
                .unwrap();
            game.as_guest(12, |game| {
                let selector = |token: Option<&str>| sunrise_command::args::Selector {
                    kind: 'e',
                    include_entities: true,
                    token: token.map(Into::into),
                    limit: usize::MAX,
                    ..Default::default()
                };
                let guests = |game: &mut sunrise_gameplay::game::ServerGame<'_>, token| {
                    let mut found: Vec<_> = game
                        .select(&selector(token))
                        .into_iter()
                        .filter(|t| {
                            matches!(t, commands::Target::Player | commands::Target::Guest(_))
                        })
                        .collect();
                    found.sort_by_key(|t| matches!(t, commands::Target::Guest(_)));
                    found
                };
                // EntitySelectorParser gives the built-in @e `Entity::isAlive`;
                // a registered type decides for itself and sees the dead guest.
                assert_eq!(guests(game, None), [commands::Target::Player]);
                assert_eq!(
                    guests(game, Some("fallen")),
                    [commands::Target::Player, commands::Target::Guest(13)]
                );
            });
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
