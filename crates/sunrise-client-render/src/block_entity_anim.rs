//! Client-ticked block-entity animation state, as vanilla's block entities
//! keep it: `ChestLidController` (chests, trapped and copper chests, ender
//! chests), `EnchantingTableBlockEntity.bookAnimationTick`,
//! `ShulkerBoxBlockEntity` (open count, animation status, progress),
//! `BellBlockEntity` (ring ticks, shake direction, resonation),
//! `ConduitBlockEntity.clientTick` (tick count, shape check, rotation,
//! hunting, nautilus particles), `SkullBlockEntity.animation` (powered dragon
//! and piglin heads) and `DecoratedPotBlockEntity` wobbles.
//!
//! [`BlockEntityAnims`] holds the state per position and turns it into the
//! renderer's [`BlockAnim`]. Its tick methods take what the level answers
//! (blocks, entities, game time) as closures, so the parity fixture can drive
//! them with scripted inputs (`tools/test_block_entity_anim_parity.py`).
use crate::block_entity_render::BlockAnim;
use crate::fast_hash::FxHashMap;
use sunrise_core::{
    geometry::trig::{self, Mth},
    random::{LegacyRandom, RandomSource},
};

pub use sunrise_level::block_entity_ticks::{
    Bell, BellEntity, BellTick, BlockEntityTicks, ConduitCheck, ConduitServer, EndGateway, Ticker,
    closer_to_center_than, ticker,
};
pub use sunrise_level::shulker_box::{
    DIRECTIONS, Shulker, ShulkerStatus, ShulkerTick, direction_index, from_3d_data_value,
    progress_delta_aabb,
};

/// `ChestLidController`, shared by `ChestBlockEntity` (chests, trapped and
/// copper chests) and `EnderChestBlockEntity`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChestLid {
    pub should_be_open: bool,
    pub openness: f32,
    pub o_openness: f32,
}

impl ChestLid {
    /// `triggerEvent(1, openCount)`: the lid opens while anyone has it open.
    pub fn trigger_event(&mut self, b0: i32, b1: i32) -> bool {
        if b0 == 1 {
            self.should_be_open = b1 > 0;
            true
        } else {
            false
        }
    }
    /// `tickLid` (the client ticker `lidAnimateTick`).
    pub fn tick(&mut self) {
        self.o_openness = self.openness;
        if !self.should_be_open && self.openness > 0.0 {
            self.openness = (self.openness - 0.1).max(0.0);
        } else if self.should_be_open && self.openness < 1.0 {
            self.openness = (self.openness + 0.1).min(1.0);
        }
    }
    /// `getOpenness(a)`: `Mth.lerp(a, oOpenness, openness)`.
    pub fn get_openness(&self, a: f32) -> f32 {
        self.o_openness + a * (self.openness - self.o_openness)
    }
}

/// `ChestBlock.opennessCombiner`: a double chest opens as far as its more
/// open half (`Math.max(first, second)`, first being the `right` half).
pub fn combined_openness(first: f32, second: Option<f32>) -> f32 {
    match second {
        Some(second) => java_max(first, second),
        None => first,
    }
}

/// `Math.max(float, float)`.
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && a.to_bits() != b.to_bits() {
        0.0
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `ChestRenderer.submit` and `ChestModel.setupAnim`: the lid and lock
/// `xRot` for a render-state openness.
pub fn chest_lid_x_rot(open: f32) -> f32 {
    let open = 1.0 - open;
    let open = 1.0 - open * open * open;
    -(open * std::f32::consts::FRAC_PI_2)
}

/// `EnchantingTableBlockEntity` book fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnchantingBook {
    pub time: i32,
    pub flip: f32,
    pub o_flip: f32,
    pub flip_t: f32,
    pub flip_a: f32,
    pub open: f32,
    pub o_open: f32,
    pub rot: f32,
    pub o_rot: f32,
    pub t_rot: f32,
}

/// The `while (a >= PI) a -= 2PI; while (a < -PI) a += 2PI` loops.
fn wrap_pi(mut a: f32) -> f32 {
    use std::f32::consts::PI;
    while a >= PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// `Mth.frac(float)`.
pub fn mth_frac(v: f32) -> f32 {
    v - sunrise_core::mth::floor_f32(v) as f32
}

/// `EnchantTableRenderState` (`EnchantTableRenderer.extractRenderState`) and
/// the `BookModel.State` its `submit` builds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BookRender {
    pub flip: f32,
    pub open: f32,
    pub time: f32,
    pub y_rot: f32,
    /// `BookModel.State.forAnimation`: openness, pageFlip1, pageFlip2.
    pub openness: f32,
    pub page_flip1: f32,
    pub page_flip2: f32,
}

impl EnchantingBook {
    /// `bookAnimationTick`. `players` are the feet positions of the level's
    /// non-spectator players (`getNearestPlayer(centre, 3.0, false)`);
    /// `random` is the block entity class's shared static `RANDOM`.
    pub fn tick(&mut self, pos: [i32; 3], players: &[[f64; 3]], random: &mut impl RandomSource) {
        self.o_open = self.open;
        self.o_rot = self.rot;
        let c = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let mut best = -1.0;
        let mut nearest = None;
        for p in players {
            let (dx, dy, dz) = (p[0] - c[0], p[1] - c[1], p[2] - c[2]);
            let dist = dx * dx + dy * dy + dz * dz;
            if dist < 9.0 && (best == -1.0 || dist < best) {
                best = dist;
                nearest = Some(*p);
            }
        }
        if let Some(p) = nearest {
            let xd = p[0] - c[0];
            let zd = p[2] - c[2];
            self.t_rot = trig::atan2(zd, xd) as f32;
            self.open += 0.1;
            if self.open < 0.5 || random.next_int_bound(40).unwrap_or(1) == 0 {
                let old = self.flip_t;
                loop {
                    let a = random.next_int_bound(4).unwrap_or(0);
                    let b = random.next_int_bound(4).unwrap_or(0);
                    self.flip_t += (a - b) as f32;
                    if old != self.flip_t {
                        break;
                    }
                }
            }
        } else {
            self.t_rot += 0.02;
            self.open -= 0.1;
        }
        self.rot = wrap_pi(self.rot);
        self.t_rot = wrap_pi(self.t_rot);
        let dir = wrap_pi(self.t_rot - self.rot);
        self.rot += dir * 0.4;
        self.open = self.open.clamp(0.0, 1.0);
        self.time += 1;
        self.o_flip = self.flip;
        let diff = ((self.flip_t - self.flip) * 0.4).clamp(-0.2, 0.2);
        self.flip_a += (diff - self.flip_a) * 0.9;
        self.flip += self.flip_a;
    }
    /// `EnchantTableRenderer.extractRenderState` and the book state of its
    /// `submit`.
    pub fn render(&self, a: f32) -> BookRender {
        let flip = self.o_flip + a * (self.flip - self.o_flip);
        let open = self.o_open + a * (self.open - self.o_open);
        let time = self.time as f32 + a;
        let y_rot = self.o_rot + wrap_pi(self.rot - self.o_rot) * a;
        let ff1 = mth_frac(flip + 0.25) * 1.6 - 0.3;
        let ff2 = mth_frac(flip + 0.75) * 1.6 - 0.3;
        BookRender {
            flip,
            open,
            time,
            y_rot,
            openness: ((time * 0.02).msin() * 0.1 + 1.25) * open,
            page_flip1: ff1.clamp(0.0, 1.0),
            page_flip2: ff2.clamp(0.0, 1.0),
        }
    }
}

/// The conduit's `destroyTarget` as the client sees it.
#[derive(Clone, Copy, Debug)]
pub struct ConduitTarget {
    pub position: [f64; 3],
    pub eye_y: f64,
    pub width: f32,
    pub height: f32,
}

/// `ConduitBlockEntity` (client side).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conduit {
    pub tick_count: i32,
    pub active_rotation: f32,
    pub is_active: bool,
    pub is_hunting: bool,
    pub effect_blocks: Vec<[i32; 3]>,
}

/// One nautilus particle: position and velocity.
pub type Particle = ([f64; 3], [f64; 3]);

impl Conduit {
    /// `updateShape`: water all around, then the frame blocks
    /// (`BlockTags.CONDUIT_EFFECT_BLOCK`) in the three 5x5 rings.
    fn update_shape(
        &mut self,
        pos: [i32; 3],
        is_water: &impl Fn([i32; 3]) -> bool,
        is_frame: &impl Fn([i32; 3]) -> bool,
    ) -> bool {
        sunrise_level::block_entity_ticks::update_conduit_shape(
            &mut self.effect_blocks,
            pos,
            is_water,
            is_frame,
        )
    }
    /// `ConduitBlockEntity.clientTick`: returns the nautilus particles of
    /// `animationTick`, drawn from the client level's random.
    pub fn client_tick(
        &mut self,
        pos: [i32; 3],
        game_time: i64,
        is_water: impl Fn([i32; 3]) -> bool,
        is_frame: impl Fn([i32; 3]) -> bool,
        target: Option<ConduitTarget>,
        random: &mut impl RandomSource,
    ) -> Vec<Particle> {
        self.tick_count += 1;
        if game_time % 40 == 0 {
            self.is_active = self.update_shape(pos, &is_water, &is_frame);
            self.is_hunting = self.effect_blocks.len() >= 42;
        }
        let particles = self.animation_tick(pos, target, random);
        if self.is_active {
            self.active_rotation += 1.0;
        }
        particles
    }
    fn animation_tick(
        &self,
        pos: [i32; 3],
        target: Option<ConduitTarget>,
        random: &mut impl RandomSource,
    ) -> Vec<Particle> {
        let mut out = Vec::new();
        let hh = f64::from(((self.tick_count + 35) as f32 * 0.1).msin() / 2.0 + 0.5);
        let hh = (hh * hh + hh) * f64::from(0.3f32);
        let end = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.5 + hh,
            f64::from(pos[2]) + 0.5,
        ];
        for p in &self.effect_blocks {
            if random.next_int_bound(50).unwrap_or(1) == 0 {
                let dx = -0.5 + random.next_float() + (p[0] - pos[0]) as f32;
                let dy = -2.0 + random.next_float() + (p[1] - pos[1]) as f32;
                let dz = -0.5 + random.next_float() + (p[2] - pos[2]) as f32;
                out.push((end, [f64::from(dx), f64::from(dy), f64::from(dz)]));
            }
        }
        if let Some(t) = target {
            let randx = (-0.5 + random.next_float()) * (3.0 + t.width);
            let randy = -1.0 + random.next_float() * t.height;
            let randz = (-0.5 + random.next_float()) * (3.0 + t.width);
            out.push((
                [t.position[0], t.eye_y, t.position[2]],
                [f64::from(randx), f64::from(randy), f64::from(randz)],
            ));
        }
        out
    }
}

/// `SkullBlockEntity` animation (client ticker of dragon and piglin heads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Skull {
    pub animating: bool,
    pub tick_count: i32,
}
impl Skull {
    /// `SkullBlockEntity.animation`.
    pub fn tick(&mut self, powered: bool) {
        if powered {
            self.animating = true;
            self.tick_count += 1;
        } else {
            self.animating = false;
        }
    }
}

/// `DecoratedPotBlockEntity.WobbleStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WobbleStyle {
    Positive,
    Negative,
}
impl WobbleStyle {
    pub fn duration(self) -> i32 {
        match self {
            Self::Positive => 7,
            Self::Negative => 10,
        }
    }
}

/// `DecoratedPotBlockEntity` wobble fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PotWobble {
    pub started_at: i64,
    pub style: Option<WobbleStyle>,
}
impl PotWobble {
    /// `triggerEvent(1, style)`.
    pub fn trigger_event(&mut self, event: i32, data: i32, game_time: i64) -> bool {
        if event == 1 && (0..2).contains(&data) {
            self.started_at = game_time;
            self.style = Some(if data == 0 {
                WobbleStyle::Positive
            } else {
                WobbleStyle::Negative
            });
            true
        } else {
            false
        }
    }
    /// `DecoratedPotRenderer.extractRenderState`'s `wobbleProgress`.
    pub fn progress(&self, game_time: i64, partial: f32) -> f32 {
        match self.style {
            Some(style) => {
                ((game_time - self.started_at) as f32 + partial) / style.duration() as f32
            }
            None => 0.0,
        }
    }
}

/// The animated block entities of a level, by position.
#[derive(Clone, Debug)]
pub struct BlockEntityAnims {
    pub chests: FxHashMap<[i32; 3], ChestLid>,
    pub books: FxHashMap<[i32; 3], EnchantingBook>,
    /// `EnchantingTableBlockEntity.RANDOM`, shared by every table.
    pub book_random: LegacyRandom,
    pub shulkers: FxHashMap<[i32; 3], Shulker>,
    pub bells: FxHashMap<[i32; 3], Bell>,
    pub conduits: FxHashMap<[i32; 3], Conduit>,
    pub skulls: FxHashMap<[i32; 3], Skull>,
    pub pots: FxHashMap<[i32; 3], PotWobble>,
    pub gateways: FxHashMap<[i32; 3], EndGateway>,
    /// The client level's random (conduit particles).
    pub random: LegacyRandom,
    /// [`Ticker`] by block state id (see [`BlockEntityAnims::classify`]).
    pub kinds: Vec<Option<Ticker>>,
    /// The block entity object (its generation) each position's client animation state belongs
    /// to: a block entity made again there starts fresh, as vanilla's new object does.
    pub owners: FxHashMap<[i32; 3], u64>,
}
impl Default for BlockEntityAnims {
    fn default() -> Self {
        Self {
            chests: FxHashMap::default(),
            books: FxHashMap::default(),
            book_random: LegacyRandom::new(0x626f6f6b),
            shulkers: FxHashMap::default(),
            bells: FxHashMap::default(),
            conduits: FxHashMap::default(),
            skulls: FxHashMap::default(),
            pots: FxHashMap::default(),
            gateways: FxHashMap::default(),
            random: LegacyRandom::new(0x6c6576656c),
            kinds: Vec::new(),
            owners: FxHashMap::default(),
        }
    }
}
impl BlockEntityAnims {
    /// Gives the animation state at `pos` to the block entity object `generation`: the state of
    /// another object that was there is dropped.
    pub fn claim(&mut self, pos: [i32; 3], generation: u64) {
        if self
            .owners
            .insert(pos, generation)
            .is_some_and(|g| g != generation)
        {
            self.forget_client(pos);
        }
    }
    /// Drops the client animation state of every owner `alive` refuses, and of the decorated
    /// pots (which are no block entity here, so own nothing) whose block `state_at` is not one.
    pub fn retain_owned(
        &mut self,
        alive: impl Fn([i32; 3], u64) -> bool,
        state_at: impl Fn([i32; 3]) -> u32,
    ) {
        let Self {
            owners,
            chests,
            books,
            shulkers,
            bells,
            conduits,
            skulls,
            gateways,
            pots,
            kinds,
            ..
        } = self;
        owners.retain(|p, g| {
            let keep = alive(*p, *g);
            if !keep {
                chests.remove(p);
                books.remove(p);
                shulkers.remove(p);
                bells.remove(p);
                conduits.remove(p);
                skulls.remove(p);
                gateways.remove(p);
            }
            keep
        });
        pots.retain(|p, _| matches!(kinds.get(state_at(*p) as usize), Some(Some(Ticker::Pot))));
    }
    /// Drops the client animation state at `pos` (not the integrated server's).
    fn forget_client(&mut self, pos: [i32; 3]) {
        self.chests.remove(&pos);
        self.books.remove(&pos);
        self.shulkers.remove(&pos);
        self.bells.remove(&pos);
        self.conduits.remove(&pos);
        self.skulls.remove(&pos);
        self.gateways.remove(&pos);
    }
    /// Fills [`Self::kinds`] for every state of the registry.
    pub fn classify(&mut self, registry: &sunrise_world::registry::BlockRegistry) {
        self.kinds = sunrise_level::block_entity_ticks::classify_tickers(registry);
    }
    /// The ticker of a block state.
    pub fn ticker(&self, state: u32) -> Option<Ticker> {
        self.kinds.get(state as usize).copied().flatten()
    }
    /// Drops the state of block entities that are gone or changed kind:
    /// `present` holds the loaded block entities' tickers, sorted by position.
    pub fn retain_present(&mut self, present: &[([i32; 3], Ticker)]) {
        self.retain_animations(present, false);
    }
    /// An integrated server owns bell state independently of renderer visibility.
    /// Its world ticker removes bells when their actual block changes.
    pub fn retain_present_preserving_bells(&mut self, present: &[([i32; 3], Ticker)]) {
        self.retain_animations(present, true);
    }
    fn retain_animations(&mut self, present: &[([i32; 3], Ticker)], keep_bells: bool) {
        let is = |p: &[i32; 3], f: fn(&Ticker) -> bool| {
            present
                .binary_search_by_key(p, |(q, _)| *q)
                .is_ok_and(|i| f(&present[i].1))
        };
        self.owners
            .retain(|p, _| present.binary_search_by_key(p, |(q, _)| *q).is_ok());
        self.chests.retain(|p, _| is(p, |t| *t == Ticker::Lid));
        self.books.retain(|p, _| is(p, |t| *t == Ticker::Book));
        self.shulkers
            .retain(|p, _| is(p, |t| matches!(t, Ticker::Shulker { .. })));
        if !keep_bells {
            self.bells.retain(|p, _| is(p, |t| *t == Ticker::Bell));
        }
        self.conduits
            .retain(|p, _| is(p, |t| *t == Ticker::Conduit));
        self.skulls
            .retain(|p, _| is(p, |t| matches!(t, Ticker::Skull { .. })));
        self.pots.retain(|p, _| is(p, |t| *t == Ticker::Pot));
        self.gateways
            .retain(|p, _| is(p, |t| *t == Ticker::Gateway));
    }
    /// Drops the state of positions whose block entity is gone.
    /// Invalidate a moved holder without scanning all visible block entities.
    pub fn remove_position(&mut self, pos: [i32; 3]) {
        self.chests.remove(&pos);
        self.books.remove(&pos);
        self.shulkers.remove(&pos);
        self.bells.remove(&pos);
        self.conduits.remove(&pos);
        self.skulls.remove(&pos);
        self.pots.remove(&pos);
        self.gateways.remove(&pos);
    }
    pub fn retain(&mut self, keep: impl Fn([i32; 3]) -> bool) {
        self.chests.retain(|p, _| keep(*p));
        self.books.retain(|p, _| keep(*p));
        self.shulkers.retain(|p, _| keep(*p));
        self.bells.retain(|p, _| keep(*p));
        self.conduits.retain(|p, _| keep(*p));
        self.skulls.retain(|p, _| keep(*p));
        self.pots.retain(|p, _| keep(*p));
        self.gateways.retain(|p, _| keep(*p));
    }
    /// `getOpenNess(partial)` of the chest or ender chest at `pos`.
    pub fn lid(&self, pos: [i32; 3], partial: f32) -> f32 {
        self.chests
            .get(&pos)
            .map_or(0.0, |l| l.get_openness(partial))
    }
    /// The renderer's view of the block entity at `pos`; `sherds` are the
    /// pot's decorations (back, left, right, front), when it is a pot.
    pub fn block_anim(
        &self,
        pos: [i32; 3],
        game_time: i64,
        partial: f32,
        sherds: [Option<String>; 4],
    ) -> BlockAnim {
        let mut anim = BlockAnim {
            sherds,
            // Gateways not ticked yet render settled (no beam).
            gateway: self
                .gateways
                .get(&pos)
                .map_or((i64::MAX, 0), |g| (g.age, g.teleport_cooldown)),
            ..BlockAnim::default()
        };
        if let Some(b) = self.books.get(&pos) {
            anim.book = *b;
        }
        if let Some(s) = self.shulkers.get(&pos) {
            anim.shulker = (s.progress, s.progress_old);
        }
        if let Some(b) = self.bells.get(&pos) {
            anim.bell_ticks = b.ticks;
            anim.bell_shake = b
                .shaking
                .then_some(b.click_direction)
                .flatten()
                .map(|d| DIRECTIONS[usize::from(d)]);
        }
        if let Some(c) = self.conduits.get(&pos) {
            anim.conduit = (c.tick_count, c.is_active, c.is_hunting, c.active_rotation);
        }
        if let Some(s) = self.skulls.get(&pos) {
            anim.skull = (s.animating, s.tick_count);
        }
        if let Some(w) = self.pots.get(&pos)
            && w.style.is_some()
        {
            // extractRenderState never copies lastWobbleStyle into the render
            // state, so DecoratedPotRenderer.submit always takes the
            // non-POSITIVE branch (with the style's own duration).
            anim.wobble = Some((false, w.progress(game_time, partial)));
        }
        anim
    }
}
