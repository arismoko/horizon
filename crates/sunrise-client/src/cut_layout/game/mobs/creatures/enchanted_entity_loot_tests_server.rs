use super::super::native_entity_refs::tests::with_game;
use crate::game::creatures::enchanted_entity_loot_tests::Answer;
use crate::game::creatures::enchanted_entity_loot_tests::Heard;
use crate::game::creatures::enchanted_entity_loot_tests::dropped;
use crate::game::creatures::enchanted_entity_loot_tests::heard;
use crate::game::creatures::enchanted_entity_loot_tests::listening;
use mod_api::ServerHost;
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_gameplay_common::game::combat_common::PlayerCredit;
use sunrise_sim::mobs::{Damage, DamageKind, Mob, Who};
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

/// A draw the listener makes from the level's random during `/loot ... kill`
/// stays made: the roll's sequence is its own, and the level random moves.
#[test]
#[ignore = "daily"]
fn a_draw_made_during_loot_kill_stays_in_the_level_random() {
    struct Drawing;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Drawing {
        fn enchanted_entity_loot(
            &self,
            host: &mut dyn ServerHost,
            _: &mut mod_api::EnchantedEntityLootEvent<'_>,
        ) {
            let level = host.level("minecraft:overworld").unwrap();
            level.with_random(host, |random| random.next_int()).unwrap();
        }
    }
    let mods = vec![mod_api::ModEntry::new("enchanted-loot", Box::new(Drawing))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.load_server_loot();
            game.set_allow_commands(true);
            game.ui.registries.permission = 4;
            game.server.next_entity = 1000;
            zombie(game, Damage::mob(0, [0.0; 3]));
            game.mobs.last_mut().unwrap().health = 20.0;
            game.world.random.set_seed(7);
            let source = game.player_source();
            game.server.command_source = Some(
                sunrise_gameplay::game::server_side::commands::execute::Source {
                    entity: None,
                    ..source
                },
            );
            assert!(game.run_command("loot spawn 0 64 0 kill @e[type=minecraft:zombie,limit=1]"));
            game.server.command_source = None;
            let mut expected = LegacyRandom::new(7);
            expected.next_int();
            assert_eq!(game.world.random.next_long(), expected.next_long());
        });
    });
}

/// `EnchantedEntityLootEvent` through a whole kill: the zombie's table
/// reads Looting for its flesh's `enchanted_count_increase` and its rare
/// drops' `random_chance_with_enchanted_bonus`. The mod gets the dying
/// zombie and the attacker as live handles, the attacker's own level, and
/// the level it answers decides the drops.
#[test]
#[ignore = "daily"]
fn a_kills_loot_uses_the_level_a_mod_answers() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "enchanted-loot",
        Box::new(Answer(Arc::clone(&seen), 1000)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.load_server_loot();
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(5));
            // The mobs' ids, clear of the guest's.
            game.server.next_entity = 1000;
            game.as_guest(5, |g| {
                let mut sword = sunrise_items::items::ItemStack::new("minecraft:iron_sword", 1);
                sword.enchant("minecraft:looting", 2);
                g.common.inventory[g.common.selected] = Some(sword);
            })
            .unwrap();
            let by_player = Damage {
                kind: DamageKind::PlayerAttack,
                attacker: Some(Who::Player),
                from: Some([0.0; 3]),
                direct: None,
                attacker_species: None,
            };
            let mut victim = zombie(game, by_player);
            victim.ai.last_hurt_by_player = 100;
            game.server
                .last_hurt_by_player
                .insert(victim.id, PlayerCredit(127));
            game.mob_die(&victim);
            let looted = |level| {
                heard(
                    "minecraft:zombie",
                    "minecraft:player",
                    "minecraft:player_attack",
                    level,
                )
            };
            assert_eq!(*seen.lock().unwrap(), [looted(2), looted(2)]);
            // A flesh count of at most 2 grew by the level's share, and
            // a chance of 0.035 + 0.01 a level (above 1 at 1000) always
            // holds, so one of the three rare drops came.
            assert!(dropped(game, "minecraft:rotten_flesh") > 2);
            let rare = [
                "minecraft:iron_ingot",
                "minecraft:carrot",
                "minecraft:potato",
            ];
            let rare: u32 = rare.iter().map(|item| dropped(game, item)).sum();
            assert_eq!(rare, 1);

            // In the guest's own turn, their record is out of the hub
            // and the handle still names them.
            seen.lock().unwrap().clear();
            let mut victim = zombie(game, by_player);
            victim.ai.last_hurt_by_player = 100;
            game.server
                .last_hurt_by_player
                .insert(victim.id, PlayerCredit(127));
            game.as_guest(5, |g| g.mob_die(&victim)).unwrap();
            assert_eq!(*seen.lock().unwrap(), [looted(2), looted(2)]);

            // Another mob's blow: no player, so its rare drops read no
            // kill, and the flesh is still asked, from a level of 0.
            seen.lock().unwrap().clear();
            let skeleton = zombie(game, Damage::mob(0, [0.0; 3]));
            // Keep the admitted row's residency and attachment lifetime.
            game.mobs.last_mut().unwrap().health = 20.0;
            let victim = zombie(game, Damage::mob(skeleton.id, [0.0; 3]));
            game.mob_die(&victim);
            assert_eq!(
                *seen.lock().unwrap(),
                [heard(
                    "minecraft:zombie",
                    "minecraft:zombie",
                    "minecraft:mob_attack",
                    0
                )]
            );
        });
    });
}

/// A source that isn't living is the attacker all the same, read from a
/// level of 0; a source with no entity is no attacker. The zombie's rare
/// drops read no kill then, so only the flesh asks.
#[test]
#[ignore = "daily"]
fn loot_kill_asks_with_a_non_living_source_and_with_none() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = listening(&seen, 3);
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.load_server_loot();
            game.set_allow_commands(true);
            game.ui.registries.permission = 4;
            game.server.next_entity = 1000;
            zombie(game, Damage::mob(0, [0.0; 3]));
            game.mobs.last_mut().unwrap().health = 20.0;
            let kill = "loot spawn 0 64 0 kill @e[type=minecraft:zombie,limit=1]";

            let source = game.player_source();
            game.server.command_source = Some(
                sunrise_gameplay::game::server_side::commands::execute::Source {
                    entity: None,
                    ..source.clone()
                },
            );
            assert!(game.run_command(kill));
            assert_eq!(
                *seen.lock().unwrap(),
                [Heard {
                    attacker: None,
                    ..heard("minecraft:zombie", "", "minecraft:magic", 0)
                }]
            );
            seen.lock().unwrap().clear();

            game.server.command_source = None;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let stick = sunrise_items::items::ItemStack::new("minecraft:stick", 1);
            game.spawn_item(stick, [0.5, 64.0, 0.5], [0.0; 3], 0);
            let item = game.dropped.iter().last().unwrap().id;
            let item = sunrise_gameplay_common::game::commands_common::Target::Item(item);
            game.server.command_source = Some(
                sunrise_gameplay::game::server_side::commands::execute::Source {
                    entity: Some(item),
                    ..source
                },
            );
            assert!(game.run_command(kill));
            game.server.command_source = None;
            assert_eq!(
                *seen.lock().unwrap(),
                [heard(
                    "minecraft:zombie",
                    "minecraft:item",
                    "minecraft:magic",
                    0
                )]
            );
        });
    });
}

/// `LootCommand.dropKillLoot`: nothing dies, the damage is `magic()`, whose
/// source names nobody, and the attacker is the command's source. Both
/// readers ask, with the source's level, and the answer decides the drops.
#[test]
#[ignore = "daily"]
fn loot_kill_asks_with_the_commands_source_as_the_attacker() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = listening(&seen, 1000);
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.load_server_loot();
            game.set_allow_commands(true);
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(5));
            game.server.next_entity = 1000;
            let victim = zombie(game, Damage::mob(0, [0.0; 3]));
            game.mobs.last_mut().unwrap().health = 20.0;
            let looted = |level| {
                heard(
                    "minecraft:zombie",
                    "minecraft:player",
                    "minecraft:magic",
                    level,
                )
            };

            game.as_guest(5, |g| {
                let mut sword = sunrise_items::items::ItemStack::new("minecraft:iron_sword", 1);
                sword.enchant("minecraft:looting", 2);
                g.common.inventory[g.common.selected] = Some(sword);
                g.ui.registries.permission = 4;
                assert!(g.run_command("loot spawn 0 64 0 kill @e[type=minecraft:zombie,limit=1]"));
            })
            .unwrap();
            assert_eq!(*seen.lock().unwrap(), [looted(2), looted(2)]);
            assert!(
                game.mobs
                    .iter()
                    .any(|m| m.id == victim.id && m.health > 0.0)
            );
            assert!(dropped(game, "minecraft:rotten_flesh") > 2);
            let rare = [
                "minecraft:iron_ingot",
                "minecraft:carrot",
                "minecraft:potato",
            ];
            let rare: u32 = rare.iter().map(|item| dropped(game, item)).sum();
            assert_eq!(rare, 1);
        });
    });
}

/// The host's own kill names the host: its id, not the 0 of no attacker.
#[test]
#[ignore = "daily"]
fn the_hosts_kill_hands_the_host_as_the_attacker() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "enchanted-loot",
        Box::new(Answer(Arc::clone(&seen), 0)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.load_server_loot();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 9000;
            // The host's own turn: the uuid is acting, no guest connection.
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 9000, 99);
            game.server.lan_acting_uuid = Some(99);
            game.server.lan_acting_entity = Some(9000);
            let by_player = Damage {
                kind: DamageKind::PlayerAttack,
                attacker: Some(Who::Player),
                from: Some([0.0; 3]),
                direct: None,
                attacker_species: None,
            };
            let mut victim = zombie(game, by_player);
            victim.ai.last_hurt_by_player = 100;
            game.server
                .last_hurt_by_player
                .insert(victim.id, PlayerCredit(99));
            game.mob_die(&victim);
            let looted = || {
                heard(
                    "minecraft:zombie",
                    "minecraft:player",
                    "minecraft:player_attack",
                    0,
                )
            };
            assert_eq!(*seen.lock().unwrap(), [looted(), looted()]);
        });
    });
}

pub(crate) fn zombie(game: &mut sunrise_gameplay::game::ServerGame<'_>, damage: Damage) -> Mob {
    game.server.next_entity += 1;
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    let mut mob = Mob::new(
        game.server.next_entity,
        kind,
        Vec3::new(0.5, 64.0, 0.5),
        0.0,
    );
    mob.health = 0.0;
    mob.last_damage = Some((damage, 0));
    game.mobs.push(mob.clone());
    mob
}
