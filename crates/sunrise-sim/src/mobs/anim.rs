//! Model animation: ports of each vanilla `EntityModel.setupAnim` family
//! (quadrupeds, humanoids, creepers, spiders, chickens...) applied to a map of
//! part poses before the model is drawn.
use super::{Mob, keyframes::AnimationState, living::wrap};
use crate::fast_hash::{FxHashMap, FxHashSet};
use mod_api::HumanoidPart;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_6, PI};
use std::sync::{Arc, LazyLock};
use sunrise_core::geometry::trig::Mth;
use sunrise_core::use_animation::UseAnimation;

/// The setupAnim implementation a model uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    Nautilus,
    HappyGhast,
    Sniffer,
    Breeze,
    Warden,
    Allay,
    Creaking,
    Camel,
    Wither,
    Vex,
    Ravager,
    Guardian,
    Shulker,
    Quadruped,
    Sheep,
    Chicken,
    Zombie,
    Drowned,
    ZombieVillager,
    Skeleton,
    Creeper,
    Spider,
    Enderman,
    Silverfish,
    Slime,
    MagmaCube,
    Witch,
    Villager,
    IronGolem,
    SnowGolem,
    CopperGolem,
    Illager,
    Piglin,
    ZombifiedPiglin,
    Hoglin,
    Wolf,
    Feline,
    Fox,
    Rabbit,
    Horse,
    Llama,
    Goat,
    Panda,
    PolarBear,
    Parrot,
    Bee,
    Turtle,
    Armadillo,
    Frog,
    Axolotl,
    Squid,
    Fish,
    Pufferfish,
    Dolphin,
    Bat,
    Phantom,
    Ghast,
    Blaze,
    Strider,
    /// The humanoid model of the earlier simple renderer.
    Humanoid {
        zombie_arms: bool,
    },
}
/// Client-side animation clocks advanced each tick.
#[derive(Clone, Debug, Default)]
pub struct AnimState {
    pub attack_anim: f32,
    pub attack_anim_o: f32,
    pub flap: f32,
    pub flap_o: f32,
    pub flap_speed: f32,
    pub flap_speed_o: f32,
    pub flapping: f32,
    /// Generic per-species keyframe animation states.
    pub states: [AnimationState; 6],
    /// Per-species animation values (tail wag, squid tentacles, squish...)
    /// with their previous-tick copies for interpolation.
    pub v: [f32; 8],
    pub vo: [f32; 8],
}
impl AnimState {
    pub fn begin_tick(&mut self) {
        self.attack_anim_o = self.attack_anim;
        self.vo = self.v;
    }
    /// Interpolated value `i` at the render partial tick.
    pub fn lerp(&self, i: usize, partial: f32) -> f32 {
        self.vo[i] + (self.v[i] - self.vo[i]) * partial
    }
    pub fn end_tick(&mut self, _swinging: bool) {}
}
/// A part name as `Poses` keys it: its Fx hash, so filling a pose map allocates
/// nothing (a mob farm resets one per layer per mob each frame). Names are short
/// and few per model; `no_two_part_names_share_a_key` checks every vanilla model.
#[inline]
pub fn key(name: &str) -> u64 {
    use std::hash::Hasher;
    let mut h = crate::fast_hash::FxHasher::default();
    h.write(name.as_bytes());
    // The length too, so "a" and "a\0" differ (str's own Hash adds a terminator).
    h.write_usize(name.len());
    h.finish()
}
// Index by HumanoidPart, keying only the editable mid-animation inputs.
static HUMANOID_POSE_KEYS: LazyLock<[Option<u64>; HumanoidPart::ALL.len()]> = LazyLock::new(|| {
    HumanoidPart::ALL.map(|part| match part {
        HumanoidPart::Head
        | HumanoidPart::Hat
        | HumanoidPart::Body
        | HumanoidPart::RightArm
        | HumanoidPart::LeftArm
        | HumanoidPart::RightLeg
        | HumanoidPart::LeftLeg => Some(key(part.model_name())),
        _ => None,
    })
});

pub trait PosePart {
    fn part_name(&self) -> &str;
    fn bind_pose(&self) -> [f32; 9];
    /// Root-first preorder traversal; `Poses::reset_animation` preserves first-name-wins insertion.
    fn visit_parts(&self, f: &mut impl FnMut(&str, [f32; 9]))
    where
        Self: Sized;
    fn part_identity(&self) -> usize {
        self as *const Self as *const () as usize
    }
}

impl<T: PosePart> PosePart for &T {
    fn part_name(&self) -> &str {
        (**self).part_name()
    }

    fn bind_pose(&self) -> [f32; 9] {
        (**self).bind_pose()
    }

    fn visit_parts(&self, f: &mut impl FnMut(&str, [f32; 9])) {
        (**self).visit_parts(f);
    }

    fn part_identity(&self) -> usize {
        (**self).part_identity()
    }
}

impl<T: PosePart> PosePart for Arc<T> {
    fn part_name(&self) -> &str {
        (**self).part_name()
    }

    fn bind_pose(&self) -> [f32; 9] {
        (**self).bind_pose()
    }

    fn visit_parts(&self, f: &mut impl FnMut(&str, [f32; 9])) {
        (**self).visit_parts(f);
    }

    fn part_identity(&self) -> usize {
        (**self).part_identity()
    }
}

/// Concrete portable part-address-to-node-index table built once at bake time and shared via `Arc`
/// between `BakedFrameModel` and `Poses`. Used for identity only; never dereferenced.
#[derive(Clone, Debug, Default)]
pub struct FramePartIndex {
    node_count: usize,
    by_part: FxHashMap<usize, usize>,
}

impl FramePartIndex {
    pub fn new(node_count: usize, by_part: FxHashMap<usize, usize>) -> Self {
        Self {
            node_count,
            by_part,
        }
    }

    #[inline]
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    #[inline]
    pub fn index_of_identity(&self, part_ptr: usize) -> Option<usize> {
        self.by_part.get(&part_ptr).copied()
    }
}

/// Static-dispatch seam for borrowing the baked concrete index during frame preparation.
pub trait FrameLayoutIndex {
    fn part_index(&self) -> &Arc<FramePartIndex>;
}

#[derive(Clone, Copy)]
struct PoseEntry {
    pose: [f32; 9],
    base_hidden: bool,
}

impl PoseEntry {
    fn new(pose: [f32; 9]) -> Self {
        Self {
            pose,
            base_hidden: false,
        }
    }
}

/// Part poses for one draw, keyed by part name.
#[derive(Clone, Default)]
pub struct Poses {
    map: FxHashMap<u64, PoseEntry>,
    dummy: [f32; 9],
    /// Parts with `skipDraw` set: their own cubes are not drawn, children are.
    skipped: FxHashSet<u64>,
    /// Number of parts with `visible` cleared by animation.
    base_hidden_count: usize,
    /// Draw-local visibility indexed by the selected tree, independent of animation names.
    frame_layout: Option<Arc<FramePartIndex>>,
    frame_hidden: Vec<bool>,
    has_frame_edits: bool,
    apply_frame_visibility: bool,
    /// Mods' own values per part ([`Poses::set_data`]), keyed like `map` and by the value's
    /// type; the engine never reads them.
    data: FxHashMap<(u64, std::any::TypeId), PartData>,
    /// Names the draw state, kept by copies ([`Poses::version`]).
    version: u64,
}
/// A mod's value on a part ([`Poses::set_data`]): shared, not cloned, by a copy of the poses.
type PartData = std::sync::Arc<dyn std::any::Any + Send + Sync>;
/// The next [`Poses::version`].
static NEXT_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
thread_local! {
    /// Spare `Poses` for [`Poses::pooled`].
    static POOL: std::cell::RefCell<Vec<Poses>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// A [`Poses`] from this thread's pool, given back when dropped.
pub struct PooledPoses(Poses);
impl std::ops::Deref for PooledPoses {
    type Target = Poses;
    fn deref(&self) -> &Poses {
        &self.0
    }
}
impl std::ops::DerefMut for PooledPoses {
    fn deref_mut(&mut self) -> &mut Poses {
        &mut self.0
    }
}
impl Drop for PooledPoses {
    fn drop(&mut self) {
        let poses = std::mem::take(&mut self.0);
        // A few per thread: a mob's body and layers, a rider's.
        let _ = POOL.try_with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < 8 {
                pool.push(poses);
            }
        });
    }
}
impl Poses {
    pub fn new<P: PosePart>(model: &P) -> Self {
        let mut poses = Self::default();
        poses.reset(model);
        poses
    }
    /// Portable constructor for portable unit tests without a client `Part` tree.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut poses = Self::default();
        for name in names {
            poses.map.entry(key(name)).or_insert(PoseEntry::new([
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0,
            ]));
        }
        poses.changed();
        poses
    }
    /// Spare `Poses` of an earlier draw on this thread, to fill with `reset` or
    /// `copy_from` before use: a mob farm poses every mob each frame, and fresh
    /// maps grow (and rehash) part by part.
    pub fn pooled() -> PooledPoses {
        PooledPoses(
            POOL.try_with(|pool| pool.borrow_mut().pop())
                .ok()
                .flatten()
                .unwrap_or_default(),
        )
    }
    /// Names the draw state: two `Poses` with the same version pose every part
    /// alike and have the same effective visibility. A draw of the same model
    /// under the same matrix with the same version can reuse the first's part
    /// matrices and jobs (`skin::Skin::draw`).
    pub fn version(&self) -> u64 {
        self.version
    }
    fn changed(&mut self) {
        self.version = NEXT_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    /// Makes this a copy of `other`, keeping this one's capacity.
    pub fn copy_from(&mut self, other: &Poses) {
        self.map.clone_from(&other.map);
        self.dummy = other.dummy;
        self.skipped.clone_from(&other.skipped);
        self.base_hidden_count = other.base_hidden_count;
        self.frame_layout.clone_from(&other.frame_layout);
        self.frame_hidden.clone_from(&other.frame_hidden);
        self.has_frame_edits = other.has_frame_edits;
        self.apply_frame_visibility = other.apply_frame_visibility;
        self.data.clone_from(&other.data);
        self.version = other.version;
    }
    /// Refills this for another model, keeping the map and set capacity:
    /// a mob draws its body and every equipment layer each frame.
    pub fn reset<P: PosePart>(&mut self, model: &P) {
        self.frame_layout = None;
        self.frame_hidden.clear();
        self.has_frame_edits = false;
        self.reset_animation(model);
    }

    /// Restore the selected model's bind pose and base visibility for another mirror, without
    /// losing its shared Pre edits. The next setupAnim starts with the override inactive.
    pub fn reset_animation<P: PosePart>(&mut self, model: &P) {
        self.map.clear();
        self.dummy = [0.0; 9];
        self.skipped.clear();
        self.base_hidden_count = 0;
        self.apply_frame_visibility = false;
        self.data.clear();
        model.visit_parts(&mut |name, pose| {
            // ModelPart.createPartLookup fills its map with putIfAbsent: the first part keeps a name.
            self.map.entry(key(name)).or_insert(PoseEntry::new(pose));
        });
        self.changed();
    }
    pub fn get(&mut self, name: &str) -> &mut [f32; 9] {
        self.changed();
        match self.map.get_mut(&key(name)) {
            Some(entry) => &mut entry.pose,
            None => &mut self.dummy,
        }
    }
    /// The pose of the part named `name`, when the model has one.
    pub fn pose_of(&self, name: &str) -> Option<[f32; 9]> {
        self.pose_by_key(key(name)).copied()
    }
    /// Overwrites the pose of the part keyed `key`; false when the model has no such part.
    pub fn set_pose_by_key(&mut self, key: u64, pose: [f32; 9]) -> bool {
        let Some(entry) = self.map.get_mut(&key) else {
            return false;
        };
        entry.pose = pose;
        self.changed();
        true
    }
    pub fn pose_by_key(&self, key: u64) -> Option<&[f32; 9]> {
        self.map.get(&key).map(|entry| &entry.pose)
    }
    pub fn pose<P: PosePart>(&self, part: &P) -> [f32; 9] {
        self.pose_of(part.part_name())
            .unwrap_or_else(|| part.bind_pose())
    }
    fn set_humanoid_vector(&mut self, part: HumanoidPart, start: usize, values: [f32; 3]) -> bool {
        let Some(entry) = HUMANOID_POSE_KEYS[part as usize].and_then(|key| self.map.get_mut(&key))
        else {
            return false;
        };
        entry.pose[start..start + 3].copy_from_slice(&values);
        self.changed();
        true
    }
    pub fn skip_draw(&mut self, name: &str) {
        if self.skipped.insert(key(name)) {
            self.changed();
        }
    }
    pub fn skips<P: PosePart>(&self, part: &P) -> bool {
        self.skips_named(part.part_name())
    }
    /// `skips` by the part's name.
    pub fn skips_named(&self, name: &str) -> bool {
        !self.skipped.is_empty() && self.skips_key(key(name))
    }
    pub fn skips_key(&self, key: u64) -> bool {
        self.skipped.contains(&key)
    }
    pub fn has(&self, name: &str) -> bool {
        self.map.contains_key(&key(name))
    }
    pub fn xrot(&mut self, n: &str, v: f32) {
        self.get(n)[3] = v;
    }
    pub fn yrot(&mut self, n: &str, v: f32) {
        self.get(n)[4] = v;
    }
    pub fn zrot(&mut self, n: &str, v: f32) {
        self.get(n)[5] = v;
    }
    /// Set `ModelPart.visible`; animation changes the base visibility, not the frame override.
    pub fn visible(&mut self, name: &str, visible: bool) {
        let Some(entry) = self.map.get_mut(&key(name)) else {
            return;
        };
        let hidden = !visible;
        if entry.base_hidden == hidden {
            return;
        }
        entry.base_hidden = hidden;
        if hidden {
            self.base_hidden_count += 1;
        } else {
            self.base_hidden_count -= 1;
        }
        self.changed();
    }
    /// Base visibility for animation, without the draw-local overlay.
    pub fn is_hidden_named(&self, name: &str) -> bool {
        self.base_hidden_count != 0 && self.is_hidden_key(key(name))
    }
    pub fn is_hidden_key(&self, key: u64) -> bool {
        self.base_hidden_count != 0 && self.map.get(&key).is_some_and(|entry| entry.base_hidden)
    }
    /// Refill pooled visibility storage before calling Pre, so edits never grow it.
    pub fn prepare_frame_visibility<L: FrameLayoutIndex>(&mut self, layout: &L) {
        let index = layout.part_index();
        self.frame_hidden.resize(index.node_count(), false);
        self.frame_layout = Some(Arc::clone(index));
        self.apply_frame_visibility(true);
    }

    pub fn hide_frame_part(&mut self, index: usize) -> bool {
        let Some(hidden) = self.frame_hidden.get_mut(index) else {
            return false;
        };
        if !*hidden {
            *hidden = true;
            self.has_frame_edits = true;
            self.changed();
        }
        true
    }
    /// Stores `value`, a mod's own data of any type, on the part named `name`, when the model
    /// has one: the engine never reads it. A mod's mixin on [`setup`] writes, say, a limb's
    /// bend here, and its replacement of `render::compile_part` reads it back for the part it
    /// draws. One value per part and type: a mod keeps its data in a type of its own, so two
    /// mods never read each other's. A write changes [`Poses::version`], so a later draw never
    /// reuses part matrices of the walk before it. `reset` (and `reset_animation`, before each
    /// `setupAnim`) drops the values, `copy_from`
    /// shares them with the copy.
    pub fn set_data<T: std::any::Any + Send + Sync>(&mut self, name: &str, value: T) {
        if self.map.contains_key(&key(name)) {
            self.data.insert(
                (key(name), std::any::TypeId::of::<T>()),
                std::sync::Arc::new(value),
            );
            self.changed();
        }
    }
    /// The value of type `T` [`Poses::set_data`] stored on the part named `name`.
    pub fn data<T: std::any::Any + Send + Sync>(&self, name: &str) -> Option<&T> {
        self.data
            .get(&(key(name), std::any::TypeId::of::<T>()))
            .and_then(|value| value.downcast_ref())
    }
    /// Removes the value of type `T` [`Poses::set_data`] stored on the part named `name`.
    pub fn clear_data<T: std::any::Any + Send + Sync>(&mut self, name: &str) {
        if self
            .data
            .remove(&(key(name), std::any::TypeId::of::<T>()))
            .is_some()
        {
            self.changed();
        }
    }
    /// Select whether the next draw sees the frame override or only the animated visibility.
    pub fn apply_frame_visibility(&mut self, apply: bool) {
        if self.apply_frame_visibility != apply {
            self.apply_frame_visibility = apply;
            if self.has_frame_edits {
                self.changed();
            }
        }
    }
    /// Effective visibility for drawing; an inactive overlay costs no part lookup.
    pub fn is_hidden<P: PosePart>(&self, part: &P) -> bool {
        self.is_hidden_named(part.part_name())
            || self.frame_visibility_active()
                && self
                    .frame_layout
                    .as_ref()
                    .and_then(|layout| layout.index_of_identity(part.part_identity()))
                    .is_some_and(|index| self.frame_hidden[index])
    }
    pub fn has_hidden_parts(&self) -> bool {
        self.base_hidden_count != 0 || self.frame_visibility_active()
    }
    pub fn is_hidden_at(&self, index: usize, key: u64) -> bool {
        self.is_hidden_key(key) || self.frame_visibility_active() && self.frame_hidden[index]
    }
    fn frame_visibility_active(&self) -> bool {
        self.apply_frame_visibility && self.has_frame_edits
    }
}

impl mod_api::HumanoidPose for Poses {
    fn get(&self, part: HumanoidPart) -> Option<[[f32; 3]; 2]> {
        let key = HUMANOID_POSE_KEYS[part as usize]?;
        let [x, y, z, x_rot, y_rot, z_rot, ..] = self.map.get(&key)?.pose;
        Some([[x, y, z], [x_rot, y_rot, z_rot]])
    }

    fn set_position(&mut self, part: HumanoidPart, position: [f32; 3]) -> bool {
        self.set_humanoid_vector(part, 0, position)
    }

    fn set_rotation(&mut self, part: HumanoidPart, rotation: [f32; 3]) -> bool {
        self.set_humanoid_vector(part, 3, rotation)
    }
}

/// Gameplay's callback at the humanoid mid-setupAnim seam.
/// This library knows only animation inputs and poses, never Game or a Host.
pub trait HumanoidSetup {
    fn before_arm_pose(&self, mob: &Mob, state: &State<'_>, poses: &mut Poses);
}

#[derive(Clone, Copy)]
pub struct Observation<'a> {
    source: &'a Mob,
    observer: &'a dyn HumanoidSetup,
}

impl std::fmt::Debug for Observation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HumanoidSetup")
    }
}

/// Render data queried by an animation while it poses a mob.
pub trait AnimDataSource {
    fn humanoid_setup(&self) -> Option<&dyn HumanoidSetup> {
        None
    }
    fn piglin_loves(&self, _item: &str) -> bool {
        false
    }
    fn has_tool(&self, _stack: &crate::items::ItemStack) -> bool {
        false
    }
}

impl std::fmt::Debug for dyn AnimDataSource + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnimDataSource")
    }
}

/// The LivingEntityRenderState values setupAnim reads.
#[derive(Clone, Copy, Debug)]
pub struct State<'a> {
    pub pos: f32,
    pub speed: f32,
    pub age: f32,
    /// Head yaw relative to the body, degrees.
    pub y_rot: f32,
    /// Head pitch, degrees.
    pub x_rot: f32,
    pub swing: f32,
    pub baby: bool,
    pub partial: f32,
    /// Optional frame-local observer; absent without subscribers.
    pub observer: Option<Observation<'a>>,
    /// The resources the frame draws from, which answer its tag reads (a piglin's
    /// `#piglin_loved`); absent for tools, probes and tests, which then read no tag.
    pub data: Option<&'a dyn AnimDataSource>,
}

impl State<'static> {
    /// Attach the gameplay observer for this particular animation invocation.
    pub fn observed<'a>(
        self,
        source: &'a Mob,
        observer: Option<&'a dyn HumanoidSetup>,
    ) -> State<'a> {
        State {
            observer: observer.map(|observer| Observation { source, observer }),
            ..self
        }
    }
    /// [`Self::observed`] by `data`'s observer, with `data` answering the frame's tag reads.
    pub fn drawn_from<'a>(self, source: &'a Mob, data: &'a dyn AnimDataSource) -> State<'a> {
        State {
            data: Some(data),
            ..self.observed(source, data.humanoid_setup())
        }
    }
}

/// `LivingEntityRenderer.isEntityUpsideDown`: named Dinnerbone or Grumm.
pub fn upside_down(m: &Mob) -> bool {
    matches!(m.custom_name.as_deref(), Some("Dinnerbone" | "Grumm"))
}
/// `WalkAnimationState.position(partial)`: babies' legs move 3 times as far.
pub fn walk_position(m: &Mob, alpha: f32) -> f32 {
    (m.limb_position - m.limb_speed * (1.0 - alpha)) * if m.is_baby() { 3.0 } else { 1.0 }
}
/// `LivingEntityRenderer.extractRenderState`'s `bodyRot` (`solveBodyRot`):
/// the rider of a living vehicle is drawn from the vehicle's body and its own
/// head, any other mob from its own body.
pub fn body_rot(m: &Mob, alpha: f32) -> f32 {
    match m.extra.vehicle_body.filter(|v| m.extra.mount == Some(v.0)) {
        Some((_, before, now)) => {
            let head = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
            super::riders::solve_body_rot(head, before + wrap(now - before) * alpha)
        }
        None => m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * alpha,
    }
}
pub fn state(m: &Mob, alpha: f32) -> State<'static> {
    let body = body_rot(m, alpha);
    let head = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
    let mut swing = m.anim.attack_anim - m.anim.attack_anim_o;
    if swing < 0.0 {
        swing += 1.0;
    }
    let swing = m.anim.attack_anim_o + swing * alpha;
    // LivingEntityRenderer.extractRenderState: riding motion is not walking.
    let walking = m.alive() && !m.extra.passenger;
    State {
        pos: if walking {
            walk_position(m, alpha)
        } else {
            0.0
        },
        speed: if walking {
            (m.previous_limb_speed + (m.limb_speed - m.previous_limb_speed) * alpha).min(1.0)
        } else {
            0.0
        },
        age: m.age as f32 + alpha,
        y_rot: wrap(head - body),
        x_rot: m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha,
        swing,
        baby: m.is_baby(),
        partial: alpha,
        observer: None,
        data: None,
    }
}
fn head(p: &mut Poses, s: &State) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
}
/// QuadrupedModel.setupAnim
pub(super) fn quadruped(p: &mut Poses, s: &State) {
    head(p, s);
    let (pos, sp) = (s.pos, s.speed);
    p.xrot("right_hind_leg", (pos * 0.6662).mcos() * 1.4 * sp);
    p.xrot("left_hind_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp);
    p.xrot("right_front_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp);
    p.xrot("left_front_leg", (pos * 0.6662).mcos() * 1.4 * sp);
}
/// AnimationUtils.bobModelPart
fn bob(p: &mut Poses, n: &str, age: f32, scale: f32) {
    let q = p.get(n);
    q[5] += scale * ((age * 0.09).mcos() * 0.05 + 0.05);
    q[3] += scale * ((age * 0.067).msin() * 0.05);
}
pub use sunrise_core::arm_pose::ArmPose;
/// Whether this renderer extracts `HumanoidRenderState` arm poses.
pub fn extracts_humanoid(m: &Mob) -> bool {
    matches!(
        m.kind.anim,
        Anim::Zombie
            | Anim::Drowned
            | Anim::ZombieVillager
            | Anim::ZombifiedPiglin
            | Anim::Skeleton
            | Anim::Piglin
            | Anim::Humanoid { .. }
    )
}

/// Select the main/off-hand poses without dispatching hooks or allocating.
/// `declared` contains item declarations or the gameplay hook's replacements.
pub fn humanoid_arm_poses(m: &Mob, declared: [Option<ArmPose>; 2], swing: f32) -> [ArmPose; 2] {
    let player = matches!(m.kind.anim, Anim::Humanoid { zombie_arms: false });
    if !player {
        let default = if m.kind.anim == Anim::Skeleton
            && m.extra.main_hand_id() == Some("minecraft:bow")
            && m.ai.aggressive
        {
            ArmPose::Bow
        } else if m.kind.anim == Anim::Drowned
            && m.ai.aggressive
            && m.extra.main_hand_id() == Some("minecraft:trident")
        {
            ArmPose::ThrowTrident
        } else if m.kind.anim != Anim::Skeleton
            && m.extra
                .main_hand_id()
                .and_then(crate::spear_sim::Kinetic::of)
                .is_some()
        {
            ArmPose::Spear
        } else {
            ArmPose::Empty
        };
        let main = m.extra.main_hand_id().and(declared[0]).unwrap_or(default);
        let off = if main.affects_offhand() {
            ArmPose::Empty
        } else {
            m.extra
                .off_hand_id()
                .and(declared[1])
                .unwrap_or(ArmPose::Empty)
        };
        return [main, off];
    }
    let using_main = m.extra.using_item > 0 && !m.extra.using_off_hand;
    let using_off = m.extra.using_item > 0 && m.extra.using_off_hand;
    // AvatarRenderer.getArmPose: an arm holding a spear (at rest too).
    let spear_in = |hand: Option<&str>| hand.and_then(crate::spear_sim::Kinetic::of);
    let (main_spear, off_spear) = (
        spear_in(m.extra.main_hand_id()),
        spear_in(m.extra.off_hand_id()),
    );
    // AvatarRenderer.getArmPose: a loaded crossbow is held up, an item in
    // use poses its arm by its use animation, a spear is held out.
    let hand_pose = |held: Option<&str>, declared: Option<ArmPose>, using: bool, spear: bool| match (
        held, declared,
    ) {
        (None, _) => ArmPose::Empty,
        // A mod item's `arm_pose` replaces what the item would do.
        (Some(_), Some(declared)) => declared,
        (Some("minecraft:crossbow"), None) if m.extra.charged && swing == 0.0 => {
            ArmPose::CrossbowHold
        }
        (Some(_), None) => match using.then_some(m.extra.use_animation) {
            Some(UseAnimation::Block) => ArmPose::Block,
            Some(UseAnimation::Bow) => ArmPose::Bow,
            Some(UseAnimation::Trident) => ArmPose::ThrowTrident,
            Some(UseAnimation::Crossbow) => ArmPose::CrossbowCharge,
            Some(UseAnimation::Spyglass) => ArmPose::Spyglass,
            Some(UseAnimation::TootHorn) => ArmPose::TootHorn,
            Some(UseAnimation::Brush) => ArmPose::Brush,
            Some(UseAnimation::Spear) => ArmPose::Spear,
            _ if spear => ArmPose::Spear,
            _ => ArmPose::Item,
        },
    };
    let pose = hand_pose(
        m.extra.main_hand_id(),
        declared[0],
        using_main,
        main_spear.is_some(),
    );
    let off = if !pose.affects_offhand() {
        hand_pose(
            m.extra.off_hand_id(),
            declared[1],
            using_off,
            off_spear.is_some(),
        )
    } else {
        ArmPose::Empty
    };
    [pose, off]
}

/// HumanoidModel.setupAnim with empty hands (endermen).
/// Passes `isPassenger` so a seated enderman keeps the humanoid riding pose
/// (vanilla EndermanModel.setupAnim calls super first, then halves/clamps xRot).
pub fn humanoid_pub(p: &mut Poses, s: &State, m: &Mob) {
    let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
    h.passenger = m.extra.passenger;
    humanoid(p, s, &h);
}
/// `AnimationUtils.swingWeaponDown`: the main arm swings down, the other
/// one hangs and bobs.
pub(super) fn swing_weapon_down(p: &mut Poses, swing: f32, age: f32, left_main: bool) {
    let attack2 = (swing * PI).msin();
    let attack = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).msin();
    let (main, off) = if left_main {
        ("left_arm", "right_arm")
    } else {
        ("right_arm", "left_arm")
    };
    p.zrot("right_arm", 0.0);
    p.zrot("left_arm", 0.0);
    p.yrot("right_arm", PI / 20.0);
    p.yrot("left_arm", -PI / 20.0);
    p.xrot(main, -1.884_955_8 + (age * 0.09).mcos() * 0.15);
    p.xrot(off, -0.0 + (age * 0.19).mcos() * 0.5);
    p.get(main)[3] += attack2 * 2.2 - attack * 0.4;
    p.get(off)[3] += attack2 * 1.2 - attack * 0.4;
    bob(p, "right_arm", age, 1.0);
    bob(p, "left_arm", age, -1.0);
}
/// What `HumanoidModel.setupAnim` reads besides the walk and head inputs.
#[derive(Clone, Copy, Debug)]
pub struct Hum {
    pub right: ArmPose,
    pub left: ArmPose,
    pub crouching: bool,
    pub passenger: bool,
    pub fall_flying: bool,
    pub fly_speed_scale: f32,
    /// `Some(aggressive)` for `ZombieModel`-style `animateZombieArms`.
    pub zombie: Option<bool>,
    /// The main hand holds something (`getMainHandItemStack() != EMPTY`).
    pub holding: bool,
    /// `maxCrossbowChargeDuration`, `ticksUsingItem`.
    pub charge: (f32, f32),
    /// `PiglinModel.setupAttackAnimation`: swing a held weapon down.
    pub weapon_swing: bool,
    /// `swimAmount` at this frame.
    pub swim: f32,
    /// `isUsingItem`: the swim stroke leaves the arms alone.
    pub using_item: bool,
    /// The arm the current swing moves (`Some(true)` for the right one).
    pub swing_right_arm: Option<bool>,
    /// The swing is a STAB (`SwingAnimationType` 2): a spear's jab.
    pub stab: bool,
    /// `mainArm` is the left one (`Mob.isLeftHanded`): a mob's swing uses it.
    pub left_main: bool,
    /// The spear each arm (right, left) holds, and how long that arm has
    /// been using it (`ticksUsingItem(arm)`, 0 when it is not).
    pub spear: [Option<crate::spear_sim::Kinetic>; 2],
    pub spear_ticks: [f32; 2],
}
impl Hum {
    pub fn new(right: ArmPose, left: ArmPose) -> Self {
        Self {
            right,
            left,
            crouching: false,
            passenger: false,
            fall_flying: false,
            fly_speed_scale: 1.0,
            zombie: None,
            holding: false,
            charge: (0.0, 0.0),
            weapon_swing: false,
            swim: 0.0,
            using_item: false,
            swing_right_arm: None,
            stab: false,
            left_main: false,
            spear: [None; 2],
            spear_ticks: [0.0; 2],
        }
    }
}
/// Kinetic data belongs to the physical arm holding the item, including a mob's off hand.
pub(super) fn humanoid_spears(m: &Mob, h: &mut Hum, partial: f32) {
    let main_side = usize::from(m.main_arm_left());
    h.spear[main_side] = m
        .extra
        .main_hand_id()
        .and_then(crate::spear_sim::Kinetic::of);
    h.spear[1 - main_side] = m
        .extra
        .off_hand_id()
        .and_then(crate::spear_sim::Kinetic::of);
    if m.extra.using_item > 0 {
        let using_side = if m.extra.using_off_hand {
            1 - main_side
        } else {
            main_side
        };
        h.spear_ticks[using_side] = m.extra.using_item as f32 + partial;
    }
}

/// `AnimationUtils.animateCrossbowHold`.
pub(super) fn crossbow_hold(p: &mut Poses, hx: f32, hy: f32, in_right: bool) {
    let (hold, shoot) = if in_right {
        ("right_arm", "left_arm")
    } else {
        ("left_arm", "right_arm")
    };
    p.yrot(hold, if in_right { -0.3 } else { 0.3 } + hy);
    p.yrot(shoot, if in_right { 0.6 } else { -0.6 } + hy);
    p.xrot(hold, -PI / 2.0 + hx + 0.1);
    p.xrot(shoot, -1.5 + hx);
}
/// `AnimationUtils.animateCrossbowCharge`.
pub(super) fn crossbow_charge(p: &mut Poses, max: f32, ticks: f32, in_right: bool) {
    let (hold, pull) = if in_right {
        ("right_arm", "left_arm")
    } else {
        ("left_arm", "right_arm")
    };
    p.yrot(hold, if in_right { -0.8 } else { 0.8 });
    p.xrot(hold, -0.970_796_35);
    p.xrot(pull, -0.970_796_35);
    let use_ticks = ticks.clamp(0.0, max);
    let alpha = use_ticks / max;
    let lerp = |a: f32, p0: f32, p1: f32| p0 + a * (p1 - p0);
    p.yrot(
        pull,
        lerp(alpha, 0.4, 0.85) * if in_right { 1.0 } else { -1.0 },
    );
    let x = lerp(alpha, p.get(pull)[3], -PI / 2.0);
    p.xrot(pull, x);
}
/// `HumanoidModel.SPYGLASS_ARM_ROT_X`.
const SPYGLASS_ARM_ROT_X: f32 = 1.919_862_2;
/// `HumanoidModel.poseRightArm` / `poseLeftArm`.
#[path = "anim/custom_arm_pose.rs"]
mod custom_arm_pose;

fn pose_arm(p: &mut Poses, pose: ArmPose, right: bool, h: &Hum) {
    let arm = if right { "right_arm" } else { "left_arm" };
    let (hx, hy) = (p.get("head")[3], p.get("head")[4]);
    match pose {
        ArmPose::Custom { angles, .. } => custom_arm_pose::apply(p, angles),
        ArmPose::Empty => p.yrot(arm, 0.0),
        ArmPose::Item => {
            let x = p.get(arm)[3];
            p.xrot(arm, x * 0.5 - PI / 10.0);
            p.yrot(arm, 0.0);
        }
        // Vanilla's BRUSH case: the ITEM pose with twice the drop.
        ArmPose::Brush => {
            let x = p.get(arm)[3];
            p.xrot(arm, x * 0.5 - PI / 5.0);
            p.yrot(arm, 0.0);
        }
        ArmPose::Block => {
            let x = p.get(arm)[3] * 0.5 - 0.942_477_9 + hx.clamp(-1.396_263_4, 0.436_332_32);
            p.xrot(arm, x);
            p.yrot(
                arm,
                if right { -FRAC_PI_6 } else { FRAC_PI_6 } + hy.clamp(-FRAC_PI_6, FRAC_PI_6),
            );
        }
        ArmPose::Bow => {
            p.yrot("right_arm", -0.1 + hy - if right { 0.0 } else { 0.4 });
            p.yrot("left_arm", 0.1 + hy + if right { 0.4 } else { 0.0 });
            p.xrot("right_arm", -PI / 2.0 + hx);
            p.xrot("left_arm", -PI / 2.0 + hx);
        }
        ArmPose::ThrowTrident => {
            let x = p.get(arm)[3];
            p.xrot(arm, x * 0.5 - PI);
            p.yrot(arm, 0.0);
        }
        ArmPose::Spyglass => {
            let crouch = if h.crouching { 0.261_799_4 } else { 0.0 };
            p.xrot(arm, (hx - SPYGLASS_ARM_ROT_X - crouch).clamp(-2.4, 3.3));
            p.yrot(arm, hy + if right { -0.261_799_4 } else { 0.261_799_4 });
        }
        ArmPose::TootHorn => {
            p.xrot(arm, hx.clamp(-1.2, 1.2) - 1.483_529_8);
            p.yrot(arm, hy + if right { -FRAC_PI_6 } else { FRAC_PI_6 });
        }
        ArmPose::CrossbowHold => crossbow_hold(p, hx, hy, right),
        ArmPose::CrossbowCharge => crossbow_charge(p, h.charge.0, h.charge.1, right),
        // SpearAnimations.thirdPersonHandUse.
        ArmPose::Spear => {
            let side = usize::from(!right);
            let Some(kinetic) = h.spear[side] else {
                return;
            };
            let (x, y, z) = crate::spear_sim::third_person_arm(
                &kinetic,
                h.spear_ticks[side],
                right,
                (hx, hy),
                h.fall_flying || h.swim > 0.0,
            );
            p.xrot(arm, x);
            p.yrot(arm, y);
            p.get(arm)[5] += z;
        }
    }
}
/// `HumanoidModel.setupAnim` (and `ZombieModel.setupAttackAnimation`).
pub(super) fn humanoid(p: &mut Poses, s: &State, h: &Hum) {
    humanoid_with_setup(p, s, h, |p| {
        if let Some(observation) = s.observer {
            observation
                .observer
                .before_arm_pose(observation.source, s, p);
        }
    });
}

/// Run the shared humanoid animation, observing the prepared pose before arm
/// posing. Specialized adapters can supply their own entity view at this seam.
pub fn humanoid_with_setup(p: &mut Poses, s: &State, h: &Hum, setup: impl FnOnce(&mut Poses)) {
    head(p, s);
    if h.fall_flying {
        p.xrot("head", -std::f32::consts::FRAC_PI_4);
    }
    let (pos, sp) = (s.pos, s.speed / h.fly_speed_scale);
    p.xrot("right_arm", (pos * 0.6662 + PI).mcos() * 2.0 * sp * 0.5);
    p.xrot("left_arm", (pos * 0.6662).mcos() * 2.0 * sp * 0.5);
    p.xrot("right_leg", (pos * 0.6662).mcos() * 1.4 * sp);
    p.xrot("left_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp);
    p.yrot("right_leg", 0.005);
    p.yrot("left_leg", -0.005);
    p.zrot("right_leg", 0.005);
    p.zrot("left_leg", -0.005);
    if h.passenger {
        p.get("right_arm")[3] += -PI / 5.0;
        p.get("left_arm")[3] += -PI / 5.0;
        p.xrot("right_leg", -1.413_716_7);
        p.yrot("right_leg", PI / 10.0);
        p.zrot("right_leg", 0.078_539_82);
        p.xrot("left_leg", -1.413_716_7);
        p.yrot("left_leg", -PI / 10.0);
        p.zrot("left_leg", -0.078_539_82);
    }
    setup(p);
    // The main arm is the right one: the off-hand pose goes first unless it
    // needs both hands.
    for right in sunrise_core::arm_pose::application_order(h.right, h.left)
        .into_iter()
        .flatten()
    {
        pose_arm(p, if right { h.right } else { h.left }, right, h);
    }
    // setupAttackAnimation: a WHACK or STAB swing of the swinging hand's arm.
    let hx = p.get("head")[3];
    let swing = s.swing;
    let swing_left = h.swing_right_arm.map_or(h.left_main, |right| !right);
    if swing > 0.0 && h.weapon_swing {
        swing_weapon_down(p, swing, s.age, h.left_main);
    } else if swing > 0.0 {
        let mut body_y = (swing.sqrt() * PI * 2.0).msin() * 0.2;
        if swing_left {
            body_y = -body_y;
        }
        p.yrot("body", body_y);
        let age_scale = if s.baby { 0.5 } else { 1.0 };
        {
            let r = p.get("right_arm");
            r[2] = body_y.msin() * 5.0 * age_scale;
            r[0] = -body_y.mcos() * 5.0 * age_scale;
        }
        {
            let l = p.get("left_arm");
            l[2] = -body_y.msin() * 5.0 * age_scale;
            l[0] = body_y.mcos() * 5.0 * age_scale;
        }
        let arm = if swing_left { "left_arm" } else { "right_arm" };
        if h.stab {
            // SpearAnimations.thirdPersonAttackHand: the arms keep their turn.
            p.get(arm)[3] += crate::spear_sim::third_person_attack_arm(swing);
        } else {
            p.get("right_arm")[4] += body_y;
            p.get("left_arm")[4] += body_y;
            p.get("left_arm")[3] += body_y;
            // Ease.outQuart
            let eased = 1.0 - (1.0 - swing).powi(4);
            let aa = (eased * PI).msin();
            let bb = (swing * PI).msin() * -(hx - 0.7) * 0.75;
            let r = p.get(arm);
            r[3] -= aa * 1.2 + bb;
            r[4] += body_y * 2.0;
            r[5] += (swing * PI).msin() * -0.4;
        }
    }
    // AnimationUtils.animateZombieArms leaves the arms of a spear alone.
    let using_spear = h.right == ArmPose::Spear || h.left == ArmPose::Spear;
    if let Some(aggressive) = h.zombie.filter(|_| !using_spear) {
        // AnimationUtils.animateZombieArms
        let raise = !s.baby || !h.holding;
        let drop = if raise {
            -PI / if aggressive { 1.5 } else { 2.25 }
        } else {
            0.0
        };
        let ym = if raise { 1.0 } else { -1.0 } * (swing * PI).msin();
        let xm = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).msin();
        let x = drop + ym * 1.2 - xm * 0.4;
        let y = 0.1 - ym * 0.6;
        // An arm raising a trident keeps its pose.
        if h.right != ArmPose::ThrowTrident {
            p.xrot("right_arm", x);
            p.yrot("right_arm", if raise { -y } else { y });
            p.zrot("right_arm", 0.0);
        }
        if h.left != ArmPose::ThrowTrident {
            p.xrot("left_arm", x);
            p.yrot("left_arm", if raise { y } else { -y });
            p.zrot("left_arm", 0.0);
        }
    }
    if h.crouching {
        p.xrot("body", 0.5);
        p.get("right_arm")[3] += 0.4;
        p.get("left_arm")[3] += 0.4;
        p.get("right_leg")[2] += 4.0;
        p.get("left_leg")[2] += 4.0;
        p.get("head")[1] += 4.2;
        p.get("body")[1] += 3.2;
        p.get("left_arm")[1] += 3.2;
        p.get("right_arm")[1] += 3.2;
    }
    bob(p, "right_arm", s.age, 1.0);
    bob(p, "left_arm", s.age, -1.0);
    if h.swim > 0.0 {
        swim(p, s.pos, h);
    }
}
/// `Mth.rotLerpRad`.
fn rot_lerp_rad(a: f32, from: f32, to: f32) -> f32 {
    let mut diff = to - from;
    // A mod's `humanoid_setup_anim` answer may be any finite angle: past a few turns, bring it
    // into one turn first, or the wrapping loops below never finish (adding 2π to a huge f32 rounds
    // back to it). Ordinary angles skip this and wrap exactly as vanilla's loops do.
    if diff.abs() > 64.0 * PI {
        diff = diff.rem_euclid(PI * 2.0);
    }
    while diff < -PI {
        diff += PI * 2.0;
    }
    while diff >= PI {
        diff -= PI * 2.0;
    }
    from + a * diff
}
/// `DrownedModel.setupSwimAnimation`: the arms reach up and sway, the legs
/// kick and the head lies straight.
fn drowned_swim(p: &mut Poses, s: &State, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let sway = amount * 0.35 * (0.1 * s.age).msin();
    let kick = amount * 0.55 * (0.1 * s.age).msin();
    let reach = -PI * 4.0 / 5.0;
    let (right_x, left_x) = (p.get("right_arm")[3], p.get("left_arm")[3]);
    let (right_z, left_z) = (p.get("right_arm")[5], p.get("left_arm")[5]);
    p.xrot("right_arm", rot_lerp_rad(amount, right_x, reach) + sway);
    p.xrot("left_arm", rot_lerp_rad(amount, left_x, reach) - sway);
    p.zrot("right_arm", rot_lerp_rad(amount, right_z, -0.15));
    p.zrot("left_arm", rot_lerp_rad(amount, left_z, 0.15));
    let (left_leg, right_leg) = (p.get("left_leg")[3], p.get("right_leg")[3]);
    p.xrot("left_leg", left_leg - kick);
    p.xrot("right_leg", right_leg + kick);
    p.xrot("head", 0.0);
}
/// `HumanoidModel.setupSwimAnimation`: the stroke of both arms over 26
/// walk-animation units, the head down and a slow flutter kick. The left arm
/// eases the short way round, the right one linearly, as in vanilla.
fn swim(p: &mut Poses, pos: f32, h: &Hum) {
    let amount = h.swim;
    if !h.fall_flying {
        let x = p.get("head")[3];
        p.xrot(
            "head",
            rot_lerp_rad(amount, x, -std::f32::consts::FRAC_PI_4),
        );
    }
    let swim_pos = pos % 26.0;
    let right_amount = if h.swing_right_arm == Some(true) {
        0.0
    } else {
        amount
    };
    let left_amount = if h.swing_right_arm == Some(false) {
        0.0
    } else {
        amount
    };
    let lerp = |a: f32, from: f32, to: f32| from + a * (to - from);
    let quadratic = |x: f32| -65.0 * x + x * x;
    if !h.using_item {
        // Targets (x, y, z) for the left and right arm.
        let targets = if swim_pos < 14.0 {
            let stroke = 1.870_796_4 * quadratic(swim_pos) / quadratic(14.0);
            Some(([0.0, PI, PI + stroke], [0.0, PI, PI - stroke]))
        } else if swim_pos < 22.0 {
            let t = (swim_pos - 14.0) / 8.0;
            let x = PI / 2.0 * t;
            Some((
                [x, PI, 5.012_389 - 1.870_796_4 * t],
                [x, PI, 1.270_796_3 + 1.870_796_4 * t],
            ))
        } else if swim_pos < 26.0 {
            let t = (swim_pos - 22.0) / 4.0;
            let x = PI / 2.0 - PI / 2.0 * t;
            Some(([x, PI, PI], [x, PI, PI]))
        } else {
            None
        };
        if let Some((left, right)) = targets {
            for (i, target) in left.into_iter().enumerate() {
                let angle = &mut p.get("left_arm")[3 + i];
                *angle = rot_lerp_rad(left_amount, *angle, target);
            }
            for (i, target) in right.into_iter().enumerate() {
                let angle = &mut p.get("right_arm")[3 + i];
                *angle = lerp(right_amount, *angle, target);
            }
        }
    }
    let x = p.get("left_leg")[3];
    p.xrot(
        "left_leg",
        lerp(amount, x, 0.3 * (pos * 0.333_333_34 + PI).mcos()),
    );
    let x = p.get("right_leg")[3];
    p.xrot(
        "right_leg",
        lerp(amount, x, 0.3 * (pos * 0.333_333_34).mcos()),
    );
}
/// Apply the mob's model animation to `p` (at rest).
#[sunrise_mod_api_macros::replaceable]
pub fn setup(
    m: &Mob,
    s: &State,
    p: &mut Poses,
    w_anims: &HashMap<String, super::keyframes::Animation>,
) {
    if super::body_anim::anim(m, s, p, w_anims) {
        return;
    }
    match m.kind.anim {
        Anim::Camel => super::camel::anim(m, s, p, w_anims),
        Anim::Breeze => super::breeze::anim(m, s, p, w_anims),
        Anim::Sniffer => super::sniffer::anim(m, s, p, w_anims),
        Anim::CopperGolem => super::copper_golem::anim(m, s, p, w_anims),
        Anim::HappyGhast => super::happy_ghast::anim(m, s, p),
        Anim::Nautilus => super::nautilus::anim(m, s, p, w_anims),
        // body_anim::llama runs first.
        Anim::Llama => quadruped(p, s),
        Anim::Warden => super::warden::anim(m, s, p, w_anims),
        Anim::Allay => super::allay::anim(m, s, p),
        Anim::Creaking => super::creaking::anim(m, s, p, w_anims),
        Anim::Squid | Anim::Fish | Anim::Pufferfish | Anim::Dolphin => {
            super::aquatic::anim(m, s, p)
        }
        Anim::Quadruped | Anim::PolarBear | Anim::Panda | Anim::Goat => quadruped(p, s),
        Anim::Sheep => {
            quadruped(p, s);
            // SheepModel: lower the head while eating grass.
            let t = m.extra.anim_ticks;
            let partial = s.partial;
            let pos_scale = if t <= 0 {
                0.0
            } else if (4..=36).contains(&t) {
                1.0
            } else if t < 4 {
                (t as f32 - partial) / 4.0
            } else {
                -((t - 40) as f32 - partial) / 4.0
            };
            let angle = if t > 4 && t <= 36 {
                let f = ((t - 4) as f32 - partial) / 32.0;
                PI / 5.0 + 0.219_911_49 * (f * 28.7).msin()
            } else if t > 0 {
                PI / 5.0
            } else {
                s.x_rot.to_radians()
            };
            let age_scale = if s.baby { 0.5 } else { 1.0 };
            p.get("head")[1] += pos_scale * 9.0 * age_scale;
            p.xrot("head", angle);
        }
        Anim::Chicken => {
            let flap = m.anim.flap_o + (m.anim.flap - m.anim.flap_o) * s.partial;
            let flap_speed =
                m.anim.flap_speed_o + (m.anim.flap_speed - m.anim.flap_speed_o) * s.partial;
            let angle = (flap.msin() + 1.0) * flap_speed;
            p.xrot("right_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed);
            p.xrot("left_leg", (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed);
            p.zrot("right_wing", angle);
            p.zrot("left_wing", -angle);
            if !s.baby {
                head(p, s);
            }
        }
        Anim::Zombie
        | Anim::Drowned
        | Anim::ZombieVillager
        | Anim::ZombifiedPiglin
        | Anim::Humanoid { zombie_arms: true } => {
            let [main, off] = humanoid_arm_poses(m, m.extra.held_arm_pose, s.swing);
            let mut h = if m.main_arm_left() {
                Hum::new(off, main)
            } else {
                Hum::new(main, off)
            };
            h.left_main = m.main_arm_left();
            humanoid_spears(m, &mut h, s.partial);
            h.charge = (m.extra.ints[5].max(1) as f32, m.extra.using_item as f32);
            h.zombie = Some(m.ai.aggressive);
            h.holding = m.extra.main_hand_id().is_some();
            h.crouching = m.extra.crouching;
            h.passenger = m.extra.passenger;
            humanoid(p, s, &h);
            if m.kind.anim == Anim::Drowned {
                drowned_swim(p, s, m.extra.swim.amount(s.partial));
            }
            if m.kind.anim == Anim::ZombifiedPiglin {
                super::piglin::ears(p, s);
            }
        }
        Anim::Skeleton | Anim::Humanoid { zombie_arms: false } => {
            let bow = m.extra.main_hand_id() == Some("minecraft:bow");
            let player = matches!(m.kind.anim, Anim::Humanoid { .. });
            let [pose, off] = humanoid_arm_poses(m, m.extra.held_arm_pose, s.swing);
            let mut h = if m.main_arm_left() {
                Hum::new(off, pose)
            } else {
                Hum::new(pose, off)
            };
            h.charge = (m.extra.ints[5] as f32, m.extra.using_item as f32);
            h.fall_flying = m.extra.fall_flying;
            if h.fall_flying {
                let v = m.velocity;
                h.fly_speed_scale =
                    (((v.x * v.x + v.y * v.y + v.z * v.z) as f32 / 0.2).powi(3)).max(1.0);
            }
            h.crouching = m.extra.crouching;
            h.passenger = m.extra.passenger;
            h.swim = m.extra.swim.amount(s.partial);
            h.using_item = m.extra.using_item > 0;
            h.swing_right_arm = m.extra.swing_right_arm;
            h.stab = m.extra.swing_stab;
            h.left_main = m.main_arm_left();
            humanoid_spears(m, &mut h, s.partial);
            humanoid(p, s, &h);
            if player {
                player_model_visibility(p, m.extra.hidden_model_parts, m.extra.spectator);
            }
            if m.ai.aggressive && !bow {
                // SkeletonModel melee pose.
                let sw = s.swing;
                let s2 = (sw * PI).msin();
                let s1 = ((1.0 - (1.0 - sw) * (1.0 - sw)) * PI).msin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", -(0.1 - s2 * 0.6));
                p.yrot("left_arm", 0.1 - s2 * 0.6);
                p.xrot("right_arm", -PI / 2.0 - (s2 * 1.2 - s1 * 0.4));
                p.xrot("left_arm", -PI / 2.0 - (s2 * 1.2 - s1 * 0.4));
                bob(p, "right_arm", s.age, 1.0);
                bob(p, "left_arm", s.age, -1.0);
            }
            // BoggedModel.setupAnim: the mushrooms on the head come off once sheared.
            if m.species() == super::Species::Bogged {
                p.visible("mushrooms", !m.extra.sheared);
            }
        }
        Anim::Witch => super::witch::anim(m, s, p),
        Anim::Villager => super::villager::anim(m, s, p),
        Anim::Wolf => super::tame::anim(m, s, p),
        Anim::IronGolem => super::golem::iron_anim(m, s, p),
        Anim::Illager => super::illager::anim(m, s, p),
        Anim::Piglin => super::piglin::piglin_anim(m, s, p),
        Anim::Phantom => super::phantom::anim(m, s, p),
        Anim::Guardian => super::guardian::anim(m, s, p),
        Anim::Vex => super::vex::anim(m, s, p),
        Anim::Wither => super::wither::anim(m, s, p),
        Anim::Ravager => super::ravager::anim(m, s, p),
        Anim::Shulker => super::shulker::anim(m, s, p),
        Anim::Horse => {
            super::equine::anim(m, s, p);
            // DonkeyModel: the chests show once fitted.
            if !m.extra.chested {
                for part in ["left_chest", "right_chest"] {
                    p.visible(part, false);
                }
            }
            // EquineSaddleModel.setupAnim: the reins show only while ridden
            // (`isRidden` is `Entity.isVehicle`: a player or a mob rides it).
            for part in ["left_saddle_line", "right_saddle_line"] {
                p.visible(part, m.rider.is_some() || m.extra.rider.is_some());
            }
        }
        Anim::Hoglin => super::piglin::hoglin_anim(m, s, p),
        Anim::Blaze => super::nether::blaze_anim(s, p),
        Anim::Ghast => super::nether::ghast_anim(s, p),
        Anim::Feline => super::tame::cat_anim(m, s, p),
        Anim::Enderman => super::enderman::anim(m, s, p),
        Anim::Slime | Anim::MagmaCube => super::cube::anim(m, s, p),
        Anim::Creeper => {
            head(p, s);
            // CreeperModel binds its fields to the opposite-side parts, so the
            // left formulas drive the right parts and the reverse.
            p.xrot("left_hind_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed);
            p.xrot(
                "right_hind_leg",
                (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed,
            );
            p.xrot(
                "left_front_leg",
                (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed,
            );
            p.xrot("right_front_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed);
        }
        Anim::Spider => {
            head(p, s);
            let pos = s.pos * 0.6662;
            let sp = s.speed;
            let swing = |o: f32| -((pos * 2.0 + o).mcos() * 0.4) * sp;
            let step = |o: f32| ((pos + o).msin() * 0.4).abs() * sp;
            let legs = [
                ("right_hind_leg", "left_hind_leg", 0.0f32, 0.0f32),
                ("right_middle_hind_leg", "left_middle_hind_leg", PI, PI),
                (
                    "right_middle_front_leg",
                    "left_middle_front_leg",
                    PI / 2.0,
                    PI / 2.0,
                ),
                (
                    "right_front_leg",
                    "left_front_leg",
                    PI * 3.0 / 2.0,
                    PI * 3.0 / 2.0,
                ),
            ];
            for (right, left, so, to) in legs {
                let (sw, st) = (swing(so), step(to));
                p.get(right)[4] += sw;
                p.get(left)[4] -= sw;
                p.get(right)[5] += st;
                p.get(left)[5] -= st;
            }
        }
        _ => {
            if p.has("head") {
                head(p, s);
            }
            if p.has("right_hind_leg") {
                quadruped(p, s);
            }
        }
    }
}

/// `PlayerModel.setupAnim`'s visibility: a spectator shows only its head (`showBody`), and the
/// skin layers the player hid (`PlayerModelPart`, bit set = hidden) are off.
fn player_model_visibility(p: &mut Poses, hidden: u8, spectator: bool) {
    for part in ["body", "right_arm", "left_arm", "right_leg", "left_leg"] {
        p.visible(part, !spectator);
    }
    for (bit, part) in [
        (1, "jacket"),
        (2, "left_sleeve"),
        (3, "right_sleeve"),
        (4, "left_pants"),
        (5, "right_pants"),
        (6, "hat"),
    ] {
        p.visible(part, hidden & (1 << bit) == 0);
    }
}
#[cfg(test)]
mod humanoid_pose_tests {
    use super::*;

    #[test]
    fn live_humanoid_pose_writes_all_components_and_refuses_only_missing_or_uneditable_parts() {
        let mut poses = Poses {
            map: [
                ("hat", [1.0, 2.0, 3.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0]),
                ("jacket", [0.0, 1.0, 2.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0]),
            ]
            .into_iter()
            .map(|(name, pose)| (key(name), PoseEntry::new(pose)))
            .collect(),
            ..Default::default()
        };
        let before = poses.version();
        let view = &mut poses as &mut dyn mod_api::HumanoidPose;
        assert_eq!(
            view.get(HumanoidPart::Hat),
            Some([[1.0, 2.0, 3.0], [0.25, 0.5, 0.75]])
        );
        assert!(view.set_position(HumanoidPart::Hat, [f32::NAN, 4.0, f32::INFINITY]));
        assert_ne!(poses.version(), before);

        let before = poses.version();
        let view = &mut poses as &mut dyn mod_api::HumanoidPose;
        assert!(view.set_rotation(HumanoidPart::Hat, [f32::INFINITY, f32::NAN, -0.75]));
        let [pos, rot] = view.get(HumanoidPart::Hat).unwrap();
        assert!(pos[0].is_nan() && pos[1] == 4.0 && pos[2] == f32::INFINITY);
        assert!(rot[0] == f32::INFINITY && rot[1].is_nan() && rot[2] == -0.75);
        assert_ne!(poses.version(), before);
        let hat = poses.pose_of("hat").unwrap();
        assert!(hat[0].is_nan() && hat[1] == 4.0 && hat[2] == f32::INFINITY);
        assert!(hat[3] == f32::INFINITY && hat[4].is_nan() && hat[5] == -0.75);
        assert_eq!(hat[6..9], [1.0, 1.0, 1.0]);

        let before = poses.version();
        let view = &mut poses as &mut dyn mod_api::HumanoidPose;
        assert_eq!(view.get(HumanoidPart::LeftArm), None);
        assert_eq!(view.get(HumanoidPart::Jacket), None);
        assert!(!view.set_position(HumanoidPart::LeftArm, [9.0; 3]));
        assert!(!view.set_rotation(HumanoidPart::Jacket, [9.0; 3]));
        assert_eq!(poses.pose_of("left_arm"), None);
        assert_eq!(
            poses.pose_of("jacket"),
            Some([0.0, 1.0, 2.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(poses.dummy, [0.0; 9]);
        assert_eq!(poses.version(), before);

        // Non-finite incoming poses do not suppress pose access or later writes.
        poses.xrot("hat", f32::NAN);
        let view = &mut poses as &mut dyn mod_api::HumanoidPose;
        let [_, rot] = view.get(HumanoidPart::Hat).unwrap();
        assert!(rot[0].is_nan());
        assert!(view.set_rotation(HumanoidPart::Hat, [0.5, 0.25, -0.5]));
        assert_eq!(view.get(HumanoidPart::Hat).unwrap()[1], [0.5, 0.25, -0.5]);
    }
}

#[cfg(test)]
mod rot_lerp_tests {
    use super::*;

    #[test]
    fn a_huge_finite_angle_wraps_instead_of_looping() {
        for from in [f32::MAX, -f32::MAX, 1.0e9, -1.0e9] {
            let out = rot_lerp_rad(0.5, from, -std::f32::consts::FRAC_PI_4);
            assert!(out.is_finite(), "from {from}");
        }
        // Ordinary angles are untouched: half way from 0 to 3π/2 goes the short way, to -π/4.
        assert!((rot_lerp_rad(0.5, 0.0, 1.5 * PI) + 0.25 * PI).abs() < 1e-6);
    }
}

#[cfg(test)]
mod flight_pose_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    #[test]
    fn spectators_show_only_the_head_and_its_hat() {
        let mut p = Poses {
            map: [
                "head",
                "hat",
                "body",
                "jacket",
                "right_arm",
                "left_arm",
                "right_leg",
                "left_leg",
            ]
            .into_iter()
            .map(|n| (key(n), PoseEntry::new([0.0; 9])))
            .collect(),
            ..Default::default()
        };
        player_model_visibility(&mut p, 0, true);
        for part in ["body", "right_arm", "left_arm", "right_leg", "left_leg"] {
            assert!(p.is_hidden_named(part), "{part}");
        }
        assert!(!p.is_hidden_named("head") && !p.is_hidden_named("hat"));
        // Back in another mode they all show, but for the layers the player hid.
        player_model_visibility(&mut p, 1 << 6, false);
        assert!(!p.is_hidden_named("body") && !p.is_hidden_named("left_leg"));
        assert!(p.is_hidden_named("hat"));
    }
    #[test]
    fn blocking_arm_follows_the_head_with_limits_in_both_hands() {
        for right in [false, true] {
            for (pitch, yaw, expected_pitch, expected_yaw) in [
                (0.0, 0.0, -0.742_477_9, 0.0),
                (2.0, 2.0, -0.306_145_58, FRAC_PI_6),
                (-2.0, -2.0, -2.138_741_3, -FRAC_PI_6),
            ] {
                let mut p = Poses {
                    map: ["head", "right_arm", "left_arm"]
                        .into_iter()
                        .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                        .collect(),
                    ..Default::default()
                };
                let arm = if right { "right_arm" } else { "left_arm" };
                p.xrot("head", pitch);
                p.yrot("head", yaw);
                p.xrot(arm, 0.4);
                pose_arm(
                    &mut p,
                    ArmPose::Block,
                    right,
                    &Hum::new(ArmPose::Empty, ArmPose::Empty),
                );
                let pose = *p.get(arm);
                assert!((pose[3] - expected_pitch).abs() < 1e-6);
                let side = if right { -FRAC_PI_6 } else { FRAC_PI_6 };
                assert!((pose[4] - (side + expected_yaw)).abs() < 1e-6);
                assert_eq!(p.get(if right { "left_arm" } else { "right_arm" })[3], 0.0);
            }
        }
    }
    /// HumanoidModel.poseRightArm / poseLeftArm's SPYGLASS case: pitch, yaw,
    /// crouching, and the arm's pitch.
    #[test]
    fn a_spyglass_raises_the_arm_with_the_head_in_both_hands() {
        for right in [false, true] {
            for (pitch, yaw, crouching, expected_pitch) in [
                (0.0, 0.0, false, -1.919_862_2),
                (0.5, 0.3, false, -1.419_862_2),
                (0.0, 0.0, true, -2.181_661_6),
                // Looking up: limited to -2.4.
                (-1.5, 0.0, false, -2.4),
            ] {
                let mut p = Poses {
                    map: ["head", "right_arm", "left_arm"]
                        .into_iter()
                        .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                        .collect(),
                    ..Default::default()
                };
                let arm = if right { "right_arm" } else { "left_arm" };
                p.xrot("head", pitch);
                p.yrot("head", yaw);
                let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
                h.crouching = crouching;
                pose_arm(&mut p, ArmPose::Spyglass, right, &h);
                let pose = *p.get(arm);
                assert!((pose[3] - expected_pitch).abs() < 1e-6, "{pose:?}");
                let side = if right { -0.261_799_4 } else { 0.261_799_4 };
                assert!((pose[4] - (yaw + side)).abs() < 1e-6, "{pose:?}");
                assert_eq!(p.get(if right { "left_arm" } else { "right_arm" })[3], 0.0);
            }
        }
    }
    /// EndermanModel.setupAnim (#4565): humanoid_pub forwards isPassenger so the
    /// riding leg y/z survive; only xRot is halved and clamped to ±0.4 after.
    #[test]
    fn a_seated_enderman_keeps_the_riding_leg_twist() {
        use super::super::enderman;
        use super::super::{Mob, Species};
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::Enderman;
        let mut seated = Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0);
        seated.extra.passenger = true;
        let state = State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        let mut poses = Poses {
            map: [
                "head",
                "hat",
                "body",
                "right_arm",
                "left_arm",
                "right_leg",
                "left_leg",
            ]
            .into_iter()
            .map(|n| (key(n), PoseEntry::new([0.0; 9])))
            .collect(),
            ..Default::default()
        };
        enderman::anim(&seated, &state, &mut poses);
        let right = *poses.get("right_leg");
        let left = *poses.get("left_leg");
        assert!((right[3] - (-0.4)).abs() < 1e-5, "right xRot {}", right[3]);
        assert!((left[3] - (-0.4)).abs() < 1e-5, "left xRot {}", left[3]);
        assert!(
            (right[4] - PI / 10.0).abs() < 1e-5,
            "right yRot {}",
            right[4]
        );
        assert!(
            (left[4] - (-PI / 10.0)).abs() < 1e-5,
            "left yRot {}",
            left[4]
        );
        assert!(
            (right[5] - 0.078_539_82).abs() < 1e-5,
            "right zRot {}",
            right[5]
        );
        assert!(
            (left[5] - (-0.078_539_82)).abs() < 1e-5,
            "left zRot {}",
            left[5]
        );
    }

    #[test]
    fn flight_slows_limbs_and_sets_the_head_pose() {
        let state = State {
            pos: 0.0,
            speed: 1.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        let make = || Poses {
            map: [
                "head",
                "right_arm",
                "left_arm",
                "right_leg",
                "left_leg",
                "body",
            ]
            .into_iter()
            .map(|n| (key(n), PoseEntry::new([0.0; 9])))
            .collect(),
            ..Default::default()
        };
        let mut ground = make();
        let mut flight = make();
        let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
        humanoid(&mut ground, &state, &h);
        h.fall_flying = true;
        h.fly_speed_scale = 8.0;
        humanoid(&mut flight, &state, &h);
        assert_eq!(flight.get("head")[3], -std::f32::consts::FRAC_PI_4);
        assert_eq!(flight.get("right_leg")[3], ground.get("right_leg")[3] / 8.0);
        // THROW_TRIDENT: the arm goes up over the head, and zombie arms leave it there.
        let mut throw = make();
        let mut h = Hum::new(ArmPose::ThrowTrident, ArmPose::Item);
        h.zombie = Some(true);
        humanoid(&mut throw, &state, &h);
        let still = (state.pos * 0.6662 + PI).mcos() * 2.0 * 0.5;
        assert_eq!(throw.get("right_arm")[3], still * 0.5 - PI);
        assert_eq!(throw.get("right_arm")[4], 0.0);
        assert_eq!(throw.get("left_arm")[3], -PI / 1.5);
    }

    #[test]
    fn swimming_strokes_the_arms_and_kicks() {
        let state = State {
            pos: 7.0,
            speed: 1.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 30.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        let make = || Poses {
            map: ["head", "right_arm", "left_arm", "right_leg", "left_leg"]
                .into_iter()
                .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                .collect(),
            ..Default::default()
        };
        let same_angle = |a: f32, b: f32| ((a - b + PI).rem_euclid(PI * 2.0) - PI).abs() < 1e-5;
        let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
        h.swim = 1.0;
        let mut p = make();
        humanoid(&mut p, &state, &h);
        // Mid-stroke: both arms sweep out from overhead by the quadratic.
        let stroke = 1.870_796_4 * (-65.0 * 7.0 + 49.0) / (-65.0 * 14.0 + 196.0);
        assert_eq!(p.get("right_arm")[5], PI - stroke);
        assert!(same_angle(p.get("left_arm")[5], PI + stroke));
        assert_eq!(p.get("right_arm")[4], PI);
        assert!(same_angle(p.get("head")[3], -std::f32::consts::FRAC_PI_4));
        assert!(same_angle(
            p.get("right_leg")[3],
            0.3 * (7.0f32 * 0.333_333_34).mcos()
        ));
        // Using an item keeps the arms; a swing frees its own arm only.
        let mut using = make();
        h.using_item = true;
        humanoid(&mut using, &state, &h);
        assert_eq!(using.get("right_arm")[4], 0.0);
        h.using_item = false;
        h.swing_right_arm = Some(true);
        let mut swinging = make();
        humanoid(
            &mut swinging,
            &State {
                swing: 0.5,
                ..state
            },
            &h,
        );
        assert!(swinging.get("right_arm")[4] < 1.0);
        assert!(same_angle(swinging.get("left_arm")[4], PI));
        // A STAB swing turns that arm's pitch alone, by the spear's curve.
        let mut stabbing = make();
        h.swim = 0.0;
        h.stab = true;
        let mut still = make();
        humanoid(
            &mut still,
            &state,
            &Hum::new(ArmPose::Empty, ArmPose::Empty),
        );
        let stab = State {
            swing: 0.2,
            ..state
        };
        humanoid(&mut stabbing, &stab, &h);
        let arm_pitch = crate::spear_sim::third_person_attack_arm(0.2);
        assert!(same_angle(
            stabbing.get("right_arm")[3],
            still.get("right_arm")[3] + arm_pitch
        ));
        assert_eq!(stabbing.get("left_arm")[3], still.get("left_arm")[3]);
        // Out of the pose nothing changes.
        let mut dry = make();
        humanoid(&mut dry, &state, &Hum::new(ArmPose::Empty, ArmPose::Empty));
        assert_eq!(dry.get("head")[3], 30.0f32.to_radians());
    }

    #[test]
    fn walk_position_triples_for_babies() {
        // WalkAnimationState.position(partial) = (position - speed * (1 - partial)) * scale.
        let mut m = player();
        m.limb_position = 2.0;
        m.limb_speed = 0.5;
        assert!(!m.is_baby());
        assert_eq!(walk_position(&m, 0.5), 1.75);
        m.extra.age = -100;
        assert!(m.is_baby());
        assert_eq!(walk_position(&m, 0.5), 5.25);
    }

    /// A player: a humanoid model with a shield in its main hand.
    fn player() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.anim = Anim::Humanoid { zombie_arms: false };
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    #[test]
    fn a_raised_shield_holds_the_arm_across_the_body() {
        // AvatarRenderer.getArmPose: the used item's use animation decides
        // the arm's pose, and a shield (an item with `blocks_attacks`) uses
        // the BLOCK animation, so the arm is held up across the body
        // (#2289).
        let state = State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        let mut p = Poses {
            map: ["head", "right_arm", "left_arm"]
                .into_iter()
                .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                .collect(),
            ..Default::default()
        };
        let mut m = player();
        m.extra.set_slot_id(
            sunrise_items::equipment::MAINHAND,
            Some("minecraft:shield".into()),
        );
        m.extra.using_item = 20;
        m.extra.use_animation = UseAnimation::Block;
        setup(&m, &state, &mut p, &HashMap::new());
        // HumanoidModel.setupAnim's BLOCK pose.
        assert!((p.get("right_arm")[3] + 0.942_477_9).abs() < 1e-6);
        assert!((p.get("right_arm")[4] + FRAC_PI_6).abs() < 1e-6);
        // An item that does not block attacks leaves the arm holding it.
        let mut m = player();
        m.extra.set_slot_id(
            sunrise_items::equipment::MAINHAND,
            Some("minecraft:stick".into()),
        );
        m.extra.using_item = 20;
        let mut p = Poses {
            map: ["head", "right_arm", "left_arm"]
                .into_iter()
                .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                .collect(),
            ..Default::default()
        };
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("right_arm")[3] + PI / 10.0).abs() < 1e-6);
    }

    #[test]
    fn a_brush_being_used_sweeps_the_arm_lower_than_a_plain_item() {
        // AvatarRenderer.getArmPose: a brush being used is the BRUSH pose, and
        // HumanoidModel.poseRightArm drops that arm twice as far as ITEM
        // (#4273: brushing had no third-person arm pose at all).
        let state = State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        let make = || Poses {
            map: ["head", "right_arm", "left_arm"]
                .into_iter()
                .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                .collect(),
            ..Default::default()
        };
        let mut m = player();
        m.extra
            .set_slot_id(sunrise_items::equipment::MAINHAND, Some("ruby:wand".into()));
        m.extra.using_item = 20;
        // A content item's consumable names the animation, not its id.
        m.extra.use_animation = UseAnimation::Brush;
        let mut p = make();
        setup(&m, &state, &mut p, &HashMap::new());
        // HumanoidModel.setupAnim's BRUSH pose.
        assert!((p.get("right_arm")[3] + PI / 5.0).abs() < 1e-6);
        assert_eq!(p.get("right_arm")[4], 0.0);
        // An item with no use animation keeps the ITEM pose: BRUSH cannot leak.
        let mut m = player();
        m.extra.set_slot_id(
            sunrise_items::equipment::MAINHAND,
            Some("minecraft:stick".into()),
        );
        m.extra.using_item = 20;
        let mut p = make();
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("right_arm")[3] + PI / 10.0).abs() < 1e-6);
        assert_eq!(p.get("right_arm")[4], 0.0);
        // AvatarRenderer.getArmPose builds the pose per hand: brushing with the
        // brush in the off hand is BRUSH on the left arm, and the main hand
        // keeps its ITEM pose (the two must not swap).
        let mut m = player();
        m.extra.set_slot_id(
            sunrise_items::equipment::MAINHAND,
            Some("minecraft:stick".into()),
        );
        m.extra.set_slot_id(
            sunrise_items::equipment::OFFHAND,
            Some("minecraft:brush".into()),
        );
        m.extra.using_item = 20;
        m.extra.using_off_hand = true;
        m.extra.use_animation = UseAnimation::Brush;
        let mut p = make();
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("left_arm")[3] + PI / 5.0).abs() < 1e-6);
        assert_eq!(p.get("left_arm")[4], 0.0);
        assert!((p.get("right_arm")[3] + PI / 10.0).abs() < 1e-6);
        assert_eq!(p.get("right_arm")[4], 0.0);
    }

    #[test]
    fn a_declared_arm_pose_replaces_the_pose_the_held_item_gives() {
        let state = State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        let make = || Poses {
            map: ["head", "right_arm", "left_arm", "right_leg", "left_leg"]
                .into_iter()
                .map(|n| (key(n), PoseEntry::new([0.0; 9])))
                .collect(),
            ..Default::default()
        };
        let mut m = player();
        m.extra.set_slot_id(
            sunrise_items::equipment::MAINHAND,
            Some("ruby:skillet".into()),
        );
        // A held item with no declaration keeps the ITEM pose.
        let mut p = make();
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("right_arm")[3] + PI / 10.0).abs() < 1e-6);
        // HumanoidModel.setupAnim's BRUSH pose, though nothing is being used.
        m.extra.held_arm_pose[0] = Some(ArmPose::Brush);
        let mut p = make();
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("right_arm")[3] + PI / 5.0).abs() < 1e-6);
        // The off hand's declaration poses the left arm, and the main hand keeps its own.
        m.extra.set_slot_id(
            sunrise_items::equipment::OFFHAND,
            Some("ruby:skillet".into()),
        );
        m.extra.held_arm_pose = [None, Some(ArmPose::Brush)];
        let mut p = make();
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("left_arm")[3] + PI / 5.0).abs() < 1e-6);
        assert!((p.get("right_arm")[3] + PI / 10.0).abs() < 1e-6);
    }

    #[test]
    fn humanoid_mob_item_poses_reach_both_arms_and_ignore_empty_hands() {
        let state = State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
            observer: None,
            data: None,
        };
        for (anim, pose, expected) in [
            (Anim::Zombie, ArmPose::ThrowTrident, -PI),
            (Anim::Drowned, ArmPose::ThrowTrident, -PI),
            (Anim::ZombieVillager, ArmPose::ThrowTrident, -PI),
            (Anim::ZombifiedPiglin, ArmPose::ThrowTrident, -PI),
            (Anim::Skeleton, ArmPose::Brush, -PI / 5.0),
            (Anim::Piglin, ArmPose::Brush, -PI / 5.0),
        ] {
            let mut m = player();
            Arc::make_mut(&mut m.kind).anim = anim;
            m.extra.set_main_hand(Some("minecraft:stick".into()));
            m.extra.held_arm_pose = [Some(pose), None];
            for left in [false, true] {
                m.extra.main_hand_left = left;
                let mut p = Poses::default();
                // The test model only needs these parts; no prepared assets or GPU.
                for part in [
                    "head",
                    "body",
                    "right_arm",
                    "left_arm",
                    "right_leg",
                    "left_leg",
                ] {
                    p.map.insert(key(part), PoseEntry::new([0.0; 9]));
                }
                setup(&m, &state, &mut p, &HashMap::new());
                let arm = if left { "left_arm" } else { "right_arm" };
                assert!((p.get(arm)[3] - expected).abs() < 1e-6, "{anim:?} {left}");
            }
            m.extra.set_main_hand(None);
            assert_eq!(
                humanoid_arm_poses(&m, m.extra.held_arm_pose, 0.0),
                [ArmPose::Empty; 2]
            );
        }
    }

    #[test]
    fn setup_observer_precedes_item_crouch_and_bob_for_each_invocation() {
        use std::cell::Cell;
        struct Observer(Cell<usize>);
        impl HumanoidSetup for Observer {
            fn before_arm_pose(&self, mob: &Mob, state: &State<'_>, poses: &mut Poses) {
                self.0.set(self.0.get() + 1);
                assert_eq!(state.partial, 0.5);
                if mob.extra.using_item > 0 {
                    let arm = if mob.main_arm_left() {
                        "left_arm"
                    } else {
                        "right_arm"
                    };
                    // This is after walking, before ITEM halves the angle.
                    assert_eq!(poses.get(arm)[3], 0.0);
                    poses.xrot(arm, (-20.0_f32).to_radians());
                }
            }
        }
        let observer = Observer(Cell::new(0));
        let make = || {
            let mut poses = Poses::default();
            for name in [
                "head",
                "hat",
                "body",
                "right_arm",
                "left_arm",
                "right_leg",
                "left_leg",
            ] {
                poses.map.insert(
                    key(name),
                    PoseEntry::new([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
                );
            }
            poses
        };
        for left in [false, true] {
            for using in [false, true] {
                for crouching in [false, true] {
                    let mut mob = player();
                    mob.extra.set_main_hand(Some("minecraft:stick".into()));
                    mob.extra.main_hand_left = left;
                    mob.extra.using_item = i32::from(using);
                    mob.extra.crouching = crouching;
                    let state = state(&mob, 0.5);
                    let observed = state.observed(&mob, Some(&observer));
                    for _ in 0..2 {
                        let before = observer.0.get();
                        let mut poses = make();
                        setup(&mob, &observed, &mut poses, &HashMap::new());
                        assert_eq!(observer.0.get(), before + 1);
                        let arm = if left { "left_arm" } else { "right_arm" };
                        let input = if using { -20.0 * PI / 180.0 } else { 0.0 };
                        let bob = (0.5_f32 * 0.067).sin() * 0.05 * if left { -1.0 } else { 1.0 };
                        let expected =
                            input * 0.5 - PI / 10.0 + if crouching { 0.4 } else { 0.0 } + bob;
                        // The engine's Mth sine table quantizes bob; the
                        // independent analytic sine differs by a few microradians.
                        assert!(
                            (poses.get(arm)[3] - expected).abs() < 1.0e-5,
                            "left={left} using={using} crouching={crouching}: got {}, expected {expected}",
                            poses.get(arm)[3]
                        );
                        if !using {
                            let mut baseline = make();
                            setup(&mob, &state, &mut baseline, &HashMap::new());
                            assert_eq!(poses.map.len(), baseline.map.len());
                            for (&part, entry) in &baseline.map {
                                assert_eq!(poses.pose_by_key(part), Some(&entry.pose));
                            }
                        }
                    }
                }
            }
        }
        for animation in [
            Anim::Zombie,
            Anim::Humanoid { zombie_arms: true },
            Anim::Enderman,
            Anim::Piglin,
        ] {
            let mut mob = player();
            Arc::make_mut(&mut mob.kind).anim = animation;
            let state = state(&mob, 0.5).observed(&mob, Some(&observer));
            let before = observer.0.get();
            setup(&mob, &state, &mut make(), &HashMap::new());
            assert_eq!(observer.0.get(), before + 1, "{animation:?}");
        }
    }

    #[test]
    fn swimming_setup_returns_after_a_listener_writes_the_largest_finite_head_angle() {
        use std::cell::Cell;
        struct Listener(Cell<bool>);
        impl HumanoidSetup for Listener {
            fn before_arm_pose(&self, _: &Mob, _: &State<'_>, poses: &mut Poses) {
                self.0.set(true);
                poses.xrot("head", f32::MAX);
            }
        }
        let listener = Listener(Cell::new(false));
        let mut mob = player();
        mob.extra.swim.old = 1.0;
        mob.extra.swim.amount = 1.0;
        mob.extra.swim.visually = true;
        let state = state(&mob, 0.5).observed(&mob, Some(&listener));
        let mut poses = Poses::default();
        poses.map.insert(
            key("head"),
            PoseEntry::new([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
        );
        setup(&mob, &state, &mut poses, &HashMap::new());
        assert!(listener.0.get());
        assert!(poses.get("head")[3].is_finite());
    }

    #[test]
    fn a_humanoid_mobs_two_handed_item_pose_keeps_vanillas_other_arm() {
        let mut m = player();
        Arc::make_mut(&mut m.kind).anim = Anim::Skeleton;
        m.extra.set_main_hand(Some("minecraft:stick".into()));
        m.extra.set_slot_id(
            sunrise_items::equipment::OFFHAND,
            Some("minecraft:stick".into()),
        );
        m.extra.held_arm_pose = [Some(ArmPose::Bow), Some(ArmPose::Brush)];
        assert_eq!(
            humanoid_arm_poses(&m, m.extra.held_arm_pose, 0.0),
            [ArmPose::Bow, ArmPose::Empty]
        );
        for left in [false, true] {
            m.extra.main_hand_left = left;
            let mut p = Poses::default();
            for part in [
                "head",
                "body",
                "right_arm",
                "left_arm",
                "right_leg",
                "left_leg",
            ] {
                p.map.insert(key(part), PoseEntry::new([0.0; 9]));
            }
            let state = State {
                pos: 0.0,
                speed: 0.0,
                age: 0.0,
                y_rot: 0.0,
                x_rot: 0.0,
                swing: 0.0,
                baby: false,
                partial: 0.0,
                observer: None,
                data: None,
            };
            setup(&m, &state, &mut p, &HashMap::new());
            for arm in ["right_arm", "left_arm"] {
                assert!((p.get(arm)[3] + PI / 2.0).abs() < 1e-6);
            }
            assert!((p.get("right_arm")[4] - if left { -0.5 } else { -0.1 }).abs() < 1e-6);
            assert!((p.get("left_arm")[4] - if left { 0.1 } else { 0.5 }).abs() < 1e-6);
        }
    }

    /// ModelPart.createPartLookup fills the name map with putIfAbsent, so of two
    /// parts sharing a name the first registered keeps it, not the last.
    #[test]
    fn a_name_two_parts_share_belongs_to_the_first() {
        let child = |xrot: f32| TestPart {
            name: "leg".into(),
            pose: [0.0, 0.0, 0.0, xrot, 0.0, 0.0, 1.0, 1.0, 1.0],
            children: Vec::new(),
        };
        let root = TestPart {
            name: "root".into(),
            pose: [0.0; 9],
            children: vec![child(0.5), child(-0.5)],
        };
        let mut p = Poses::new(&root);
        assert_eq!(p.get("leg")[3], 0.5);
    }
}

#[cfg(test)]
struct TestPart {
    name: String,
    pose: [f32; 9],
    children: Vec<TestPart>,
}

#[cfg(test)]
impl PosePart for TestPart {
    fn part_name(&self) -> &str {
        &self.name
    }

    fn bind_pose(&self) -> [f32; 9] {
        self.pose
    }

    fn visit_parts(&self, f: &mut impl FnMut(&str, [f32; 9])) {
        f(&self.name, self.pose);
        for child in &self.children {
            child.visit_parts(f);
        }
    }
}

#[cfg(test)]
mod part_data_tests {
    use super::*;

    /// One mod's data on a part: a bend.
    #[derive(Debug, PartialEq)]
    struct Bend {
        axis: f32,
        angle: f32,
    }
    /// Another mod's, of a type of its own, on the same part.
    #[derive(Debug, PartialEq)]
    struct Stretch(Vec<f32>);

    /// Mods' values on a part are a change of the poses: every write gives a new `version`,
    /// so `Skin::draw` never reuses the part matrices of a walk before it; a copy keeps them
    /// and the version, and a reset for another draw drops them. Values of two types live
    /// side by side on one part.
    #[test]
    fn writing_a_parts_data_changes_the_version() {
        let leaf = |name: &str| TestPart {
            name: name.into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            children: Vec::new(),
        };
        let model = TestPart {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            children: vec![leaf("left_arm"), leaf("right_arm")],
        };
        let mut poses = Poses::new(&model);
        let start = poses.version();
        poses.set_data(
            "right_arm",
            Bend {
                axis: 0.0,
                angle: 0.5,
            },
        );
        let set = poses.version();
        assert_ne!(set, start);
        let bend = Bend {
            axis: 0.0,
            angle: 0.5,
        };
        assert_eq!(poses.data::<Bend>("right_arm"), Some(&bend));
        poses.set_data("right_arm", Stretch(vec![1.0, 2.0, 3.0, 4.0, 5.0]));
        assert_ne!(poses.version(), set, "a second type is a write too");
        assert_eq!(poses.data::<Bend>("right_arm"), Some(&bend));
        assert_eq!(
            poses.data::<Stretch>("right_arm").map(|s| s.0.len()),
            Some(5)
        );
        assert_eq!(poses.data::<Bend>("left_arm"), None);
        let mut copy = Poses::default();
        copy.copy_from(&poses);
        assert_eq!(copy.version(), poses.version());
        assert_eq!(copy.data::<Bend>("right_arm"), Some(&bend));
        // A part the model lacks has no slot: nothing stored, nothing changed.
        let before = poses.version();
        poses.set_data("tail", Stretch(Vec::new()));
        assert_eq!(
            (poses.data::<Stretch>("tail"), poses.version()),
            (None, before)
        );
        poses.clear_data::<Bend>("left_arm");
        assert_eq!(poses.version(), before, "nothing to clear");
        poses.clear_data::<Bend>("right_arm");
        assert_ne!(poses.version(), before);
        assert_eq!(poses.data::<Bend>("right_arm"), None);
        assert!(poses.data::<Stretch>("right_arm").is_some());
        assert_eq!(
            copy.data::<Bend>("right_arm"),
            Some(&bend),
            "the copy keeps its own"
        );
        copy.reset(&model);
        assert_eq!(copy.data::<Bend>("right_arm"), None);
        assert_eq!(copy.data::<Stretch>("right_arm"), None);
    }
}
