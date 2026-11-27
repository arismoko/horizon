use crate::game::ClientGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::{
    boats_common::Boat,
    collision_source::{Mover, boat, cart, cart_reach, player},
    commands_common::Target,
    entity::{EntityHandle, EntityLists},
    minecarts::Minecart,
    remote::passengers::passenger_root,
};
use sunrise_sim::entities::{ArmorStand, Cushion, ItemEntity, ItemFrame, Painting};
use sunrise_sim::join_seq::JoinSeq;
use sunrise_sim::mobs::Mob;

/// Identity and glowing state needed by the render occlusion culler.
pub(in crate::game) trait RenderEntity {
    fn render_id(&self) -> u32;
    #[cfg(feature = "client")]
    fn render_glowing(&self, game: &crate::game::ClientGame<'_>) -> bool {
        game.tracked_entity_glowing(self.render_id())
    }
}

/// Bounds and identity needed by the render occlusion culler. Moving renderers
/// use interpolated entity bounds; attached entities use their own world bounds.
pub(in crate::game) trait RenderEntityBounds: RenderEntity {
    fn render_bounds(&self, alpha: f64) -> Aabb;
}

impl RenderEntity for Mob {
    fn render_id(&self) -> u32 {
        self.id
    }

    #[cfg(feature = "client")]
    fn render_glowing(&self, _game: &crate::game::ClientGame<'_>) -> bool {
        self.is_currently_glowing()
    }
}

#[cfg(feature = "client")]
pub(in crate::game) fn mob_render_bounds(
    mob: &Mob,
    models: &sunrise_client::mobs_render::ClientMobModels,
    alpha: f64,
) -> Option<Aabb> {
    use sunrise_sim::mobs::Species;
    let visuals = models.get(&mob.kind.id)?;
    let (width, height, _) = mob.awake_dims();
    let at = mob.previous.lerp(mob.position, f64::from(alpha as f32));
    if matches!(
        mob.species(),
        Species::Guardian | Species::ElderGuardian | Species::Phantom
    ) {
        let root_scale = visuals.root_scale();
        let radius = width.max(height) * 2.5 + 3.0 * f64::from(root_scale * mob.model_scale());
        return Some(Aabb::of_size(
            at,
            Vec3::new(radius * 2.0, radius * 2.0, radius * 2.0),
        ));
    }

    let horizontal = (width * 2.0).max(height * 1.5).max(3.5);
    let vertical = height * 3.0 + 0.5;
    Some(Aabb::new(
        at.subtract(Vec3::new(horizontal, vertical, horizontal)),
        at.add_vector(Vec3::new(horizontal, vertical, horizontal)),
    ))
}

impl RenderEntity for ArmorStand {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for ArmorStand {
    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        let scale = if self.small { 0.5 } else { 1.0 };
        let equipped = self.equipment.iter().any(Option::is_some);
        let horizontal = if equipped { 2.0 } else { 1.5 * scale };
        let lower = if equipped { -1.0 } else { -0.5 * scale };
        let upper = 3.5 * scale
            + if self.equipment[5].is_some() {
                1.5
            } else {
                0.0
            };
        Aabb::new(
            at.subtract(Vec3::new(horizontal, -lower, horizontal)),
            at.add_vector(Vec3::new(horizontal, upper, horizontal)),
        )
    }
}

impl RenderEntity for ItemEntity {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for ItemEntity {
    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        Aabb::new(
            at.subtract(Vec3::new(0.75, 0.1, 0.75)),
            at.add_vector(Vec3::new(0.75, 1.0, 0.75)),
        )
    }
}

impl RenderEntity for Boat {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for Boat {
    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        Aabb::new(
            at.subtract(Vec3::new(1.75, 0.75, 1.75)),
            at.add_vector(Vec3::new(1.75, 1.5, 1.75)),
        )
    }
}

impl RenderEntity for Minecart {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for Minecart {
    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        let offset = self.custom_offset.map_or_else(
            || {
                if self.kind == super::minecarts::CartKind::Hopper {
                    1.0
                } else {
                    6.0
                }
            },
            f64::from,
        );
        let display_shift = 0.75 * (offset - 8.0) / 16.0;
        let display_low = at.y + 0.375 - 0.4 + display_shift;
        let display_high = at.y + 0.375 + 0.4 + display_shift + 0.9;
        Aabb::new(
            Vec3::new(at.x - 1.25, (at.y - 0.5).min(display_low), at.z - 1.25),
            Vec3::new(at.x + 1.25, (at.y + 1.0).max(display_high), at.z + 1.25),
        )
    }
}

fn aabb_from_extents(bounds: ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(bounds.0[0], bounds.0[1], bounds.0[2]),
        Vec3::new(bounds.1[0], bounds.1[1], bounds.1[2]),
    )
}

impl RenderEntity for Painting {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for Painting {
    fn render_bounds(&self, _alpha: f64) -> Aabb {
        aabb_from_extents(self.bounds())
    }
}

impl RenderEntity for ItemFrame {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for ItemFrame {
    fn render_bounds(&self, _alpha: f64) -> Aabb {
        let (lo, hi) = self.bounds();
        let step = self.step().map(f64::from);
        let center: [f64; 3] = std::array::from_fn(|axis| (lo[axis] + hi[axis]) * 0.5);
        // Include both maps and protruding held items in the occlusion envelope.
        let half: [f64; 3] = std::array::from_fn(|axis| if step[axis] != 0.0 { 0.75 } else { 0.5 });
        aabb_from_extents((
            std::array::from_fn(|axis| center[axis] - half[axis]),
            std::array::from_fn(|axis| center[axis] + half[axis]),
        ))
    }
}

impl RenderEntity for Cushion {
    fn render_id(&self) -> u32 {
        self.id
    }
}

impl RenderEntityBounds for Cushion {
    fn render_bounds(&self, _alpha: f64) -> Aabb {
        aabb_from_extents(self.bounds())
    }
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// The entities of the families `entity_geometry` draws in one ordered pass, as runs in the
    /// order they joined the level.
    pub(in crate::game) fn ordered_draw_runs(
        &self,
        draw_general: bool,
        draw_crystals: bool,
        runs: &mut Vec<DrawRun>,
    ) {
        let local_player = self
            .player_model_shown()
            .then(|| self.common.resident_entities.local_player.join());
        draw_runs(
            sunrise_gameplay_common::game::entity::entity_lists_of!(self),
            DrawExtras {
                local_player,
                dragon: self.common.dragon.as_ref().map(|dragon| dragon.join),
                crystals: if draw_crystals {
                    &self.common.crystals
                } else {
                    &[]
                },
                general: self
                    .client
                    .remote
                    .as_ref()
                    .filter(|_| draw_general)
                    .map(|r| &r.general),
            },
            runs,
        );
    }

    pub(in crate::game) fn client_player_equipped(
        &mut self,
        _slot: &str,
        _new: Option<&sunrise_items::items::ItemStack>,
    ) {
    }

    #[inline(always)]
    pub(in crate::game) fn client_game_side_fields(
        &self,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        self.game_side_client()
    }

    pub(in crate::game) fn client_finish_body_restitution(
        &mut self,
        _entity: (u32, &str),
        _feet: Vec3,
        response: sunrise_sim::motion::BodyRestitution,
    ) -> Vec3 {
        response.velocity
    }

    pub(in crate::game) fn client_body_step_on(
        &mut self,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &sunrise_world::ground_support::GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite> {
        let side = client_game_side!(self, self.game_side_client());
        self.common
            .body_motion(&side)
            .step_on(entity, footprint, ground, velocity);
        Vec::new()
    }

    pub(in crate::game) fn client_item_fall_on(&mut self, _id: u32, _on: BlockPos, _distance: f64) {
    }

    pub(in crate::game) fn client_item_post_move_contacts(
        &mut self,
        item: &mut ItemEntity,
        _client: bool,
        _sources: sunrise_gameplay_common::game::entity_common::InsideSources<'_>,
    ) -> bool {
        let _ = self.client_body_step_on(
            (item.id, "minecraft:item"),
            (item.position, item.bounds()),
            &item.ground,
            &mut item.velocity,
        );
        true
    }

    pub(in crate::game) fn client_boat_collision_root(
        &self,
        id: u32,
        _target: Target,
    ) -> Option<u32> {
        self.client
            .remote
            .as_ref()
            .and_then(|r| passenger_root(&r.passenger_vehicles, id).ok())
    }

    pub(in crate::game) fn client_vehicle_solids(
        &self,
        mover: Mover<'_>,
        solids: &sunrise_sim::entity_solids::Solids,
        each: &mut dyn FnMut(&sunrise_sim::entity_solids::Candidate, Option<usize>),
    ) {
        use sunrise_sim::entity_solids::{Candidate, SolidKind};
        match mover {
            Mover::Cart(c) => {
                for o in
                    self.common
                        .carts
                        .iter()
                        .filter(|o| o.id != c.id)
                        .map(|c| cart(|id, p, bounds| self.client_copy_box(id, p, bounds), c))
                        .chain(
                            self.common.boats.iter().map(|b| {
                                boat(|id, p, bounds| self.client_copy_box(id, p, bounds), b)
                            }),
                        )
                {
                    if solids.accepts(&o, || None) {
                        each(&o, None);
                    }
                }
                let reach = cart_reach(c);
                for (j, m) in self.common.mobs.iter().enumerate() {
                    if m.bounds().intersects(reach) {
                        let cand = m.solid_candidate();
                        if solids.accepts(&cand, || None) {
                            each(&cand, Some(j));
                        }
                    }
                }
                if self.common.riding != Some(c.id)
                    && let Some(p) = self.common.player.as_ref()
                    && !self.common.spectator()
                {
                    let p = player(p.bounds());
                    if solids.accepts(&p, || None) {
                        each(&p, None);
                    }
                }
            }
            Mover::Boat { id, area } => {
                for (j, m) in self.common.mobs.iter().enumerate() {
                    if !m.removed && m.bounds().intersects(area) {
                        let cand = m.solid_candidate();
                        if solids.accepts(&cand, || {
                            self.client_boat_collision_root(m.id, Target::Mob(m.id))
                        }) {
                            each(&cand, Some(j));
                        }
                    }
                }
                for (c, target) in self
                    .common
                    .boats
                    .iter()
                    .map(|b| {
                        (
                            boat(|id, p, bounds| self.client_copy_box(id, p, bounds), b),
                            Target::Boat(b.id),
                        )
                    })
                    .chain(self.common.carts.iter().map(|c| {
                        (
                            cart(|id, p, bounds| self.client_copy_box(id, p, bounds), c),
                            Target::Cart(c.id),
                        )
                    }))
                {
                    if solids.accepts(&c, || self.client_boat_collision_root(c.id, target)) {
                        each(&c, None);
                    }
                }
                if self.common.riding != Some(id)
                    && let Some(p) = self.common.player.as_ref()
                    && !self.common.spectator()
                    && !p.health.dead_or_dying()
                {
                    let pid = self
                        .client
                        .remote
                        .as_ref()
                        .map_or(p.entity_id as u32, |r| r.entity_id as u32);
                    let cand = Candidate::fixed(SolidKind::Player, pid, p.bounds());
                    if solids.accepts(&cand, || {
                        self.client_boat_collision_root(pid, Target::Player)
                    }) {
                        each(&cand, None);
                    }
                }
            }
            Mover::Player | Mover::PlayerAt { .. } | Mover::Passive => {}
        }
    }
}

/// The families `entity_geometry` draws in one ordered pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawFamily {
    Mob,
    LocalPlayer,
    RemoteGeneral,
    Dragon,
    Crystal,
    Item,
    Orb,
    Falling,
    Tnt,
    ArmorStand,
    Boat,
    Cart,
    Trident,
    Rocket,
}

/// Entities of one family, by list index, that joined back to back as far as the ordered
/// pass is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawRun {
    pub family: DrawFamily,
    pub(in crate::game) range: std::ops::Range<usize>,
}

impl DrawFamily {
    /// The drawn family of an entity and its index in that family's list; `None` for the
    /// entities the pass does not draw.
    fn of(handle: EntityHandle) -> Option<(Self, usize)> {
        use EntityHandle as H;
        Some(match handle {
            H::Mob(i) => (Self::Mob, i),
            H::Item(i) => (Self::Item, i),
            H::Orb(i) => (Self::Orb, i),
            H::Falling(i) => (Self::Falling, i),
            H::Tnt(i) => (Self::Tnt, i),
            H::ArmorStand(i) => (Self::ArmorStand, i),
            H::Boat(i) => (Self::Boat, i),
            H::Cart(i) => (Self::Cart, i),
            H::Trident(i) => (Self::Trident, i),
            H::Rocket(i) => (Self::Rocket, i),
            _ => return None,
        })
    }
}

/// Draw sources that the shared `EntityLists` walk does not own.
#[derive(Default)]
pub(in crate::game) struct DrawExtras<'a> {
    pub(in crate::game) local_player: Option<JoinSeq>,
    pub(in crate::game) dragon: Option<JoinSeq>,
    pub(in crate::game) crystals: &'a [sunrise_gameplay_common::game::dragon_common::Crystal],
    pub(in crate::game) general:
        Option<&'a crate::game::client_side::remote_client::general::RemoteGenerals>,
}

/// The drawn entities as runs in the order they joined the level. Keep the next draw-only
/// entity cached, so the existing body walk only compares its join with that one head.
pub(in crate::game) fn draw_runs(
    lists: EntityLists<'_>,
    mut extra: DrawExtras<'_>,
    runs: &mut Vec<DrawRun>,
) {
    use DrawFamily as F;
    enum Head {
        LocalPlayer,
        Dragon,
        Crystal(usize),
        General(usize),
    }
    let mut crystals = extra.crystals.iter().enumerate().peekable();
    let mut general = extra
        .general
        .into_iter()
        .flat_map(|g| g.rows.values())
        .enumerate()
        .peekable();
    let mut next_extra = || {
        let (join, head) = [
            extra.local_player.map(|seq| (seq, Head::LocalPlayer)),
            extra.dragon.map(|seq| (seq, Head::Dragon)),
            crystals.peek().map(|&(i, c)| (c.join, Head::Crystal(i))),
            general.peek().map(|&(i, row)| (row.join, Head::General(i))),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|&(seq, _)| seq)?;
        let (family, index) = match head {
            Head::LocalPlayer => {
                extra.local_player = None;
                (F::LocalPlayer, 0)
            }
            Head::Dragon => {
                extra.dragon = None;
                (F::Dragon, 0)
            }
            Head::Crystal(index) => {
                crystals.next();
                (F::Crystal, index)
            }
            Head::General(index) => {
                general.next();
                (F::RemoteGeneral, index)
            }
        };
        Some((join, family, index))
    };
    runs.clear();
    let mut push = |family, index| match runs.last_mut() {
        Some(run) if run.family == family && run.range.end == index => run.range.end += 1,
        _ => runs.push(DrawRun {
            family,
            range: index..index + 1,
        }),
    };
    let mut pending = next_extra();
    for (handle, seq) in lists.merge_in_join_order(|handle| DrawFamily::of(handle).is_some()) {
        while let Some((join, family, index)) = pending
            && join < seq
        {
            push(family, index);
            pending = next_extra();
        }
        if let Some((family, index)) = DrawFamily::of(handle) {
            push(family, index);
        }
    }
    while let Some((_, family, index)) = pending {
        push(family, index);
        pending = next_extra();
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
