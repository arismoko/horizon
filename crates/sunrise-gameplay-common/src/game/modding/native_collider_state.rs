use crate::game::{
    Dim,
    remote::passengers::{PassengerSeat, passenger_root},
};
use std::collections::{HashMap, HashSet};
use sunrise_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_mod_runtime::attachments::Attachments;
use sunrise_sim::colliders::ColliderState;

/// The per-entity maps the game keeps by entity id: attachments, and colliders. It reads as
/// the attachments map, which is what most of the game wants; the colliders are beside it so
/// that what saves, loads and forgets an entity's attachments handles its collider too.
#[derive(Default)]
pub struct EntityMaps {
    pub(in crate::game) attachments: FxHashMap<u32, Attachments>,
    pub(in crate::game) colliders: FxHashMap<u32, ColliderState>,
    /// The ids of the colliders each level holds, so a level's tick places only its own.
    pub(in crate::game) by_level: HashMap<Dim, HashSet<u32>>,
}

impl std::ops::Deref for EntityMaps {
    type Target = FxHashMap<u32, Attachments>;
    fn deref(&self) -> &Self::Target {
        &self.attachments
    }
}

impl std::ops::DerefMut for EntityMaps {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.attachments
    }
}

impl EntityMaps {
    pub fn collider(&self, id: u32) -> Option<&ColliderState> {
        self.colliders.get(&id)
    }

    pub fn collider_mut(&mut self, id: u32) -> Option<&mut ColliderState> {
        self.colliders.get_mut(&id)
    }

    /// Gives the entity `id`, which stands in `level`, this collider.
    pub fn insert_collider(&mut self, id: u32, level: Dim, entry: ColliderState) {
        if self.colliders.insert(id, entry).is_some() {
            self.unindex(id);
        }
        self.by_level.entry(level).or_default().insert(id);
    }

    /// Takes the collider off the entity, wherever it stands.
    pub fn remove_collider(&mut self, id: u32) -> bool {
        let had = self.colliders.remove(&id).is_some();
        if had {
            self.unindex(id);
        }
        had
    }

    /// The entity `id` arrived in `level` with its collider.
    pub fn move_collider(&mut self, id: u32, level: Dim) {
        if self.colliders.contains_key(&id) {
            self.unindex(id);
            self.by_level.entry(level).or_default().insert(id);
        }
    }

    pub(in crate::game) fn unindex(&mut self, id: u32) {
        self.by_level.retain(|_, ids| {
            ids.remove(&id);
            !ids.is_empty()
        });
    }

    /// The ids of the colliders in `level`.
    pub fn colliders_in(&self, level: Dim) -> impl Iterator<Item = u32> + '_ {
        self.by_level.get(&level).into_iter().flatten().copied()
    }

    /// Writes the entity's collider into its saved compound.
    pub fn write_collider(&self, root: &mut Compound, id: u32) {
        match self.colliders.get(&id) {
            Some(entry) => {
                root.insert(SAVE_KEY.into(), Tag::Compound(entry.to_nbt()));
            }
            None => {
                root.remove(&SAVE_KEY.into());
            }
        }
    }
}

/// What a placed collider was made from: when all of these are as they were, so is it.
#[derive(PartialEq)]
pub struct PlacementKey {
    pub id: u32,
    pub serial: u64,
    pub origin: [u64; 3],
    /// The entities of its vehicle stack.
    pub stack: Vec<u32>,
}

/// The vehicle stack of `id` in a riding graph (passenger to vehicle): the root and everything
/// whose way up reaches it, `id` included. The walks are bounded by the graph's size, so a
/// cycle in what a server sent ends them.
pub fn stack_in_graph(vehicles: &HashMap<u32, PassengerSeat>, id: u32) -> Vec<u32> {
    let root_of = |at| passenger_root(vehicles, at).unwrap_or_else(|last| last);
    let root = root_of(id);
    let mut stack = vec![root];
    stack.extend(
        vehicles
            .keys()
            .copied()
            .filter(|&rider| root_of(rider) == root),
    );
    if !stack.contains(&id) {
        stack.push(id);
    }
    stack
}

pub fn origin_bits(origin: Vec3) -> [u64; 3] {
    [origin.x.to_bits(), origin.y.to_bits(), origin.z.to_bits()]
}

pub struct Placement {
    pub keys: Vec<PlacementKey>,
    pub prepared: sunrise_world::moving_colliders::Prepared,
}

/// The placements of each level's colliders, kept from tick to tick.
#[derive(Default)]
pub struct Placements(pub HashMap<Dim, Placement>);

/// The save key of an entity's collider, beside its attachments.
pub const SAVE_KEY: &str = "SunriseCollider";
