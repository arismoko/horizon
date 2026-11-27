//! Scoped editors for pending children in `baby_entity_spawn` and `mob_split`.
//! No resident entity ID, attachment holder, or join is published here: children live on the
//! dispatch stack or inside owned [`mod_api::UnspawnedMob`] values and drop cleanly via RAII if
//! left unattached.

use super::native_unspawned_mob;
use mod_api::{
    AttributeModifier, BabyEntitySpawnSlot, EquipmentSlot, Identifier, ItemStack,
    MobSplitChildList, PendingChildEditor, PendingChildWrite, PendingMobSpec, ServerHost,
    UnspawnedMob, UnspawnedMobError, UnspawnedMobRead, UnspawnedMobView, UnspawnedMobWrite,
};
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_mod_runtime::attachments::Attachments;
use sunrise_sim::mobs::{Mob, Species, finalize};
use sunrise_world::attributes::entity::Catalog;

/// Offspring constraint enforced at construction and attachment boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildConstraint {
    /// `MobSplitEvent.getChildren()` (`List<Mob>`): any registered mob species.
    Split,
    /// `BabyEntitySpawnEvent.setChild(AgeableMob)` from `Animal.spawnChildFromBreeding`.
    BreedingAgeable,
    /// `BabyEntitySpawnEvent.setChild` from `Fox.FoxBreedGoal.breed` (`(Fox) event.getChild()`).
    BreedingFox,
}

impl ChildConstraint {
    fn allows_species(self, species: Species) -> bool {
        match self {
            Self::Split => true,
            Self::BreedingAgeable => finalize::ageable(species),
            Self::BreedingFox => species == Species::Fox,
        }
    }
}

/// All unpublished per-object data travels with its actual Mob. A route must move its
/// existing UUID, attachments, and initialization state here, then install them only at
/// residency commit under an entity ID belonging to the destination world.
pub struct PendingMob {
    pub mob: Mob,
    /// The mob's UUID, kept off-table until publication at residency commit.
    pub uuid: Option<[i32; 4]>,
    /// All unpublished attachments (including populated constructor attachments, unbound).
    pub attachments: Attachments,
    /// Whether `EntityConstructing` has already run for this mob.
    pub initialized: bool,
    /// Constructor ID reservation `(action, id)`, valid only in the originating action.
    pub origin_reservation: Option<(u64, u32)>,
    pub dimension: super::Dim,
    pub resources: Arc<super::Resources>,
    pub attributes: Arc<Catalog>,
}

impl UnspawnedMobRead for PendingMob {
    #[inline]
    fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        native_unspawned_mob::read_mob_kind(&self.mob)
    }

    #[inline]
    fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        native_unspawned_mob::read_mob_position(&self.mob)
    }

    #[inline]
    fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        native_unspawned_mob::read_mob_equipment(&self.mob, slot)
    }

    #[inline]
    fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        native_unspawned_mob::read_mob_attribute_base(&self.mob, &self.attributes, attribute)
    }

    #[inline]
    fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        native_unspawned_mob::read_mob_attribute_value(&self.mob, &self.attributes, attribute)
    }

    #[inline]
    fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        native_unspawned_mob::read_mob_tags(&self.mob)
    }

    #[inline]
    fn custom_name(&self) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
        native_unspawned_mob::read_mob_custom_name(&self.mob)
    }
}

impl UnspawnedMobWrite for PendingMob {
    #[inline]
    fn set_equipment(
        &mut self,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError> {
        native_unspawned_mob::write_mob_equipment(
            &mut self.mob,
            self.resources.items(),
            &self.attributes,
            slot,
            item,
        )
    }

    #[inline]
    fn set_attribute_base(
        &mut self,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError> {
        native_unspawned_mob::write_mob_attribute_base(
            &mut self.mob,
            self.resources.items(),
            &self.attributes,
            attribute,
            base,
        )
    }

    #[inline]
    fn add_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError> {
        native_unspawned_mob::write_mob_add_attribute_modifier(
            &mut self.mob,
            self.resources.items(),
            &self.attributes,
            attribute,
            modifier,
        )
    }

    #[inline]
    fn remove_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError> {
        native_unspawned_mob::write_mob_remove_attribute_modifier(
            &mut self.mob,
            self.resources.items(),
            &self.attributes,
            attribute,
            id,
        )
    }

    #[inline]
    fn add_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        native_unspawned_mob::write_mob_add_tag(&mut self.mob, tag)
    }

    #[inline]
    fn remove_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        native_unspawned_mob::write_mob_remove_tag(&mut self.mob, tag)
    }

    #[inline]
    fn set_custom_name(&mut self, name: Option<mod_api::Text>) -> Result<(), UnspawnedMobError> {
        native_unspawned_mob::write_mob_set_custom_name(&mut self.mob, name)
    }
}

impl PendingChildWrite for PendingMob {
    #[inline]
    fn set_position(&mut self, position: [f64; 3]) -> Result<(), UnspawnedMobError> {
        if !super::native_mobs::spawnable(position) {
            return Err(UnspawnedMobError::Invalid);
        }
        self.mob.position = Vec3::new(position[0], position[1], position[2]);
        self.mob.previous = self.mob.position;
        Ok(())
    }

    #[inline]
    fn set_yaw(&mut self, yaw: f32) -> Result<(), UnspawnedMobError> {
        if !yaw.is_finite() {
            return Err(UnspawnedMobError::Invalid);
        }
        self.mob.yaw = yaw;
        self.mob.previous_yaw = yaw;
        self.mob.body_yaw = yaw;
        self.mob.previous_body_yaw = yaw;
        self.mob.head_yaw = yaw;
        self.mob.previous_head_yaw = yaw;
        Ok(())
    }

    #[inline]
    fn set_baby(&mut self, baby: bool) -> Result<(), UnspawnedMobError> {
        if baby && !finalize::supports_baby(self.mob.species()) {
            return Err(UnspawnedMobError::Invalid);
        }
        finalize::set_baby(&mut self.mob, baby);
        Ok(())
    }

    #[inline]
    fn set_size(&mut self, size: u32) -> Result<(), UnspawnedMobError> {
        if !size_ok(self.mob.species(), Some(size)) {
            return Err(UnspawnedMobError::Invalid);
        }
        apply_size(&mut self.mob, size);
        Ok(())
    }

    #[inline]
    fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// Constructs an unspawned pending child from `spec`, validating all domain invariants before
/// allocating or drawing RNG.
fn construct_pending_mob(
    resources: &Arc<super::Resources>,
    attributes: &Arc<Catalog>,
    dimension: super::Dim,
    constraint: ChildConstraint,
    spec: PendingMobSpec,
) -> Result<PendingMob, UnspawnedMobError> {
    if spec.dimension.to_string() != dimension.id()
        || !super::native_mobs::spawnable(spec.position)
        || !spec.yaw.is_finite()
    {
        return Err(UnspawnedMobError::Invalid);
    }
    let kind = resources
        .mob_types()
        .get(&spec.kind.to_string())
        .ok_or(UnspawnedMobError::Invalid)?
        .clone();
    if !constraint.allows_species(kind.species)
        || (spec.baby && !finalize::supports_baby(kind.species))
        || !size_ok(kind.species, spec.size)
        || (kind.species == Species::SulfurCube && !spec.baby && spec.size == Some(1))
    {
        return Err(UnspawnedMobError::Invalid);
    }
    // The UUID and complete constructor RNG are retained without publishing persist data.
    let (random, uuid) = sunrise_sim::mobs::living_random();
    let mut mob = Mob::with_random(
        0,
        kind,
        Vec3::new(spec.position[0], spec.position[1], spec.position[2]),
        spec.yaw,
        random,
    );
    finalize::set_baby(&mut mob, spec.baby);
    if let Some(size) = spec.size {
        apply_size(&mut mob, size);
    }
    Ok(PendingMob {
        mob,
        uuid: Some(uuid),
        attachments: Attachments::default(),
        initialized: false,
        origin_reservation: None,
        dimension,
        resources: resources.clone(),
        attributes: attributes.clone(),
    })
}

/// Scoped single-slot editor backing [`mod_api::BabyEntitySpawnChild`] for `baby_entity_spawn`.
pub struct BabyChildSlot<'a> {
    pub action: u64,
    pub slot: &'a mut Option<PendingMob>,
    pub resources: Arc<super::Resources>,
    pub attributes: Arc<Catalog>,
    pub dimension: super::Dim,
    pub constraint: ChildConstraint,
}

impl BabyEntitySpawnSlot for BabyChildSlot<'_> {
    fn is_present(&self) -> bool {
        self.slot.is_some()
    }

    fn get(&self) -> Option<UnspawnedMobView<'_>> {
        self.slot.as_ref().map(|p| UnspawnedMobView::new(p))
    }

    fn get_mut(&mut self) -> Option<PendingChildEditor<'_>> {
        self.slot.as_mut().map(|p| PendingChildEditor::new(p))
    }

    fn construct(
        &self,
        host: &mut dyn ServerHost,
        spec: PendingMobSpec,
    ) -> Result<UnspawnedMob, UnspawnedMobError> {
        let pending = construct_pending_mob(
            &self.resources,
            &self.attributes,
            self.dimension,
            self.constraint,
            spec,
        )?;
        mod_api::backend_mut_of(host)
            .ok_or(UnspawnedMobError::Unsupported)?
            .initialize_pending_child(UnspawnedMob::new(Box::new(pending)), self.action)
    }

    fn replace(&mut self, child: UnspawnedMob) -> Result<Option<UnspawnedMob>, UnspawnedMobError> {
        let mut pending = child
            .into_any()
            .downcast::<PendingMob>()
            .map_err(|_| UnspawnedMobError::Invalid)?;
        if !valid_child(&self.resources, self.dimension, self.constraint, &pending) {
            return Err(UnspawnedMobError::Invalid);
        }
        pending.resources = self.resources.clone();
        pending.attributes = self.attributes.clone();
        Ok(self
            .slot
            .replace(*pending)
            .map(|prev| UnspawnedMob::new(Box::new(prev))))
    }

    fn take(&mut self) -> Option<UnspawnedMob> {
        self.slot.take().map(|p| UnspawnedMob::new(Box::new(p)))
    }
}

/// Scoped unbounded list editor backing [`mod_api::MobSplitChildren`] for `mob_split`.
pub struct SplitChildList<'a> {
    pub action: u64,
    pub children: &'a mut Vec<PendingMob>,
    pub resources: Arc<super::Resources>,
    pub attributes: Arc<Catalog>,
    pub dimension: super::Dim,
}

impl MobSplitChildList for SplitChildList<'_> {
    fn len(&self) -> usize {
        self.children.len()
    }

    fn get(&self, index: usize) -> Option<UnspawnedMobView<'_>> {
        self.children.get(index).map(|p| UnspawnedMobView::new(p))
    }

    fn get_mut(&mut self, index: usize) -> Option<PendingChildEditor<'_>> {
        self.children
            .get_mut(index)
            .map(|p| PendingChildEditor::new(p))
    }

    fn construct(
        &self,
        host: &mut dyn ServerHost,
        spec: PendingMobSpec,
    ) -> Result<UnspawnedMob, UnspawnedMobError> {
        let pending = construct_pending_mob(
            &self.resources,
            &self.attributes,
            self.dimension,
            ChildConstraint::Split,
            spec,
        )?;
        mod_api::backend_mut_of(host)
            .ok_or(UnspawnedMobError::Unsupported)?
            .initialize_pending_child(UnspawnedMob::new(Box::new(pending)), self.action)
    }

    fn push(&mut self, child: UnspawnedMob) -> Result<(), UnspawnedMobError> {
        let mut pending = child
            .into_any()
            .downcast::<PendingMob>()
            .map_err(|_| UnspawnedMobError::Invalid)?;
        if !valid_child(
            &self.resources,
            self.dimension,
            ChildConstraint::Split,
            &pending,
        ) {
            return Err(UnspawnedMobError::Invalid);
        }
        pending.resources = self.resources.clone();
        pending.attributes = self.attributes.clone();
        self.children.push(*pending);
        Ok(())
    }

    fn insert(&mut self, index: usize, child: UnspawnedMob) -> Result<(), UnspawnedMobError> {
        if index > self.children.len() {
            return Err(UnspawnedMobError::Invalid);
        }
        let mut pending = child
            .into_any()
            .downcast::<PendingMob>()
            .map_err(|_| UnspawnedMobError::Invalid)?;
        if !valid_child(
            &self.resources,
            self.dimension,
            ChildConstraint::Split,
            &pending,
        ) {
            return Err(UnspawnedMobError::Invalid);
        }
        pending.resources = self.resources.clone();
        pending.attributes = self.attributes.clone();
        self.children.insert(index, *pending);
        Ok(())
    }

    fn replace(
        &mut self,
        index: usize,
        child: UnspawnedMob,
    ) -> Result<UnspawnedMob, UnspawnedMobError> {
        if index >= self.children.len() {
            return Err(UnspawnedMobError::Invalid);
        }
        let mut pending = child
            .into_any()
            .downcast::<PendingMob>()
            .map_err(|_| UnspawnedMobError::Invalid)?;
        if !valid_child(
            &self.resources,
            self.dimension,
            ChildConstraint::Split,
            &pending,
        ) {
            return Err(UnspawnedMobError::Invalid);
        }
        pending.resources = self.resources.clone();
        pending.attributes = self.attributes.clone();
        let prev = std::mem::replace(&mut self.children[index], *pending);
        Ok(UnspawnedMob::new(Box::new(prev)))
    }

    fn remove(&mut self, index: usize) -> Option<UnspawnedMob> {
        if index < self.children.len() {
            Some(UnspawnedMob::new(Box::new(self.children.remove(index))))
        } else {
            None
        }
    }

    fn clear(&mut self) {
        self.children.clear();
    }
}

/// Whether a final child satisfies every object invariant: a registered kind allowed by
/// `constraint`, the action's dimension, a finite spawnable position and yaw, baby state the
/// species supports and a size the species allows.
fn valid_child(
    resources: &super::Resources,
    dimension: super::Dim,
    constraint: ChildConstraint,
    child: &PendingMob,
) -> bool {
    let mob = &child.mob;
    let species = mob.species();
    child.dimension == dimension
        && resources.mob_types().contains_key(&mob.kind.id)
        && super::native_mobs::spawnable([mob.position.x, mob.position.y, mob.position.z])
        && [
            mob.yaw,
            mob.previous_yaw,
            mob.body_yaw,
            mob.previous_body_yaw,
            mob.head_yaw,
            mob.previous_head_yaw,
        ]
        .into_iter()
        .all(f32::is_finite)
        && constraint.allows_species(species)
        && (!mob.is_baby() || finalize::supports_baby(species))
        && match species {
            Species::Slime | Species::MagmaCube | Species::SulfurCube | Species::Phantom => {
                u32::try_from(mob.extra.size).is_ok_and(|size| size_ok(species, Some(size)))
            }
            _ => true,
        }
}

fn size_ok(species: Species, size: Option<u32>) -> bool {
    match (species, size) {
        (Species::Slime | Species::MagmaCube | Species::SulfurCube, Some(size)) => {
            i32::try_from(size).is_ok_and(|size| sunrise_sim::mobs::cube::SIZES.contains(&size))
        }
        (Species::Phantom, Some(size)) => {
            i32::try_from(size).is_ok_and(|size| sunrise_sim::mobs::phantom::SIZES.contains(&size))
        }
        (Species::Slime | Species::MagmaCube | Species::SulfurCube | Species::Phantom, None) => {
            false
        }
        (_, None) => true,
        (_, Some(_)) => false,
    }
}

fn apply_size(mob: &mut Mob, size: u32) {
    match mob.species() {
        Species::Slime | Species::MagmaCube => sunrise_sim::mobs::cube::set_size(mob, size as i32),
        Species::SulfurCube => sunrise_sim::mobs::sulfur_cube::set_size(mob, size as i32, true),
        Species::Phantom => sunrise_sim::mobs::phantom::set_size(mob, size as i32),
        _ => unreachable!("species size checked before mutation"),
    }
}
