use mod_api::{EntityRef, WorldEpoch};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use sunrise_mod_runtime::attachments::{
    AttachmentRegistration, AttachmentType, Attachments, HasAttachmentLifetime, Holder,
    index::AttachmentIndex,
};
use sunrise_sim::join_seq::JoinList;

use std::cell::Cell;

type Spawn = fn(&mut crate::game::ServerGame<'_>);
thread_local! {
    static AFTER_TAKE: Cell<Option<Spawn>> = const { Cell::new(None) };
}

pub fn with_spawn(spawn: Spawn, tick: impl FnOnce()) {
    assert!(AFTER_TAKE.replace(Some(spawn)).is_none());
    tick();
    assert!(AFTER_TAKE.take().is_none(), "tick did not lift its batch");
}

pub fn after_take(game: &mut crate::game::ServerGame<'_>) {
    if let Some(spawn) = AFTER_TAKE.take() {
        spawn(game);
    }
}

pub struct ScratchDirectory(pub std::path::PathBuf);

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run each fixture's complete setup and actual action in both dimensions.
pub fn in_both_dimensions(mut test: impl FnMut(&mut crate::game::ServerGame<'_>, &PassBalance)) {
    for dimension in [
        sunrise_gameplay_common::game::Dim::Overworld,
        sunrise_gameplay_common::game::Dim::Nether,
    ] {
        // with_game isolates the initial SaveDir; newly opened dimensions use
        // world_root instead. Keep that root alive until the Game is dropped.
        let root = ScratchDirectory(std::env::temp_dir().join(format!(
            "sunrise-caller-balance-{}-{:?}-{dimension:?}",
            std::process::id(),
            std::thread::current().id()
        )));
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.server.world_root = root.0.clone();
            game.enter_level(dimension).unwrap();
            use sunrise_sim::join_seq::residency::{Kind, Owner};
            let balance = PassBalance {
                journal: game
                    .resident_entities
                    .table
                    .borrow()
                    .producer()
                    .observe_structural_events(),
                owner: Owner {
                    dimension: game.dim.key(),
                    kind: Kind::Mob,
                },
            };
            test(game, &balance);
            balance.assert_parked(game);
            assert_eq!(
                game.dim, dimension,
                "fixture changed the selected dimension"
            );
        });
    }
}

pub struct PassBalance {
    journal: sunrise_sim::join_seq::residency::Journal,
    owner: sunrise_sim::join_seq::residency::Owner,
}
impl PassBalance {
    pub fn drain_balanced(&self) -> usize {
        use sunrise_sim::join_seq::residency::Change;
        let mut events = Vec::new();
        self.journal.drain_into(&mut events);
        let mut depth = 0usize;
        let mut lifts = 0;
        for event in events {
            if event.owner != self.owner {
                continue;
            }
            match event.change {
                Change::Lifted => {
                    depth += 1;
                    lifts += 1;
                }
                Change::Restored => depth = depth.checked_sub(1).expect("restore without lift"),
                _ => {}
            }
        }
        assert_eq!(depth, 0, "actual caller abandoned a lifted pass");
        lifts
    }
    fn assert_parked(&self, game: &mut crate::game::ServerGame<'_>) {
        let home = game.dim;
        let away = match home {
            sunrise_gameplay_common::game::Dim::Overworld => {
                sunrise_gameplay_common::game::Dim::Nether
            }
            _ => sunrise_gameplay_common::game::Dim::Overworld,
        };
        game.enter_level(away).unwrap();
        self.drain_balanced();
        game.enter_level(home).unwrap();
    }
}

/// Inject a nested spawn into an actual lifted mob operation, then exercise the
/// next real entity pass with just that new owner so the caller's other state
/// remains available for its own assertions.
pub fn assert_mob_spawn_survives(
    game: &mut crate::game::ServerGame<'_>,
    balance: &PassBalance,
    action: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    const SPAWNED: u32 = 1_900_000_000;
    fn spawn(game: &mut crate::game::ServerGame<'_>) {
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut mob = sunrise_sim::mobs::Mob::new(
            SPAWNED,
            kind,
            sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            0.0,
        );
        mob.no_ai = true;
        game.mobs.push(mob);
    }
    balance.drain_balanced();
    let mut expected: Vec<_> = game.mobs.iter().map(|mob| (mob.id, mob.join)).collect();
    with_spawn(spawn, || action(game));
    assert!(
        balance.drain_balanced() > 0,
        "actual caller did not emit its lift"
    );
    let index = game
        .mobs
        .iter()
        .position(|m| m.id == SPAWNED)
        .expect("nested spawn was dropped");
    assert_eq!(game.mobs[index].age, 0, "spawn ticked in the current pass");
    let join = game.mobs[index].join;
    expected.push((SPAWNED, join));
    assert_eq!(
        game.mobs
            .iter()
            .map(|mob| (mob.id, mob.join))
            .collect::<Vec<_>>(),
        expected,
        "caller changed survivor identity or spawn admission order"
    );
    let mut original = game.mobs.lift();
    let spawned = original.extract(join).unwrap();
    game.mobs.push(spawned);
    if !game.world.chunks.contains_key(&(0, 0)) {
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
    }
    game.tick_creature_entities().unwrap();
    assert_eq!(game.mobs[0].id, SPAWNED);
    assert_eq!(game.mobs[0].join, join);
    assert_eq!(game.mobs[0].age, 1, "spawn missed the next entity pass");
    game.mobs.clear();
    game.mobs.restore(
        original,
        sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
    );
    balance.assert_parked(game);
}

/// Check a real player/guest projectile pickup pass and the next server flight pass.
pub fn assert_projectile_spawn_survives(
    game: &mut crate::game::ServerGame<'_>,
    action: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    const ID: u32 = 1_900_000_002;
    fn spawn(game: &mut crate::game::ServerGame<'_>) {
        let mut p = sunrise_sim::projectiles::Projectile::new(
            sunrise_sim::projectiles::ProjectileKind::Egg,
            sunrise_core::geometry::Vec3::new(3.5, 80.0, 3.5),
            None,
        );
        p.id = ID;
        game.creatures.projectiles.push(p);
    }
    let mut expected: Vec<_> = game
        .creatures
        .projectiles
        .iter()
        .filter(|p| p.id != 72)
        .map(|p| (p.id, p.join))
        .collect();
    with_spawn(spawn, || action(game));
    let p = game
        .creatures
        .projectiles
        .iter()
        .find(|p| p.id == ID)
        .expect("pickup dropped a nested projectile");
    assert_eq!(p.age, 0);
    let join = p.join;
    expected.push((ID, join));
    assert_eq!(
        game.creatures
            .projectiles
            .iter()
            .map(|p| (p.id, p.join))
            .collect::<Vec<_>>(),
        expected
    );
    let mut original = game.creatures.projectiles.lift();
    game.creatures
        .projectiles
        .push(original.extract(join).unwrap());
    game.tick_projectiles().unwrap();
    assert_eq!(game.creatures.projectiles[0].age, 1);
    assert_eq!(game.creatures.projectiles[0].join, join);
    game.creatures.projectiles.clear();
    game.creatures.projectiles.restore(
        original,
        sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
    );
}

static LIFECYCLE_VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "lifecycle_owner",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

/// Registration memory shared by slot-bearing adapters to the lifecycle walk.
#[derive(Default)]
pub struct SlotIdentities {
    values: Attachments,
    index: Arc<Mutex<AttachmentIndex<EntityRef>>>,
    registrations: HashMap<u32, AttachmentRegistration>,
}

impl SlotIdentities {
    pub fn observe<T: HasAttachmentLifetime>(
        &mut self,
        owners: &mut JoinList<T, false>,
        epoch: WorldEpoch,
        id: fn(&T) -> u32,
    ) {
        owners.for_each_mut(|owner| {
            let id = id(owner);
            self.observe_one(owner, epoch, id);
        });
    }

    pub fn observe_one<T: HasAttachmentLifetime>(
        &mut self,
        owner: &mut T,
        epoch: WorldEpoch,
        id: u32,
    ) {
        use std::collections::hash_map::Entry;
        match self.registrations.entry(id) {
            Entry::Occupied(original) => {
                let current = owner
                    .attachment_lifetime()
                    .registration()
                    .expect("known owner lost its registration");
                assert!(
                    original.get().same_registration(&current),
                    "owner {id} changed registration"
                );
            }
            Entry::Vacant(entry) => {
                LIFECYCLE_VALUE
                    .set(&mut self.values, Holder::Entity(id), id as i32)
                    .unwrap();
                owner.attachment_lifetime().bind(
                    &mut self.values,
                    &self.index,
                    EntityRef::new(epoch, id),
                );
                entry.insert(owner.attachment_lifetime().registration().unwrap());
            }
        }
    }
}
