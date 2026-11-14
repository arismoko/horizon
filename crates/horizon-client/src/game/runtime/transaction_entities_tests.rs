//! Entities added and removed inside a transaction (#796), on a real `Game`: an abort takes
//! back what the action spawned and undoes what it removed, telling no one; a commit announces
//! joins and runs removals at the root commit; and a spawn refuses near the end of the ids.
use std::sync::{Arc, Mutex};

use horizon_client::{chunk::Chunk, items::ItemStack, mobs::StoredEffect};
use horizon_core::nbt::{Tag, parse_snbt};
use horizon_mod_runtime::attachments::Holder;
use mod_api::{
    AttachmentType, EntityConstructingEvent, EntityJoinLevelEvent, EntityLeaveLevelEvent,
    EntityRef, Flow, Hooks, HostSide, RemovalReason, ServerHost,
};

use super::{Dim, Game, commands::Target, lan_entities::ENTITY_ID_HEADROOM};

/// Kept for the session only.
static MARK: AttachmentType<i32> = AttachmentType {
    namespace: "journal",
    name: "mark",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

/// What the observer heard, in order.
#[derive(Debug, Default, PartialEq)]
struct Heard {
    constructed: Vec<u32>,
    joined: Vec<u32>,
    left: Vec<u32>,
}

static HEARD: Mutex<Heard> = Mutex::new(Heard {
    constructed: Vec::new(),
    joined: Vec::new(),
    left: Vec::new(),
});

/// Hears construction, joins and leaves; marks each entity it constructs; keeps sheep out.
struct Observer;
#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
        if let (HostSide::Server(host), Holder::Entity(id)) = (host, event.holder) {
            HEARD.lock().unwrap().constructed.push(id);
            MARK.set(host, event.holder, 1).unwrap();
        }
    }
    fn entity_join_level(
        &self,
        _: &mut dyn ServerHost,
        event: &mut EntityJoinLevelEvent<'_>,
    ) -> Flow {
        HEARD.lock().unwrap().joined.push(event.entity);
        if event.kind == "minecraft:sheep" {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn entity_leave_level(&self, _: &mut dyn ServerHost, event: &mut EntityLeaveLevelEvent<'_>) {
        HEARD.lock().unwrap().left.push(event.entity);
    }
}

/// Runs the test `name` with the observer installed, in a fresh process (installed mods are
/// process-wide), on a dedicated server's overworld with chunk 0 0 loaded.
fn with_observer(name: &str, test: impl FnOnce(&mut Game<'_>)) {
    use super::native_entity_refs::tests::{run_fresh_cases, with_game};
    if run_fresh_cases(name, "HORIZON_TRANSACTION_ENTITIES_CHILD", &["valid"]) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("journal", Box::new(Observer))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.native_joins_skip();
            test(game);
        });
    });
}

/// Summons `kind` with `nbt` at 1.5 70 1.5: its id.
fn summon(game: &mut Game<'_>, kind: &str, nbt: &str) -> Option<u32> {
    let (Tag::Compound(nbt), _) = parse_snbt(nbt).unwrap() else {
        panic!("not a compound");
    };
    let dimension = game.dim.id();
    game.summon(dimension, kind, [1.5, 70.0, 1.5], nbt)
}

/// Takes what the observer heard so far.
fn heard() -> Heard {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

/// A boat with an announced cow riding it.
fn boat_with_rider(game: &mut Game<'_>) -> (EntityRef, EntityRef) {
    let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
    let cow = summon(game, "minecraft:cow", "{}").unwrap();
    let (boat, cow) = (game.entity(boat).unwrap(), game.entity(cow).unwrap());
    assert!(game.entity_start_riding(cow, boat, true));
    heard();
    (boat, cow)
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_spawn_then_abort_leaves_no_entity_map_or_hook() {
    const NAME: &str =
        "game::transaction_entities_tests::a_spawn_then_abort_leaves_no_entity_map_or_hook";
    with_observer(NAME, |game| {
        let (mobs, next) = (game.mobs.len(), game.next_entity);
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity(pig).is_some(), "in the level inside the action");
        assert_eq!(MARK.get(game, Holder::Entity(pig)), Ok(1));
        assert_eq!(heard().constructed, [pig], "constructed at the spawn call");
        game.transaction_close(&seal, false);

        assert_eq!(game.entity(pig), None);
        assert_eq!(game.mobs.len(), mobs);
        assert!(!game.entity_attachments.contains_key(&pig));
        assert!(!game.persist.uuids.contains_key(&pig));
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default(), "no join and no leave");
        assert_eq!(game.next_entity, next + 1, "its id is burnt");
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_spawn_joins_at_the_root_commit() {
    const NAME: &str = "game::transaction_entities_tests::a_spawn_joins_at_the_root_commit";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        game.native_joins_flush(false, 0);
        assert!(heard().joined.is_empty(), "the join waits");
        game.transaction_close(&seal, true);
        assert_eq!(heard().joined, [pig]);
        assert!(game.entity(pig).is_some());
        assert_eq!(MARK.get(game, Holder::Entity(pig)), Ok(1));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_join_cancelled_at_commit_leaves_a_stale_handle() {
    const NAME: &str =
        "game::transaction_entities_tests::a_join_cancelled_at_commit_leaves_a_stale_handle";
    with_observer(NAME, |game| {
        assert_eq!(summon(game, "minecraft:sheep", "{}"), None, "kept out at once");
        heard();
        let sheep_before = game.mobs.len();
        let seal = game.transaction_open();
        let sheep = summon(game, "minecraft:sheep", "{}").expect("the handle comes back at once");
        assert!(game.entity(sheep).is_some());
        game.transaction_close(&seal, true);
        assert_eq!(game.entity(sheep), None);
        assert_eq!(game.mobs.len(), sheep_before);
        game.native_leaves_flush();
        let heard = heard();
        assert_eq!(heard.joined, [sheep]);
        assert!(heard.left.is_empty(), "never announced, so it never leaves");
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn outside_a_transaction_joins_and_leaves_are_told_at_the_call() {
    const NAME: &str = "game::transaction_entities_tests::outside_a_transaction_joins_and_leaves_are_told_at_the_call";
    with_observer(NAME, |game| {
        let next = game.next_entity;
        let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
        assert_eq!(boat, next + 1);
        assert_eq!(
            heard(),
            Heard {
                constructed: vec![boat],
                joined: vec![boat],
                left: vec![],
            }
        );
        assert!(game.entity_discard(game.entity(boat).unwrap()));
        assert_eq!(heard().left, [boat]);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_removal_then_abort_keeps_the_row_map_and_rider() {
    const NAME: &str =
        "game::transaction_entities_tests::a_removal_then_abort_keeps_the_row_map_and_rider";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        let pig = game.entity(pig).unwrap();
        MARK.set(game, Holder::Entity(boat.id()), 7).unwrap();
        heard();
        let seal = game.transaction_open();
        assert!(game.entity_discard(boat));
        assert!(game.entity_remove(pig, RemovalReason::Killed));
        assert_eq!(game.entity(boat.id()), None, "stale at once");
        assert_eq!(game.entity(pig.id()), None);
        assert!(!game.entity_discard(boat), "not removed twice");
        assert!(game.boats.iter().any(|b| b.id == boat.id()), "the row stays");
        let mount = |game: &Game<'_>| game.mobs.iter().find(|m| m.id == cow.id()).unwrap().extra.mount;
        assert_eq!(mount(game), Some(boat.id()), "the rider stays on");
        game.transaction_close(&seal, false);

        assert_eq!(game.entity(boat.id()), Some(boat));
        assert_eq!(game.entity(pig.id()), Some(pig));
        assert_eq!(MARK.get(game, Holder::Entity(boat.id())), Ok(7));
        assert_eq!(game.entity_vehicle(cow), Some(boat));
        assert!(game.persist.removals.is_empty());
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_removal_then_commit_runs_each_effect_once() {
    const NAME: &str =
        "game::transaction_entities_tests::a_removal_then_commit_runs_each_effect_once";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let seal = game.transaction_open();
        assert!(game.entity_discard(boat));
        game.transaction_close(&seal, true);

        assert_eq!(game.entity(boat.id()), None);
        assert_eq!(game.entity_vehicle(cow), None, "the rider got off");
        let boat_target = Target::Boat(boat.id());
        let queued = game.persist.removals.iter();
        assert_eq!(queued.filter(|t| **t == boat_target).count(), 1);
        game.native_leaves_flush();
        assert!(!game.boats.iter().any(|b| b.id == boat.id()));
        assert_eq!(heard().left, [boat.id()], "told once");
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_killed_mobs_oozing_spawns_only_at_commit() {
    const NAME: &str =
        "game::transaction_entities_tests::a_killed_mobs_oozing_spawns_only_at_commit";
    with_observer(NAME, |game| {
        let zombie = summon(game, "minecraft:zombie", "{}").unwrap();
        let at = game.mobs.iter().position(|m| m.id == zombie).unwrap();
        (game.mobs[at].extra.effects).push(StoredEffect::new("minecraft:oozing", 600, 0));
        let slimes = |game: &Game<'_>| {
            let slime = horizon_client::mobs::Species::Slime;
            game.mobs.iter().filter(|m| m.species() == slime).count()
        };
        let zombie = game.entity(zombie).unwrap();
        let seal = game.transaction_open();
        assert!(game.entity_remove(zombie, RemovalReason::Killed));
        assert_eq!(slimes(game), 0);
        game.transaction_close(&seal, true);
        assert_eq!(slimes(game), 2);
        assert!(game.mobs.iter().any(|m| m.id == zombie.id() && m.removed));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_nested_commit_goes_with_its_parents_abort() {
    const NAME: &str =
        "game::transaction_entities_tests::a_nested_commit_goes_with_its_parents_abort";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let outer = game.transaction_open();
        let inner = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(boat));
        game.transaction_close(&inner, true);
        assert!(game.entity(pig).is_some());
        game.transaction_close(&outer, false);

        assert_eq!(game.entity(pig), None);
        assert_eq!(game.entity(boat.id()), Some(boat));
        assert_eq!(game.entity_vehicle(cow), Some(boat));
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard().joined, Vec::<u32>::new());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_nested_abort_takes_back_only_its_own() {
    const NAME: &str = "game::transaction_entities_tests::a_nested_abort_takes_back_only_its_own";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let outer = game.transaction_open();
        let kept = summon(game, "minecraft:pig", "{}").unwrap();
        let inner = game.transaction_open();
        let dropped = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(boat));
        game.transaction_close(&inner, false);
        assert_eq!(game.entity(dropped), None);
        assert_eq!(game.entity(boat.id()), Some(boat));
        game.transaction_close(&outer, true);

        assert!(game.entity(kept).is_some());
        assert_eq!(game.entity_vehicle(cow), Some(boat));
        game.native_leaves_flush();
        assert_eq!(
            heard(),
            Heard {
                constructed: vec![kept, dropped],
                joined: vec![kept],
                left: vec![],
            }
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_spawned_then_removed_entity_never_joins_or_leaves() {
    const NAME: &str =
        "game::transaction_entities_tests::a_spawned_then_removed_entity_never_joins_or_leaves";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        let kept = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(game.entity(boat).unwrap()));
        assert!(game.entity_discard(game.entity(pig).unwrap()));
        game.transaction_close(&seal, true);
        game.native_leaves_flush();
        game.native_joins_flush(false, 0);
        let heard = heard();
        assert_eq!(heard.joined, [kept]);
        assert!(heard.left.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn another_writer_sees_the_spawn_and_its_writes_roll_back() {
    const NAME: &str =
        "game::transaction_entities_tests::another_writer_sees_the_spawn_and_its_writes_roll_back";
    with_observer(NAME, |game| {
        let witness = summon(game, "minecraft:cow", "{}").unwrap();
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        // A second mod, lent the host inside the same action.
        let host: &mut dyn ServerHost = game;
        assert!(host.entity(pig).is_some(), "it sees the spawn");
        MARK.set(host, Holder::Entity(pig), 5).unwrap();
        MARK.set(host, Holder::Entity(witness), 5).unwrap();
        game.transaction_close(&seal, false);

        assert_eq!(game.entity(pig), None);
        assert!(!game.entity_attachments.contains_key(&pig));
        assert_eq!(MARK.get(game, Holder::Entity(witness)), Ok(1));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn ids_never_go_back_and_a_stale_handle_names_no_later_entity() {
    const NAME: &str = "game::transaction_entities_tests::ids_never_go_back_and_a_stale_handle_names_no_later_entity";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let aborted = summon(game, "minecraft:pig", "{}").unwrap();
        let handle = game.entity(aborted).unwrap();
        game.transaction_close(&seal, false);
        let next = summon(game, "minecraft:pig", "{}").unwrap();
        assert_eq!(next, aborted + 1);
        assert!(game.next_entity >= next);
        assert_eq!(game.native_entity_target(handle), None);
        assert_eq!(game.entity(aborted), None);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn late_id_kinds_spawned_then_aborted_are_removed() {
    const NAME: &str =
        "game::transaction_entities_tests::late_id_kinds_spawned_then_aborted_are_removed";
    with_observer(NAME, |game| {
        let counts = |game: &Game<'_>| {
            [
                game.tnts.len(),
                game.falling.len(),
                game.rockets.len(),
                game.tridents.len(),
                game.creatures.projectiles.len(),
            ]
        };
        let before = counts(game);
        let seal = game.transaction_open();
        for (kind, nbt) in [
            ("minecraft:tnt", "{fuse:80s}"),
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:firework_rocket", "{LifeTime:20}"),
            ("minecraft:trident", "{}"),
            ("minecraft:arrow", "{}"),
        ] {
            summon(game, kind, nbt).unwrap();
        }
        assert_eq!(counts(game), before.map(|n| n + 1));
        game.transaction_close(&seal, false);
        assert_eq!(counts(game), before);
        game.native_joins_flush(false, 0);
        assert!(heard().joined.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_traveller_arriving_inside_an_aborted_action_stays() {
    const NAME: &str =
        "game::transaction_entities_tests::a_traveller_arriving_inside_an_aborted_action_stays";
    with_observer(NAME, |game| {
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let item = summon(game, "minecraft:item", "{Item:{id:\"minecraft:diamond\",count:3}}");
        game.enter_level(Dim::Overworld).unwrap();
        let diamonds = |game: &Game<'_>| {
            (game.dropped.iter())
                .filter(|d| &*d.stack.item == "minecraft:diamond")
                .count()
        };
        let seal = game.transaction_open();
        // As `/execute in the_nether run tp …` does from inside the action.
        game.enter_level(Dim::Nether).unwrap();
        assert!(game.teleport_to_level(
            Target::Item(item.unwrap()),
            Dim::Overworld,
            [1.5, 70.0, 1.5],
            None
        ));
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(diamonds(game), 1, "arrived in the action's level");
        game.transaction_close(&seal, false);
        assert_eq!(diamonds(game), 1, "not taken out with the spawns");
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn an_aborted_spawn_is_in_no_tracking_pass() {
    const NAME: &str = "game::transaction_entities_tests::an_aborted_spawn_is_in_no_tracking_pass";
    with_observer(NAME, |game| {
        // Above the guest's fixed entity id.
        game.next_entity = 100;
        let _rx = super::lan::world_spawn_sync_tests::host_for(game);
        let tracked = |game: &mut Game<'_>, id: u32| {
            let mut states = horizon_client::lan::entity::States::default();
            game.lan_entities(&mut states, false, 0, None);
            states.as_slice().iter().any(|e| e.id == id as i32)
        };
        let seal = game.transaction_open();
        let aborted = summon(game, "minecraft:pig", "{}").unwrap();
        game.transaction_close(&seal, false);
        let seal = game.transaction_open();
        let committed = summon(game, "minecraft:pig", "{}").unwrap();
        game.transaction_close(&seal, true);
        assert!(!tracked(game, aborted));
        assert!(tracked(game, committed));
    });
}

/// One mod spawn function, called with everything it needs: whether it spawned.
type Spawn = fn(&mut Game<'_>, Fixture) -> bool;

#[derive(Clone, Copy)]
struct Fixture {
    level: mod_api::LevelRef,
    mob: EntityRef,
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn exhausted_ids_refuse_every_mod_spawn() {
    const NAME: &str = "game::transaction_entities_tests::exhausted_ids_refuse_every_mod_spawn";
    let rows: [(&str, Spawn); 12] = [
        ("summon", |game, _| {
            summon(game, "minecraft:pig", "{}").is_some()
        }),
        ("spawn_falling_block", |game, _| {
            let sand = (game.pack.assets.registry)
                .default_state("minecraft:sand")
                .unwrap();
            let dimension = game.dim.id();
            game.spawn_falling_block(
                dimension,
                sand,
                [1.5, 72.0, 1.5],
                [0.0; 3],
                Default::default(),
            )
        }),
        ("entity_spawn_at_location", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            game.entity_spawn_at_location(f.mob, stack, 0.0).is_some()
        }),
        ("drop_at", |game, _| {
            let before = game.dropped.len();
            game.drop_at([1, 70, 1], ItemStack::new("minecraft:stone", 1));
            game.dropped.len() > before
        }),
        ("level_spawn_mob", |game, f| {
            let reason = mod_api::SpawnReason::Command;
            (game.level_spawn_mob(f.level, "minecraft:cow", [1.5, 70.0, 1.5], reason)).is_some()
        }),
        ("level_block_drops", |game, f| {
            game.level_block_drops(f.level, [1, 70, 1], None, None)
                .is_some()
        }),
        ("level_explode", |game, f| {
            let none = mod_api::ExplosionInteraction::None;
            game.level_explode(f.level, [1.5, 70.0, 14.5], 1.0, false, none)
        }),
        ("level_spawn_item", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            (game.level_spawn_item(f.level, 1.5, 70.0, 1.5, stack, 10)).is_some()
        }),
        ("level_spawn_item_with_motion", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            game.level_spawn_item_with_motion(f.level, 1.5, 70.0, 1.5, stack, [0.0; 3], 10)
                .is_some()
        }),
        ("living_drop", |game, f| {
            let living = game.living(f.mob).unwrap();
            let stack = ItemStack::new("minecraft:stone", 1);
            game.living_drop(living, stack, true, false).is_some()
        }),
        ("player_drop_stack", |game, _| {
            let player = game.player(123).unwrap();
            let stack = ItemStack::new("minecraft:stone", 1);
            game.player_drop_stack(player, stack, false).is_none()
        }),
        ("spawn_projectile_from_rotation", |game, f| {
            let owner = game.living(f.mob).unwrap();
            let snowball = mod_api::Identifier::parse("minecraft:snowball").unwrap();
            let stack = ItemStack::new("minecraft:snowball", 1);
            (game.spawn_projectile_from_rotation(owner, &snowball, &stack, 0.0, 1.5, 1.0))
                .is_some()
        }),
    ];
    with_observer(NAME, |game| {
        let player = horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(1.5, 70.0, 1.5),
        );
        game.player = Some(player.unwrap());
        super::native_entity_refs::tests::bind_local_player(game, 20, 123);
        let mob = summon(game, "minecraft:zombie", "{}").unwrap();
        let fixture = Fixture {
            level: game.level(game.dim.id()).unwrap(),
            mob: game.entity(mob).unwrap(),
        };
        // Each spawns below the headroom; ids only ever go up.
        let last = u32::MAX - ENTITY_ID_HEADROOM;
        game.next_entity = last - 1024;
        for (name, spawn) in rows {
            assert!(spawn(game, fixture), "{name} spawns below the headroom");
        }
        assert!(!game.native_lifecycle.ids_exhausted_told);
        game.next_entity = last;
        for (name, spawn) in rows {
            let entities = game.live_entity_ids().len();
            assert!(!spawn(game, fixture), "{name} refuses");
            assert_eq!(game.next_entity, last, "{name} takes no id");
            assert_eq!(game.live_entity_ids().len(), entities, "{name} adds nothing");
        }
        assert!(game.native_lifecycle.ids_exhausted_told);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn exhausted_ids_stop_the_tick_spawners() {
    const NAME: &str = "game::transaction_entities_tests::exhausted_ids_stop_the_tick_spawners";
    with_observer(NAME, |game| {
        game.next_entity = u32::MAX - ENTITY_ID_HEADROOM - 1;
        game.tick_spawners();
        assert!(!game.native_lifecycle.ids_exhausted_told);
        game.next_entity = u32::MAX - ENTITY_ID_HEADROOM;
        game.tick_spawners();
        assert!(game.native_lifecycle.ids_exhausted_told, "named in the log");
        assert_eq!(game.next_entity, u32::MAX - ENTITY_ID_HEADROOM);
    });
}
