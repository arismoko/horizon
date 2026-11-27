//! Retained copies feed the same per-view submission route as transient batches.
use super::native_block_entity_draw::BlockEntityDraw;
use crate::game::ClientGame;
use mod_api::__private::game::Fields;
use mod_api::{
    GroupError, InstanceGroupRef, InstanceGroups, ItemDisplayContext, ModelError, ModelInstance,
    ModelRef, PreparedItemRef,
};

use glam::{DVec3, Mat4, Vec3};
use sunrise_client::{
    gfx::PreparedModelInstance,
    item_render::ItemRenderState,
    terrain_model,
    world_geometry::{self, row_major_3x4},
};

struct Group {
    generation: u64,
    owner: Option<&'static str>,
    submitted: bool,
    model: ModelRef,
    copies: Vec<ModelInstance>,
    len: usize,
    transform: Mat4,
    composed: Vec<ModelInstance>,
    records: Vec<PreparedModelInstance>,
    #[cfg(test)]
    converted_bytes: usize,
}

impl Group {
    fn rewrite(&mut self, range: std::ops::Range<usize>) {
        let mut builder = sunrise_client::gfx::PreparedInstanceBuilder::default();
        for i in range {
            let copy = self.copies[i];
            let local = self.transform * row_major_3x4(copy.transform);
            let m = local.to_cols_array_2d();
            self.composed[i] = copy;
            self.composed[i].transform = [
                m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1], m[0][2],
                m[1][2], m[2][2], m[3][2],
            ];
            let mut record = builder.instance(
                local,
                DVec3::ZERO,
                DVec3::ZERO,
                super::native_block_entity_draw::prepared_tint(copy.tint),
                sunrise_client::world_geometry::pack_light(copy.light),
            );
            record.tint[3] = (copy.tint >> 24) as f32 / 255.0;
            self.records[i] = record;
            #[cfg(test)]
            {
                self.converted_bytes += std::mem::size_of::<PreparedModelInstance>();
            }
        }
    }
}

#[derive(Default)]
pub(in crate::game) struct InstanceGroupStorage {
    next: u64,
    groups: Vec<Option<Group>>,
    retired: sunrise_client::fast_hash::FxHashMap<u64, Group>,
}
impl InstanceGroupStorage {
    #[cfg(test)]
    pub(in crate::game::client_side) fn converted_bytes(&self) -> usize {
        self.groups
            .iter()
            .flatten()
            .map(|g| g.converted_bytes)
            .sum()
    }
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.groups.clear();
        self.retired.clear();
    }
    pub(in crate::game::client_side) fn retire_disabled(
        &mut self,
        disabled: &std::collections::BTreeSet<String>,
    ) {
        for slot in &mut self.groups {
            if slot
                .as_ref()
                .is_some_and(|g| g.owner.is_some_and(|id| disabled.contains(id)))
            {
                let group = slot.take().expect("disabled slot has a group");
                if group.submitted {
                    self.retired.insert(group.generation, group);
                }
            }
        }
    }
    fn get(
        &mut self,
        handle: InstanceGroupRef,
        model_generation: u64,
    ) -> Result<&mut Group, GroupError> {
        self.get_owned(
            mod_api::content::crash::current_native_mod(),
            handle,
            model_generation,
        )
    }
    fn get_owned(
        &mut self,
        _owner: Option<&'static str>,
        handle: InstanceGroupRef,
        model_generation: u64,
    ) -> Result<&mut Group, GroupError> {
        let fields = handle.fields();
        self.groups
            .get_mut(fields.slot as usize)
            .and_then(Option::as_mut)
            .filter(|g| {
                g.generation == fields.generation && g.model.fields().generation == model_generation
            })
            .ok_or(GroupError::Stale)
    }
}

impl InstanceGroups for ClientGame<'_> {
    fn create_group(&self, model: ModelRef, capacity: u32) -> Result<InstanceGroupRef, GroupError> {
        let fields = model.fields();
        if fields.generation != self.client.standalone_geometry.generation() {
            return Err(GroupError::Model(mod_api::ModelError::StaleModel));
        }
        self.client
            .standalone_geometry
            .instance_slot(fields.slot)
            .ok_or(GroupError::Model(mod_api::ModelError::UnknownModel))?;
        let owner = mod_api::content::crash::current_native_mod();
        let mut storage = self.client.instance_groups.borrow_mut();
        self.client
            .prepared_model_draws
            .borrow_mut()
            .reserve((capacity as usize).min(16_384));
        storage.next = storage
            .next
            .checked_add(1)
            .expect("instance group epoch exhausted");
        let mut group = Group {
            generation: storage.next,
            owner,
            submitted: false,
            model,
            copies: vec![ModelInstance::at([0.0; 3], 0xFFFF_FFFF); capacity as usize],
            len: 0,
            transform: Mat4::IDENTITY,
            composed: vec![ModelInstance::at([0.0; 3], 0xFFFF_FFFF); capacity as usize],
            records: vec![
                PreparedModelInstance::new(
                    Mat4::IDENTITY,
                    DVec3::ZERO,
                    DVec3::ZERO,
                    [1.0; 4],
                    0
                );
                capacity as usize
            ],
            #[cfg(test)]
            converted_bytes: 0,
        };
        group.rewrite(0..capacity as usize);
        let index = storage
            .groups
            .iter()
            .position(Option::is_none)
            .unwrap_or(storage.groups.len());
        let handle = mod_api::__private::built::instance_group_ref(group.generation, index as u32);
        if index == storage.groups.len() {
            storage.groups.push(Some(group));
        } else {
            storage.groups[index] = Some(group);
        }
        Ok(handle)
    }
}

impl ClientGame<'_> {
    pub(in crate::game) fn instance_group_write(
        &self,
        group: InstanceGroupRef,
        first: u32,
        copies: &[ModelInstance],
    ) -> Result<(), GroupError> {
        let mut storage = self.client.instance_groups.borrow_mut();
        let group = storage.get(group, self.client.standalone_geometry.generation())?;
        let end = (first as usize)
            .checked_add(copies.len())
            .filter(|end| *end <= group.copies.len())
            .ok_or(GroupError::OutOfRange)?;
        sunrise_client::world_geometry::validate_instances(copies)?;
        group.copies[first as usize..end].copy_from_slice(copies);
        group.rewrite((first as usize).min(group.len)..end);
        group.len = group.len.max(end);
        Ok(())
    }
    pub(in crate::game) fn instance_group_set_transform(
        &self,
        handle: InstanceGroupRef,
        transform: [f32; 12],
    ) -> Result<(), GroupError> {
        let mut storage = self.client.instance_groups.borrow_mut();
        let group = storage.get(handle, self.client.standalone_geometry.generation())?;
        if transform.iter().any(|v| !v.is_finite()) {
            return Err(GroupError::NonFinite);
        }
        group.transform = row_major_3x4(transform);
        group.rewrite(0..group.len);
        Ok(())
    }
    pub(in crate::game) fn instance_group_set_len(
        &self,
        group: InstanceGroupRef,
        len: u32,
    ) -> Result<(), GroupError> {
        let mut storage = self.client.instance_groups.borrow_mut();
        let group = storage.get(group, self.client.standalone_geometry.generation())?;
        if len as usize > group.copies.len() {
            return Err(GroupError::OutOfRange);
        }
        if len as usize > group.len {
            group.rewrite(group.len..len as usize);
        }
        group.len = len as usize;
        Ok(())
    }
    pub(in crate::game) fn instance_group_destroy(
        &self,
        handle: InstanceGroupRef,
    ) -> Result<(), GroupError> {
        let mut storage = self.client.instance_groups.borrow_mut();
        storage.get(handle, self.client.standalone_geometry.generation())?;
        let group = storage.groups[handle.fields().slot as usize]
            .take()
            .expect("validated group slot");
        if group.submitted {
            storage.retired.insert(group.generation, group);
        }
        Ok(())
    }
    pub(in crate::game) fn draw_instance_group(
        &self,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
        handle: InstanceGroupRef,
    ) -> Result<(), GroupError> {
        let mut storage = self.client.instance_groups.borrow_mut();
        let group = storage.get(handle, self.client.standalone_geometry.generation())?;
        group.submitted = true;
        draw.queue_group(handle);
        Ok(())
    }

    /// Resolve live data after the prepare hook, or the last data of an accepted group that
    /// was destroyed. The latter no longer occupies a public handle slot.
    pub(in crate::game) fn resolve_submitted_groups(
        &self,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
    ) {
        let mut storage = self.client.instance_groups.borrow_mut();
        let mut submitted = draw.take_submitted_groups();
        for &handle in &submitted {
            let fields = handle.fields();
            let group = if let Some(group) = storage.retired.get(&fields.generation) {
                group
            } else if let Some(group) = storage.groups[fields.slot as usize]
                .as_mut()
                .filter(|group| group.generation == fields.generation)
            {
                group.submitted = false;
                group
            } else {
                unreachable!("accepted group must be live or retained");
            };
            let _ = self.draw_cached_model_instances(
                draw,
                group.model,
                &group.composed[..group.len],
                Some(&group.records[..group.len]),
            );
        }
        for handle in submitted.drain(..) {
            storage.retired.remove(&handle.fields().generation);
        }
        draw.recycle_submitted_groups(submitted);
    }

    /// A frame can stop before prepare, for example on graphics device loss.
    pub(in crate::game) fn discard_submitted_groups(
        &self,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
    ) {
        let mut storage = self.client.instance_groups.borrow_mut();
        for handle in draw.drain_submitted_groups() {
            let fields = handle.fields();
            if storage.retired.remove(&fields.generation).is_none()
                && let Some(group) = storage.groups[fields.slot as usize]
                    .as_mut()
                    .filter(|group| group.generation == fields.generation)
            {
                group.submitted = false;
            }
        }
    }

    /// Draw a retained [`InstanceGroupRef`] under `draw`'s active pose (entity renderers,
    /// entity layers, block entities, and screen scenes).
    pub(in crate::game) fn native_draw_instance_group(
        &self,
        draw: &mut BlockEntityDraw<'_>,
        handle: InstanceGroupRef,
    ) -> Result<(), GroupError> {
        let model_gen = self.client.standalone_geometry.generation();
        let mut storage = self.client.instance_groups.borrow_mut();
        let group = storage.get(handle, model_gen)?;
        let model_fields = group.model.fields();
        let slot_info = self
            .client
            .standalone_geometry
            .instance_slot(model_fields.slot)
            .ok_or(GroupError::Model(ModelError::UnknownModel))?;
        let base_pose = draw.pose();
        let layers = &self.client.client_pack.render.sprite_layers;
        for copy in &group.composed[..group.len] {
            let copy_local = Mat4::from_translation(Vec3::new(
                copy.origin[0] as f32,
                copy.origin[1] as f32,
                copy.origin[2] as f32,
            )) * row_major_3x4(copy.transform);
            let copy_pose = base_pose * copy_local;
            let copy_light = world_geometry::pack_light(copy.light);
            let copy_overlay = world_geometry::pack_overlay(copy.overlay);
            let material = slot_info.translucent || slot_info.state || (copy.color >> 24) != 0xFF;
            if !material
                && copy_overlay == 0
                && copy.color == u32::MAX
                && copy.hidden_faces == 0
                && draw.defer_model_at(
                    model_fields.slot,
                    model_fields.generation,
                    copy_pose,
                    copy.tint,
                    copy_light,
                )
            {
                continue;
            }
            if material {
                let Some(faces) = self.model_faces(model_fields.slot) else {
                    continue;
                };
                let start = draw.vertices.len();
                let emit_light = if draw.unlit {
                    sunrise_client::mobs::render::FULL_BRIGHT
                } else if draw.level_lit {
                    0
                } else {
                    copy_light
                };
                let options = terrain_model::NativeModelRenderOptions {
                    shade: if draw.unlit {
                        terrain_model::NativeModelShade::Off
                    } else if slot_info.state {
                        terrain_model::NativeModelShade::Model
                    } else {
                        terrain_model::NativeModelShade::Entity
                    },
                    tint: copy.tint & 0xFF_FFFF,
                    tint_alpha: (copy.tint >> 24) as u8,
                    color: copy.color,
                    hidden_faces: copy.hidden_faces,
                    ..Default::default()
                };
                {
                    let mut scope = draw.quad_scope();
                    terrain_model::draw_model(
                        faces,
                        copy_pose,
                        emit_light,
                        options,
                        |name| {
                            layers
                                .get(name)
                                .or_else(|| layers.get(name.strip_prefix("minecraft:")?))
                                .copied()
                        },
                        |quad| scope.submit_quad(quad),
                    );
                }
                if !draw.unlit {
                    draw.lit_since(
                        start,
                        || copy_local.transform_point3(Vec3::splat(0.5)).as_dvec3(),
                        copy_light & 0xffff,
                    );
                }
            } else {
                let Some(faces) = self.client.standalone_geometry.faces(model_fields.slot) else {
                    continue;
                };
                let room = draw.quad_scope().remaining_quads();
                if room == 0 {
                    break;
                }
                let start = draw.vertices.len();
                let emit_light = if draw.level_lit { 0 } else { copy_light };
                let shown = || {
                    faces
                        .iter()
                        .filter(|face| !face.hidden_by(copy.hidden_faces))
                        .take(room)
                };
                super::minecarts_client::emit_model_faces(
                    shown().copied().map(|face| (face, emit_light)),
                    copy_pose,
                    true,
                    |index| (index >= 0).then_some(copy.tint & 0xFF_FFFF),
                    &mut draw.vertices,
                );
                if copy_overlay != 0 || copy.color != u32::MAX {
                    for vertex in &mut draw.vertices[start..] {
                        vertex.layer = copy_overlay;
                        if copy.color != u32::MAX {
                            vertex.color =
                                terrain_model::multiply_draw_color(vertex.color, copy.color);
                        }
                    }
                }
                super::minecarts_client::scale_tint_alpha(
                    &mut draw.vertices[start..],
                    shown(),
                    copy.tint,
                );
                draw.quad_scope().retain_valid_since(start);
                draw.drawn_since(
                    start,
                    || copy_local.transform_point3(Vec3::splat(0.5)).as_dvec3(),
                    copy_light & 0xffff,
                );
            }
        }
        Ok(())
    }

    /// Resolve `stack`'s item render state once for `display` and `seed`
    /// (`ClientRenderClientHost::prepare_item`).
    pub(in crate::game) fn prepare_item(
        &self,
        stack: &sunrise_items::items::ItemStack,
        display: ItemDisplayContext,
        seed: i32,
    ) -> Result<PreparedItemRef, ModelError> {
        let display = sunrise_gameplay_common::game::api_values::engine_display_context(display)
            .ok_or(ModelError::UnknownModel)?;
        let state = self.resolve_item(stack, display, None, seed);
        if state.is_empty() {
            return Err(ModelError::UnknownModel);
        }
        let owner = mod_api::content::crash::current_native_mod();
        let resource_generation = self.client.standalone_geometry.generation();
        let mut storage = self.client.prepared_items.borrow_mut();
        storage.next = storage
            .next
            .checked_add(1)
            .expect("prepared item epoch exhausted");
        let entry = PreparedItemEntry {
            generation: storage.next,
            resource_generation,
            owner,
            state,
        };
        let index = storage
            .items
            .iter()
            .position(Option::is_none)
            .unwrap_or(storage.items.len());
        let handle = mod_api::__private::built::prepared_item_ref(entry.generation, index as u32);
        if index == storage.items.len() {
            storage.items.push(Some(entry));
        } else {
            storage.items[index] = Some(entry);
        }
        Ok(handle)
    }

    /// Retire a prepared item render state (`PreparedItemRef::destroy`).
    pub(in crate::game) fn prepared_item_destroy(
        &self,
        handle: PreparedItemRef,
    ) -> Result<(), ModelError> {
        let resource_generation = self.client.standalone_geometry.generation();
        let mut storage = self.client.prepared_items.borrow_mut();
        storage.get(handle, resource_generation)?;
        storage.items[handle.fields().slot as usize] = None;
        Ok(())
    }

    /// Draw a retained [`PreparedItemRef`] under `draw`'s active pose
    /// (`BlockEntityDrawContext::prepared_item`).
    pub(in crate::game) fn native_draw_prepared_item(
        &self,
        draw: &mut BlockEntityDraw<'_>,
        handle: PreparedItemRef,
    ) -> Result<(), ModelError> {
        let resource_generation = self.client.standalone_geometry.generation();
        let storage = self.client.prepared_items.borrow();
        let entry = storage.get(handle, resource_generation)?;
        let pose = draw.pose();
        if draw.unlit {
            let full = sunrise_client::mobs::render::FULL_BRIGHT;
            self.submit_item_unlit(&entry.state, pose, full, &mut draw.vertices);
        } else {
            self.submit_item_world(&entry.state, pose, draw.light(), &mut draw.vertices);
        }
        Ok(())
    }
}

struct PreparedItemEntry {
    generation: u64,
    resource_generation: u64,
    owner: Option<&'static str>,
    state: ItemRenderState,
}

#[derive(Default)]
pub(in crate::game) struct PreparedItemStorage {
    next: u64,
    items: Vec<Option<PreparedItemEntry>>,
}

impl PreparedItemStorage {
    pub(in crate::game) fn clear(&mut self) {
        self.items.clear();
    }

    pub(in crate::game) fn retire_disabled(
        &mut self,
        disabled: &std::collections::BTreeSet<String>,
    ) {
        for slot in &mut self.items {
            if slot
                .as_ref()
                .is_some_and(|e| e.owner.is_some_and(|id| disabled.contains(id)))
            {
                slot.take();
            }
        }
    }

    fn get(
        &self,
        handle: PreparedItemRef,
        resource_generation: u64,
    ) -> Result<&PreparedItemEntry, ModelError> {
        let fields = handle.fields();
        self.items
            .get(fields.slot as usize)
            .and_then(Option::as_ref)
            .filter(|e| {
                e.generation == fields.generation && e.resource_generation == resource_generation
            })
            .ok_or(ModelError::StaleModel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_cache_preserves_other_records_and_world_origins() {
        let copy = ModelInstance::at([10.0, 20.0, 30.0], 0xFFFF_FFFF);
        let mut group = Group {
            generation: 1,
            owner: None,
            submitted: false,
            model: mod_api::__private::built::model_ref(1, 0),
            copies: vec![copy; 2],
            len: 2,
            transform: row_major_3x4([0.0, -2.0, 0.0, 3.0, 2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 2.0, 5.0]),
            composed: vec![copy; 2],
            records: vec![
                PreparedModelInstance::new(
                    Mat4::IDENTITY,
                    DVec3::ZERO,
                    DVec3::ZERO,
                    [1.0; 4],
                    0
                );
                2
            ],
            converted_bytes: 0,
        };
        group.rewrite(0..2);
        let untouched = group.records[0];
        group.copies[1].transform[3] = 1.0;
        group.copies[1].transform[7] = 2.0;
        group.copies[1].transform[11] = 3.0;
        group.rewrite(1..2);
        assert_eq!(group.records[0].model, untouched.model);
        assert_eq!(
            group.records[1]
                .matrix()
                .transform_point3(glam::Vec3::new(2.0, 3.0, 4.0)),
            glam::Vec3::new(-7.0, 10.0, 19.0)
        );
        assert_eq!(group.composed[1].origin, [10.0, 20.0, 30.0]);
        assert_eq!(
            group.converted_bytes,
            3 * std::mem::size_of::<PreparedModelInstance>()
        );
        // A quarter turn after scale(2, 4, 8): inverse-transpose columns
        // are (0, 1/2, 0), (-1/4, 0, 0), (0, 0, 1/8).
        group.transform =
            row_major_3x4([0.0, -4.0, 0.0, 3.0, 2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 8.0, 5.0]);
        group.rewrite(0..1);
        assert_eq!(
            group.records[0].normal,
            [
                [0.0, 0.5, 0.0, 0.0],
                [-0.25, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.125, 0.0]
            ]
        );
        // Reflect the first scaled axis: only the first normal column changes sign.
        group.transform =
            row_major_3x4([0.0, -4.0, 0.0, 3.0, -2.0, 0.0, 0.0, 4.0, 0.0, 0.0, 8.0, 5.0]);
        group.rewrite(0..1);
        assert_eq!(
            group.records[0].normal,
            [
                [0.0, -0.5, 0.0, 0.0],
                [-0.25, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.125, 0.0]
            ]
        );
        let handle = mod_api::__private::built::instance_group_ref(1, 0);
        let owned_handle = mod_api::__private::built::instance_group_ref(2, 1);
        let owned = Group {
            generation: 2,
            owner: Some("group-owner"),
            submitted: false,
            model: mod_api::__private::built::model_ref(1, 0),
            copies: vec![copy],
            len: 1,
            transform: Mat4::IDENTITY,
            composed: vec![copy],
            records: vec![PreparedModelInstance::new(
                Mat4::IDENTITY,
                DVec3::ZERO,
                DVec3::ZERO,
                [1.0; 4],
                0,
            )],
            converted_bytes: 0,
        };
        let mut storage = InstanceGroupStorage {
            next: 2,
            groups: vec![Some(group), Some(owned)],
            retired: Default::default(),
        };
        assert!(storage.get(handle, 1).is_ok());
        assert!(storage.get(owned_handle, 1).is_ok());
        // Access checks generation and model lifetime, not the TLS caller mod name.
        {
            let _other =
                mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "other",
                    hook: "test",
                    part: None,
                });
            assert!(storage.get(handle, 1).is_ok());
            assert!(storage.get(owned_handle, 1).is_ok());
            assert!(matches!(storage.get(handle, 2), Err(GroupError::Stale)));
        }
        // Disabling the owner still retires its groups without touching engine-owned ones.
        storage.retire_disabled(&std::collections::BTreeSet::from(["group-owner".into()]));
        assert!(storage.get(handle, 1).is_ok());
        assert!(matches!(
            storage.get(owned_handle, 1),
            Err(GroupError::Stale)
        ));
    }
}
