//! A mod's write to a player in another level reaches their inventory at once. A give drops
//! overflow in the player's level; a raw throw or block break still needs that level in place.
use super::native_entity_refs::tests::run_fresh_cases;
use super::native_guest_turn_tests::with_guests;
use mod_api::{HostTransaction, ItemPlace, ServerHost};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::playerdata;

pub(crate) const NETHER: &str = "minecraft:the_nether";

/// How many item entities lie in each of the Overworld and the Nether.
pub(crate) fn dropped(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> [usize; 2] {
    let here = game.dim;
    let overworld = game.dropped.len();
    game.enter_level(Dim::Nether).unwrap();
    let nether = game.dropped.len();
    game.enter_level(here).unwrap();
    [overworld, nether]
}

/// Guests 12 (the world's owner) and 13 in the Nether with full inventories but for slot 8;
/// the Overworld in place, as for an Overworld hook. A diamond to give.
pub(crate) fn in_the_nether(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> mod_api::ItemStack {
    game.level.player_uuid = Some(playerdata::uuid_ints(12));
    // A level is open while a player is in it.
    game.enter_level(Dim::Nether).unwrap();
    game.enter_level(Dim::Overworld).unwrap();
    for conn in [12, 13] {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(conn, |g| {
                g.dimension = NETHER.to_owned();
                g.inventory
                    .fill(Some(ItemStack::new("minecraft:stone", 64)));
                g.inventory[8] = None;
            })
            .unwrap();
    }
    let host: &dyn mod_api::Host = game;
    host.item_from_snbt(r#"{id:"minecraft:diamond",count:1}"#)
        .unwrap()
}

/// `count` diamonds, as a mod reads them from SNBT.
pub(crate) fn some_diamonds(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    count: u32,
) -> mod_api::ItemStack {
    let host: &dyn mod_api::Host = game;
    host.item_from_snbt(&format!(r#"{{id:"minecraft:diamond",count:{count}}}"#))
        .unwrap()
}

pub(crate) fn diamonds(game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> u32 {
    let guest = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
    let held = guest.inventory.iter().flatten();
    held.filter(|s| &*s.item == "minecraft:diamond")
        .map(|s| s.count)
        .sum()
}

/// A gift that fits uses the recipient's RNG and sounds at their feet in their own level.
#[test]
#[ignore = "daily"]
fn a_give_that_fits_plays_pickup_in_the_players_level() {
    use std::sync::{Arc, mpsc};
    use sunrise_core::random::{EntityRandom, LegacyRandom, RandomSource};
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};
    use sunrise_sim::lan::{Out, PlayerSender};

    if run_fresh_cases(
        "cut_layout::game::server_side::native_player_level_tests::a_give_that_fits_plays_pickup_in_the_players_level",
        "SUNRISE_GIVE_OVERFLOW_CHILD",
        &["sound"],
    ) {
        return;
    }
    with_guests(|game| {
        let diamond = in_the_nether(game);
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        let [overworld, nether] = [12, 13].map(|conn| {
            let (tx, rx) = mpsc::channel();
            hub.update_guest(conn, |g| {
                if conn == 12 {
                    g.dimension = "minecraft:overworld".to_owned();
                }
                g.sender = PlayerSender::new(tx, Arc::default());
            })
            .unwrap();
            rx
        });
        let C::Sound(pickup) = hub.sound_packet(
            "minecraft:entity.item.pickup",
            [3.0, 64.0, 5.0],
            0.2,
            2.0,
            7,
            0,
        ) else {
            unreachable!()
        };
        game.as_guest(13, |g| {
            g.player.as_mut().unwrap().random = EntityRandom::own(LegacyRandom::new(0x5eed));
        })
        .unwrap();
        game.start_audio_capture();
        let player = game.player(13).unwrap();
        let host: &mut dyn ServerHost = game;
        let given = host.transaction(|host, _| Ok::<_, ()>(player.give_stack(host, diamond)));
        assert_eq!(given, Ok(Ok(None)));
        assert_eq!(diamonds(game, 13), 1);
        assert_eq!(game.dim, Dim::Overworld);
        game.lan_broadcast_effects();

        let pickups = |rx: mpsc::Receiver<Out>| {
            rx.try_iter()
                .flat_map(|out| match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => vec![],
                })
                .filter_map(|body| {
                    let mut r = Reader::new(&body);
                    let id = r.varint().unwrap();
                    match C::decode(id, r.rest()).unwrap() {
                        C::Sound(s) if s.sound == pickup.sound => {
                            Some(([s.x, s.y, s.z], s.source, s.volume, s.pitch.to_bits()))
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
        };
        assert!(pickups(overworld).is_empty());
        // Java Random(0x5eed): four nextFloat draws for the fake item, two for pitch.
        assert_eq!(pickups(nether), [([24, 512, 40], 7, 0.2, 0x3fdcbf84)]);
        game.as_guest(13, |g| {
            assert_eq!(g.player.as_mut().unwrap().random.next_int(), -786623561);
        })
        .unwrap();
        assert_eq!(dropped(game), [0, 1]);
        game.enter_level(Dim::Nether).unwrap();
        let fake = game.dropped.last().unwrap();
        assert_eq!(
            (&*fake.stack.item, fake.stack.count),
            ("minecraft:diamond", 1)
        );
        assert_eq!((fake.pickup_delay, fake.age), (32767, 5999));
        assert_eq!((fake.thrower, fake.target), (None, None));
    });
}

/// A stack given from an Overworld hook to a player in the Nether goes into their inventory
/// at once, for the owner and another guest. What doesn't fit drops only in the Nether, after
/// commit in a transaction; aborting restores the inventory and drops nothing.
#[test]
#[ignore = "daily"]
fn a_give_to_a_player_in_another_level_drops_overflow_there() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_player_level_tests::a_give_to_a_player_in_another_level_drops_overflow_there",
        "SUNRISE_GIVE_OVERFLOW_CHILD",
        &["overflow"],
    ) {
        return;
    }
    with_guests(|game| {
        let diamond = in_the_nether(game);
        let resource = mod_api::ItemResource::of(&diamond);
        for conn in [12, 13] {
            let before = dropped(game);
            let player = game.player(u128::from(conn)).unwrap();
            let case = format!("guest {conn}");
            // Slot 8 takes a diamond and then 63 more.
            let host: &mut dyn ServerHost = game;
            assert_eq!(player.give_stack(host, diamond.clone()), Ok(None), "{case}");
            let given = host
                .transaction(|host, _| {
                    assert_eq!(host.dimension(), "minecraft:overworld");
                    Ok::<_, ()>(player.give_stack(host, diamond.clone()))
                })
                .unwrap();
            assert_eq!(given, Ok(None), "{case}");
            assert_eq!(diamonds(game, conn), 2, "{case}");
            assert_eq!(
                dropped(game),
                [before[0], before[1] + 2],
                "{case}: fake pickups"
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |g| {
                    g.inventory[8] = Some(ItemStack::new("minecraft:stone", 64))
                })
                .unwrap();
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                player.give_stack(host, diamond.clone()),
                Ok(None),
                "{case}: full"
            );
            let taken = host
                .transaction(|host, tx| {
                    assert_eq!(host.dimension(), "minecraft:overworld");
                    let slot = ItemPlace::PlayerSlot {
                        player: u128::from(conn),
                        slot: 0,
                    };
                    slot.insert(host, tx, &resource, 1)
                })
                .unwrap();
            assert_eq!(taken, 0, "{case}: a full slot takes nothing");
            assert_eq!(diamonds(game, conn), 0, "{case}");
            assert_eq!(dropped(game), [before[0], before[1] + 3], "{case}: full");
            // One unit of room in the only matching stack, three offered: one is taken, two
            // drop, directly and in a transaction.
            let three = some_diamonds(game, 3);
            for in_transaction in [false, true] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| {
                        g.inventory[8] = Some(ItemStack::new("minecraft:diamond", 63))
                    })
                    .unwrap();
                let host: &mut dyn ServerHost = game;
                let given = if in_transaction {
                    let aborted = host.transaction(|host, _| {
                        Err::<(), _>(player.give_stack(host, three.clone()))
                    });
                    assert_eq!(
                        aborted,
                        Err(mod_api::TransactionError::Aborted(Ok(None))),
                        "{case}: abort"
                    );
                    assert_eq!(diamonds(game, conn), 63, "{case}: inventory restored");
                    assert_eq!(
                        dropped(game),
                        [before[0], before[1] + 4],
                        "{case}: no abort drop"
                    );
                    let host: &mut dyn ServerHost = game;
                    host.transaction(|host, _| Ok::<_, ()>(player.give_stack(host, three.clone())))
                        .unwrap()
                } else {
                    player.give_stack(host, three.clone())
                };
                let case = format!("{case}, partial, in transaction: {in_transaction}");
                assert_eq!(given, Ok(None), "{case}");
                assert_eq!(diamonds(game, conn), 64, "{case}");
                assert_eq!(game.dim, Dim::Overworld, "{case}: hook level restored");
            }
            assert_eq!(dropped(game), [before[0], before[1] + 5], "{case}");
            game.enter_level(Dim::Nether).unwrap();
            for (item, count) in game.dropped.iter().skip(before[1] + 2).zip([1, 2, 2]) {
                assert_eq!(&*item.stack.item, "minecraft:diamond", "{case}");
                assert_eq!(item.stack.count, count, "{case}");
                assert_eq!([item.position.x, item.position.z], [3.0, 5.0], "{case}");
                assert_eq!(item.pickup_delay, 0, "{case}");
                assert_eq!(
                    item.target,
                    Some(playerdata::uuid_ints(u128::from(conn))),
                    "{case}"
                );
            }
            game.enter_level(Dim::Overworld).unwrap();
        }
    });
}

#[derive(Clone, Default)]
pub(in crate::game) struct GiveJoin(pub(in crate::game) std::sync::Arc<std::sync::Mutex<Vec<u32>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for GiveJoin {
    fn item_toss(&self, _: &mut dyn ServerHost, _: &mut mod_api::ItemTossEvent) -> mod_api::Flow {
        mod_api::Flow::Cancel
    }

    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EntityJoinLevelEvent<'_>,
    ) -> mod_api::Flow {
        if event.kind == "minecraft:item" && !event.loaded_from_disk {
            assert_eq!(event.dimension, NETHER);
            let item = host
                .item_entity(host.entity(event.subject.id()).unwrap())
                .unwrap();
            assert_eq!(item.pickup_delay(host), Some(0));
            assert_eq!(item.target(host), Some(13));
            self.0.lock().unwrap().push(event.subject.id());
        }
        mod_api::Flow::Continue
    }
}

/// A guest borrowed into an Overworld turn while they are in the Nether gets a give's
/// overflow in the Nether, even when a mod vetoes player tosses. The item joins with
/// its pickup metadata already set. The guest cannot throw a stack in the Overworld.
#[test]
#[ignore = "daily"]
fn a_give_to_a_borrowed_player_drops_in_their_level() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_player_level_tests::a_give_to_a_borrowed_player_drops_in_their_level",
        "SUNRISE_GIVE_OVERFLOW_CHILD",
        &["borrowed"],
    ) {
        return;
    }
    with_guests(|game| {
        let diamond = in_the_nether(game);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| {
                g.inventory[8] = Some(ItemStack::new("minecraft:stone", 64))
            })
            .unwrap();
        let player = game.player(13).unwrap();
        let overworld_before = game.dropped.len();
        game.enter_level(Dim::Nether).unwrap();
        fixture_resident_column(&mut game.world, (0, 0));
        let nether_before = game.dropped.len();
        game.enter_level(Dim::Overworld).unwrap();
        let joined = GiveJoin::default();
        let mods = vec![mod_api::ModEntry::new(
            "give_join",
            Box::new(joined.clone()),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let (given, thrown) = game
                .as_guest(13, |game| {
                    let host: &mut dyn ServerHost = game;
                    let given = player.give_stack(host, diamond.clone());
                    let thrown = player.drop_stack(host, diamond.clone(), true);
                    (given, thrown)
                })
                .unwrap();
            assert_eq!(given, Ok(None));
            assert_eq!(thrown, Ok(Some(diamond)));
            assert_eq!(game.dim, Dim::Overworld);
            assert_eq!(dropped(game), [overworld_before, nether_before + 1]);
        });
        game.enter_level(Dim::Nether).unwrap();
        let item = game.dropped.last().unwrap();
        assert_eq!(
            (&*item.stack.item, item.stack.count),
            ("minecraft:diamond", 1)
        );
        assert_eq!(*joined.0.lock().unwrap(), [item.id]);
    });
}

/// `/give` run through the console inside an open transaction, while a Nether guest with a full
/// inventory is borrowed into an Overworld turn: the command takes the stack in the guest's own
/// level, so on commit the overflow lies in the Nether at the guest, nothing in the Overworld
/// and the Overworld is in place again. On abort nothing lies in either level and the
/// inventory is back. A guest at zero health by the commit still has the accepted overflow
/// lie in the Nether where they were.
#[test]
#[ignore = "daily"]
fn a_give_command_in_a_transaction_for_a_borrowed_player_lands_in_their_level() {
    with_guests(|game| {
        game.build_registries();
        in_the_nether(game);
        let player = game.player(13).unwrap();
        let before = dropped(game);
        // The zero-health case last: the guest stays dead.
        for (commit, dies) in [(true, false), (false, false), (true, true)] {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.inventory
                        .fill(Some(ItemStack::new("minecraft:stone", 64)))
                })
                .unwrap();
            let case = format!("commit: {commit}, dies: {dies}");
            let ran = game
                .as_guest(13, |game| {
                    let host: &mut dyn ServerHost = game;
                    host.transaction(|host, _| {
                        assert_eq!(host.dimension(), "minecraft:overworld");
                        let result = host
                            .commands()
                            .run_command_with_output("give guest13 minecraft:diamond");
                        let ok = result.success;
                        assert!(ok, "{}", result.feedback.join("|"));
                        assert_eq!(host.dimension(), "minecraft:overworld");
                        if dies {
                            let living = host.living(player.entity()).unwrap();
                            assert!(living.set_health(host, 0.0));
                        }
                        if commit { Ok(ok) } else { Err(ok) }
                    })
                })
                .unwrap();
            assert!(
                ran.unwrap_or_else(|err| match err {
                    mod_api::TransactionError::Aborted(ok) => ok,
                    other => panic!("unexpected transaction error: {other:?}"),
                }),
                "{case}"
            );
            assert_eq!(game.dim, Dim::Overworld, "{case}");
            assert_eq!(diamonds(game, 13), 0, "{case}: the inventory is full");
            if commit {
                assert_eq!(dropped(game), [before[0], before[1] + 1], "{case}");
                game.enter_level(Dim::Nether).unwrap();
                let lying = game.dropped.iter().next().unwrap();
                assert_eq!(&*lying.stack.item, "minecraft:diamond", "{case}");
                assert_eq!([lying.position.x, lying.position.z], [3.0, 5.0], "{case}");
                game.dropped.clear();
                game.enter_level(Dim::Overworld).unwrap();
            } else {
                assert_eq!(dropped(game), before, "{case}");
                let guest = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
                assert!(
                    guest
                        .inventory
                        .iter()
                        .all(|s| s.as_ref().is_some_and(|s| &*s.item == "minecraft:stone")),
                    "{case}: the inventory is back"
                );
            }
        }
    });
}

/// A throw or a break for a player in another level changes nothing: the stack comes back, and
/// the same coordinates keep their block in both levels.
#[test]
#[ignore = "daily"]
fn a_throw_or_break_for_a_player_in_another_level_changes_nothing() {
    with_guests(|game| {
        let diamond = in_the_nether(game);
        let pos = [6, 64, 4];
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let netherrack = game
            .pack
            .assets
            .registry
            .default_state("minecraft:netherrack")
            .unwrap();
        for (dim, state) in [(Dim::Nether, netherrack), (Dim::Overworld, stone)] {
            game.enter_level(dim).unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(pos[0], pos[1], pos[2], state).unwrap();
        }
        let player = game.player(13).unwrap();
        let before = dropped(game);
        let host: &mut dyn ServerHost = game;
        assert_eq!(
            player.drop_stack(host, diamond.clone(), true),
            Ok(Some(diamond))
        );
        assert!(!player.destroy_block(host, pos, false));
        assert_eq!(dropped(game), before);
        assert_eq!(game.world.block(pos[0], pos[1], pos[2]), stone);
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(game.world.block(pos[0], pos[1], pos[2]), netherrack);
    });
}

/// A give to a guest in another level who has a menu open keeps the menu: the inventory write
/// isn't tied to a level.
#[test]
#[ignore = "daily"]
fn a_give_to_a_player_in_another_level_keeps_their_open_menu() {
    use sunrise_core::nbt::Tag;
    with_guests(|game| {
        let diamond = in_the_nether(game);
        let window = game
            .server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .open_native_menu(13, 5, Tag::String("menu".into()), vec![None; 45])
            .unwrap();
        let player = game.player(13).unwrap();
        let host: &mut dyn ServerHost = game;
        assert_eq!(player.give_stack(host, diamond), Ok(None));
        assert_eq!(diamonds(game, 13), 1);
        let guest = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(guest.menu.as_ref().map(|m| m.window), Some(window));
    });
}

/// A diamond that fills a Nether guest's last slot from an Overworld hook completes their
/// `inventory_changed` advancement in their own level, on their own tick: not at the give, in
/// the hook's level, where its item reward would drop out of their reach.
#[test]
#[ignore = "daily"]
fn an_inventory_reward_for_a_player_in_another_level_waits_for_their_own_tick() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_player_level_tests::an_inventory_reward_for_a_player_in_another_level_waits_for_their_own_tick",
        "SUNRISE_GIVE_OVERFLOW_CHILD",
        &["advancement"],
    ) {
        return;
    }
    const ID: &str = "snapshot:diamonds";
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    with_guests(|game| {
        let world = tempfile::tempdir().unwrap();
        let data = world.path().join("datapacks/rewards/data/snapshot");
        std::fs::create_dir_all(data.join("advancement")).unwrap();
        std::fs::create_dir_all(data.join("loot_table")).unwrap();
        std::fs::write(
            world.path().join("datapacks/rewards/pack.mcmeta"),
            r#"{"pack":{"description":"rewards"}}"#,
        )
        .unwrap();
        std::fs::write(
            data.join("advancement/diamonds.json"),
            r#"{"criteria":{"d":{"trigger":"minecraft:inventory_changed","conditions":{"items":[{"items":"minecraft:diamond"}]}}},
                "requirements":[["d"]],"rewards":{"loot":["snapshot:reward"]}}"#,
        )
        .unwrap();
        std::fs::write(
            data.join("loot_table/reward.json"),
            r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald"}]}]}"#,
        )
        .unwrap();
        std::fs::write(
            world.path().join("datapacks.json"),
            r#"{"enabled":["file/rewards"],"disabled":[]}"#,
        )
        .unwrap();
        game.server.world_root = world.path().to_path_buf();
        game.reload_datapacks(true, &[]).unwrap();
        let diamond = in_the_nether(game);
        let before = dropped(game);
        let done = |game: &sunrise_gameplay::game::ServerGame<'_>, conn| {
            let guest = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            guest.advancements.has(ID, "d")
        };
        for (conn, in_transaction) in [(13, false), (12, true)] {
            let player = game.player(u128::from(conn)).unwrap();
            let case = format!("guest {conn}, in transaction: {in_transaction}");
            let host: &mut dyn ServerHost = game;
            let given = if in_transaction {
                host.transaction(|host, _| Ok::<_, ()>(player.give_stack(host, diamond.clone())))
                    .unwrap()
            } else {
                player.give_stack(host, diamond.clone())
            };
            assert_eq!(given, Ok(None), "{case}");
            assert_eq!(diamonds(game, conn), 1, "{case}");
            assert!(!done(game, conn), "{case}: not judged in the hook's level");
            let fake_only = [before[0], before[1] + 1];
            assert_eq!(dropped(game), fake_only, "{case}: only the fake pickup");
            // Another command run in the borrowed turn leaves the judgment to their tick too.
            game.as_guest(conn, |game| {
                assert!(game.native_run_command("time query gametime"));
            })
            .unwrap();
            assert!(!done(game, conn), "{case}: not judged by another command");
            assert_eq!(dropped(game), fake_only, "{case}: nothing dropped by it");

            game.enter_level(Dim::Nether).unwrap();
            game.as_guest(conn, |game| game.tick_advancements())
                .unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert!(done(game, conn), "{case}: done on their own tick");
            // Their inventory is full: the reward lies in the Nether, where they are.
            assert_eq!(
                dropped(game),
                [before[0], before[1] + 2],
                "{case}: the reward drops in their level"
            );
            game.enter_level(Dim::Nether).unwrap();
            let reward = game.dropped.last().unwrap();
            assert_eq!(&*reward.stack.item, "minecraft:emerald");
            game.dropped.clear();
            game.enter_level(Dim::Overworld).unwrap();
        }
    });
}

/// `execute in the_nether run give @s` by an Overworld guest whose inventory is full: the
/// diamond that doesn't fit lies in their own level, the Overworld, and nothing is in the
/// Nether, which the command put in place.
#[test]
#[ignore = "daily"]
fn give_to_oneself_under_execute_in_another_level_drops_in_the_players_level() {
    use sunrise_gameplay_common::game::lan_actions::GuestCommand;
    with_guests(|game| {
        game.build_registries();
        game.level.player_uuid = Some(playerdata::uuid_ints(12));
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| {
                g.inventory
                    .fill(Some(ItemStack::new("minecraft:stone", 64)))
            })
            .unwrap();
        let before = dropped(game);
        let told = game.run_guest_command_told(GuestCommand {
            conn: 13,
            command: "execute in minecraft:the_nether run give @s minecraft:diamond".into(),
            permission: 4,
            integrated: true,
            quiet: false,
        });
        assert!(
            told.as_ref().is_some_and(|(ok, _)| *ok),
            "{:?}",
            told.map(|t| t.1.iter().map(|l| l.plain()).collect::<Vec<_>>())
        );
        assert_eq!(game.dim, Dim::Overworld, "the command's own level is back");
        assert_eq!(diamonds(game, 13), 0);
        assert_eq!(dropped(game), [before[0] + 1, before[1]]);
        let lying = game.dropped.iter().next().unwrap();
        assert_eq!(&*lying.stack.item, "minecraft:diamond");
        assert!((lying.position.x - 3.0).abs() < 3.0 && (lying.position.z - 5.0).abs() < 3.0);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
