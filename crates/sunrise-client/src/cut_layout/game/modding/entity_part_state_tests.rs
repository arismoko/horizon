//! Entity Part state (#796 stage 3) on real entities: one value each, kept with its entity
//! through edits, transactions, removal, unloading, reloading and changes of level.
use std::{
    collections::HashSet,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};
use sunrise_gameplay_common::game::GameGameExt1;

use mod_api::{
    AttachmentDescriptor, AttachmentKey, EntityPart, EntityPartState, EntityRef, EntityResidency,
    Hooks, HostTransaction, PartStateError, Registrar, ServerHost, WorldEpoch, edit_part_state,
    read_state,
};
use sunrise_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};
use sunrise_level::chunk::Chunk;
use sunrise_mod_runtime::attachments::{
    AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
};

use sunrise_gameplay_common::game::Dim;

fn i32_codec() -> AttachmentCodec<i32> {
    AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved: &Compound| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }
}

static CHARGE: AttachmentType<i32> = AttachmentType {
    namespace: "part_state",
    name: "charge",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved: &Compound| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync_codec: None,
};

/// Keeps a charge on each entity of its types.
#[derive(Clone)]
struct Charge {
    charge: AttachmentKey<i32>,
}
#[mod_api::sunrise_part]
impl EntityPart for Charge {
    fn register_goals(
        &self,
        _host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterGoalsEvent<'_, '_>,
    ) {
        let _ = event;
    }
}
impl EntityPartState for Charge {
    type Data = i32;
    fn state_key(&self) -> &AttachmentKey<i32> {
        &self.charge
    }
}

#[derive(Clone)]
struct StateKeys {
    charge: Charge,
    /// Kept for the session only: no codec.
    scratch: AttachmentKey<i32>,
}

/// Declares the general type `part_state:robot` and attaches `Charge` with state to two mob
/// types and to the robot; the `refused` case also attaches it to the refused kind `refused`.
struct StateMod {
    refused: Option<&'static str>,
    keys: Arc<Mutex<Option<StateKeys>>>,
}
#[mod_api::__subscriptions]
impl Hooks for StateMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let charge = r
            .register_attachment_type(
                AttachmentDescriptor::new("part_state:charge", || 0_i32).serialize(i32_codec()),
            )
            .unwrap();
        let scratch = r
            .register_attachment_type(AttachmentDescriptor::new("part_state:scratch", || 0_i32))
            .unwrap();
        let part = Charge { charge };
        *self.keys.lock().unwrap() = Some(StateKeys {
            charge: part.clone(),
            scratch,
        });
        r.declare_general_entity(
            "robot",
            mod_api::GeneralEntityCategory::Misc,
            0.8,
            1.9,
            None,
            mod_api::GeneralSavePolicy::Save,
            true,
            false,
            false,
            None,
            8,
            3,
            1,
            &[],
        )
        .unwrap();
        for kind in ["minecraft:cow", "minecraft:pig", "part_state:robot"]
            .into_iter()
            .chain(self.refused)
        {
            r.entity_part_with_state(kind, part.clone()).unwrap();
        }
    }
}

/// Libtest name of a `#[test]` in this module, derived from `module_path!()`.
macro_rules! test_name {
    ($test:ident) => {{
        let _: fn() = $test;
        let module = module_path!()
            .split_once("::")
            .map_or(module_path!(), |(_, path)| path);
        format!("{module}::{}", stringify!($test))
    }};
}

/// Runs the test `name` in a fresh process (installed mods are process-wide) with `StateMod`
/// installed, on a dedicated server's overworld with chunk 0 0 loaded.
fn with_state_mod(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &StateKeys),
) {
    if super::native_entity_refs::tests::run_fresh_cases(
        name,
        "SUNRISE_ENTITY_PART_STATE_CHILD",
        &["valid"],
    ) {
        return;
    }
    let keys = Arc::new(Mutex::new(None));
    with_mod(
        StateMod {
            refused: None,
            keys: keys.clone(),
        },
        |game| {
            let state_keys = keys.lock().unwrap().clone().expect("StateMod registered");
            // Registration is over once the game is built: the test runs in the world's phase.
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(Dim::Overworld);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.native_joins_skip();
            test(game, &state_keys);
        },
    );
}

/// `test` on a game with `state_mod` installed. The registration phase is scoped around the
/// whole fixture, since the game fixture registers the installed mods again and a general type
/// is only declared while that phase is current.
fn with_mod(state_mod: StateMod, test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    mod_platform::__private::initialize().expect("test registration requires the mod API runtime");
    let _phase = mod_platform::__private::RegistrationGuard::open();
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("part_state", Box::new(state_mod))].into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_game(test);
    });
}

/// A saved mob of `kind` with `uuid` at 1 70 3, in chunk 0 0.
fn mob_tag(kind: &str, uuid: i32) -> Tag {
    Tag::compound([
        ("id", Tag::String(kind.into())),
        ("UUID", Tag::IntArray(vec![uuid, 0, 0, 1])),
        (
            "Pos",
            Tag::List(
                [1.0f64, 70.0, 3.0]
                    .map(|n| Tag::Double(n.to_bits()))
                    .to_vec(),
            ),
        ),
    ])
}

/// The Nether, with the chunk the fixtures' mobs stand in: a mob is visible to the entity
/// getter only in an accessible section (`PersistentEntitySectionManager.getEntityGetter`).
fn enter_nether(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.enter_level(Dim::Nether).unwrap();
    if !game.world.chunks.contains_key(&(0, 0)) {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    }
}

/// A fresh mob of `kind` in the level the game is in, and its handle.
fn mob(game: &mut sunrise_gameplay::game::ServerGame<'_>, kind: &str, uuid: i32) -> EntityRef {
    let id = game.load_mob(mob_tag(kind, uuid), None).unwrap();
    game.entity(id).unwrap()
}

/// A saved `part_state:robot` record with `uuid`.
fn robot_tag(uuid: i32) -> Tag {
    sunrise_gameplay_common::game::general::GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0, 90.0],
        owner: None,
        custom_name: None,
        custom_name_visible: false,
        base: Default::default(),
        on_ground: false,
        fall_distance: 0.0,
        uuid: [uuid, 0, 0, 2],
        envelope: sunrise_gameplay_common::game::general::GeneralTypeEnvelope {
            type_key: "part_state:robot".to_owned(),
            payload_version: 1,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    }
    .to_tag()
}

fn stored(
    charge: &Charge,
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    entity: EntityRef,
) -> Option<i32> {
    charge
        .state_existing(game, entity, |value| value.cloned())
        .unwrap()
}

/// What `level`'s attachment index holds for `CHARGE`, by entity id.
fn indexed(game: &sunrise_gameplay::game::ServerGame<'_>, level: Dim) -> Vec<(u32, i32)> {
    let Some(index) = game.attachment_indexes.get(&level) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = index
        .lock()
        .unwrap()
        .snapshot(&("part_state".into(), "charge".into()))
        .iter()
        .map(|row| {
            let mod_api::AttachmentHolder::Entity(entity) = row.holder else {
                panic!("not an entity row");
            };
            (entity.id(), *row.entry.read(&CHARGE).unwrap())
        })
        .collect();
    rows.sort();
    rows
}

#[test]
#[ignore = "daily"]
fn entity_part_state_is_per_entity_through_edits_transactions_and_panics() {
    let name = test_name!(entity_part_state_is_per_entity_through_edits_transactions_and_panics);
    with_state_mod(&name, |game, keys| {
        let charge = &keys.charge;
        let (a, b) = (
            mob(game, "minecraft:cow", 11),
            mob(game, "minecraft:cow", 12),
        );
        // Reads insert nothing: an entity no edit reached keeps no map at all.
        assert_eq!(a.residency(game), EntityResidency::InPlace);
        assert_eq!(charge.state(game, a, Clone::clone), Ok(0));
        assert_eq!(stored(charge, game, a), None);
        assert!(!game.entity_attachments.contains_key(&a.id()));
        assert_eq!(charge.edit_state(game, a, |_, _tx, n| *n = 7), Ok(()));
        assert_eq!(
            (stored(charge, game, a), stored(charge, game, b)),
            (Some(7), None)
        );
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(a.id(), 7)],
            "bound at first store"
        );

        // A handle of another world open is refused, and creates nothing.
        let foreign = EntityRef::new(WorldEpoch::new(a.epoch().get() + 1), b.id());
        assert_eq!(foreign.residency(game), EntityResidency::Gone);
        assert_eq!(
            charge.state(game, foreign, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(
            charge.edit_state(game, foreign, |_, _tx, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert!(!game.entity_attachments.contains_key(&b.id()));

        // An inner edit commits into the outer transaction, which then aborts: the value goes
        // back, and with it the save gate.
        let aborted: Result<(), mod_api::TransactionError<()>> = (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(charge.edit_state(host, a, |_, _tx, n| *n = 11), Ok(()));
                assert_eq!(
                    charge.state(host, a, Clone::clone),
                    Ok(11),
                    "later steps see it"
                );
                assert!(
                    !sunrise_sim::join_seq::visibility::passes_settled(),
                    "no save while it is tentative"
                );
                Err(())
            });
        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
        assert_eq!(stored(charge, game, a), Some(7));
        assert!(sunrise_sim::join_seq::visibility::passes_settled());
        assert_eq!(indexed(game, Dim::Overworld), [(a.id(), 7)]);

        // A panic inside an edit leaves no default behind.
        let host: &mut dyn ServerHost = game;
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            charge.edit_state(host, b, |_, _tx, n| {
                *n = 5;
                panic!("edit fails");
            })
        }));
        assert!(panicked.is_err());
        assert!(game.transactions.is_empty());
        assert_eq!(stored(charge, game, b), None);

        // The same state can't be edited from inside its own edit; another key can.
        let inner = charge.edit_state(game, a, |host, _tx, n| {
            *n = 10;
            (
                charge.edit_state(host, a, |_, _tx, n| *n = 99),
                edit_part_state(host, a, &keys.scratch, |_, _tx, n| *n = 1),
            )
        });
        assert_eq!(inner, Ok((Err(PartStateError::Reentrant), Ok(()))));
        assert_eq!(stored(charge, game, a), Some(10));
        assert_eq!(
            read_state(game, a, &keys.scratch, |value| value.cloned()),
            Ok(Some(1))
        );

        // An undecodable saved value refuses before the closure runs and keeps its bytes.
        let mut saved = Attachments::default();
        CHARGE.set(&mut saved, Holder::Level, 0).unwrap();
        let mut raw = saved.try_to_nbt().unwrap();
        let Some(Tag::Compound(names)) = raw.get_mut(&"part_state".into()) else {
            unreachable!()
        };
        let Some(Tag::Compound(entry)) = names.get_mut(&"charge".into()) else {
            unreachable!()
        };
        entry.insert(
            "value".into(),
            Tag::Compound([("n".into(), Tag::String("ten".into()))].into()),
        );
        game.entity_attachments
            .insert(b.id(), Attachments::from_nbt(raw.clone()));
        game.bind_resident_entity_attachments(b.id()).unwrap();
        let decode = PartStateError::Attachment {
            namespace: "part_state",
            name: "charge",
            error: AttachmentError::Decode,
        };
        assert_eq!(charge.state(game, b, Clone::clone), Err(decode.clone()));
        let mut ran = false;
        assert_eq!(
            charge.edit_state(game, b, |_, _tx, _| ran = true),
            Err(decode)
        );
        assert!(!ran);
        assert_eq!(game.entity_attachments[&b.id()].try_to_nbt().unwrap(), raw);

        // A mob out of its list for its own tick can't be reached until it is back.
        let taken = game.mobs.lift();
        assert_eq!(a.residency(game), EntityResidency::Busy);
        assert_eq!(
            charge.state(game, a, Clone::clone),
            Err(PartStateError::HolderBusy)
        );
        assert_eq!(
            charge.edit_state(game, a, |_, _tx, n| *n = 1),
            Err(PartStateError::HolderBusy)
        );
        game.mobs
            .restore(taken, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        assert_eq!(a.residency(game), EntityResidency::InPlace);
        assert_eq!(stored(charge, game, a), Some(10));

        // A handle answers only in its own level.
        enter_nether(game);
        assert_eq!(a.residency(game), EntityResidency::OtherLevel);
        assert_eq!(
            charge.state(game, a, Clone::clone),
            Err(PartStateError::OtherLevel)
        );
        assert_eq!(
            charge.edit_state(game, a, |_, _tx, n| *n = 1),
            Err(PartStateError::OtherLevel)
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(a.residency(game), EntityResidency::InPlace);
        assert_eq!(stored(charge, game, a), Some(10));
    });
}

#[test]
#[ignore = "daily"]
fn a_removed_or_unloaded_entity_is_stale_and_a_reloaded_one_reads_its_saved_value() {
    let name =
        test_name!(a_removed_or_unloaded_entity_is_stale_and_a_reloaded_one_reads_its_saved_value);
    with_state_mod(&name, |game, keys| {
        let charge = &keys.charge;
        // No declaration means no allocation: a mob type no Part keeps state on gets no
        // attachment map and no index row from its spawn, a tick and a save.
        let sheep = mob(game, "minecraft:sheep", 20);
        game.server.tickets.0.update(None);
        game.tick_creature_entities().unwrap();
        let saved = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        assert!(
            saved
                .iter()
                .flat_map(|(_, tags)| tags)
                .any(|tag| sunrise_sim::entity_nbt::uuid(tag) == Some([20, 0, 0, 1])),
            "saved"
        );
        assert!(game.entity(sheep.id()).is_some(), "still in its level");
        assert!(!game.entity_attachments.contains_key(&sheep.id()));
        assert!(indexed(game, Dim::Overworld).is_empty(), "no index row");

        // Removed, then a new cow: the old handle is stale, the new one reads the default.
        let gone = mob(game, "minecraft:cow", 21);
        assert_eq!(charge.edit_state(game, gone, |_, _tx, n| *n = 3), Ok(()));
        assert!(game.entity_discard(gone));
        game.native_removals_flush();
        assert_eq!(
            charge.state(game, gone, Clone::clone),
            Err(PartStateError::Stale)
        );
        let again = mob(game, "minecraft:cow", 22);
        assert_ne!(again.id(), gone.id());
        assert_eq!(charge.state(game, again, Clone::clone), Ok(0));

        // Unloaded by the residency owner's walk, which writes each map into the entity's NBT
        // before the rows go, then loaded back from that NBT under a fresh id.
        assert_eq!(charge.edit_state(game, again, |_, _tx, n| *n = 7), Ok(()));
        assert_eq!(
            edit_part_state(game, again, &keys.scratch, |_, _tx, n| *n = 4),
            Ok(())
        );
        let saved = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert_eq!(
            charge.state(game, again, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert!(indexed(game, Dim::Overworld).is_empty());
        let saved = saved
            .into_iter()
            .flat_map(|(_, tags)| tags)
            .find(|tag| sunrise_sim::entity_nbt::uuid(tag) == Some([22, 0, 0, 1]))
            .unwrap();
        game.entity_attachments.remove(&again.id());
        game.world
            .loaded_entities
            .push(((0, 0), vec![saved.into()]));
        game.spawn_loaded_entities();
        assert_eq!(game.mobs.len(), 1, "the reloaded cow alone");
        let reloaded = game.mobs[0].id;
        let reloaded = game.entity(reloaded).unwrap();
        assert_ne!(reloaded, again);
        assert_eq!(
            charge.state(game, again, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(stored(charge, game, reloaded), Some(7));
        assert_eq!(
            read_state(game, reloaded, &keys.scratch, |value| value.cloned()),
            Ok(None),
            "session only"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_spawn_or_removal_inside_an_aborted_action_keeps_the_bindings() {
    let name = test_name!(a_spawn_or_removal_inside_an_aborted_action_keeps_the_bindings);
    with_state_mod(&name, |game, keys| {
        let charge = &keys.charge;
        let cow = mob(game, "minecraft:cow", 31);
        assert_eq!(charge.edit_state(game, cow, |_, _tx, n| *n = 7), Ok(()));

        // Removed inside an action: stale at once, but bound until the root commits; the abort
        // brings it back with its value.
        let pig = game.storage_scope(|game, _| {
            assert!(game.entity_discard(cow));
            assert_eq!(
                charge.state(game, cow, Clone::clone),
                Err(PartStateError::Stale)
            );
            assert_eq!(
                charge.edit_state(game, cow, |_, _tx, n| *n = 1),
                Err(PartStateError::Stale)
            );
            assert_eq!(
                indexed(game, Dim::Overworld),
                [(cow.id(), 7)],
                "nothing retired yet"
            );
            // A spawn inside it exists at once and keeps state; the abort removes both.
            let (Tag::Compound(nbt), _) = sunrise_core::nbt::parse_snbt("{}").unwrap() else {
                unreachable!()
            };
            let level = game.level("minecraft:overworld").unwrap();
            let pig = game
                .level_summon(level, "minecraft:pig", [1.5, 70.0, 1.5], nbt)
                .unwrap()
                .unwrap();
            assert_eq!(charge.edit_state(game, pig, |_, _tx, n| *n = 5), Ok(()));
            assert_eq!(
                indexed(game, Dim::Overworld),
                [(cow.id(), 7), (pig.id(), 5)]
            );
            (pig, false)
        });
        assert_eq!(charge.state(game, cow, Clone::clone), Ok(7));
        assert_eq!(
            charge.state(game, pig, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert!(!game.entity_attachments.contains_key(&pig.id()));
        assert_eq!(indexed(game, Dim::Overworld), [(cow.id(), 7)]);

        // A nested removal committed into its parent still waits; the root commit retires it.
        game.storage_scope(|game, _| {
            game.storage_scope(|game, _| {
                assert!(game.entity_discard(cow));
                ((), true)
            });
            assert_eq!(indexed(game, Dim::Overworld), [(cow.id(), 7)]);
            ((), true)
        });
        assert_eq!(
            charge.state(game, cow, Clone::clone),
            Err(PartStateError::Stale)
        );
        // The removal ran at the root commit; its row goes when the mob does, at the sweep.
        game.native_removals_flush();
        game.server.tickets.0.update(None);
        game.tick_creature_entities().unwrap();
        assert!(game.mobs.iter().all(|m| m.id != cow.id()));
        assert!(indexed(game, Dim::Overworld).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_lone_mob_or_general_row_moves_under_a_new_id_and_a_ridden_pig_keeps_its_handle() {
    let name = test_name!(
        a_lone_mob_or_general_row_moves_under_a_new_id_and_a_ridden_pig_keeps_its_handle
    );
    with_state_mod(&name, |game, keys| {
        let charge = &keys.charge;
        // A lone mob: rebuilt under a new id in the other level, its state with it.
        let cow = mob(game, "minecraft:cow", 41);
        assert_eq!(charge.edit_state(game, cow, |_, _tx, n| *n = 7), Ok(()));
        let state = game.take_transferred_entity_state(cow.id());
        game.mobs.retain(|m| m.id != cow.id());
        game.detach_entity(cow.id());
        assert!(indexed(game, Dim::Overworld).is_empty());
        enter_nether(game);
        let moved = game
            .load_mob_transferred(mob_tag("minecraft:cow", 41), None, Some(state))
            .unwrap();
        let moved = game.entity(moved).unwrap();
        assert_ne!(moved, cow);
        assert_eq!(stored(charge, game, moved), Some(7));
        assert_eq!(indexed(game, Dim::Nether), [(moved.id(), 7)]);
        assert_eq!(
            charge.state(game, cow, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(
            charge.edit_state(game, cow, |_, _tx, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert!(
            !game.entity_attachments.contains_key(&cow.id()),
            "no map on the old handle"
        );
        game.enter_level(Dim::Overworld).unwrap();

        // A general row: bound at its first store, then moved with its binding.
        let next = game.server.next_entity;
        game.spawn_loaded_entity((0, 0), robot_tag(42), None);
        let robot = game.resident_entities.general.rows[0].id;
        assert!(robot > next);
        let robot = game.entity(robot).unwrap();
        assert_eq!(charge.state(game, robot, Clone::clone), Ok(0));
        assert_eq!(charge.edit_state(game, robot, |_, _tx, n| *n = 9), Ok(()));
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(robot.id(), 9)],
            "first store"
        );
        let state = game.take_transferred_entity_state(robot.id());
        game.resident_entities
            .general
            .rows
            .retain(|row| row.id != robot.id());
        game.detach_entity(robot.id());
        enter_nether(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.spawn_loaded_entity((0, 0), robot_tag(42), Some(state));
        let arrived = game.resident_entities.general.rows[0].id;
        let arrived = game.entity(arrived).unwrap();
        assert_ne!(arrived, robot);
        assert_eq!(stored(charge, game, arrived), Some(9));
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7), (arrived.id(), 9)],
            "bound on transfer adopt"
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(
            charge.state(game, robot, Clone::clone),
            Err(PartStateError::Stale)
        );

        // A saddled pig a player rides keeps its id across: a guest on it steps into a portal
        // with it, the carrier path starts the guest's trip and parks the pig, and the arrival
        // (`arrive_vehicle`, what the trip's arrival runs once the exit is found) brings it to
        // the other level under the same handle.
        let rider = sunrise_gameplay::game::native_player_death::tests::add_guest(game);
        let mut tag = mob_tag("minecraft:pig", 43);
        if let Tag::Compound(map) = &mut tag {
            map.insert(
                "equipment".into(),
                Tag::compound([(
                    "saddle",
                    Tag::compound([("id", Tag::String("minecraft:saddle".into()))]),
                )]),
            );
            map.insert("NoAI".into(), Tag::Byte(1));
        }
        let pig = game.load_mob(tag, None).unwrap();
        let pig = game.entity(pig).unwrap();
        assert_eq!(charge.edit_state(game, pig, |_, _tx, n| *n = 5), Ok(()));
        let conn = game.server.lan.as_ref().unwrap().hub.guests[0].conn;
        assert_eq!(game.player(123), Some(rider));
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.vehicle = Some(pig.id());
            });
        super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            sunrise_core::position::ChunkPos::new(0, 0),
        );
        game.server.tickets.0.add_portal((0, 0));
        game.update_chunk_tickets();
        game.server
            .portals
            .entities
            .entry(pig.id())
            .or_default()
            .set_inside(
                sunrise_core::position::BlockPos::new(1, 70, 3),
                sunrise_sim::nether_portal::ENTITY_DELAY,
            );
        game.handle_entity_portals();
        game.carry_riders_through_portals();
        assert!(
            game.on_server_trip(conn),
            "the rider's trip carries the pig"
        );
        assert!(
            game.vehicle_waits_in(Dim::Overworld),
            "the pig waits for it"
        );
        assert_eq!(
            stored(charge, game, pig),
            Some(5),
            "still in its level while it waits"
        );
        enter_nether(game);
        game.arrive_vehicle(Dim::Overworld, pig.id(), Vec3::new(1.0, 70.0, 3.0), 0.0);
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].vehicle,
            Some(pig.id())
        );
        assert_eq!(game.entity(pig.id()), Some(pig));
        assert_eq!(stored(charge, game, pig), Some(5));
        assert_eq!(charge.edit_state(game, pig, |_, _tx, n| *n += 1), Ok(()));
        assert_eq!(stored(charge, game, pig), Some(6));
        assert!(indexed(game, Dim::Nether).contains(&(pig.id(), 6)));
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(
            charge.state(game, pig, Clone::clone),
            Err(PartStateError::OtherLevel)
        );
        assert!(indexed(game, Dim::Overworld).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_transfer_inside_an_aborted_action_leaves_the_state_with_the_entity() {
    let name = test_name!(a_transfer_inside_an_aborted_action_leaves_the_state_with_the_entity);
    with_state_mod(&name, |game, keys| {
        let charge = &keys.charge;
        let cow = mob(game, "minecraft:cow", 51);
        assert_eq!(charge.edit_state(game, cow, |_, _tx, n| *n = 7), Ok(()));
        let moved = game.storage_scope(|game, _| {
            assert_eq!(charge.edit_state(game, cow, |_, _tx, n| *n = 9), Ok(()));
            let state = game.take_transferred_entity_state(cow.id());
            game.mobs.retain(|m| m.id != cow.id());
            game.detach_entity(cow.id());
            assert_eq!(
                indexed(game, Dim::Overworld),
                [(cow.id(), 9)],
                "the source index keeps its row until the root closes"
            );
            enter_nether(game);
            let moved = game
                .load_mob_transferred(mob_tag("minecraft:cow", 51), None, Some(state))
                .unwrap();
            let moved = game.entity(moved).unwrap();
            assert_eq!(stored(charge, game, moved), Some(9));
            assert_eq!(charge.edit_state(game, moved, |_, _tx, n| *n = 11), Ok(()));
            game.enter_level(Dim::Overworld).unwrap();
            (moved, false)
        });

        // Travel is not undone: the map stays with the cow under its new id, with the writes the
        // action made rolled back; the old handle stays stale and its row is gone.
        assert!(indexed(game, Dim::Overworld).is_empty());
        assert_eq!(
            charge.state(game, cow, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert!(!game.entity_attachments.contains_key(&cow.id()));
        enter_nether(game);
        assert_eq!(stored(charge, game, moved), Some(7));
        assert_eq!(indexed(game, Dim::Nether), [(moved.id(), 7)]);

        // Two transfers in one action, A to B and B to C: the second take keeps B's map as it
        // is then (9), but the abort restores what the action first found (7), at C.
        game.enter_level(Dim::Overworld).unwrap();
        let a = mob(game, "minecraft:cow", 52);
        assert_eq!(charge.edit_state(game, a, |_, _tx, n| *n = 7), Ok(()));
        let (b, c) = game.storage_scope(|game, _| {
            assert_eq!(charge.edit_state(game, a, |_, _tx, n| *n = 9), Ok(()));
            let state = game.take_transferred_entity_state(a.id());
            game.mobs.retain(|m| m.id != a.id());
            game.detach_entity(a.id());
            enter_nether(game);
            let b = game
                .load_mob_transferred(mob_tag("minecraft:cow", 52), None, Some(state))
                .unwrap();
            let b = game.entity(b).unwrap();
            assert_eq!(stored(charge, game, b), Some(9));
            let state = game.take_transferred_entity_state(b.id());
            game.mobs.retain(|m| m.id != b.id());
            game.detach_entity(b.id());
            game.enter_level(Dim::Overworld).unwrap();
            let c = game
                .load_mob_transferred(mob_tag("minecraft:cow", 52), None, Some(state))
                .unwrap();
            let c = game.entity(c).unwrap();
            assert_eq!(stored(charge, game, c), Some(9));
            ((b, c), false)
        });

        assert!(game.transactions.is_empty());
        assert_eq!(
            stored(charge, game, c),
            Some(7),
            "the value the action first found"
        );
        assert_eq!(indexed(game, Dim::Overworld), [(c.id(), 7)]);
        for stale in [a, b] {
            assert_eq!(
                charge.state(game, stale, Clone::clone),
                Err(PartStateError::Stale)
            );
            assert!(
                !game.entity_attachments.contains_key(&stale.id()),
                "nothing left behind"
            );
        }
        enter_nether(game);
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7)],
            "no row for B"
        );

        // Taken inside an action that is rolled back before the arrival is adopted, as a portal
        // arrival waits for a later pass: the arrival carries what the action first found.
        game.enter_level(Dim::Overworld).unwrap();
        let late = mob(game, "minecraft:cow", 53);
        assert_eq!(charge.edit_state(game, late, |_, _tx, n| *n = 7), Ok(()));
        let state = game.storage_scope(|game, _| {
            assert_eq!(charge.edit_state(game, late, |_, _tx, n| *n = 9), Ok(()));
            let state = game.take_transferred_entity_state(late.id());
            game.mobs.retain(|m| m.id != late.id());
            game.detach_entity(late.id());
            (state, false)
        });
        assert!(game.transactions.is_empty());
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(c.id(), 7)],
            "its row retired"
        );
        assert!(!game.entity_attachments.contains_key(&late.id()));
        enter_nether(game);
        let arrived = game
            .load_mob_transferred(mob_tag("minecraft:cow", 53), None, Some(state))
            .unwrap();
        let arrived = game.entity(arrived).unwrap();
        assert_eq!(stored(charge, game, arrived), Some(7), "not the aborted 9");
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7), (arrived.id(), 7)]
        );

        // Both at once: A to B adopted inside the action, then B taken again and the action
        // rolled back before C arrives. The arrival carries what the action first found at A.
        game.enter_level(Dim::Overworld).unwrap();
        let first = mob(game, "minecraft:cow", 54);
        assert_eq!(charge.edit_state(game, first, |_, _tx, n| *n = 7), Ok(()));
        let (state, second) = game.storage_scope(|game, _| {
            assert_eq!(charge.edit_state(game, first, |_, _tx, n| *n = 9), Ok(()));
            let state = game.take_transferred_entity_state(first.id());
            game.mobs.retain(|m| m.id != first.id());
            game.detach_entity(first.id());
            enter_nether(game);
            let second = game
                .load_mob_transferred(mob_tag("minecraft:cow", 54), None, Some(state))
                .unwrap();
            let second = game.entity(second).unwrap();
            assert_eq!(stored(charge, game, second), Some(9));
            let state = game.take_transferred_entity_state(second.id());
            game.mobs.retain(|m| m.id != second.id());
            game.detach_entity(second.id());
            game.enter_level(Dim::Overworld).unwrap();
            ((state, second), false)
        });
        assert!(game.transactions.is_empty());
        let third = game
            .load_mob_transferred(mob_tag("minecraft:cow", 54), None, Some(state))
            .unwrap();
        let third = game.entity(third).unwrap();
        assert_eq!(stored(charge, game, third), Some(7), "not the aborted 9");
        for stale in [first, second] {
            assert_eq!(
                charge.state(game, stale, Clone::clone),
                Err(PartStateError::Stale)
            );
            assert!(
                !game.entity_attachments.contains_key(&stale.id()),
                "nothing left behind"
            );
        }
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(c.id(), 7), (third.id(), 7)]
        );
        enter_nether(game);
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7), (arrived.id(), 7)],
            "no row for B"
        );
    });
}

#[test]
#[ignore = "daily"]
fn entity_part_state_on_a_player_a_vanilla_family_or_an_unknown_type_refuses_the_load() {
    let name = test_name!(
        entity_part_state_on_a_player_a_vanilla_family_or_an_unknown_type_refuses_the_load
    );
    const CHILD: &str = "SUNRISE_ENTITY_PART_STATE_KIND";
    if super::native_entity_refs::tests::run_fresh_cases(
        &name,
        CHILD,
        &["player", "boat", "minecart", "unknown"],
    ) {
        return;
    }
    let (kind, reason) = match std::env::var(CHILD).unwrap().as_str() {
        "player" => (
            "minecraft:player",
            "players: respawn identity is not decided",
        ),
        "boat" => ("minecraft:oak_boat", "no entity part runs on this kind"),
        "minecart" => ("minecraft:minecart", "no entity part runs on this kind"),
        _ => ("part_state:nothing", "no such entity type"),
    };
    let loaded = std::panic::catch_unwind(AssertUnwindSafe(|| {
        with_mod(
            StateMod {
                refused: Some(kind),
                keys: Arc::new(Mutex::new(None)),
            },
            |_| {},
        );
    }));
    let panic = loaded.expect_err("the load is refused");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    let refusal = format!(
        "part_state: entity part {}::Charge on {kind}: state part_state:charge of type i32 \
         refused: entity type {kind}: {reason}",
        module_path!()
    );
    assert!(message.contains("Mod parts refused:"), "{message}");
    assert!(message.contains(&refusal), "{message}");
    // The accepted kinds of the same mod are not named.
    for accepted in ["minecraft:cow", "minecraft:pig", "part_state:robot"] {
        assert!(!message.contains(&format!("on {accepted}:")), "{message}");
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
