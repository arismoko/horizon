//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use std::sync::atomic::{AtomicU64, Ordering};
/// A mod's block entity, read and changed through its fields
/// (`ModBlockEntityFields`) as a struct's: `entity.nbt`. Each change goes
/// through `DerefMut` and gives it a new `revision`, so a client tells
/// whether what it kept of it (`BlockEntityDraw::keep`) is still right
/// without comparing the whole entity, and no write can skip that. Its type
/// identity is fixed at construction and is not part of the mutable fields.
/// Returned guest snapshots apply only changed mutable fields through the same revision path.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[horizon_mod_api_macros::wasm(via = ModBlockEntitySnapshot, writeback = apply_snapshot)]
pub struct ModBlockEntity {
    pub(crate) id: String,
    pub(crate) fields: ModBlockEntityFields,
    pub(crate) revision: ModRevision,
}
/// What a `ModBlockEntity` can change: its saved compound without `id`,
/// `x`, `y`, `z` and `Items`, its item slots
/// (`Items`) when its block has a container (`ModContainer`), and the data
/// slots its menu shows (`ContainerData`: a furnace menu's flame and arrow),
/// which the mod sets and nothing saves.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModBlockEntityFields {
    pub nbt: Compound,
    pub items: Vec<Option<ItemStack>>,
    pub data: Vec<i32>,
}
/// Which contents a `ModBlockEntity` has had: one per entity made (a clone
/// is another), counted on by each change. Equal revisions, equal contents.
/// Neither saved nor compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModRevision {
    pub(crate) born: u64,
    pub(crate) changes: u64,
}
static MOD_BLOCK_ENTITIES_MADE: AtomicU64 = AtomicU64::new(0);
/// An entity's transport form; identity and revision are read-only on writeback.
pub(crate) struct ModBlockEntitySnapshot {
    pub(crate) id: String,
    pub(crate) fields: ModBlockEntityFields,
    pub(crate) revision: ModRevision,
}
impl From<&ModBlockEntity> for ModBlockEntitySnapshot {
    fn from(entity: &ModBlockEntity) -> Self {
        Self {
            id: entity.id.clone(),
            fields: entity.fields.clone(),
            revision: entity.revision,
        }
    }
}
impl From<ModBlockEntitySnapshot> for ModBlockEntity {
    fn from(snapshot: ModBlockEntitySnapshot) -> Self {
        Self {
            id: snapshot.id,
            fields: snapshot.fields,
            revision: snapshot.revision,
        }
    }
}
impl ModBlockEntity {
    /// Apply returned mutable state through the native revision path, never its identity.
    pub(crate) fn apply_snapshot(&mut self, snapshot: ModBlockEntitySnapshot) {
        if self.fields != snapshot.fields {
            *std::ops::DerefMut::deref_mut(self) = snapshot.fields;
        }
    }
    pub fn new(id: String, fields: ModBlockEntityFields) -> Self {
        Self {
            id,
            fields,
            revision: ModRevision {
                born: MOD_BLOCK_ENTITIES_MADE.fetch_add(1, Ordering::Relaxed),
                changes: 0,
            },
        }
    }
    /// The block entity's immutable type id (`mymod:machine`).
    ///
    /// ```compile_fail
    /// let mut entity = horizon_client_render::containers::ModBlockEntity::default();
    /// entity.id = "other:machine".into();
    /// ```
    ///
    /// ```compile_fail
    /// let mut entity = horizon_client_render::containers::ModBlockEntity::default();
    /// entity.id().push_str("_changed");
    /// ```
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn revision(&self) -> ModRevision {
        self.revision
    }
    /// A client's copy loads the tag the server sent (`loadWithComponents`
    /// in `ClientPacketListener.handleBlockEntityData`): `tag`'s fields
    /// replace its own of the same names. One it already has as sent is
    /// left alone, so a tag sent again unchanged keeps its revision.
    pub fn merge_update_tag(&mut self, tag: &Compound) {
        for (k, v) in tag {
            if self.nbt.get(k) != Some(v) {
                self.nbt.insert(k.clone(), v.clone());
            }
        }
    }
}
impl Default for ModBlockEntity {
    fn default() -> Self {
        Self::new(String::new(), ModBlockEntityFields::default())
    }
}
impl Clone for ModBlockEntity {
    fn clone(&self) -> Self {
        Self::new(self.id.clone(), self.fields.clone())
    }
}
impl PartialEq for ModBlockEntity {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.fields == other.fields
    }
}
impl std::fmt::Debug for ModBlockEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModBlockEntity")
            .field("id", &self.id)
            .field("fields", &self.fields)
            .finish()
    }
}
impl std::ops::Deref for ModBlockEntity {
    type Target = ModBlockEntityFields;
    fn deref(&self) -> &ModBlockEntityFields {
        &self.fields
    }
}
impl std::ops::DerefMut for ModBlockEntity {
    fn deref_mut(&mut self) -> &mut ModBlockEntityFields {
        self.revision.changes += 1;
        &mut self.fields
    }
}
/// The container of a mod block's block entity, as a `WorldlyContainer`
/// says it: its slot count, the slots each face reaches
/// (`getSlotsForFace`, by `Direction`: down, up, north, south, west, east),
/// and the slots hoppers may put into (`canPlaceItemThroughFace`) and take
/// from (`canTakeItemThroughFace`); and the vanilla menu that shows its
/// slots when the block is used, if any (`MOD_MENUS`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModContainer {
    pub slots: usize,
    pub faces: [Vec<usize>; 6],
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    pub menu: Option<String>,
}
/// The fluid tanks of a mod block's block entity, in the container's shape:
/// each tank's capacity in millibuckets, the tanks each face reaches, and
/// the tanks a fill may reach and a drain may take. The tanks' contents are
/// not here: they live in the entity's saved compound, under `fluids`, so
/// they survive restarts and the block's own tick reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModFluidTanks {
    /// Each tank's capacity in millibuckets, in tank order.
    pub tanks: Vec<i32>,
    pub faces: [Vec<usize>; 6],
    /// The tanks a fill may reach (`canPlaceItemThroughFace`).
    pub inputs: Vec<usize>,
    /// The tanks a drain may take from (`canTakeItemThroughFace`).
    pub outputs: Vec<usize>,
}
/// The energy buffer of a mod block's block entity (NeoForge's
/// `SimpleEnergyHandler`): its capacity, the most one insert and one extract
/// moves, and the faces a fill may come through and a drain may leave by.
/// The amount is not here: it lives in the entity's saved compound, under
/// `energy`, so it survives restarts and the block's own tick reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModEnergyBuffer {
    pub capacity: i32,
    /// The most one insert moves; 0 takes none.
    pub max_insert: i32,
    /// The most one extract moves; 0 gives none.
    pub max_extract: i32,
    /// The faces, by `Direction` number, a fill may come through.
    pub insert_faces: [bool; 6],
    /// The faces a drain may leave by.
    pub extract_faces: [bool; 6],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_block_entitys_revision_moves_with_each_change_and_only_then() {
        let mut machine = ModBlockEntity::new(
            "mymod:machine".into(),
            ModBlockEntityFields {
                data: vec![0],
                ..Default::default()
            },
        );
        let made = machine.revision();
        // Reads leave it.
        assert_eq!(machine.id(), "mymod:machine");
        assert_eq!(machine.data[0] + machine.nbt.len() as i32, 0);
        assert_eq!(machine.revision(), made);
        // Every way to change a field moves it.
        let mut seen = vec![made];
        let changes: [fn(&mut ModBlockEntity); 4] = [
            |m| m.data[0] = 1,
            |m| _ = m.nbt.insert("progress".into(), Tag::Int(3)),
            |m| m.items.push(None),
            |m| **m = ModBlockEntityFields::default(),
        ];
        for change in changes {
            change(&mut machine);
            assert_eq!(machine.id(), "mymod:machine");
            assert!(!seen.contains(&machine.revision()));
            seen.push(machine.revision());
        }
        // Another entity, or a clone, never shares one, even with equal
        // contents; equality is the contents' alone.
        let copy = machine.clone();
        assert_eq!(copy, machine);
        assert!(!seen.contains(&copy.revision()));
        let other_kind = ModBlockEntity::new("mymod:other".into(), machine.fields.clone());
        assert_ne!(other_kind, machine);
        let fresh = ModBlockEntity::default();
        assert_ne!(fresh.revision(), ModBlockEntity::default().revision());
    }

    #[test]
    fn returned_snapshot_changes_only_fields_and_keeps_native_revision_bookkeeping() {
        let mut entity = ModBlockEntity::new("test:machine".into(), Default::default());
        let revision = entity.revision();
        let mut snapshot = ModBlockEntitySnapshot::from(&entity);
        snapshot.id = "forged:identity".into();
        snapshot.revision = ModBlockEntity::default().revision();
        entity.apply_snapshot(snapshot);
        assert_eq!(entity.id(), "test:machine");
        assert_eq!(entity.revision(), revision);
        let mut snapshot = ModBlockEntitySnapshot::from(&entity);
        snapshot.fields.data = vec![17];
        entity.apply_snapshot(snapshot);
        assert_eq!(entity.data, [17]);
        assert_eq!(entity.id(), "test:machine");
        assert_ne!(entity.revision(), revision);
        let restored = ModBlockEntity::from(ModBlockEntitySnapshot::from(&entity));
        assert_eq!(restored.revision(), entity.revision());
    }

    #[test]
    fn an_update_tag_sent_again_unchanged_keeps_the_revision() {
        let mut machine = ModBlockEntity::default();
        let tag = Compound::from([
            ("progress".into(), Tag::Int(3)),
            ("fuel".into(), Tag::Int(9)),
        ]);
        machine.merge_update_tag(&tag);
        let merged = machine.revision();
        machine.merge_update_tag(&tag);
        assert_eq!(machine.revision(), merged);
        // One field changed: replaced, the other kept, the revision moved.
        machine.merge_update_tag(&Compound::from([("progress".into(), Tag::Int(4))]));
        assert_ne!(machine.revision(), merged);
        assert_eq!(
            machine.nbt,
            Compound::from([
                ("progress".into(), Tag::Int(4)),
                ("fuel".into(), Tag::Int(9)),
            ])
        );
    }
}
