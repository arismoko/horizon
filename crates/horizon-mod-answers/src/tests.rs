//! The moved bodies against a small fake of the subsystem traits, with no game.
use crate::bodies;
use engine_access::{
    EntityIdentity, EntityLeash, EntityRead, EntityView, LevelRead, LevelWrite, UuidState,
};
use horizon_world::registry::BlockStateId;
use mod_api::{
    BlockUpdateFlags, ClipBlock, ClipFluid, ClipHit, EntityRef, LevelRef, LevelWriteError,
    WorldEpoch,
};
use std::collections::HashMap;

const EPOCH: u64 = 7;

fn entity(id: u32) -> EntityRef {
    EntityRef::new(WorldEpoch::new(EPOCH), id)
}

fn level() -> LevelRef {
    LevelRef::new(WorldEpoch::new(EPOCH), mod_api::__private::dimension_key("minecraft:overworld"))
}

enum Kind {
    /// Simulated by the fake's level: has everything.
    Simulated { position: [f64; 3], rotation: [f32; 2], kind: &'static str },
    /// A client's copy of a server entity: a position and a kind, no rotation.
    Tracked { position: [f64; 3], kind: &'static str },
}

#[derive(Default)]
struct Fake {
    entities: HashMap<u32, Kind>,
    uuids: HashMap<u32, u128>,
    assigned: u32,
    blocks_written: Vec<([i32; 3], u32)>,
    fences: Vec<[i32; 3]>,
    leashed: Vec<(u32, [i32; 3])>,
}

struct View<'a>(&'a Kind, EntityRef);

impl EntityView for View<'_> {
    fn position(&self) -> Option<[f64; 3]> {
        Some(match self.0 {
            Kind::Simulated { position, .. } | Kind::Tracked { position, .. } => *position,
        })
    }
    fn rotation(&self) -> Option<[f32; 2]> {
        match self.0 {
            Kind::Simulated { rotation, .. } => Some(*rotation),
            Kind::Tracked { .. } => None,
        }
    }
    fn kind(&self) -> Option<String> {
        Some(match self.0 {
            Kind::Simulated { kind, .. } | Kind::Tracked { kind, .. } => (*kind).to_owned(),
        })
    }
    fn level(&self) -> Option<LevelRef> {
        Some(level())
    }
}

impl EntityRead for Fake {
    type View<'a> = View<'a>;
    fn resolve(&self, entity: EntityRef) -> Option<View<'_>> {
        // A stale handle is one of another epoch.
        (entity.epoch() == WorldEpoch::new(EPOCH))
            .then(|| self.entities.get(&entity.id()))
            .flatten()
            .map(|kind| View(kind, entity))
    }
}

impl EntityIdentity for Fake {
    fn uuid_state(&self, entity: EntityRef) -> Option<UuidState> {
        self.resolve(entity)?;
        Some(
            self.uuids
                .get(&entity.id())
                .map_or(UuidState::Unassigned, |u| UuidState::Known(*u)),
        )
    }
    fn assign_uuid(&mut self, entity: EntityRef) -> u128 {
        self.assigned += 1;
        let uuid = 0xAB00 + u128::from(entity.id());
        self.uuids.insert(entity.id(), uuid);
        uuid
    }
}

impl EntityLeash for Fake {
    fn leash_to_knot(&mut self, entity: EntityRef, pos: [i32; 3]) -> bool {
        self.leashed.push((entity.id(), pos));
        true
    }
}

impl LevelRead for Fake {
    fn level_of(&self, dimension: &str) -> Option<LevelRef> {
        (dimension == "minecraft:overworld").then(level)
    }
    fn block_has_tag(&self, pos: [i32; 3], tag: &str) -> bool {
        tag == "minecraft:fences" && self.fences.contains(&pos)
    }
    fn clip(&self, _: LevelRef, from: [f64; 3], to: [f64; 3], _: ClipBlock, _: ClipFluid) -> Option<ClipHit> {
        // The ray's end is the hit: enough to see which ray was cast.
        Some(ClipHit { miss: false, block: to.map(|v| v.floor() as i32), face: mod_api::BlockFace::Up, location: from, inside: false, world_border: false })
    }
}

impl LevelWrite for Fake {
    fn set_block(&mut self, _: LevelRef, pos: [i32; 3], state: BlockStateId, _: BlockUpdateFlags) -> Result<bool, LevelWriteError> {
        self.blocks_written.push((pos, state.0));
        Ok(true)
    }
}

fn fake() -> Fake {
    let mut f = Fake::default();
    f.entities.insert(1, Kind::Simulated { position: [-0.5, 64.0, 10.9], rotation: [90.0, -15.0], kind: "minecraft:zombie" });
    f.entities.insert(2, Kind::Tracked { position: [3.0, 70.25, -1.0], kind: "minecraft:cow" });
    f
}

#[test]
fn block_position_floors_toward_negative_infinity() {
    assert_eq!(bodies::entity_block_position(&fake(), entity(1)), Some([-1, 64, 10]));
}

#[test]
fn tracked_only_entity_has_position_and_kind_but_no_rotation() {
    let f = fake();
    assert_eq!(bodies::entity_position(&f, entity(2)), Some([3.0, 70.25, -1.0]));
    assert_eq!(bodies::entity_kind(&f, entity(2)).as_deref(), Some("minecraft:cow"));
    assert_eq!(bodies::entity_rotation(&f, entity(2)), None);
    assert_eq!(bodies::entity_rotation(&f, entity(1)), Some([90.0, -15.0]));
}

#[test]
fn stale_or_unknown_handles_answer_none() {
    let f = fake();
    let stale = EntityRef::new(WorldEpoch::new(EPOCH + 1), 1);
    assert_eq!(bodies::entity_position(&f, stale), None);
    assert_eq!(bodies::entity_level(&f, entity(99)), None);
}

#[test]
fn uuid_is_assigned_on_first_ask_and_kept() {
    let mut f = fake();
    let first = bodies::entity_uuid(&mut f, entity(1));
    assert_eq!(first, Some(0xAB01));
    assert_eq!(bodies::entity_uuid(&mut f, entity(1)), first);
    assert_eq!(f.assigned, 1, "the second ask reads the one assigned");
    assert_eq!(bodies::entity_uuid(&mut f, entity(99)), None);
    assert_eq!(f.assigned, 1, "a handle that names nothing assigns nothing");
}

#[test]
fn a_block_is_written_where_the_entity_stands() {
    let mut f = fake();
    let written = bodies::entity_set_block_at(&mut f, entity(1), BlockStateId(5), BlockUpdateFlags::NONE);
    assert_eq!(written, Ok(true));
    assert_eq!(f.blocks_written, vec![([-1, 64, 10], 5)]);
    assert_eq!(
        bodies::entity_set_block_at(&mut f, entity(99), BlockStateId(5), BlockUpdateFlags::NONE),
        Err(LevelWriteError::Unavailable)
    );
    assert_eq!(f.blocks_written.len(), 1, "a stale handle writes nothing");
}

#[test]
fn leashing_needs_a_fence_and_an_entity() {
    let mut f = fake();
    f.fences.push([4, 64, 4]);
    assert!(bodies::entity_leash_to_knot(&mut f, entity(1), [4, 64, 4]));
    assert!(!bodies::entity_leash_to_knot(&mut f, entity(1), [5, 64, 4]), "no fence there");
    assert!(!bodies::entity_leash_to_knot(&mut f, entity(99), [4, 64, 4]), "no such entity");
    assert_eq!(f.leashed, vec![(1, [4, 64, 4])]);
}

#[test]
fn the_pick_ray_runs_along_the_view_for_the_distance_asked() {
    let f = fake();
    let hit = crate::goal::pick(&f, "minecraft:overworld", [0.0, 65.0, 0.0], [0.0, 0.0, 1.0], 4.5).unwrap();
    assert_eq!(hit.block, [0, 65, 4]);
    assert!(crate::goal::pick(&f, "minecraft:overworld", [0.0; 3], [0.0, 0.0, 1.0], -1.0).is_none());
    assert!(crate::goal::pick(&f, "minecraft:overworld", [0.0; 3], [0.0, 0.0, 1.0], f64::NAN).is_none());
    assert!(crate::goal::pick(&f, "minecraft:the_nether", [0.0; 3], [0.0, 0.0, 1.0], 1.0).is_none());
}
