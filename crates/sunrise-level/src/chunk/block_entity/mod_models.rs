//! Engine-owned mod block entity storage and revision tracking.
use super::published_shapes::{PublishedShapes, SHAPES_KEY};
use std::sync::{Arc, LazyLock};
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_game_data::{
    block_entity_nbt::saved_item_slot,
    block_entity_types::{self, RegisteredBlockEntityType},
};
use sunrise_items::items::ItemStack;

static ITEMS: LazyLock<JavaString> = LazyLock::new(|| "Items".into());

/// A mod's block entity, read and changed through its fields
/// (`ModBlockEntityFields`) as a struct's: `entity.nbt`. Each change goes
/// through `DerefMut` and gives it a new `revision`, so a client tells
/// whether what it kept of it (`BlockEntityDraw::keep`) is still right
/// without comparing the whole entity, and no write can skip that. Its type
/// identity is fixed at construction and is not part of the mutable fields.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModBlockEntity {
    pub(crate) id: String,
    /// Looked up once when the entity is made. `None` for an unknown type or before the
    /// process's type table is installed. Both the numeric id and keyed Parts are frozen.
    registered_type: Option<RegisteredBlockEntityType>,
    /// Its type's key, resolved with `registered_type`; `None` for an unregistered type.
    kind: Option<mod_api::BlockEntityTypeKey>,
    pub(crate) fields: ModBlockEntityFields,
    pub(crate) revision: ModRevision,
    /// What its block's collision and outline are while it holds some
    /// (`BlockEntityRef::set_shapes`). Saved and sent with it, outside its `nbt`.
    pub(crate) shapes: Option<Arc<PublishedShapes>>,
    /// Whether the shapes were cleared after holding some, which an update tag says
    /// (`shapes_update_tag`) so a client drops its copy. Neither saved, compared nor
    /// part of a mod's replaceable client fields.
    pub(crate) shapes_cleared: bool,
}
/// What a `ModBlockEntity` can change: its saved compound without `id`,
/// `x`, `y`, `z` or modeled `Items` rows, its item slots
/// when its block has a container (`ModContainer`), and the data
/// slots its menu shows (`ContainerData`: a furnace menu's flame and arrow),
/// which the mod sets and nothing saves.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModBlockEntityFields {
    pub nbt: Compound,
    pub items: Vec<Option<ItemStack>>,
    pub data: Vec<i32>,
}

impl ModBlockEntityFields {
    /// The retained `Items` rows are their own ownership record. A modeled
    /// stack taking a slot drops its pending saved row from the same snapshot.
    fn claim_modeled_item_rows(&mut self) {
        if let Some(Tag::List(rows)) = self.nbt.get_mut(&ITEMS) {
            rows.retain(|row| {
                saved_item_slot(row)
                    .is_none_or(|slot| self.items.get(slot).is_none_or(Option::is_none))
            });
        }
    }
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

impl ModRevision {
    /// The unique construction identity, copied into API revision snapshots.
    pub fn identity(self) -> u64 {
        self.born
    }
    /// The number of field writes since construction.
    pub fn changes(self) -> u64 {
        self.changes
    }
}
impl ModBlockEntity {
    /// Apply a validated callback snapshot in place, keeping this entity's identity.
    /// The game passes the snapshot's original write count, including no-op writes.
    pub fn apply_snapshot(&mut self, mut fields: ModBlockEntityFields, changes: u64) {
        fields.claim_modeled_item_rows();
        self.fields = fields;
        self.revision.changes = changes;
    }

    pub fn has_opaque_item_slot(&self, slot: usize) -> bool {
        matches!(self.nbt.get(&ITEMS), Some(Tag::List(rows)) if
            rows.iter().any(|row| saved_item_slot(row) == Some(slot)))
    }

    /// A write (including an explicit clear) supersedes raw rows for this index. `items` holds
    /// the slots up to the last one written, not the holder's capacity (its block declares
    /// that), so a slot past it reads as empty and a write grows `items` to reach it only.
    /// The caller has checked `slot` against the declared capacity.
    pub fn item_slot_mut(&mut self, slot: usize) -> &mut Option<ItemStack> {
        if let Some(Tag::List(rows)) = self.fields.nbt.get_mut(&ITEMS) {
            rows.retain(|row| saved_item_slot(row) != Some(slot));
        }
        if slot >= self.items.len() {
            self.items.resize(slot + 1, None);
        }
        &mut self.items[slot]
    }

    pub fn new(id: String, mut fields: ModBlockEntityFields) -> Self {
        fields.claim_modeled_item_rows();
        let registered_type = block_entity_types::registered(&id);
        let kind = registered_type.and_then(|_| mod_api::__private::block_entity_type_key(&id));
        Self::with_type(id, registered_type, kind, fields)
    }
    fn with_type(
        id: String,
        registered_type: Option<RegisteredBlockEntityType>,
        kind: Option<mod_api::BlockEntityTypeKey>,
        fields: ModBlockEntityFields,
    ) -> Self {
        Self {
            id,
            registered_type,
            kind,
            fields,
            revision: ModRevision {
                born: sunrise_mod_runtime::block_entities::new_identity(),
                changes: 0,
            },
            shapes: None,
            shapes_cleared: false,
        }
    }
    /// The shapes this block entity published for its block, if it holds any.
    pub fn shapes(&self) -> Option<&PublishedShapes> {
        self.shapes.as_deref()
    }
    /// Publishes `shapes`, or clears them with None; whether that changed them. A change moves
    /// the revision, so what was drawn from the old shapes is drawn again.
    pub fn set_shapes(&mut self, shapes: Option<Arc<PublishedShapes>>) -> bool {
        if self.shapes == shapes {
            return false;
        }
        self.shapes_cleared = shapes.is_none();
        self.shapes = shapes;
        self.revision.changes += 1;
        true
    }
    /// What goes beside the saved fields for the published shapes: they are not among
    /// `nbt`, so a mod's own writes to it never touch them.
    pub fn shapes_tag(&self) -> Option<(&'static str, Tag)> {
        self.shapes().map(|shapes| (SHAPES_KEY, shapes.to_tag()))
    }
    /// Whether the shapes were cleared after holding some and clients may not know yet.
    pub fn shapes_cleared(&self) -> bool {
        self.shapes_cleared
    }
    /// Carries a clear over to the entity that replaces this one on a data write.
    pub fn keep_shapes_cleared(&mut self, cleared: bool) {
        self.shapes_cleared = cleared && self.shapes.is_none();
    }
    /// [`Self::shapes_tag`] for an update tag, which also says when they were cleared (an
    /// empty compound), always as the entity holds them now.
    pub fn shapes_update_tag(&self) -> Option<(&'static str, Tag)> {
        self.shapes_tag().or_else(|| {
            self.shapes_cleared
                .then(|| (SHAPES_KEY, Tag::Compound(Compound::new())))
        })
    }
    /// The block entity's immutable type id (`mymod:machine`).
    ///
    /// ```compile_fail
    /// let mut entity = sunrise_level::containers::ModBlockEntity::default();
    /// entity.id = "other:machine".into();
    /// ```
    ///
    /// ```compile_fail
    /// let mut entity = sunrise_level::containers::ModBlockEntity::default();
    /// entity.id().push_str("_changed");
    /// ```
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The type's numeric wire ID, resolved from [`Self::id`] when the entity was made; `None`
    /// for a type the game has not registered.
    pub fn numeric_id(&self) -> Option<u32> {
        self.registered_type.map(|kind| kind.numeric_id)
    }

    /// The resolved route for a resident server tick, only when its type subscribes.
    pub fn tick_kind(&self) -> Option<mod_api::BlockEntityTypeKey> {
        self.registered_type?.server_ticker
    }
    /// Its type's key (`BlockEntity.getType`), resolved from [`Self::id`] when the entity was
    /// made; `None` for a type the game has not registered.
    pub fn kind(&self) -> Option<mod_api::BlockEntityTypeKey> {
        self.kind
    }
    pub fn revision(&self) -> ModRevision {
        self.revision
    }
    /// A client's copy loads the tag the server sent (`loadWithComponents`
    /// in `ClientPacketListener.handleBlockEntityData`): `tag`'s fields
    /// replace its own of the same names. One it already has as sent is
    /// left alone, so a tag sent again unchanged keeps its revision.
    ///
    /// The published shapes' field (`SHAPES_KEY`) is not one of the fields: it sets them, an
    /// empty compound clears them, and one that does not check is left out.
    pub fn merge_update_tag(&mut self, tag: &Compound) {
        for (k, v) in tag {
            if k.to_string().is_ok_and(|k| k == SHAPES_KEY) {
                match v {
                    Tag::Compound(empty) if empty.is_empty() => _ = self.set_shapes(None),
                    v => {
                        if let Some(shapes) = PublishedShapes::from_tag(v) {
                            self.set_shapes(Some(Arc::new(shapes)));
                        }
                    }
                }
            } else if self.nbt.get(k) != Some(v) {
                self.nbt.insert(k.clone(), v.clone());
            }
        }
    }
    /// Queue the one-shot fields the next block change sends, in place of any
    /// waiting; an empty batch clears them. The runtime keeps them under this
    /// entity's identity, so a save, a clone or a comparison never sees them
    /// and the revision stays.
    pub fn set_client_update_fields(&self, fields: Compound) {
        sunrise_mod_runtime::block_entities::set_client_update_fields(self.revision.born, fields);
    }
    /// Keep the queued one-shot fields for transaction rollback without consuming them.
    pub fn snapshot_client_update_fields(&self) -> Option<Compound> {
        sunrise_mod_runtime::block_entities::snapshot_client_update_fields(self.revision.born)
    }
    /// Take the queued one-shot fields for a tag about to go to every viewer.
    pub fn take_client_update_fields(&self) -> Option<Compound> {
        sunrise_mod_runtime::block_entities::take_client_update_fields(self.revision.born)
    }
    /// Whether one-shot fields wait for a block change.
    pub fn has_client_update_fields(&self) -> bool {
        sunrise_mod_runtime::block_entities::has_client_update_fields(self.revision.born)
    }
}
/// A removed or unloaded entity's queued fields go with it.
impl Drop for ModBlockEntity {
    fn drop(&mut self) {
        drop(self.take_client_update_fields());
    }
}
/// An empty mod block entity with no type id, for tests and defaults.
impl Default for ModBlockEntity {
    fn default() -> Self {
        Self::with_type(String::new(), None, None, ModBlockEntityFields::default())
    }
}
impl Clone for ModBlockEntity {
    fn clone(&self) -> Self {
        let mut copy = Self::with_type(
            self.id.clone(),
            self.registered_type,
            self.kind,
            self.fields.clone(),
        );
        copy.shapes.clone_from(&self.shapes);
        copy.shapes_cleared = self.shapes_cleared;
        copy
    }
}
impl PartialEq for ModBlockEntity {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.fields == other.fields && self.shapes == other.shapes
    }
}
impl std::fmt::Debug for ModBlockEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModBlockEntity")
            .field("id", &self.id)
            .field("fields", &self.fields)
            .field("shapes", &self.shapes)
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
    fn a_clients_copy_takes_published_shapes_from_the_update_tag_and_clears_them() {
        use mod_api::BlockEntityShapes;
        let shapes = PublishedShapes::new(BlockEntityShapes {
            outline: vec![],
            collision: vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]],
        })
        .unwrap();
        let mut sent = Compound::from([("progress".into(), Tag::Int(2))]);
        sent.insert(SHAPES_KEY.into(), shapes.to_tag());
        let mut copy = ModBlockEntity::new("mymod:machine".into(), Default::default());
        copy.merge_update_tag(&sent);
        assert_eq!(copy.shapes().unwrap().boxes(), shapes.boxes());
        // The shapes are no field of the mod's, and a clone keeps them.
        assert_eq!(copy.nbt.len(), 1);
        assert_eq!(copy.clone().shapes().unwrap().boxes(), shapes.boxes());
        // The same tag again leaves the revision; an empty compound clears them.
        let revision = copy.revision();
        copy.merge_update_tag(&sent);
        assert_eq!(copy.revision(), revision);
        copy.merge_update_tag(&Compound::from([(
            SHAPES_KEY.into(),
            Tag::Compound(Compound::new()),
        )]));
        assert!(copy.shapes().is_none());
        assert_ne!(copy.revision(), revision);
    }

    #[test]
    fn queued_client_update_fields_stay_out_of_clones_and_go_with_their_entity() {
        let machine = ModBlockEntity::new("mymod:machine".into(), Default::default());
        let made = machine.revision();
        let batch = Compound::from([("effect".into(), Tag::Int(23))]);
        machine.set_client_update_fields(batch.clone());
        assert_eq!(machine.revision(), made);
        let copy = machine.clone();
        assert_eq!(copy, machine);
        assert!(!copy.has_client_update_fields());
        drop(copy);
        assert!(machine.has_client_update_fields());
        assert_eq!(machine.take_client_update_fields(), Some(batch.clone()));
        assert_eq!(machine.take_client_update_fields(), None);
        machine.set_client_update_fields(batch);
        drop(machine);
        assert_eq!(
            sunrise_mod_runtime::block_entities::take_client_update_fields(made.identity()),
            None
        );
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
