//! A draw's selected body, baked hierarchy, and captured pose.

use super::{Atlas, body_with_variants, cem_key, cem_texture};
use crate::fast_hash::FxHashMap;
use crate::mobs::{Mob, Part, anim, cem};
use crate::mobs_render::{ClientMobData, ClientMobModels, ClientMobVisuals};
use mod_api::{
    __private::frame_model_part, FrameModel, FrameModelEdit, FrameModelPart, FrameModelPoseEdit,
};
use std::{borrow::Cow, sync::Arc};

/// The exact body selected for one draw, with an owned pack snapshot rather than a read lock.
/// The same selection supplies the Pre bind pose and all the vanilla draws that follow it.
pub(super) struct FrameBody<'a> {
    pub(super) visuals: &'a ClientMobVisuals,
    model_key: Cow<'static, str>,
    vanilla: &'a Arc<Part>,
    pub(super) cem: Arc<cem::CemModels>,
    pub(super) custom: Option<Arc<cem::CemLayer>>,
    texture: Cow<'a, str>,
}

impl<'a> FrameBody<'a> {
    pub(super) fn select(
        mob: &'a Mob,
        data: &ClientMobData,
        models: &'a ClientMobModels,
        variants: Option<&crate::mobs::variants::VariantTables>,
    ) -> Option<Self> {
        let visuals = models.get(&mob.kind.id)?;
        let cem = data
            .cem
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (model_key, texture) = body_with_variants(mob, visuals, variants);
        let vanilla = visuals.layer(&model_key).unwrap_or(&visuals.model);
        let custom = cem.layer_ref(cem_key(&model_key)).cloned();
        Some(Self {
            visuals,
            model_key,
            vanilla,
            cem,
            custom,
            texture,
        })
    }

    pub(super) fn model(&self) -> &Arc<Part> {
        self.custom
            .as_ref()
            .map_or(self.vanilla, |layer| &layer.model)
    }

    pub(super) fn texture(&self, atlas: Atlas<'_>) -> &str {
        cem_texture(self.custom.as_deref(), &self.texture, atlas).unwrap_or(&self.texture)
    }

    /// Both names of the body, so overlays on it don't take the `.jem` texture.
    pub(super) fn keys(&self) -> [&str; 2] {
        [&self.model_key, cem_key(&self.model_key)]
    }

    pub(super) fn layout<'b>(
        &'b self,
        data: &'b ClientMobData,
    ) -> Option<&'b Arc<BakedFrameModel>> {
        match &self.custom {
            Some(layer) => Some(&layer.frame_model),
            None => data.frame_models.get(self.vanilla),
        }
    }
}

/// One part of a baked frame model. Links are indices into that model's nodes.
pub struct BakedFrameNode {
    pub name: Arc<str>,
    pub anim_key: u64,
    pub parent: Option<usize>,
    pub first_child: Option<usize>,
    pub next_sibling: Option<usize>,
    bind_pose: [f32; 9],
}

/// A final model tree flattened in preorder, with root at index zero.
pub struct BakedFrameModel {
    nodes: Vec<BakedFrameNode>,
    by_name: FxHashMap<Arc<str>, usize>,
    // The immutable tree lives at these addresses for the lifetime of the drawing model.
    // Used for identity only; it is never dereferenced through this table.
    part_index: Arc<anim::FramePartIndex>,
}

impl BakedFrameModel {
    pub(in crate::mobs) fn bake(root: &Part) -> Self {
        let mut nodes = Vec::new();
        let mut by_name = FxHashMap::default();
        let mut by_part = FxHashMap::default();
        Self::append(root, None, &mut nodes, &mut by_name, &mut by_part);
        let part_index = Arc::new(anim::FramePartIndex::new(nodes.len(), by_part));
        Self {
            nodes,
            by_name,
            part_index,
        }
    }

    fn append(
        part: &Part,
        parent: Option<usize>,
        nodes: &mut Vec<BakedFrameNode>,
        by_name: &mut FxHashMap<Arc<str>, usize>,
        by_part: &mut FxHashMap<usize, usize>,
    ) -> usize {
        let index = nodes.len();
        let name: Arc<str> = part.name.as_str().into();
        // Like ModelPart.createPartLookup, the first visit to a name wins.
        by_name.entry(name.clone()).or_insert(index);
        by_part.insert(part as *const Part as usize, index);
        nodes.push(BakedFrameNode {
            name,
            anim_key: anim::key(&part.name),
            parent,
            first_child: None,
            next_sibling: None,
            bind_pose: part.pose,
        });
        let mut previous: Option<usize> = None;
        for child in &part.children {
            let child = Self::append(child, Some(index), nodes, by_name, by_part);
            if let Some(previous) = previous {
                nodes[previous].next_sibling = Some(child);
            } else {
                nodes[index].first_child = Some(child);
            }
            previous = Some(child);
        }
        index
    }

    pub fn nodes(&self) -> &[BakedFrameNode] {
        &self.nodes
    }

    /// The first depth-first match for this exact name, including the root.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }
}

impl anim::FrameLayoutIndex for BakedFrameModel {
    fn part_index(&self) -> &Arc<anim::FramePartIndex> {
        &self.part_index
    }
}

/// Layouts for the vanilla mob and player models held by one set of client resources.
#[derive(Default)]
pub struct BakedFrameModels {
    // Retain each root so its address cannot be reused by a different model.
    by_model: FxHashMap<usize, (Arc<Part>, Arc<BakedFrameModel>)>,
}

impl BakedFrameModels {
    /// Bake each visual entry's default and all its layers, once per shared model root.
    pub fn bake<'a>(visuals: impl IntoIterator<Item = &'a ClientMobVisuals>) -> Self {
        let mut layouts = Self::default();
        for kind in visuals {
            for model in std::iter::once(&kind.model).chain(kind.layers.values()) {
                layouts
                    .by_model
                    .entry(Arc::as_ptr(model) as usize)
                    .or_insert_with(|| (model.clone(), Arc::new(BakedFrameModel::bake(model))));
            }
        }
        layouts
    }

    /// Find the layout for the exact tree selected by the renderer.
    pub fn get(&self, model: &Arc<Part>) -> Option<&Arc<BakedFrameModel>> {
        self.by_model
            .get(&(Arc::as_ptr(model) as usize))
            .map(|(_, layout)| layout)
    }
}

/// The body poses lent to Pre, layers and Post. Dropping this returns the poses to the render
/// thread's pool. The shared layout also outlives the pack snapshot.
pub struct FrameModelPose {
    pub(super) layout: Arc<BakedFrameModel>,
    pub(super) poses: anim::PooledPoses,
    pub(super) bind: bool,
}

/// A part of `layout` as `poses` pose it, or as its bind pose: the one reading both lent
/// models share.
fn view_part<'a>(
    layout: &'a BakedFrameModel,
    poses: &'a anim::Poses,
    bind: bool,
    index: usize,
    node: &'a BakedFrameNode,
) -> FrameModelPart<'a> {
    let mut ancestor = poses.has_hidden_parts().then_some(index);
    let mut visible = true;
    while let Some(i) = ancestor {
        let current = &layout.nodes[i];
        if poses.is_hidden_at(i, current.anim_key) {
            visible = false;
            break;
        }
        ancestor = current.parent;
    }
    frame_model_part(
        index,
        &node.name,
        node.parent,
        node.first_child,
        node.next_sibling,
        if bind {
            &node.bind_pose
        } else {
            poses
                .pose_by_key(node.anim_key)
                .expect("poses were reset from the baked model's tree")
        },
        visible,
    )
}

impl FrameModelPose {
    fn view<'a>(&'a self, index: usize, node: &'a BakedFrameNode) -> FrameModelPart<'a> {
        view_part(&self.layout, &self.poses, self.bind, index, node)
    }
}

impl FrameModel for FrameModelPose {
    fn root(&self) -> FrameModelPart<'_> {
        self.view(0, &self.layout.nodes[0])
    }

    fn part(&self, name: &str) -> Option<FrameModelPart<'_>> {
        self.part_at(self.layout.find(name)?)
    }

    fn part_at(&self, index: usize) -> Option<FrameModelPart<'_>> {
        Some(self.view(index, self.layout.nodes.get(index)?))
    }
}

impl FrameModelEdit for FrameModelPose {
    fn hide(&mut self, index: usize) -> bool {
        self.poses.hide_frame_part(index)
    }
}

/// The posed model a `living_model_posed` listener edits: the draw's own poses, borrowed for
/// the callback, so the body or armour model draws with what the listeners leave.
pub(super) struct PosedEdit<'a> {
    layout: &'a BakedFrameModel,
    poses: &'a mut anim::Poses,
}

impl<'a> PosedEdit<'a> {
    pub(super) fn new(layout: &'a BakedFrameModel, poses: &'a mut anim::Poses) -> Self {
        Self { layout, poses }
    }
}

impl FrameModel for PosedEdit<'_> {
    fn root(&self) -> FrameModelPart<'_> {
        view_part(self.layout, self.poses, false, 0, &self.layout.nodes[0])
    }

    fn part(&self, name: &str) -> Option<FrameModelPart<'_>> {
        self.part_at(self.layout.find(name)?)
    }

    fn part_at(&self, index: usize) -> Option<FrameModelPart<'_>> {
        Some(view_part(
            self.layout,
            self.poses,
            false,
            index,
            self.layout.nodes.get(index)?,
        ))
    }
}

impl FrameModelPoseEdit for PosedEdit<'_> {
    fn set_pose(&mut self, index: usize, pose: mod_api::PartPose) -> bool {
        let Some(node) = self.layout.nodes.get(index) else {
            return false;
        };
        let values = [
            pose.x,
            pose.y,
            pose.z,
            pose.x_rot,
            pose.y_rot,
            pose.z_rot,
            pose.x_scale,
            pose.y_scale,
            pose.z_scale,
        ];
        values.iter().all(|v| v.is_finite()) && self.poses.set_pose_by_key(node.anim_key, values)
    }

    fn reset_pose(&mut self, index: usize) -> bool {
        let Some(node) = self.layout.nodes.get(index) else {
            return false;
        };
        self.poses.set_pose_by_key(node.anim_key, node.bind_pose)
    }
}
