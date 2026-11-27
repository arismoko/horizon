//! `player_destroyed_block` heard through the game's own break path: a creative and a survival
//! break are heard once with the removed state; a break `break_block` cancels and a piston's
//! are not heard.
use super::native_entity_refs::tests::{collect_all_entities, with_game};
use sunrise_command::args::GameMode;
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_sim::motion::PlayerMotion;

use mod_api::{BreakBlockEvent, Flow, Hooks, HostSide, PlayerDestroyedBlockEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// One heard break: the position, the state's block name and the block entity's saved data.
type Heard = ([i32; 3], String, Option<mod_api::Compound>);

struct Probe {
    heard: Arc<Mutex<Vec<Heard>>>,
    /// The hook and the `minecraft:mined` awards, in the order they came.
    order: Arc<Mutex<Vec<String>>>,
    cancel_break: Arc<AtomicBool>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn player_stat_award(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerStatAwardEvent<'_>,
    ) -> Flow {
        if event.stat_type.as_str() == "minecraft:mined" {
            let award = format!("{} {}", event.stat_type, event.stat);
            self.order.lock().unwrap().push(award);
        }
        Flow::Continue
    }
    fn break_block(&self, _: &mut dyn ServerHost, _: &mut BreakBlockEvent<'_>) -> Flow {
        if self.cancel_break.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn player_destroyed_block(&self, host: HostSide<'_>, event: &mut PlayerDestroyedBlockEvent) {
        let HostSide::Server(host) = host else {
            panic!("these breaks run on the server");
        };
        // The state is the block's default state, which the test placed.
        let name = ["minecraft:stone", "minecraft:chest"]
            .into_iter()
            .find(|name| host.block_state_id(name) == Some(event.state))
            .expect("the broken state is a default state the test placed")
            .to_owned();
        // The level is already past the removal: the block is gone from it.
        let level = host.level("minecraft:overworld").expect("the overworld");
        assert_eq!(
            level.block_state(host, event.pos),
            host.block_state_id("minecraft:air"),
        );
        assert!(
            host.players().acting_player().is_some(),
            "the breaker is the acting player"
        );
        self.order.lock().unwrap().push("destroyed".into());
        self.heard
            .lock()
            .unwrap()
            .push((event.pos, name, event.block_entity.clone()));
    }
}

fn place(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3], block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
}

fn block(game: &sunrise_gameplay_common::game::CommonGame<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// A server's game with its own player standing in the origin chunk, as the native player-break
/// tests set one up: their handle.
fn with_player(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    mode: GameMode,
    held: Option<ItemStack>,
) -> mod_api::PlayerRef {
    game.options.dedicated = Some(Vec::new());
    let _ = game
        .block_ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0));
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
    game.server.lan_acting_uuid = Some(123);
    game.server.lan_acting_entity = Some(20);
    game.server.lan_acting_name = Some("Breaker".into());
    game.server.lan_acting_permission = Some(0);
    game.set_mode(mode);
    game.common.inventory[game.common.selected] = held;
    // The acting player as a hook would name them: this server's entity 20.
    let epoch = game.native_lifecycle.epoch().unwrap();
    mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 20), 123)
}

#[test]
#[ignore = "daily"]
fn a_players_completed_break_is_heard_once_with_the_removed_state() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let order = Arc::new(Mutex::new(Vec::new()));
    let cancel_break = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "destroyed",
        Box::new(Probe {
            heard: heard.clone(),
            order: order.clone(),
            cancel_break: cancel_break.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let take = || std::mem::take(&mut *heard.lock().unwrap());

            // Survival, with the right tool: stone drops, and the break is heard.
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_pickaxe", 1)),
            );
            let stone = [6, 64, 4];
            place(game, stone, "minecraft:stone");
            assert!(game.player_destroy_block(player, stone, false));
            assert_eq!(block(game, stone), "minecraft:air");
            assert_eq!(take(), [(stone, "minecraft:stone".to_owned(), None)]);
            // Heard before `playerDestroy` counts the block mined; the count comes after.
            assert_eq!(
                std::mem::take(&mut *order.lock().unwrap()),
                ["destroyed", "minecraft:mined minecraft:stone"]
            );

            // Creative: no drops and an early return in vanilla, still heard once. A chest
            // carries its saved contents.
            let player = with_player(game, GameMode::Creative, None);
            let chest = [7, 64, 4];
            place(game, chest, "minecraft:chest");
            let mut contents = vec![None; 27];
            contents[0] = Some(ItemStack::new("minecraft:diamond", 3));
            game.world.block_entities.insert(
                (chest[0], chest[1], chest[2]),
                sunrise_level::containers::BlockEntity::Chest(contents),
            );
            assert!(game.player_destroy_block(player, chest, false));
            assert_eq!(block(game, chest), "minecraft:air");
            let heard_now = take();
            assert_eq!(heard_now.len(), 1);
            assert_eq!(
                (heard_now[0].0, heard_now[0].1.as_str()),
                (chest, "minecraft:chest")
            );
            let data = heard_now[0]
                .2
                .as_ref()
                .expect("the removed chest's saved data");
            let Some(mod_api::Tag::List(items)) = data.get(&"Items".into()) else {
                panic!("a chest saves its Items");
            };
            assert_eq!(items.len(), 1, "the one stack the chest held");
            let mod_api::Tag::Compound(stack) = &items[0] else {
                panic!("a saved stack is a compound");
            };
            assert_eq!(stack.get(&"count".into()), Some(&mod_api::Tag::Int(3)));
            assert_eq!(
                stack.get(&"id".into()),
                Some(&mod_api::Tag::String("minecraft:diamond".into()))
            );

            // A break `break_block` cancels keeps the block and is not heard.
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_pickaxe", 1)),
            );
            cancel_break.store(true, Ordering::Relaxed);
            let kept = [8, 64, 4];
            place(game, kept, "minecraft:stone");
            assert!(!game.player_destroy_block(player, kept, false));
            assert_eq!(block(game, kept), "minecraft:stone");
            assert!(take().is_empty());
            cancel_break.store(false, Ordering::Relaxed);

            // A piston pushing a torch breaks it from the level's side: not heard.
            let registry = game.pack.assets.registry;
            let piston = registry.default_state("minecraft:piston").unwrap();
            let piston = game.with_props(piston, &[("facing", "east")]).unwrap();
            let torch = registry.default_state("minecraft:torch").unwrap();
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            game.world.set(10, 70, 4, piston).unwrap();
            game.world.set(11, 70, 4, torch).unwrap();
            game.with_level(|b, level, ticks| {
                b.set_block(level, ticks, BlockPos::new(9, 70, 4), power, 3);
                b.run_block_events(level, ticks);
            });
            assert_ne!(
                block(game, [11, 70, 4]),
                "minecraft:torch",
                "the torch was pushed away"
            );
            assert!(take().is_empty(), "a level-side break is not a player's");
        });
    });
}

/// What one `player_destroyed_block` saw of the level and its player while it ran.
#[derive(Debug, PartialEq)]
struct Seen {
    pos: [i32; 3],
    /// `BlockStateParser.serialize` of the state the hook was told.
    state: String,
    /// Whether the acting player was the fake that broke the block.
    by_breaker: bool,
    /// Item entities in the level at that moment.
    items: usize,
    /// Experience points lying in orbs around the block at that moment.
    orb_points: i64,
    /// Whether the event carried block entity data.
    block_entity: bool,
}

/// Item entities in the overworld.
fn items(host: &dyn mod_api::Host) -> usize {
    let level = host.level("minecraft:overworld").expect("the overworld");
    collect_all_entities(host, level)
        .unwrap_or_default()
        .into_iter()
        .filter(|entity| {
            entity.kind(host).and_then(|kind| kind.id(host)).as_deref() == Some("minecraft:item")
        })
        .count()
}

/// Experience points in the orbs within 3 blocks of `pos`.
fn orb_points(host: &dyn mod_api::Host, pos: [i32; 3]) -> i64 {
    let level = host.level("minecraft:overworld").expect("the overworld");
    let at = pos.map(f64::from);
    let mut total = 0i64;
    let visit = level
        .visit_experience_orbs(host, at.map(|v| v - 3.0), at.map(|v| v + 4.0), &mut |orb| {
            let value = orb.value(host).expect("live orb value");
            let count = orb.count(host).expect("live orb count");
            total += i64::from(value) * i64::from(count);
            core::ops::ControlFlow::Continue(())
        })
        .expect("experience orb visit");
    assert_eq!(visit, core::ops::ControlFlow::Continue(()));
    total
}

/// Gives every break 7 experience points, and records and optionally rewrites what
/// `player_destroyed_block` sees.
struct OrderProbe {
    breaker: mod_api::PlayerRef,
    seen: Arc<Mutex<Vec<Seen>>>,
    /// A block `break_block` puts in place of the target, without cancelling the break.
    replace: Option<&'static str>,
    /// A block `player_destroyed_block` writes where the broken one stood.
    write: Option<&'static str>,
}

/// Sets the overworld's block at `pos` to `block`'s default state from a listener.
fn set(host: &mut dyn ServerHost, pos: [i32; 3], block: &str) {
    let state = host.block_state_id(block).expect("a known block");
    let level = host.level("minecraft:overworld").expect("the overworld");
    assert!(
        matches!(
            level.set_block(host, pos, state, mod_api::BlockUpdateFlags::CLIENTS),
            Ok(true)
        ),
        "the level is writable from the hook"
    );
}

#[mod_api::__subscriptions]
impl Hooks for OrderProbe {
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        *event.exp_to_drop = Some(7);
        if let Some(block) = self.replace {
            set(host, event.pos, block);
        }
        Flow::Continue
    }
    fn player_destroyed_block(&self, host: HostSide<'_>, event: &mut PlayerDestroyedBlockEvent) {
        let HostSide::Server(host) = host else {
            panic!("these breaks run on the server");
        };
        let acting = host.players().acting_player();
        let seen = Seen {
            pos: event.pos,
            state: host.block_state_string(event.state).unwrap_or_default(),
            by_breaker: acting == Some(self.breaker),
            items: items(&*host),
            orb_points: orb_points(&*host, event.pos),
            block_entity: event.block_entity.is_some(),
        };
        self.seen.lock().unwrap().push(seen);
        if let Some(block) = self.write {
            set(host, event.pos, block);
        }
    }
}

/// Runs `test` on a server with a fake player (`fake_action_tests::fake`) holding `item` in
/// survival, with an `OrderProbe` that puts `replace` and writes `write`: what the probe saw.
fn with_order_probe(
    item: &str,
    replace: Option<&'static str>,
    write: Option<&'static str>,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, mod_api::PlayerRef),
) -> Vec<Seen> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_game(|game| {
        let (player, ..) =
            sunrise_gameplay::game::native_server::fake_action_tests::fake(game, Some(item), 0);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "order",
            Box::new(OrderProbe {
                breaker: player,
                seen: seen.clone(),
                replace,
                write,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || test(game, player));
    });
    std::mem::take(&mut *seen.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn a_fakes_break_action_is_heard_before_the_drops_and_the_experience() {
    use mod_api::PlayerAction;
    let stone = [1, 63, 0];
    let mut after = None;
    let seen = with_order_probe("minecraft:diamond_pickaxe", None, None, |game, player| {
        assert_eq!(
            game.player_perform_action(player, PlayerAction::BreakBlock { pos: stone }),
            Ok(mod_api::ActionOutcome::Consumed)
        );
        assert_eq!(block(game, stone), "minecraft:air");
        after = Some((items(game), orb_points(game, stone)));
    });
    // Heard once, as the fake's own break, before `playerDestroy`: no dropped item and no
    // experience yet. (A fake keeps no statistics; the mined count's order is checked with
    // a real player in `a_players_completed_break_is_heard_once_with_the_removed_state`.)
    assert_eq!(
        seen,
        [Seen {
            pos: stone,
            state: "minecraft:stone".into(),
            by_breaker: true,
            items: 0,
            orb_points: 0,
            block_entity: false,
        }]
    );
    // Then the break finishes: the cobblestone dropped and the 7 points `break_block` asked
    // for.
    assert_eq!(after, Some((1, 7)));
}

#[test]
#[ignore = "daily"]
fn a_listener_writes_the_level_where_the_block_stood() {
    use mod_api::PlayerAction;
    let stone = [1, 63, 0];
    let mut after = None;
    let seen = with_order_probe(
        "minecraft:diamond_pickaxe",
        None,
        Some("minecraft:glass"),
        |game, player| {
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: stone }),
                Ok(mod_api::ActionOutcome::Consumed)
            );
            after = Some((items(game), orb_points(game, stone)));
            assert_eq!(block(game, stone), "minecraft:glass");
        },
    );
    assert_eq!(seen.len(), 1);
    // The write does not undo the break: the stone's drops and experience still come.
    assert_eq!(after, Some((1, 7)));
}

#[test]
#[ignore = "daily"]
fn shears_disarm_a_tripwire_and_the_hook_is_told_the_disarmed_string() {
    use mod_api::PlayerAction;
    let wire = [1, 64, 0];
    let seen = with_order_probe("minecraft:shears", None, None, |game, player| {
        place(game, wire, "minecraft:tripwire");
        assert_eq!(
            game.player_perform_action(player, PlayerAction::BreakBlock { pos: wire }),
            Ok(mod_api::ActionOutcome::Consumed)
        );
        assert_eq!(block(game, wire), "minecraft:air");
    });
    // TripWireBlock.playerWillDestroy sets the disarmed string in the level (flags 260)
    // before the removal, and the hook is told what the removal took out. (Vanilla's
    // Block.destroy is passed the armed one: playerWillDestroy returns the state it was given.)
    assert_eq!(
        seen.iter().map(|s| s.state.as_str()).collect::<Vec<_>>(),
        [
            "minecraft:tripwire[attached=false,disarmed=true,east=false,north=false,powered=false,south=false,west=false]"
        ]
    );
}

#[test]
#[ignore = "daily"]
fn a_target_break_block_replaced_is_told_with_its_own_block_entity() {
    use mod_api::PlayerAction;
    let target = [1, 63, 0];
    let seen = with_order_probe(
        "minecraft:diamond_pickaxe",
        Some("minecraft:chest"),
        None,
        |game, player| {
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: target }),
                Ok(mod_api::ActionOutcome::Consumed)
            );
            assert_eq!(block(game, target), "minecraft:air");
        },
    );
    // `break_block` put a chest where the stone was and let the break go on: the removal
    // takes the chest out, and the hook hears the chest with the chest's block entity, not
    // the stone the break started on.
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].state.as_str(), seen[0].block_entity),
        (
            "minecraft:chest[facing=north,type=single,waterlogged=false]",
            true
        )
    );
}

/// What `block_drops` named of the breaker, tool and block entity, the chest's saved
/// `Items` count and `CustomName` read while the event ran, and whether a resident handle or lent
/// data was given.
struct DropsSeen {
    breaker: Option<u128>,
    breaker_entity: Option<mod_api::EntityRef>,
    tool: Option<(String, u32)>,
    items: Option<usize>,
    custom_name: Option<mod_api::Tag>,
    has_block_entity: bool,
    has_block_entity_data: bool,
}

/// Listens to `block_drops` and keeps the block entity handle it was given, if any.
struct DropsProbe {
    seen: Arc<Mutex<Vec<DropsSeen>>>,
    handle: Arc<Mutex<Option<mod_api::BlockEntityRef>>>,
    /// Panics after reading, so the break unwinds through the event.
    panic: bool,
}

#[mod_api::__subscriptions]
impl Hooks for DropsProbe {
    fn block_drops(
        &self,
        level: &dyn mod_api::LevelHost,
        event: &mut mod_api::BlockDropsEvent<'_>,
    ) -> Flow {
        let data = match (event.block_entity, event.block_entity_data) {
            (None, Some(data)) => Some(data.clone()),
            (Some(entity), None) => level.block_entity_data(entity),
            (None, None) => None,
            (Some(_), Some(_)) => {
                panic!("a block entity is either resident or removed, never both")
            }
        };
        let items = data
            .as_ref()
            .and_then(|data| match data.get(&"Items".into()) {
                Some(mod_api::Tag::List(items)) => Some(items.len()),
                _ => None,
            });
        let custom_name = data
            .as_ref()
            .and_then(|data| data.get(&"CustomName".into()).cloned());
        let tool = event.tool.as_ref().map(|tool| {
            (
                mod_api::__private::item_key_id(tool.item())
                    .expect("the breaking tool is a known item")
                    .to_owned(),
                tool.count(),
            )
        });
        *self.handle.lock().unwrap() = event.block_entity;
        // The named-chest use: copy the chest's custom name onto the chest item it drops.
        if let Some(name) = &custom_name {
            for index in 0..event.drops.len() {
                let mut drop = event.drops.get_mut(index).unwrap();
                let is_chest = drop.stack().is_some_and(|stack| {
                    mod_api::__private::item_key_id(stack.item()) == Some("minecraft:chest")
                });
                if is_chest {
                    let mut stack = drop.stack().unwrap().project().unwrap();
                    let mut patch = mod_api::Compound::default();
                    patch.insert("minecraft:custom_name".into(), name.clone());
                    stack.set_components(patch).unwrap();
                    drop.set_stack(stack).unwrap();
                }
            }
        }
        self.seen.lock().unwrap().push(DropsSeen {
            breaker: event.breaker,
            breaker_entity: event.breaker_entity,
            tool,
            items,
            custom_name,
            has_block_entity: event.block_entity.is_some(),
            has_block_entity_data: event.block_entity_data.is_some(),
        });
        if self.panic {
            panic!("a listener fails after reading the broken block entity");
        }
        Flow::Continue
    }
}

fn put_chest(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) {
    place(game, pos, "minecraft:chest");
    let mut contents = vec![None; 27];
    contents[0] = Some(ItemStack::new("minecraft:diamond", 3));
    game.world.block_entities.insert(
        (pos[0], pos[1], pos[2]),
        sunrise_level::containers::BlockEntity::Chest(contents),
    );
}

#[test]
#[ignore = "daily"]
fn block_drops_lends_the_saved_only_pot_payload() {
    use mod_api::{Compound, Tag};
    use sunrise_gameplay_common::game::spill_common as spill;
    use sunrise_sim::mobs::MobEvent;

    enum BreakSite {
        Player,
        Level,
        Mob,
    }

    struct Observe(Arc<Mutex<Vec<(bool, Option<Compound>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn block_drops(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push((
                event.block_entity.is_some(),
                event.block_entity_data.cloned(),
            ));
            Flow::Continue
        }
    }

    with_game(|game| {
        let player = with_player(
            game,
            GameMode::Survival,
            Some(ItemStack::new("minecraft:diamond_pickaxe", 1)),
        );
        let item = Tag::compound([
            ("id", Tag::String("minecraft:diamond".into())),
            ("count", Tag::Int(3)),
        ]);
        for (pot, site) in [
            ([7, 64, 4], BreakSite::Player),
            ([8, 64, 4], BreakSite::Level),
            ([9, 64, 4], BreakSite::Mob),
        ] {
            let [x, y, z] = pot;
            place(game, pot, "minecraft:decorated_pot");
            spill::put_saved_entity(
                &mut game.world,
                pot,
                Tag::compound([
                    ("id", Tag::String("minecraft:decorated_pot".into())),
                    ("x", Tag::Int(x)),
                    ("y", Tag::Int(y)),
                    ("z", Tag::Int(z)),
                    ("keepPacked", Tag::Byte(0)),
                    ("item", item.clone()),
                ]),
            );
            assert!(game.world.block_entities.get(&(x, y, z)).is_none());
            let seen = Arc::new(Mutex::new(Vec::new()));
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "pot-probe",
                    Box::new(Observe(seen.clone())),
                )]
                .into(),
                || match site {
                    BreakSite::Player => assert!(game.player_destroy_block(player, pot, false)),
                    BreakSite::Level => {
                        // The level destroy path shared with `/setblock ... destroy`.
                        game.destroy_block_dropping(pot).unwrap();
                    }
                    BreakSite::Mob => {
                        let wither = game
                            .spawn_mob_raw("minecraft:wither", [4.5, 64.0, 4.5], false)
                            .unwrap();
                        game.handle_mob_events(vec![MobEvent::BreakBlock {
                            pos: pot,
                            drop: true,
                            by: Some(wither),
                            breaker: None,
                        }])
                        .unwrap();
                    }
                },
            );
            assert_eq!(block(game, pot), "minecraft:air");
            assert!(spill::saved_entity(&game.world, pot).is_none());
            let payload = Compound::from([("item".into(), item.clone())]);
            assert_eq!(*seen.lock().unwrap(), [(false, Some(payload))]);
        }
    });
}

#[test]
#[ignore = "daily"]
fn mob_block_drops_lend_the_original_entity_after_a_loot_listener_replaces_it() {
    use sunrise_sim::mobs::MobEvent;

    struct Replace(Arc<Mutex<Option<mod_api::BlockEntityRef>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Replace {
        fn loot_drops(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::LootDropsEvent<'_, '_>,
        ) {
            let Some((pos, _)) = event.context.block else {
                return;
            };
            let level = host.level("minecraft:overworld").unwrap();
            let before = host.block_entity(level, pos).unwrap();
            set(host, pos, "minecraft:air");
            set(host, pos, "minecraft:chest");
            let after = host.block_entity(level, pos).unwrap();
            assert_ne!(before, after);
            *self.0.lock().unwrap() = Some(after);
        }
    }

    let replacement = Arc::new(Mutex::new(None));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("replace", Box::new(Replace(replacement.clone()))),
            mod_api::ModEntry::new(
                "drops-probe",
                Box::new(DropsProbe {
                    seen: seen.clone(),
                    handle: handle.clone(),
                    panic: false,
                }),
            ),
        ]
        .into(),
        || {
            with_game(|game| {
                let _ = with_player(game, GameMode::Survival, None);
                let chest = [7, 64, 4];
                put_chest(game, chest);
                let name = mod_api::Tag::String("Original Chest".into());
                game.world.block_names.insert((7, 64, 4), name.clone());
                let wither = game
                    .spawn_mob_raw("minecraft:wither", [4.5, 64.0, 4.5], false)
                    .unwrap();
                game.handle_mob_events(vec![MobEvent::BreakBlock {
                    pos: chest,
                    drop: true,
                    by: Some(wither),
                    breaker: None,
                }])
                .unwrap();
                assert!(
                    replacement.lock().unwrap().is_some(),
                    "a different chest stood during the event"
                );
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0].items, Some(1));
                assert_eq!(seen[0].custom_name, Some(name));
                assert!(seen[0].has_block_entity_data);
                assert!(!seen[0].has_block_entity);
                assert_eq!(*handle.lock().unwrap(), None);
            })
        },
    );
}

/// A player's survival break names the player as `breaker` and `breaker_entity`, lends the
/// breaking tool as `ItemStackView` and the removed chest's saved data (including `CustomName`)
/// via `block_entity_data`, and does not expose a temporary `block_entity` handle for the
/// already-removed chest; a level-side break of a resident chest still hands out a real
/// `block_entity` handle and no lent `block_entity_data`.
#[test]
#[ignore = "daily"]
fn block_drops_lends_the_removed_chests_data_without_a_temporary_handle() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drops-probe",
        Box::new(DropsProbe {
            seen: seen.clone(),
            handle: handle.clone(),
            panic: false,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_axe", 1)),
            );
            let chest = [7, 64, 4];
            put_chest(game, chest);
            game.world.block_names.insert(
                (chest[0], chest[1], chest[2]),
                mod_api::Tag::String("Loot Chest".into()),
            );
            game.dropped.clear();
            assert!(game.player_destroy_block(player, chest, false));
            assert_eq!(block(game, chest), "minecraft:air");
            // The listener read the name the removal clears from the level and put it on the
            // chest item the break dropped.
            let named = game
                .dropped
                .iter()
                .filter(|item| &*item.stack.item == "minecraft:chest")
                .map(|item| {
                    item.stack
                        .extra
                        .as_ref()
                        .and_then(|extra| extra.custom_name_plain())
                })
                .collect::<Vec<_>>();
            assert_eq!(named, [Some("Loot Chest".to_owned())]);
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            let removed = &heard[0];
            assert_eq!(removed.breaker, Some(123));
            assert_eq!(
                removed.breaker_entity.map(|entity| entity.id()),
                Some(20),
                "the player's own entity"
            );
            assert_eq!(
                removed.tool,
                Some(("minecraft:diamond_axe".to_owned(), 1)),
                "the breaking tool is lent lazily"
            );
            assert!(
                !removed.has_block_entity,
                "no temporary handle for a removed block entity"
            );
            assert!(
                removed.has_block_entity_data,
                "removed chest data is lent read-only"
            );
            assert_eq!(
                removed.items,
                Some(1),
                "the chest's one stack, read during the event"
            );
            assert_eq!(
                removed.custom_name,
                Some(mod_api::Tag::String("Loot Chest".into())),
                "the removed chest's CustomName is included in block_entity_data"
            );
            assert!(handle.lock().unwrap().is_none());

            // A block with no block entity gives neither a handle nor lent data.
            let dirt = [8, 64, 4];
            place(game, dirt, "minecraft:dirt");
            assert!(game.player_destroy_block(player, dirt, false));
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            assert!(!heard[0].has_block_entity && !heard[0].has_block_entity_data);

            // A level-side break (`drop_resources_now`) of a resident chest placed in the world
            // hands out a real `block_entity` handle readable via `level.block_entity_data` and
            // leaves `block_entity_data` as `None`.
            let resident = [9, 64, 4];
            put_chest(game, resident);
            let chest_state = game.world.block(resident[0], resident[1], resident[2]);
            game.with_level(|_, level, _| {
                level.drop_resources_now(
                    BlockPos::new(resident[0], resident[1], resident[2]),
                    chest_state,
                );
            });
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            let resident_heard = &heard[0];
            assert_eq!(resident_heard.breaker, None);
            assert_eq!(resident_heard.breaker_entity, None);
            assert_eq!(resident_heard.tool, None);
            assert!(
                resident_heard.has_block_entity,
                "a resident chest keeps its real block_entity handle"
            );
            assert!(
                !resident_heard.has_block_entity_data,
                "a resident chest does not also lend removed block_entity_data"
            );
            assert_eq!(
                resident_heard.items,
                Some(1),
                "the resident chest's Items are read via level.block_entity_data"
            );
            let resident_handle = handle
                .lock()
                .unwrap()
                .expect("the resident chest's BlockEntityRef");
            assert_eq!(resident_handle.position(game), Some(resident));
        });
    });
}

/// A ravager's break (a mob's, not a player's) names the ravager as `breaker_entity` and no
/// `breaker` UUID; a break the level makes (no entity) names neither.
#[test]
#[ignore = "daily"]
fn block_drops_names_the_mob_that_broke_the_block() {
    use sunrise_sim::mobs::Mob;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drops-probe",
        Box::new(DropsProbe {
            seen: seen.clone(),
            handle: Arc::default(),
            panic: false,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            with_player(game, GameMode::Survival, None);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:ravager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(51, kind, Vec3::new(5.5, 64.0, 5.5), 0.0));
            let ravager = game.entity(51).expect("the ravager's handle");
            let leaves = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_leaves")
                .unwrap();
            game.world.set(9, 64, 4, leaves).unwrap();
            let no_entity = sunrise_gameplay_common::game::spill_common::BlockDropEntity::None;
            game.entity_block_dropped([9, 64, 4], leaves, Some(51), no_entity, Vec::new());
            game.world.set(10, 64, 4, leaves).unwrap();
            game.entity_block_dropped([10, 64, 4], leaves, None, no_entity, Vec::new());
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 2);
            assert_eq!(
                (heard[0].breaker, heard[0].breaker_entity),
                (None, Some(ravager))
            );
            assert_eq!((heard[1].breaker, heard[1].breaker_entity), (None, None));
        });
    });
}

/// A listener that panics inside the event leaves nothing held: no temporary handle was ever
/// handed out and the lent block-entity data drops with the break call.
#[test]
#[ignore = "daily"]
fn block_drops_holds_nothing_after_a_listener_panics() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drops-probe",
        Box::new(DropsProbe {
            seen: seen.clone(),
            handle: handle.clone(),
            panic: true,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_axe", 1)),
            );
            let chest = [7, 64, 4];
            put_chest(game, chest);
            // The platform may contain the panic or let it unwind through the break: either way.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.player_destroy_block(player, chest, false)
            }));
            let heard = seen.lock().unwrap();
            assert_eq!(heard.len(), 1, "the listener ran and read the chest");
            assert_eq!(heard[0].items, Some(1));
            assert!(!heard[0].has_block_entity);
            assert!(heard[0].has_block_entity_data);
            assert!(handle.lock().unwrap().is_none());
        });
    });
}

/// Installing a replacement chest at the broken position inside `player_destroyed_block` must
/// never leak the replacement's live `BlockEntityRef` into `block_drops`: breaking a chest still
/// lends only the original chest's `block_entity_data`, and breaking a plain block gives neither
/// `block_entity` nor `block_entity_data`.
#[test]
#[ignore = "daily"]
fn block_drops_ignores_a_replacement_block_entity_placed_after_removal() {
    struct ReplacementPlacer;
    #[mod_api::__subscriptions]
    impl Hooks for ReplacementPlacer {
        fn player_destroyed_block(
            &self,
            host: HostSide<'_>,
            event: &mut PlayerDestroyedBlockEvent,
        ) {
            let HostSide::Server(host) = host else {
                panic!("these breaks run on the server");
            };
            set(host, event.pos, "minecraft:chest");
            let level = host.level("minecraft:overworld").expect("the overworld");
            assert!(
                host.block_entity(level, event.pos).is_some(),
                "replacement chest installed a live block entity before drops fire"
            );
        }
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("replacement-placer", Box::new(ReplacementPlacer)),
        mod_api::ModEntry::new(
            "drops-probe",
            Box::new(DropsProbe {
                seen: seen.clone(),
                handle: handle.clone(),
                panic: false,
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_axe", 1)),
            );

            // Case 1: break a named chest with contents; replacement chest is placed before drops.
            let chest = [7, 64, 4];
            put_chest(game, chest);
            game.world.block_names.insert(
                (chest[0], chest[1], chest[2]),
                mod_api::Tag::String("Original Chest".into()),
            );
            assert!(game.player_destroy_block(player, chest, false));
            assert_eq!(block(game, chest), "minecraft:chest");
            assert!(
                game.world
                    .block_entities
                    .get(&(chest[0], chest[1], chest[2]))
                    .is_some(),
                "replacement chest remains resident after the break"
            );
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            assert!(
                !heard[0].has_block_entity,
                "replacement chest's live handle must not leak into block_drops"
            );
            assert!(heard[0].has_block_entity_data);
            assert_eq!(heard[0].items, Some(1));
            assert_eq!(
                heard[0].custom_name,
                Some(mod_api::Tag::String("Original Chest".into()))
            );
            assert!(handle.lock().unwrap().is_none());

            // Case 2: break stone (no block entity); replacement chest is placed before drops.
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_pickaxe", 1)),
            );
            let stone = [8, 64, 4];
            place(game, stone, "minecraft:stone");
            assert!(game.player_destroy_block(player, stone, false));
            assert_eq!(block(game, stone), "minecraft:chest");
            assert!(
                game.world
                    .block_entities
                    .get(&(stone[0], stone[1], stone[2]))
                    .is_some(),
                "replacement chest remains resident after the break"
            );
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            assert!(
                !heard[0].has_block_entity && !heard[0].has_block_entity_data,
                "a broken block with no block entity must not expose a replacement's handle"
            );
            assert!(handle.lock().unwrap().is_none());
        });
    });
}

/// What the client listener heard: whether the host was the client's, the position, the state's
/// block name and whether a block entity came with it.
#[cfg(feature = "client")]
type HeardOnClient = (bool, [i32; 3], String, bool);

#[cfg(feature = "client")]
struct ClientProbe(Arc<Mutex<Vec<HeardOnClient>>>);

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for ClientProbe {
    fn player_destroyed_block(&self, host: HostSide<'_>, event: &mut PlayerDestroyedBlockEvent) {
        let client = matches!(host, HostSide::Client(_));
        let host: &dyn mod_api::Host = match &host {
            HostSide::Server(host) => &**host,
            HostSide::Client(host) => &**host,
        };
        let name = host.block_state_string(event.state).unwrap_or_default();
        self.0
            .lock()
            .unwrap()
            .push((client, event.pos, name, event.block_entity.is_some()));
    }
}

/// The local player's survival break of a torch, which is instant: through the real attack
/// click, so the restriction checks that come before the prediction apply.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn the_breaking_client_hears_its_predicted_break_with_a_client_host() {
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_gameplay_common::game::{Hit, pick_ray::PickRayGameExt1 as _};
    use sunrise_protocol::{connection::Pipe, packets::play::SpawnInfo};

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "client-destroyed",
        Box::new(ClientProbe(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "DestroyedBlock").unwrap();
            let spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(
                &spawn,
                RemoteDimensionEntry::Login,
                sunrise_client::level_loading::Reason::Other,
            );
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let torch_pos = [1, 64, 1];
            let torch = game
                .pack
                .assets
                .registry
                .default_state("minecraft:torch")
                .unwrap();
            game.selected = 0;
            game.inventory[0] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
            game.set_test_forced_hit(Some(Hit {
                pos: torch_pos,
                face: [0, 1, 0],
                point: [1.5, 65.0, 1.5],
                inside: false,
            }));

            // Adventure with a pickaxe that has no `can_break`: the break is refused before it
            // is predicted, so nothing is heard and the torch stays.
            game.set_local_mode(GameMode::Adventure);
            game.world
                .set(torch_pos[0], torch_pos[1], torch_pos[2], torch)
                .unwrap();
            game.remote_start_attack();
            assert_eq!(block(game, torch_pos), "minecraft:torch");
            assert!(heard.lock().unwrap().is_empty());

            // Survival: the predicted removal succeeds and is heard once, by the client's host,
            // with the state that was there and no block entity.
            game.set_local_mode(GameMode::Survival);
            game.remote_start_attack();
            assert_eq!(block(game, torch_pos), "minecraft:air");
            assert_eq!(
                *heard.lock().unwrap(),
                [(true, torch_pos, "minecraft:torch".to_owned(), false)],
            );
        });
    });
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
