//! Common evoker fangs insertion shared by server spawning/loading and remote client packets.
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{LegacyRandom, RandomSource};

#[derive(Clone, Debug)]
pub struct Fangs {
    /// Entity-owned registration metadata; content clones are detached.
    pub(in crate::game) attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) join: sunrise_sim::join_seq::JoinSeq,
    pub id: u32,
    pub pos: [f64; 3],
    /// `getDeltaMovement`: nothing moves the fangs by it, but a fluid's
    /// current changes it and the save writes it.
    pub motion: Vec3,
    pub yaw: f32,
    pub warmup: i32,
    pub life: i32,
    pub owner: u32,
    pub started: bool,
    /// `Entity.remainingFireTicks`, counted down by `baseTick`.
    pub fire_ticks: i32,
    /// `Entity.DATA_SHARED_FLAGS_ID` on-fire bit, set after `baseTick` fire processing.
    pub synced_fire: bool,
    /// `Entity.hasVisualFire` (`HasVisualFire` in NBT).
    pub has_visual_fire: bool,
    /// `Entity.random`. The attack sound's pitch is its first draw.
    pub random: LegacyRandom,
}

sunrise_sim::impl_joined!(Fangs);
sunrise_mod_runtime::impl_attachment_lifetime!(Fangs);

/// `EvokerFangs.handleEntityEvent`: `nextFloat() * 0.2 + 0.85`.
pub fn fang_attack_pitch(random: &mut impl RandomSource) -> f32 {
    random.next_float() * 0.2 + 0.85
}
#[derive(Default)]
pub struct FangState {
    pub list: sunrise_sim::join_seq::ResidentList<Fangs>,
}
impl FangState {
    pub fn perf_count(&self) -> usize {
        self.list.len()
    }
}

impl crate::game::CommonGame<'_> {
    pub fn insert_fangs(&mut self, id: u32, at: [f64; 3], yaw: f32, delay: i32, owner: u32) {
        self.fangs.list.push(Fangs {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            pos: at,
            motion: Vec3::ZERO,
            yaw,
            warmup: delay,
            life: 22,
            owner,
            started: false,
            fire_ticks: 0,
            synced_fire: false,
            has_visual_fire: false,
            random: sunrise_sim::mobs::entity_random(id, Vec3::new(at[0], at[1], at[2])),
        });
    }
}
