//! The per-tick mob update in vanilla order (`Entity.baseTick`,
//! `LivingEntity.baseTick/tick/aiStep`, `Mob.serverAiStep`) and the
//! dispatch to each species family's goals and special behaviour.
pub use freezing::{can_freeze, gaze_disguised, wears_gaze_disguise};
mod feeding;
mod flower_contact;
mod freezing;
use flower_contact::flower_contact_effects;
use std::ops::ControlFlow;

pub use feeding::takes_food;

use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who, aquatic,
    control::{BodyRotation, LookControl, MoveControl},
    farm,
    goals::{Cx, GoalSelector},
    hostile,
    nav::Navigation,
    path::{PATH_TYPES, Path},
    push::root_vehicle,
    undead,
};
use crate::blocks::Fluid;
use crate::breathing::{AirStep, Breath};
use crate::health::DamageLevel;
use sunrise_core::{
    geometry::{Aabb, Axis, ShapeOp, Vec3, VoxelShape},
    random::RandomSource,
};
use sunrise_world::native_load_context::{LoopContext, LoopSide};

#[cfg(test)]
mod inside_walk_tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    /// What the test's fire does, as a part's callback would: it queues a push
    /// and a wither effect, and can kill or panic on later contacts. The
    /// mod_api dispatch hop this stands for is the player's, the item's and the
    /// the projectile's tests' to prove; the mob tests here prove the walk, its
    /// arms, the flush and the containment.
    /// Each fixture installs the hook and resolves effects in its own process.
    static CONTACTS: Mutex<Vec<[i32; 3]>> = Mutex::new(Vec::new());
    static KILL_AFTER: AtomicUsize = AtomicUsize::new(usize::MAX);
    static PANIC_AFTER: AtomicUsize = AtomicUsize::new(usize::MAX);
    static ENABLED: AtomicBool = AtomicBool::new(false);
    std::thread_local! {
        static KILL_DAMAGE: std::cell::Cell<Option<mod_api::DamageTypeKey>> = const { std::cell::Cell::new(None) };
    }

    fn in_contact_test_process(test: &str) -> bool {
        const CHILD: &str = "SUNRISE_TEST_MOB_CONTACT_CHILD";
        let module = module_path!().split_once("::").unwrap().1;
        let name = format!("{module}::{test}");
        if std::env::var_os(CHILD).is_some_and(|value| value == name.as_str()) {
            return true;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([&name, "--exact", "--include-ignored", "--test-threads=1"])
            .env(CHILD, &name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{name} failed in its own process:\n{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        false
    }

    fn reset(kill: usize, panic: usize, enabled: bool) {
        mod_api::__private::resolve_effects(
            sunrise_game_data::potions::all_effects()
                .map(|def| sunrise_game_data::potions::namespaced(def.id)),
            |id| sunrise_game_data::potions::effect(id).map(|e| e.index),
            |_| true,
        )
        .unwrap();
        CONTACTS.lock().unwrap().clear();
        KILL_AFTER.store(kill, Ordering::SeqCst);
        PANIC_AFTER.store(panic, Ordering::SeqCst);
        ENABLED.store(enabled, Ordering::SeqCst);
    }

    /// What the fire's callback does, as a part's would.
    fn fire_like_a_part(
        at: sunrise_core::position::BlockPos,
        effects: &mut mod_api::InsideEffects,
    ) {
        let mut contacts = CONTACTS.lock().unwrap();
        contacts.push([at.x, at.y, at.z]);
        let seen = contacts.len();
        drop(contacts);
        if seen > PANIC_AFTER.load(Ordering::SeqCst) {
            panic!("an entity_inside part panics");
        }
        if seen > KILL_AFTER.load(Ordering::SeqCst) {
            effects.run_after(mod_api::InsideEffect::Freeze, |view| {
                view.hurt("minecraft:generic", 1000.0);
            });
        }
        effects.run_after(mod_api::InsideEffect::Freeze, |view| {
            view.push([0.5, 0.0, 0.0]);
        });
        let wither =
            mod_api::__private::effect_key("minecraft:wither").expect("an effect key resolves");
        effects.run_after(mod_api::InsideEffect::Freeze, move |view| {
            view.add_effect(wither, 40, 0);
        });
    }

    fn install_walk_hooks() {
        let _ = crate::block_contact::HOOKS.set(crate::block_contact::Hooks {
            step_listens: |_| false,
            step_on: |_, _, _| Vec::new(),
            any_inside: || ENABLED.load(Ordering::SeqCst),
            inside_listens: |_| ENABLED.load(Ordering::SeqCst),
            pass_listens: |_| false,
            fall_listens: |_| false,
            bounce_listens: |_| false,
            fall_on: |_, _, _, _, _, _| {},
            bounce_on: |_, _, _, _, _| {},
            // The production binding's own shape: each part's callback staged
            // into its own collector and absorbed only on normal return.
            inside_fire: |_level, _dimension, _entity, _kind, _state, at, _precise, effects| {
                effects.stage("mob-inside-walk-test", |staged| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        fire_like_a_part(at, staged);
                    }))
                    .is_ok()
                });
            },
            state_to_api: |_| None,
            state_to_engine: |_| None,
        });
    }

    /// The tests' mob: a zombie whose move crosses two stone blocks.
    fn mob_at_move(types: &super::super::MobTypes) -> Mob {
        let kind = Arc::new(types.types["minecraft:zombie"].clone());
        let mut mob = Mob::new(1, kind, Vec3::new(2.5, 64.0, 1.5), 0.0);
        mob.previous = Vec3::new(0.5, 64.0, 1.5);
        mob
    }

    /// The fire's pushes and its wither effect reach the mob in its own tick,
    /// one contact a block, in the shared walk's order.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_a_mob_once_a_block_in_walk_order() {
        if !in_contact_test_process("a_part_pushes_a_mob_once_a_block_in_walk_order") {
            return;
        }
        reset(usize::MAX, usize::MAX, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = mob.velocity.x;
                crate::block_contact::in_level(
                    crate::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                    },
                );
                assert_eq!(
                    *CONTACTS.lock().unwrap(),
                    vec![[1, 64, 1], [2, 64, 1]],
                    "one contact a block, in the walk's order"
                );
                assert!(
                    (mob.velocity.x - before - 1.0).abs() < 1e-9,
                    "both pushes reached the mob"
                );
                assert!(
                    mob.extra
                        .effects
                        .iter()
                        .any(|effect| effect.id == "minecraft:wither" && effect.duration == 40),
                    "the wither effect reached the mob's own list"
                );
                assert!(crate::block_contact::take_pending().is_empty());
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// A killing write stops the mob's flush: the walk still reaches both
    /// blocks, the writes after the death never apply.
    #[test]
    #[ignore = "daily"]
    fn a_killing_write_stops_the_mobs_flush() {
        if !in_contact_test_process("a_killing_write_stops_the_mobs_flush") {
            return;
        }
        let ran = crate::mobs::world::with_built_contact_test_world(
            |world, registry| {
                reset(0, usize::MAX, true);
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types, generic| {
                KILL_DAMAGE.set(Some(generic));
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = mob.velocity.x;
                crate::block_contact::in_level(
                    crate::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                    },
                );
                assert_eq!(CONTACTS.lock().unwrap().len(), 2);
                assert!(!mob.alive(), "the killing write applied");
                assert_eq!(
                    mob.velocity.x.to_bits(),
                    before.to_bits(),
                    "the writes after the death never apply"
                );
                assert!(!mob.has_effect("minecraft:wither"));
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// A panicking part is contained mid-walk: the mob's tick answers, its
    /// state keeps what it had, and the level host still answers after (the
    /// lifted rows themselves are #2285's restoration, its own tests'; the
    /// item batch test proves the lifted pair end to end).
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_mobs_tick_restored() {
        if !in_contact_test_process("a_panicking_part_leaves_the_mobs_tick_restored") {
            return;
        }
        reset(usize::MAX, 0, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = mob.velocity.x;
                crate::block_contact::in_level(
                    crate::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                    },
                );
                assert_eq!(
                    CONTACTS.lock().unwrap().len(),
                    2,
                    "both contacts reached the fire; each panic was contained"
                );
                assert!(mob.alive(), "the mob's tick answered");
                assert!(
                    (mob.velocity.x - before).abs() < 1e-9,
                    "the panicking callback's staged writes never applied"
                );
                assert!(
                    level.level().random().next_int_bound(2).is_ok(),
                    "the level host still answers"
                );
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// Without a listening part the walk runs its vanilla moments only:
    /// nothing of the part plumbing applies, exactly what main did with no
    /// listener.
    #[test]
    #[ignore = "daily"]
    fn a_mob_without_a_part_keeps_vanilla_behavior() {
        if !in_contact_test_process("a_mob_without_a_part_keeps_vanilla_behavior") {
            return;
        }
        reset(usize::MAX, usize::MAX, false);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in [1, 2] {
                    world.set(x, 64, 1, stone).unwrap();
                }
                install_walk_hooks();
            },
            |w, types| {
                let mut mob = mob_at_move(types);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let before = (mob.velocity.x, mob.health, mob.extra.effects.len());
                crate::block_contact::in_level(
                    crate::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                    },
                );
                assert_eq!(
                    (mob.velocity.x, mob.health, mob.extra.effects.len()),
                    before,
                    "no writes without a part"
                );
                assert!(CONTACTS.lock().unwrap().is_empty());
                assert!(crate::block_contact::take_pending().is_empty());
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// The walk's block order on a diagonal move is vanilla's own sweep: the
    /// x cells of the move come before the z cells the box enters.
    #[test]
    #[ignore = "daily"]
    fn a_diagonal_move_walks_blocks_in_vanillas_order() {
        if !in_contact_test_process("a_diagonal_move_walks_blocks_in_vanillas_order") {
            return;
        }
        reset(usize::MAX, usize::MAX, true);
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                // A diagonal path: +x crosses [1] and [2], then +z crosses the
                // cell ahead.
                world.set(1, 64, 1, stone).unwrap();
                world.set(2, 64, 1, stone).unwrap();
                world.set(2, 64, 2, stone).unwrap();
                install_walk_hooks();
            },
            |w, types| {
                let kind = Arc::new(types.types["minecraft:zombie"].clone());
                let mut mob = Mob::new(1, kind, Vec3::new(2.5, 64.0, 2.5), 0.0);
                mob.previous = Vec3::new(0.5, 64.0, 0.5);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                crate::block_contact::in_level(
                    crate::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        effects_from_blocks(&mut mob, w, &mut level.level(), &mut events);
                    },
                );
                let contacts = CONTACTS.lock().unwrap().clone();
                let x_first: Vec<_> = contacts.iter().filter(|p| p[2] == 1).collect();
                let z_last: Vec<_> = contacts.iter().filter(|p| p[2] == 2).collect();
                assert_eq!(
                    x_first.len(),
                    2,
                    "the x cells of the move come first: {contacts:?}"
                );
                assert!(
                    contacts.iter().position(|p| p[2] == 2)
                        > contacts.iter().rposition(|p| p[2] == 1),
                    "the z cell the box enters comes after the x cells: {contacts:?}"
                );
                let _ = z_last;
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }

    /// #478: a part's `AddEffect` write is `LivingEntity.addEffect` on a mob,
    /// so a second write updates the one instance instead of listing it
    /// twice. The stored modifier `addEffect` adds is covered by
    /// `effect_modifier_tests`, which read no registry.
    #[test]
    #[ignore = "daily"]
    fn a_parts_add_effect_write_merges_into_one_instance() {
        if !in_contact_test_process("a_parts_add_effect_write_merges_into_one_instance") {
            return;
        }
        reset(usize::MAX, usize::MAX, false);
        let mut mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        let mut level = crate::health::TestLevel::new(0);
        let wither = mod_api::__private::effect_key("minecraft:wither").expect("a resolved key");
        let ran = crate::mobs::world::with_test_world(&[], |w, _| {
            let mut events = Vec::new();
            let mut effects = mod_api::InsideEffects::default();
            effects.advance_step(0);
            effects.run_after(mod_api::InsideEffect::Freeze, move |view| {
                view.add_effect(wither, 40, 1);
            });
            effects.run_after(mod_api::InsideEffect::Freeze, move |view| {
                view.add_effect(wither, 40, 0);
            });
            let mut level_guard = level.level();
            let mut access = MobInsideAccess {
                m: &mut mob,
                w,
                level: &mut level_guard,
                events: &mut events,
            };
            effects.apply_and_clear(&mut access);
        });
        assert!(ran, "the vanilla test resources must be available");
        assert_eq!(
            mob.extra.effects,
            vec![crate::mobs::StoredEffect::new("minecraft:wither", 40, 1)]
        );
    }
}
#[cfg(test)]
#[path = "effect_modifier_tests.rs"]
pub(crate) mod effect_modifier_tests;
#[cfg(test)]
#[path = "fall_flying_tests.rs"]
mod fall_flying_tests;
#[path = "ai_glide.rs"]
mod glide;
use glide::update_fall_flying;
#[cfg(test)]
#[path = "cramming_tests.rs"]
mod cramming_tests;
#[cfg(test)]
#[path = "wet_fire_tests.rs"]
mod wet_fire_tests;

mod step;
pub(super) use step::inherited_step_sound;

use crate::powder_snow::particle_velocity as powder_snow_particle_velocity;

#[derive(Clone, Debug, Default)]
pub struct Ai {
    /// `Mob.homePosition` and `homeRadius`: where a restricted mob stays
    /// (a lead's holder, a happy ghast's float area).
    pub home: Option<([i32; 3], i32)>,
    /// Mob keeps homePosition even when clearHome makes homeRadius -1.
    /// Active homes still use `home`; this only retains its hidden position.
    last_home_position: [i32; 3],
    pub goals: GoalSelector,
    pub targets: GoalSelector,
    pub nav: Navigation,
    pub(super) navigation_redirect: Option<Box<super::nav::Redirect>>,
    pub mov: MoveControl,
    pub look: LookControl,
    pub body: BodyRotation,
    /// JumpControl.jump (requested this tick).
    pub jump: bool,
    /// LivingEntity.jumping
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub xxa: f32,
    pub yya: f32,
    pub zza: f32,
    pub speed: f32,
    pub target: Option<Who>,
    pub last_hurt_by: Option<Who>,
    pub last_hurt_by_time: i32,
    /// The bare UUID of an unresolved `lastHurtByMob` (see `hurt_by`).
    pub last_hurt_by_ref: Option<[i32; 4]>,
    /// Ticks left during which kills count as player kills.
    pub last_hurt_by_player: i32,
    pub no_action_time: i32,
    pub ai_paused: bool, // A mod paused serverAiStep (`MobRef::set_ai_paused`): mobs/ai_pause.rs.
    pub aggressive: bool,
    pub panicking: bool,
    pub malus: [Option<f32>; PATH_TYPES],
    /// A path computed in canUse and consumed by start.
    pub pending_path: Option<Path>,
    /// `OwnerHurtTargetGoal.timestamp`: the owner's `getLastHurtMobTimestamp` the goal
    /// last started on. On the mob, not the goal, so the mob tick can tell that a pet
    /// has nothing new to read without building the level's player views.
    pub owner_hurt_timestamp: i32,
    /// `OwnerHurtByTargetGoal.timestamp`: the owner's `getLastHurtByMobTimestamp` the
    /// goal last started on. On the mob, not the goal, so the mob tick can tell that a
    /// pet has nothing new to read without building the level's player views.
    pub owner_hurt_by_timestamp: i64,
    /// Sensing: line-of-sight results cached for one tick.
    pub seen_cache: Vec<(Who, bool)>,
    /// Swing animation progress (attackAnim) and ticks.
    pub swing_time: i32,
    pub swinging: bool,
    /// The current swing's `SwingDescription.durationTicks`; 0 is the default 6.
    pub swing_duration: i32,
    /// Walked distance and the next footstep.
    pub steps: super::living::Steps,
}
impl Ai {
    /// `Mob.getHomePosition`, including the position retained by clearHome.
    pub fn home_position(&self) -> [i32; 3] {
        self.home.map_or(self.last_home_position, |(pos, _)| pos)
    }
    /// Adopt only the two persistent home fields after loading Mob data.
    pub fn restore_home(&mut self, pos: [i32; 3], radius: i32) {
        self.last_home_position = pos;
        self.home = (radius != -1).then_some((pos, radius));
    }
    /// `Mob.clearHome`: clear the radius, retaining the position.
    pub fn clear_home(&mut self) {
        self.last_home_position = self.home_position();
        self.home = None;
    }
    /// `Mob.isWithinRestriction(pos)`: an unrestricted mob is within
    /// everywhere.
    pub fn is_within_home(&self, p: [i32; 3]) -> bool {
        self.home
            .is_none_or(|(c, r)| super::goals::within_home(c, r, p))
    }
}
impl Mob {
    /// `LivingEntity.getCurrentSwingDuration`: 6 ticks unless a swing set its own.
    pub fn swing_duration(&self) -> i32 {
        if self.ai.swing_duration > 0 {
            self.ai.swing_duration
        } else {
            6
        }
    }
    /// `SwingState.startIfAble`: a swing past the first half of the current
    /// one restarts it.
    fn start_swing(&mut self, duration: i32) -> bool {
        let half = self.swing_duration() / 2;
        if self.ai.swinging && self.ai.swing_time >= 0 && self.ai.swing_time < half {
            return false;
        }
        self.ai.swing_time = -1;
        self.ai.swinging = true;
        self.ai.swing_duration = duration.max(1);
        true
    }
    /// Whether the current swing is the off hand's (`swing_hand`'s arm
    /// inverted); a swing that named no hand is the main hand's.
    pub fn swing_is_off_hand(&self) -> bool {
        self.extra
            .swing_right_arm
            .is_some_and(|right| right == self.main_arm_left())
    }
    /// `LivingEntity.isSwinging`: a swing is under way, whichever hand it names. The local
    /// player's model mirrors its swing through `swing_right_arm` only.
    pub fn is_swinging(&self) -> bool {
        self.ai.swinging || self.extra.swing_right_arm.is_some()
    }
    /// LivingEntity.swing
    pub fn swing(&mut self) {
        if self.start_swing(6) {
            self.extra.swing_right_arm = None;
            self.extra.swing_stab = false;
        }
    }
    /// `LivingEntity.swing(hand, animation, false)` for a swing of `duration`
    /// ticks (`getModifiedSwingDuration`): the arm is `hand.asArm(mainArm)`.
    pub fn swing_hand(&mut self, off_hand: bool, duration: i32) {
        if self.start_swing(duration) {
            self.extra.swing_right_arm = Some(!off_hand != self.main_arm_left());
            self.extra.swing_stab = false;
        }
    }
    /// `swing_hand` for a swing of `animation`, lengthened or shortened by
    /// this mob's haste, conduit power or mining fatigue
    /// (`LivingEntity.getModifiedSwingDuration`). Whether the swing started
    /// (`SwingState.startIfAble`).
    pub fn swing_hand_modified(
        &mut self,
        off_hand: bool,
        animation: crate::first_person_sim::SwingAnimation,
    ) -> bool {
        let amplifier = |id: &str| {
            self.extra
                .effects
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.amplifier)
        };
        let dig_speed = amplifier("minecraft:haste").max(amplifier("minecraft:conduit_power"));
        let fatigue = amplifier("minecraft:mining_fatigue");
        let duration = crate::first_person_sim::swing_duration(animation, dig_speed, fatigue);
        if self.start_swing(duration) {
            self.extra.swing_right_arm = Some(!off_hand != self.main_arm_left());
            self.extra.swing_stab = animation.kind == 2;
            return true;
        }
        false
    }
}
/// The module implementing a species' behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Farm,
    Undead,
    Hostile,
    Aquatic,
    Generic,
    /// No goals at all (Giant).
    Inert,
    Witch,
    Enderman,
    Tame,
    /// Blazes and ghasts.
    Nether,
    Golem,
    Illager,
    /// Piglins, brutes, hoglins and zoglins.
    Piglin,
    Equine,
    Phantom,
    /// Slimes and magma cubes.
    Cube,
    Guardian,
    Shulker,
    Vex,
    Ravager,
    Wither,
    Allay,
    Villager,
    Trader,
    Breeze,
    Sniffer,
    HappyGhast,
    Warden,
    Creaking,
    /// Nautili and zombie nautili.
    Nautilus,
    Strider,
    Bat,
    Bee,
    Panda,
    PolarBear,
}
pub fn family(s: Species) -> Family {
    use Species as S;
    match s {
        S::Pig | S::Cow | S::Mooshroom | S::Sheep | S::Chicken => Family::Farm,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::Parched
        | S::WitherSkeleton
        | S::ZombifiedPiglin => Family::Undead,
        S::Creeper | S::Spider | S::CaveSpider | S::Silverfish | S::Endermite => Family::Hostile,
        S::Squid
        | S::GlowSquid
        | S::Cod
        | S::Tadpole
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Dolphin
        | S::Axolotl => Family::Aquatic,
        S::Giant => Family::Inert,
        S::Witch => Family::Witch,
        S::Wolf | S::Cat => Family::Tame,
        S::Blaze | S::Ghast => Family::Nether,
        S::IronGolem | S::SnowGolem | S::CopperGolem => Family::Golem,
        S::Pillager | S::Vindicator | S::Evoker | S::Illusioner => Family::Illager,
        S::Vex => Family::Vex,
        S::Ravager => Family::Ravager,
        S::Wither => Family::Wither,
        S::Allay => Family::Allay,
        S::Creaking => Family::Creaking,
        S::Villager => Family::Villager,
        S::WanderingTrader => Family::Trader,
        S::Breeze => Family::Breeze,
        S::Sniffer => Family::Sniffer,
        S::HappyGhast => Family::HappyGhast,
        S::Piglin | S::PiglinBrute | S::Hoglin | S::Zoglin => Family::Piglin,
        S::Horse
        | S::Donkey
        | S::Mule
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk
        | S::Llama
        | S::TraderLlama => Family::Equine,
        S::Nautilus | S::ZombieNautilus => Family::Nautilus,
        S::Phantom => Family::Phantom,
        S::Enderman => Family::Enderman,
        S::Slime | S::MagmaCube | S::SulfurCube => Family::Cube,
        S::Guardian | S::ElderGuardian => Family::Guardian,
        S::Warden => Family::Warden,
        S::Shulker => Family::Shulker,
        S::Strider => Family::Strider,
        S::Bat => Family::Bat,
        S::Bee => Family::Bee,
        S::Panda => Family::Panda,
        S::PolarBear => Family::PolarBear,
        _ => Family::Generic,
    }
}
/// The `setPathfindingMalus` calls of the vanilla constructors, base classes
/// first: the species' own register may still change them.
fn constructor_maluses(m: &mut Mob) {
    use super::path::PathType as P;
    use Species as S;
    let s = m.species();
    // `Animal()`: no walking through fire, and a wide berth around it.
    let animal = matches!(
        s,
        S::Armadillo
            | S::Axolotl
            | S::Bee
            | S::Camel
            | S::CamelHusk
            | S::Cat
            | S::Chicken
            | S::Cow
            | S::Donkey
            | S::Fox
            | S::Frog
            | S::Goat
            | S::HappyGhast
            | S::Hoglin
            | S::Horse
            | S::Llama
            | S::Mule
            | S::Mooshroom
            | S::Nautilus
            | S::Ocelot
            | S::Panda
            | S::Parrot
            | S::Pig
            | S::PolarBear
            | S::Rabbit
            | S::Sheep
            | S::SkeletonHorse
            | S::Sniffer
            | S::Strider
            | S::TraderLlama
            | S::Turtle
            | S::Wolf
            | S::ZombieHorse
            | S::ZombieNautilus
    );
    // `AbstractVillager()`, `AbstractPiglin()` and `CopperGolem()` say the same.
    if animal
        || matches!(
            s,
            S::Villager | S::WanderingTrader | S::Piglin | S::PiglinBrute | S::CopperGolem
        )
    {
        m.set_malus(P::FireInNeighbor, 16.0);
        m.set_malus(P::Fire, -1.0);
    }
    match s {
        S::Fox => {
            m.set_malus(P::DamagingInNeighbor, 0.0);
            m.set_malus(P::Damaging, 0.0);
        }
        S::Parrot => {
            m.set_malus(P::FireInNeighbor, -1.0);
            m.set_malus(P::Fire, -1.0);
            m.set_malus(P::Cocoa, -1.0);
        }
        S::Frog => {
            m.set_malus(P::Water, 4.0);
            m.set_malus(P::Trapdoor, -1.0);
        }
        S::Goat | S::Wolf => {
            m.set_malus(P::PowderSnow, -1.0);
            m.set_malus(P::OnTopOfPowderSnow, -1.0);
        }
        S::CopperGolem => m.set_malus(P::DamagingInNeighbor, 16.0),
        S::Blaze => {
            m.set_malus(P::Water, -1.0);
            m.set_malus(P::Lava, 8.0);
            m.set_malus(P::FireInNeighbor, 0.0);
            m.set_malus(P::Fire, 0.0);
        }
        S::ZombifiedPiglin | S::WitherSkeleton => m.set_malus(P::Lava, 8.0),
        _ => {}
    }
}
/// Brain-backed vanilla `getTarget` overrides. Piglins and hoglins use
/// Sunrise's `ai.target` as the stand-in for their brain memory.
pub fn brain_target(mob: &Mob) -> Option<Option<Who>> {
    Some(match mob.species() {
        Species::Warden => super::warden::brain_target(mob),
        Species::Breeze | Species::Creaking => super::monster_brain::attack_target(mob),
        Species::Axolotl => mob
            .extra
            .animal_brain
            .as_ref()
            .and_then(|b| b.mem.attack_target.value()),
        Species::Frog => super::frog::attack_target(mob),
        Species::Piglin | Species::PiglinBrute | Species::Hoglin | Species::Zoglin => mob.ai.target,
        _ => return None,
    })
}

/// Whether `Brain.setMemory(ATTACK_TARGET, ...)` reaches what the mob's
/// brain acts on (#934): every species [`brain_target`] reads but the frog,
/// whose target belongs to its running tongue attack.
pub fn writes_brain_target(mob: &Mob) -> bool {
    match mob.species() {
        Species::Frog => false,
        Species::Axolotl => mob.extra.animal_brain.is_some(),
        _ => brain_target(mob).is_some(),
    }
}

/// `Brain.setMemory(ATTACK_TARGET, target)` / `eraseMemory` (#934): the
/// memory each brain keeps it in, non-expiring, with `ai.target` mirrored
/// where the brain mirrors it. No species setter side effects. False when
/// [`writes_brain_target`] is false.
pub fn set_brain_target(mob: &mut Mob, target: Option<Who>, player_id: Option<u32>) -> bool {
    if !writes_brain_target(mob) {
        return false;
    }
    match mob.species() {
        Species::Warden => super::warden_brain::mem(mob).set_target(target, player_id),
        Species::Breeze | Species::Creaking => {
            super::monster_brain::set_attack_target(mob, target);
        }
        Species::Axolotl => {
            super::animal_brain::mem(mob).attack_target.set_opt(target);
            mob.ai.target = target;
        }
        // The piglin, piglin brute, hoglin and zoglin brains keep it in the
        // mob's own target slot (what `brain_target` reads).
        _ => mob.ai.target = target,
    }
    true
}

pub fn register(m: &mut Mob) {
    constructor_maluses(m);
    // Raider subclasses call super.registerGoals() first.
    super::raider::register(m);
    match family(m.species()) {
        Family::Farm => farm::register(m),
        Family::Undead => undead::register(m),
        Family::Hostile => hostile::register(m),
        Family::Aquatic => aquatic::register(m),
        // Turtle.registerGoals has its own list, none of the generic goals.
        Family::Generic if m.species() == Species::Turtle => super::turtle::register(m),
        Family::Generic if m.species() == Species::Armadillo => super::armadillo::register(m),
        Family::Generic if m.species() == Species::Frog => super::frog::register(m),
        Family::Generic if m.species() == Species::Fox => super::fox::register(m),
        Family::Generic if m.species() == Species::Parrot => super::parrot::register(m),
        Family::Generic if m.species() == Species::Rabbit => super::rabbit::register(m),
        Family::Generic if m.species() == Species::Ocelot => super::tame::register_ocelot(m),
        Family::Generic => {
            generic(m);
            if m.species() == Species::Frog {
                // `Frog()`: SmoothSwimmingMoveControl(85, 10, 0.02, 0.1, true).
                // The speed attribute is 1.0, so the controller's scaling is
                // what keeps a frog at a walking pace on land.
                m.ai.mov.kind = super::control::MoveKind::SmoothSwimming {
                    max_turn_x: 85.0,
                    max_turn_y: 10.0,
                    in_water: 0.02,
                    outside: 0.1,
                    gravity: true,
                };
            }
            if m.species() == Species::Goat {
                super::goat::register(m);
            }
        }
        Family::Inert => {}
        Family::Witch => super::witch::register(m),
        Family::Tame if m.species() == Species::Cat => super::tame::register_cat(m),
        Family::Tame => super::tame::register(m),
        Family::Nether if m.species() == Species::Ghast => super::nether::register_ghast(m),
        Family::Nether => super::nether::register_blaze(m),
        Family::Golem if m.species() == Species::IronGolem => super::golem::register_iron(m),
        Family::Golem if m.species() == Species::CopperGolem => super::copper_golem::register(m),
        Family::Golem => super::golem::register_snow(m),
        Family::Illager if m.species() == Species::Pillager => super::illager::register_pillager(m),
        Family::Illager if m.species() == Species::Evoker => super::illager::register_evoker(m),
        Family::Illager if m.species() == Species::Illusioner => super::illusioner::register(m),
        Family::Illager => super::illager::register_vindicator(m),
        Family::Equine => super::equine::register(m),
        Family::Phantom => super::phantom::register(m),
        Family::Piglin => match m.species() {
            Species::Piglin => super::piglin::register_piglin(m),
            Species::PiglinBrute => super::piglin::register_brute(m),
            Species::Hoglin => super::piglin::register_hoglin(m),
            _ => super::piglin::register_zoglin(m),
        },
        Family::Enderman => super::enderman::register(m),
        Family::Cube => super::cube::register(m),
        Family::Guardian => super::guardian::register(m),
        Family::Shulker => super::shulker::register(m),
        Family::Vex => super::vex::register(m),
        Family::Ravager => super::ravager::register(m),
        Family::Wither => super::wither::register(m),
        Family::Allay => super::allay::register(m),
        Family::Villager => super::villager::register(m),
        Family::Trader => super::trader::register(m),
        Family::Breeze => super::breeze::register(m),
        Family::Sniffer => super::sniffer::register(m),
        Family::HappyGhast => super::happy_ghast::register(m),
        Family::Warden => super::warden::register(m),
        Family::Creaking => super::creaking::register(m),
        Family::Nautilus => super::nautilus::register(m),
        Family::Strider => super::strider::register(m),
        Family::Bat => super::bat::register(m),
        Family::Bee => super::bee::register(m),
        Family::Panda => super::panda::register(m),
        Family::PolarBear => super::polar_bear::register(m),
    }
    // FloatGoal's constructor lets the navigation float, as do the
    // constructors of these brain-driven and flying mobs. A floater rises
    // 0.04 a tick in Mob.jumpInLiquid, the others 0.3 (the bounce of #542).
    if m.ai.goals.has("Float")
        || matches!(
            m.species(),
            Species::Allay
                | Species::Armadillo
                | Species::Camel
                | Species::Goat
                | Species::Parrot
                | Species::Wither
        )
    {
        m.ai.nav.can_float = true;
    }
}
/// Mob.tick additions that run before the entity tick (cube/creeper fuses, puffing).
fn pre_tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::SulfurCube {
        // SulfurCube.tick calls tickFuse before super.tick, outside serverAiStep.
        super::sulfur_cube::tick_fuse(m, events);
    }
    if m.species() == Species::Wither && m.alive() {
        super::wither::steer(m);
    }
    if m.species() == Species::Breeze {
        super::breeze::tick(m, w, events);
    }
    match family(m.species()) {
        Family::Hostile => {
            if m.species() == Species::Creeper {
                hostile::creeper_tick(m, events)
            }
        }
        Family::Aquatic => aquatic::tick(m, events),
        _ => {}
    }
}
/// Move controls implemented by the species (MoveKind::Custom).
pub fn custom_move_control(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Drowned {
        return undead::drowned_move_control(m, w);
    }
    if m.species() == Species::Ghast {
        return super::nether::ghast_move_control(m, w);
    }
    if m.species() == Species::HappyGhast {
        return super::happy_ghast::move_control(m, w);
    }
    if m.species() == Species::Phantom {
        return super::phantom::move_control(m);
    }
    if family(m.species()) == Family::Guardian {
        return super::guardian::move_control(m);
    }
    if m.species() == Species::Vex {
        return super::vex::move_control(m);
    }
    match family(m.species()) {
        Family::Aquatic => aquatic::fish_move_control(m),
        Family::Cube => super::cube::move_control(m, events),
        _ => {}
    }
}
/// A fallback for species without bespoke goals yet: animals stroll and
/// panic, monsters hunt the player in melee.
fn generic(m: &mut Mob) {
    use super::goals::*;
    let hostile = m.hostile();
    let g = &mut m.ai.goals;
    if hostile {
        g.add(1, Float);
        g.add(2, MeleeAttack::new(1.0, false));
        g.add(5, Stroll::avoiding_water(0.8));
        g.add(6, LookAtPlayer::new(8.0));
        g.add(6, RandomLookAround::default());
        m.ai.targets.add(1, HurtByTarget::new());
        m.ai.targets.add(2, NearestTarget::player(true));
    } else {
        if m.kind.species == Species::Goat {
            // GoatAi's idle and core behaviours (the ram and long jump
            // activities are not here): panic 2.0, AnimalMakeLove, tempting
            // with goat_food and baby follow at 1.25, then stroll.
            g.add(0, Float);
            g.add(1, Panic::new(2.0));
            g.add(2, Breed::new(1.0));
            g.add(3, Tempt::new(1.25, m.kind.food.clone(), false));
            g.add(4, FollowParent::new(1.25));
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(6.0));
            g.add(7, RandomLookAround::default());
            return;
        }
        g.add(0, Float);
        g.add(1, Panic::new(1.25));
        g.add(5, Stroll::avoiding_water(1.0));
        g.add(6, LookAtPlayer::new(6.0));
        g.add(7, RandomLookAround::default());
    }
}
/// Subclasses of `net.minecraft.world.entity.monster.Monster`.
pub fn is_monster(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Zombie
            | S::Husk
            | S::Drowned
            | S::ZombieVillager
            | S::ZombifiedPiglin
            | S::Giant
            | S::Skeleton
            | S::Stray
            | S::Bogged
            | S::Parched
            | S::WitherSkeleton
            | S::Creeper
            | S::Spider
            | S::CaveSpider
            | S::Enderman
            | S::Endermite
            | S::Silverfish
            | S::Witch
            | S::Pillager
            | S::Vindicator
            | S::Evoker
            | S::Illusioner
            | S::Ravager
            | S::Vex
            | S::Piglin
            | S::PiglinBrute
            | S::Zoglin
            | S::Blaze
            | S::Guardian
            | S::ElderGuardian
            | S::Breeze
            | S::Creaking
            | S::Warden
            | S::Wither
    )
}
/// One full entity tick. Returns false when the mob is gone.
/// `Mob.getTarget` / `Mob.setTarget` (`asValidTarget`): a player in creative
/// or spectator mode or in Peaceful is never a mob's target, however the target was set (an
/// alert from another mob included). A `NeutralMob` angry at that player stops
/// being angry (`NeutralMob.updatePersistentAnger`).
fn drop_invalid_player_target(m: &mut Mob, w: &MobWorld) {
    if m.ai.target == Some(Who::Player)
        && w.selected_player()
            .is_some_and(|p| p.creative || p.spectator || w.difficulty.base == 0)
    {
        m.set_ai_target(w, None);
        if matches!(m.species(), Species::ZombifiedPiglin | Species::IronGolem) {
            super::neutral_anger::stop(m);
            m.ai.forget_hurt_by(m.ai.last_hurt_by_time);
        }
    }
}
/// A tick either completed or paused where its owner must provide a mutable host.
#[derive(Debug)]
pub enum TickProgress {
    Complete(bool),
    Paused(Pause),
}

/// Which half of `LivingEntity.aiStep` to continue after an inside-block
/// collector flush pause.
#[derive(Clone, Copy, Debug)]
pub enum InsideResume {
    /// NoAI mob: continue after `effects_from_blocks` (`freeze_tick` -> `push_entities` -> NoAI steps -> `finish_tick`).
    NoAi,
    /// AI mob: continue after `effects_from_blocks` (`freeze_tick` -> `push_entities` -> AI post-steps -> `finish_tick`).
    AiStep,
}

/// A mob's collected `InsideEffects`, paused so the owning `Game` can flush
/// them with a live level loan before freezing and the rest of `aiStep`.
#[derive(Debug)]
pub struct InsidePause {
    pub mob: u32,
    pub join: crate::join_seq::JoinSeq,
    pub effects: mod_api::InsideEffects,
    temperature: InsideTemperature,
    pub resume: InsideResume,
}

/// A point where the tick needs the owning game before it can continue.
#[derive(Debug)]
pub enum Pause {
    Consumption {
        decision: super::fox::Consumption,
    },
    Drink {
        stack: crate::items::ItemStack,
        resume: DrinkResume,
    },
    Breathing {
        air: AirPause,
        /// `Entity.computeSpeed`, read before the tick moved the mob.
        known_speed: Vec3,
    },
    /// The server found changed equipment before the living AI step.
    Equipment {
        first: sunrise_core::equipment_slot::EquipmentSlot,
    },
    /// A breeding goal or brain behaviour finished its decision. Deliver the action before
    /// resuming any later goal, movement or age and love ticks.
    Breeding {
        event: Option<Box<MobEvent>>,
        resume: BreedingResume,
    },
    /// A hit needs its owning level before the tick goes on.
    AfterDamage(DamageResume),
    /// A landing's farmland decision, before its superclass hurt.
    Trample(sunrise_world::block_landing::FarmlandTrample),
    /// A vehicle must propagate the selected fall before this move continues.
    FallToPassengers(super::living::PassengerFall),
    /// A brain was constructed or rebuilt during the tick and needs the owning
    /// game to fire `register_behaviors` with a scoped `ServerHost`.
    RegisterBehaviors {
        stage: BrainStage,
    },
    /// The mob's `InsideEffects` collector is ready to flush against the live level.
    InsideEffects(InsidePause),
}

/// Where a mob tick paused for staged `register_behaviors`.
#[derive(Clone, Copy, Debug)]
pub enum BrainStage {
    /// Brain was just built or rebuilt before running its brain tick.
    BeforeBrain,
    /// Villager brain refreshed itself during `villager::tick_brain`.
    AfterVillagerBrain,
    /// A brain was rebuilt during `AgeableMob.aiStep` (`age_boundary_reached`).
    AfterAgeBoundary { no_ai: bool, after_water: bool },
}

/// The remainder of the mob tick after a drink finishes on its owning level.
#[derive(Debug)]
pub enum DrinkResume {
    Trader,
    Witch,
}

#[derive(Debug)]
pub struct BreedingResume {
    phase: BreedingPhase,
}

#[derive(Debug)]
enum BreedingPhase {
    Goal(super::goals::GoalBreed),
    Brain(super::animal_brain::BrainBreed),
}

#[derive(Debug)]
pub struct DamageResume {
    join: crate::join_seq::JoinSeq,
    tail: DamageTail,
}

#[derive(Debug)]
enum DamageTail {
    Base {
        next: BaseStage,
        known_speed: Vec3,
    },
    Ai {
        next: AiStage,
        no_ai: bool,
    },
    BeeServerAi {
        next: super::bee::ServerAiStage,
    },
    /// The caller's movement slot holds the exact travel or goal stage.
    Movement,
}

#[derive(Debug)]
enum AiStage {
    BlockWalk,
    ResumeBlockWalk(Box<MobBlockWalk>),
    Freeze,
    Push,
    PushTail(PushPhase),
    AfterPush,
    AfterWater,
    DolphinHop,
    Finish,
}

#[derive(Debug)]
struct PushPhase {
    targets: Vec<super::world::PushTarget>,
    player_before_hurt: Option<([i32; 4], u32)>,
}

#[derive(Debug)]
enum BaseStage {
    AfterBurn,
    AfterVoid,
    Air,
    Effects,
    EffectCountdown(EffectPass),
    Finish,
}

enum AiPause {
    Movement(MovementPhase),
    Breed(Box<MobEvent>, BreedingPhase),
    BeeDamage(super::bee::ServerAiStage),
    RegisterBehaviors(BrainStage),
}

fn goal_pause_to_ai(phase: SelectorPhase, pause: super::goals::GoalPause) -> AiPause {
    match pause {
        super::goals::GoalPause::Movement(pause) => {
            AiPause::Movement(MovementPhase::Goal(phase, pause))
        }
        super::goals::GoalPause::Breed(event, pause) => {
            AiPause::Breed(event, BreedingPhase::Goal(pause))
        }
    }
}

#[derive(Debug)]
enum SelectorPhase {
    Targets,
    Goals,
}

#[derive(Debug)]
enum MovementPhase {
    Goal(SelectorPhase, super::goals::GoalMovementPause),
    Travel(super::living::TravelPause),
}

/// A suspended move and the tick state needed once the move finishes.
#[derive(Debug)]
pub struct MobMovement {
    phase: MovementPhase,
}

fn pause_movement(
    m: &Mob,
    movement: &mut Option<MobMovement>,
    phase: MovementPhase,
) -> TickProgress {
    let travel = match &phase {
        MovementPhase::Goal(_, goal) => &goal.movement,
        MovementPhase::Travel(travel) => travel,
    };
    use super::living::MovementAction;
    let pause = match travel.action() {
        MovementAction::Trample(trample) => Pause::Trample(trample),
        MovementAction::SettleHit => Pause::AfterDamage(DamageResume {
            join: m.join,
            tail: DamageTail::Movement,
        }),
        MovementAction::Passengers(fall) => Pause::FallToPassengers(fall),
    };
    *movement = Some(MobMovement { phase });
    TickProgress::Paused(pause)
}

/// A mob's air update, decided and waiting for its owner to ask the mods:
/// the entity it concerns, and what was decided.
#[derive(Debug)]
pub struct AirPause {
    pub mob: u32,
    pub step: AirStep,
    join: crate::join_seq::JoinSeq,
    damage: AirDamage,
}

/// What a mob that runs out of air takes, and how it shows.
#[derive(Clone, Copy, Debug)]
enum AirDamage {
    /// `LivingEntity.baseTick`: drowning, with bubbles and a teleport after the hurt.
    Generic,
    /// `WaterAnimal.handleAirSupply`: drowning, without bubbles.
    Drown,
    /// `Axolotl` and `Nautilus.handleAirSupply`: drying out.
    DryOut,
}

impl AirPause {
    /// Whether the mob is still the live one the air was decided for, with the
    /// air and maximum the decision read. Otherwise the update is refused.
    pub fn still_valid(&self, m: &Mob) -> bool {
        m.id == self.mob
            && m.join == self.join
            && !m.removed
            && m.alive()
            && m.air == self.step.air
            && m.max_air_supply() == self.step.max
    }
}

/// Run a whole mob tick when no owning-host pause is needed by the caller: the
/// mods are not asked about its air, and a fox consumes without a host.
pub fn tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> bool {
    let mut progress = tick_until_pause(m, w, level, events, None);
    loop {
        progress = match progress {
            TickProgress::Complete(kept) => return kept,
            TickProgress::Paused(Pause::Consumption { decision }) => {
                super::fox::apply_consumption(&decision, w, m, events);
                resume_after_consumption(m, w, level, events, None)
            }
            TickProgress::Paused(Pause::Breathing { air, known_speed }) => {
                let breath = Some(air.step.vanilla);
                resume_after_breathing(m, w, level, events, air, known_speed, breath, None)
            }
            TickProgress::Paused(Pause::Drink { stack, resume }) => {
                // The hostless probe runs the same drink's built-in behaviour.
                if stack.potion().unwrap_or(&stack.item) == "minecraft:healing" {
                    m.heal(w.loop_context, &|| w.side.hit_reads(), 4.0);
                }
                if let Some(effect) = drink_potion_effect(&stack) {
                    m.add_effect_instance(crate::brewing::namespaced(effect.id), effect);
                }
                if &*stack.item == "minecraft:milk_bucket" {
                    m.apply_consume_effect(&crate::items::ConsumeEffect::ClearAllEffects);
                }
                m.extra.set_main_hand_stack(None);
                resume_after_drink(m, w, level, events, resume, None)
            }
            TickProgress::Paused(Pause::AfterDamage(resume)) => {
                resume_after_damage(m, w, level, events, resume, None)
            }
            TickProgress::Paused(
                Pause::Equipment { .. }
                | Pause::Trample(_)
                | Pause::FallToPassengers(_)
                | Pause::RegisterBehaviors { .. }
                | Pause::Breeding { .. },
            ) => unreachable!("a hostless tick cannot reach this pause"),
            TickProgress::Paused(Pause::InsideEffects(_)) => {
                unreachable!("hostless inside-block flush cannot pause")
            }
        };
    }
}

/// Stop when the living tick needs the owning game. A suspended travel or
/// goal leaves its continuation in the caller's movement slot.
pub fn tick_until_consumption(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: &mut Option<MobMovement>,
) -> TickProgress {
    tick_until_pause(m, w, level, events, Some(movement))
}

fn tick_until_pause(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    // ServerLevel.tick checks despawning before ticking the entity.
    if !check_despawn(m, w) {
        m.removed = true;
        return TickProgress::Complete(false);
    }
    // Entity.commonTick: `invulnerableTime` counts down before setOldPosAndRot.
    if m.invulnerable_time > 0 {
        m.invulnerable_time -= 1;
    }
    // `Entity.computeSpeed`: what it moved in the last tick.
    let known_speed = m.position.subtract(m.previous);
    m.previous = m.position;
    m.previous_yaw = m.yaw;
    m.previous_body_yaw = m.body_yaw;
    m.previous_head_yaw = m.head_yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_swell = m.swell;
    m.anim.begin_tick();
    super::camel::tick_age(m, w.game_time);
    m.ai.seen_cache.clear();
    super::creaking::check_protection(m, w);
    drop_invalid_player_target(m, w);
    // Mob.tick pre-update hooks (Creeper.tick swells before super.tick).
    pre_tick(m, w, events);
    if m.removed {
        return TickProgress::Complete(false);
    }
    if m.species() == Species::ZombieVillager {
        // ZombieVillager.tick counts the cure down before super.tick, with
        // or without AI (NoAI only skips serverAiStep).
        super::zombie_villager::step(m, &mut Cx { w, level, events });
    }
    super::warden::pre_tick(m, w, events);
    if m.species() == Species::Strider {
        super::strider::pre_tick(m, w, events);
    }
    let progress = base_tick(m, w, level, events);
    finish_base_progress(m, w, level, events, progress, known_speed, movement)
}

fn finish_base_progress(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    progress: BaseTick,
    known_speed: Vec3,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    match progress {
        BaseTick::Done => after_base_tick(m, w, level, events, known_speed, movement),
        BaseTick::Air(air) => TickProgress::Paused(Pause::Breathing { air, known_speed }),
        BaseTick::AfterDamage { next } => TickProgress::Paused(Pause::AfterDamage(DamageResume {
            join: m.join,
            tail: DamageTail::Base { next, known_speed },
        })),
    }
}

/// Resume a mob after its owner asked `living_breathe`: apply `breath` to the air
/// that was decided on (`None`: the answer was refused, the air stays), then the
/// rest of the base tick and the tick.
pub fn resume_after_breathing(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    air: AirPause,
    known_speed: Vec3,
    breath: Option<Breath>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    if let Some(breath) = breath {
        finish_air(m, w, level, events, air.step, air.damage, breath);
    }
    let progress = if m.hurt_needs_owner() && movement.is_some() {
        BaseTick::AfterDamage {
            next: BaseStage::Effects,
        }
    } else {
        continue_base_tick(m, w, level, events, BaseStage::Effects)
    };
    finish_base_progress(m, w, level, events, progress, known_speed, movement)
}

/// Continue the exact tick site after the owning game settled the hit.
pub(super) fn resume_after_damage(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    resume: DamageResume,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    if m.join != resume.join {
        return TickProgress::Complete(true);
    }
    hurt_teleports(m, w, events);
    match resume.tail {
        DamageTail::Base { next, known_speed } => {
            let progress = continue_base_tick(m, w, level, events, next);
            finish_base_progress(m, w, level, events, progress, known_speed, movement)
        }
        DamageTail::Ai { next, no_ai } => {
            continue_ai_tail(m, w, level, events, no_ai, movement.is_some(), next)
        }
        DamageTail::BeeServerAi { next } => {
            let progress = match bee_server_ai_step(m, w, level, events, next) {
                ControlFlow::Continue(()) => {
                    ai_step_after_server_ai(m, w, level, events, movement.is_some())
                }
                ControlFlow::Break(next) => ControlFlow::Break(AiPause::BeeDamage(next)),
            };
            finish_ai_progress(m, w, level, events, movement, progress)
        }
        DamageTail::Movement => resume_movement(
            m,
            w,
            level,
            movement.expect("a game-owned movement returned for a hurt"),
            events,
        ),
    }
}

/// Update item use and detect equipment changes after `Entity.baseTick`.
fn after_base_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    known_speed: Vec3,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if m.impulse_grace_time > 0 {
        m.impulse_grace_time -= 1;
    }
    if m.species() == Species::Rabbit {
        super::rabbit::base_tick(m);
    }
    if !m.alive() && m.death_time >= super::creaking::death_limit(m) {
        // Creaking.tickDeath: a heart's creaking crumbles away.
        if super::creaking::is_creaking(m) && super::creaking::tearing_down(m) {
            super::creaking::tear_down(m, events);
        }
        return TickProgress::Complete(false);
    }
    // LivingEntity.tick: updatingUsingItem, the stab of a spear in use.
    if m.alive() {
        super::spear_use::update_using_item(m, w, known_speed, events);
    }
    if m.alive()
        && m.species() == Species::WanderingTrader
        && let Some(stack) = super::trader::tick_drink(m, events)
    {
        return TickProgress::Paused(Pause::Drink {
            stack,
            resume: DrinkResume::Trader,
        });
    }
    after_using_item(m, w, level, events, movement)
}

fn after_using_item(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    // LivingEntity.tick -> detectEquipmentUpdates, before the sleep check and aiStep.
    if movement.is_some()
        && !m.removed
        && w.loop_context.side() == LoopSide::Server
        && let Some(items) = w.items
        && let Some(first) =
            super::finalize::detect_equipment_updates(m, w.attributes, items, || {
                mod_api::__private::enabled_listened(mod_api::HookId::LivingEquipmentChange)
            })
    {
        return TickProgress::Paused(Pause::Equipment { first });
    }
    resume_after_equipment(m, w, level, events, movement)
}

/// Continue `finish_server_ai_step` after the owning game flushed staged
/// `register_behaviors` for a freshly built or rebuilt brain.
pub(super) fn resume_after_register_behaviors(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    stage: BrainStage,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    let pause_owner = movement.is_some();
    if let BrainStage::AfterAgeBoundary { no_ai, after_water } = stage {
        let next = if after_water {
            AiStage::Finish
        } else {
            AiStage::AfterWater
        };
        return continue_ai_tail(m, w, level, events, no_ai, pause_owner, next);
    }
    let progress = match finish_server_ai_step_from(
        m,
        w,
        level,
        events,
        Some(stage),
        pause_owner,
        pause_owner,
    ) {
        ControlFlow::Continue(()) => ai_step_after_server_ai(m, w, level, events, pause_owner),
        ControlFlow::Break(next) => ControlFlow::Break(next),
    };
    finish_ai_progress(m, w, level, events, movement, progress)
}

/// Continue without redoing base tick, item use or equipment detection.
pub(super) fn resume_after_equipment(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    super::sulfur_cube::adopt_body(m);
    let sleep_answer = m
        .extra
        .villager
        .as_deref_mut()
        .and_then(|e| e.may_continue_sleeping.take());
    // LivingEntity.tick checks the bed even for mobs without AI.
    if super::villager::sleeping(m)
        && !sleep_answer.unwrap_or_else(|| {
            super::villager::sleeping_pos(m)
                .is_some_and(|p| w.block_name(p[0], p[1], p[2]).ends_with("_bed"))
        })
    {
        super::villager::stop_sleeping(m, w, events);
    }
    // Fox.aiStep consumes before its super.aiStep, so the hook and its RNG
    // precede goals, travel and block effects at the resumed position.
    if m.species() == Species::Fox
        && !m.no_ai
        && let Some(decision) = super::fox::ai_step_until_consumption(m, w, events)
    {
        return TickProgress::Paused(Pause::Consumption { decision });
    }
    ai_and_finish_tick(m, w, level, events, movement)
}

/// Resume a fox after its item effect, before the living AI/movement step.
pub fn resume_after_consumption(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    super::fox::before_living_ai_step(m, w);
    ai_and_finish_tick(m, w, level, events, movement)
}

/// Resume the exact tick site after `finishUsingItem` or a witch's potion.
pub(super) fn resume_after_drink(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    resume: DrinkResume,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    match resume {
        DrinkResume::Trader => after_using_item(m, w, level, events, movement),
        DrinkResume::Witch => {
            super::witch::after_drink(m, events);
            let progress = ai_step(m, w, level, events, movement.is_some());
            finish_ai_progress(m, w, level, events, movement, progress)
        }
    }
}

/// The built-in status effect on either specialty drink's potion contents.
pub fn drink_potion_effect(
    stack: &crate::items::ItemStack,
) -> Option<crate::effects::EffectInstance> {
    let effect = match stack.potion().unwrap_or(&stack.item) {
        "minecraft:water_breathing" => "water_breathing",
        "minecraft:fire_resistance" => "fire_resistance",
        "minecraft:swiftness" => "speed",
        "minecraft:invisibility" => "invisibility",
        _ => return None,
    };
    Some(crate::effects::EffectInstance::new(effect, 3600, 0))
}

fn ai_and_finish_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
) -> TickProgress {
    if super::creaking::is_creaking(m) {
        super::creaking::update(m, w, events);
    }
    if m.no_ai {
        update_fall_flying(m, w, events);
        reset_effect_fall_distance(m);
        // Mob.aiStep still runs without AI: tagged mobs can burn, endermites age.
        burn_undead(m, w, events);
        if m.species() == Species::Endermite {
            super::hostile::endermite_lifetime(m);
        }
        // AbstractHorse.aiStep (it overrides LivingEntity.aiStep, which NoAI
        // doesn't skip) and AbstractHorse.tick: a NoAI horse still swishes
        // its tail before super.aiStep, then heals, grazes and runs its
        // stand counter and animations.
        let horse = super::equine::is_equine(m.species())
            && !super::camel::is_camel(m.species())
            && !super::llama::is_llama(m.species());
        if horse {
            super::equine::tail_tick(m);
        }
        // Guardian.aiStep keeps the air and flops on land before its super.aiStep.
        if matches!(m.species(), Species::Guardian | Species::ElderGuardian) {
            super::guardian::ai_step(m, w, events);
        }
        // LivingEntity.aiStep: without AI there is no serverAiStep and no
        // travel, but the motion below 0.003 is still dropped, the block
        // walk and freezing still run, and pushEntities still runs, so a
        // NoAI mob pushes what it overlaps (and is pushed back by its own
        // push).
        drop_tiny_motion(m);
        let progress = ridden_without_ai(m, w, level, events, movement.is_some());
        return finish_ai_progress(m, w, level, events, movement, progress);
    }
    // Witch.aiStep drinks before super.aiStep; resume at the following magic roll.
    if m.species() == Species::Witch {
        let far = m
            .target_info(w)
            .is_some_and(|t| t.position.distance_squared(m.position) > 121.0);
        if let Some(stack) = super::witch::ai_step_until_drink(m, events, far) {
            return TickProgress::Paused(Pause::Drink {
                stack,
                resume: DrinkResume::Witch,
            });
        }
    }
    let progress = ai_step(m, w, level, events, movement.is_some());
    finish_ai_progress(m, w, level, events, movement, progress)
}

/// Resume the rest of the selector or brain, then movement and the rest of this tick.
pub(super) fn resume_after_birth(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    resume: BreedingResume,
    movement: &mut Option<MobMovement>,
) -> TickProgress {
    if m.removed {
        return TickProgress::Complete(false);
    }
    let progress = match resume.phase {
        BreedingPhase::Goal(goal) => {
            let paused = {
                let BorrowedGoalSelectors { mob, goals, .. } = &mut BorrowedGoalSelectors::new(m);
                goals.resume_after_birth(mob, &mut Cx { w, level, events }, goal)
            };
            if let Some(pause) = paused {
                ControlFlow::Break(goal_pause_to_ai(SelectorPhase::Goals, pause))
            } else if m.removed {
                return TickProgress::Complete(false);
            } else {
                match finish_server_ai_step_from(m, w, level, events, None, true, true) {
                    ControlFlow::Continue(()) => ai_step_after_server_ai(m, w, level, events, true),
                    ControlFlow::Break(pause) => ControlFlow::Break(pause),
                }
            }
        }
        BreedingPhase::Brain(brain) => {
            if let Some((event, resume)) =
                super::animal_brain::resume_after_birth(m, w, level, events, brain)
            {
                ControlFlow::Break(AiPause::Breed(event, BreedingPhase::Brain(resume)))
            } else {
                match server_ai_controls(m, w, level, events, false) {
                    ControlFlow::Continue(()) => ai_step_after_server_ai(m, w, level, events, true),
                    ControlFlow::Break(pause) => ControlFlow::Break(pause),
                }
            }
        }
    };
    finish_ai_progress(m, w, level, events, Some(movement), progress)
}

fn finish_ai_progress(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    movement: Option<&mut Option<MobMovement>>,
    progress: ControlFlow<AiPause>,
) -> TickProgress {
    match progress {
        ControlFlow::Continue(()) => start_ai_tail(m, w, level, events, movement.is_some()),
        ControlFlow::Break(AiPause::Movement(phase)) => pause_movement(
            m,
            movement.expect("only a game-owned tick pauses movement"),
            phase,
        ),
        ControlFlow::Break(AiPause::Breed(event, phase)) => TickProgress::Paused(Pause::Breeding {
            event: Some(event),
            resume: BreedingResume { phase },
        }),
        ControlFlow::Break(AiPause::BeeDamage(next)) => {
            TickProgress::Paused(Pause::AfterDamage(DamageResume {
                join: m.join,
                tail: DamageTail::BeeServerAi { next },
            }))
        }
        ControlFlow::Break(AiPause::RegisterBehaviors(stage)) => {
            TickProgress::Paused(Pause::RegisterBehaviors { stage })
        }
    }
}

fn start_ai_tail(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    pause_damage: bool,
) -> TickProgress {
    let no_ai = m.no_ai;
    let next = AiStage::BlockWalk;
    if pause_damage && m.hurt_needs_owner() {
        return TickProgress::Paused(Pause::AfterDamage(DamageResume {
            join: m.join,
            tail: DamageTail::Ai { next, no_ai },
        }));
    }
    continue_ai_tail(m, w, level, events, no_ai, pause_damage, next)
}

fn continue_ai_tail(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    no_ai: bool,
    pause_damage: bool,
    mut next: AiStage,
) -> TickProgress {
    loop {
        next = match next {
            // `Entity.isAffectedByBlocks` is `!isRemoved() && !noPhysics`: a mob that
            // converted earlier in this tick (`Mob.convertTo` discards it) has no
            // `stepOn` and no block insides.
            AiStage::BlockWalk if m.removed => AiStage::Freeze,
            AiStage::BlockWalk => {
                if !no_ai || super::equine::controlled(m) {
                    m.apply_effects_from_blocks(w, events);
                }
                let mut walk = start_block_walk(m, w, level, events);
                if pause_damage && m.hurt_needs_owner()
                    || !walk.run(m, w, level, events, pause_damage)
                {
                    AiStage::ResumeBlockWalk(Box::new(walk))
                } else {
                    let effects = std::mem::take(&mut walk.effects);
                    if pause_damage && effects.needs_live_owner() {
                        return TickProgress::Paused(Pause::InsideEffects(InsidePause {
                            mob: m.id,
                            join: m.join,
                            effects,
                            temperature: walk.temperature,
                            resume: if no_ai {
                                InsideResume::NoAi
                            } else {
                                InsideResume::AiStep
                            },
                        }));
                    }
                    flush_mob_inside_hostless(m, w, level, events, effects);
                    walk.temperature.finish(m, w, events);
                    AiStage::Freeze
                }
            }
            AiStage::ResumeBlockWalk(mut walk) => {
                if walk.run(m, w, level, events, pause_damage) {
                    let effects = std::mem::take(&mut walk.effects);
                    if pause_damage && effects.needs_live_owner() {
                        return TickProgress::Paused(Pause::InsideEffects(InsidePause {
                            mob: m.id,
                            join: m.join,
                            effects,
                            temperature: walk.temperature,
                            resume: if no_ai {
                                InsideResume::NoAi
                            } else {
                                InsideResume::AiStep
                            },
                        }));
                    }
                    flush_mob_inside_hostless(m, w, level, events, effects);
                    walk.temperature.finish(m, w, events);
                    AiStage::Freeze
                } else {
                    AiStage::ResumeBlockWalk(walk)
                }
            }
            AiStage::Freeze => {
                if m.alive() {
                    freeze_tick(m, w, level, events);
                }
                AiStage::Push
            }
            AiStage::Push => push_entities_until_hurt(m, w, level, events, pause_damage)
                .map_or(AiStage::AfterPush, AiStage::PushTail),
            AiStage::PushTail(push) => {
                apply_entity_pushes(m, w, events, push.targets, true, push.player_before_hurt);
                AiStage::AfterPush
            }
            AiStage::AfterPush => {
                if no_ai {
                    ageable_step(m, w, level, events);
                    animal_step(m, events);
                    if m.species() == Species::Chicken {
                        farm::chicken_egg_step(m, events);
                    }
                    if m.species() == Species::Allay {
                        super::allay::passive_heal(m, w.loop_context, w.side);
                        super::allay::update_duplication_cooldown(m);
                    }
                    // AbstractHorse.aiStep, Camel.tick, Bee.aiStep and WanderingTrader.aiStep
                    // have steps without AI.
                    if super::camel::is_camel(m.species())
                        || (super::equine::is_equine(m.species())
                            && !super::llama::is_llama(m.species()))
                    {
                        super::equine::ai_step(m, w, events);
                    }
                    if m.species() == Species::Bee {
                        super::bee::ai_step(m, w);
                    }
                    if m.species() == Species::WanderingTrader {
                        super::trader::ai_step(m, w);
                    }
                }
                hurt_by_water(m, w, level, events);
                if no_ai {
                    hurt_teleports(m, w, events);
                }
                AiStage::AfterWater
            }
            AiStage::AfterWater => {
                if no_ai {
                    // These species' own steps also run outside serverAiStep.
                    if m.species() == Species::Ravager {
                        super::ravager::ai_step(m, w, events);
                    }
                    if family(m.species()) == Family::Aquatic {
                        aquatic::no_ai_step(m, w, events);
                    }
                    super::cube::no_ai_tick(m, w, events);
                    if m.species() == Species::Shulker {
                        super::shulker::shell_tick(m, w, events);
                    }
                    if m.species() == Species::TraderLlama {
                        super::caravan::trader_llama_tick(m, w);
                    }
                    if m.species() == Species::Fox {
                        super::fox::finish_ai_step(m, w, events);
                    }
                    AiStage::Finish
                } else {
                    // Spider.tick: climbing while pressed against a wall.
                    if m.kind.flags.climber {
                        m.climbing = m.ground.horizontal_collision;
                    }
                    ageable_step(m, w, level, events);
                    animal_step(m, events);
                    burn_undead(m, w, events);
                    species_ai_step(m, w, level, events)
                }
            }
            AiStage::DolphinHop => {
                aquatic::dolphin_hop(m);
                AiStage::Finish
            }
            AiStage::Finish => {
                if pause_damage && m.staged_parts.pending_behaviors.is_some() {
                    return TickProgress::Paused(Pause::RegisterBehaviors {
                        stage: BrainStage::AfterAgeBoundary {
                            no_ai,
                            after_water: true,
                        },
                    });
                }
                return TickProgress::Complete(finish_tick(m, w, level, events));
            }
        };
        if pause_damage && m.hurt_needs_owner() {
            return TickProgress::Paused(Pause::AfterDamage(DamageResume {
                join: m.join,
                tail: DamageTail::Ai { next, no_ai },
            }));
        }
        if pause_damage && m.staged_parts.pending_behaviors.is_some() {
            let after_water = matches!(next, AiStage::Finish);
            return TickProgress::Paused(Pause::RegisterBehaviors {
                stage: BrainStage::AfterAgeBoundary { no_ai, after_water },
            });
        }
    }
}

/// Resume a mob after the owning `Game` flushed its `InsideEffects` collector
/// with a live level loan.
pub(super) fn resume_after_inside_effects(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    paused: InsidePause,
) -> TickProgress {
    if m.removed || m.id != paused.mob || m.join != paused.join {
        return TickProgress::Complete(!m.removed);
    }
    let no_ai = matches!(paused.resume, InsideResume::NoAi);
    paused.temperature.finish(m, w, events);
    continue_ai_tail(m, w, level, events, no_ai, true, AiStage::Freeze)
}

/// Resume the exact move, then the remainder of `LivingEntity.aiStep` and the tick.
pub(super) fn resume_movement(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    movement: &mut Option<MobMovement>,
    events: &mut Vec<MobEvent>,
) -> TickProgress {
    let paused = movement.take().expect("the movement pause was stored");
    if m.removed {
        return TickProgress::Complete(false);
    }
    let progress = match paused.phase {
        MovementPhase::Travel(travel) => match travel.resume(m, w, level, events) {
            ControlFlow::Continue(()) => {
                hurt_teleports(m, w, events);
                ControlFlow::Continue(())
            }
            ControlFlow::Break(travel) => {
                ControlFlow::Break(AiPause::Movement(MovementPhase::Travel(travel)))
            }
        },
        MovementPhase::Goal(phase, goal) => {
            let force = goal.force;
            let paused = {
                let BorrowedGoalSelectors {
                    mob,
                    targets,
                    goals,
                } = &mut BorrowedGoalSelectors::new(m);
                let mut cx = Cx { w, level, events };
                let pause_birth = w.asks_live_birth(mob.species());
                let (active, after, breed_active) = match phase {
                    SelectorPhase::Targets => (targets, Some(goals), false),
                    SelectorPhase::Goals => (goals, None, pause_birth),
                };
                if let Some(pause) = active.resume_movement(mob, &mut cx, goal, breed_active) {
                    Some((phase, pause))
                } else if !mob.removed
                    && let Some(goals) = after
                {
                    // `Mob.serverAiStep` ticks the action selector only after
                    // the target selector's last running goal returns.
                    goals
                        .tick_until_pause(mob, &mut cx, force, pause_birth)
                        .map(|pause| (SelectorPhase::Goals, pause))
                } else {
                    None
                }
            };
            if let Some((phase, pause)) = paused {
                ControlFlow::Break(goal_pause_to_ai(phase, pause))
            } else {
                if m.removed {
                    return TickProgress::Complete(false);
                }
                let pause_birth = w.asks_live_birth(m.species());
                match finish_server_ai_step_from(m, w, level, events, None, true, pause_birth) {
                    ControlFlow::Continue(()) => ai_step_after_server_ai(m, w, level, events, true),
                    ControlFlow::Break(pause) => ControlFlow::Break(pause),
                }
            }
        }
    };
    finish_ai_progress(m, w, level, events, Some(movement), progress)
}

/// Resume the living tick after species AI (and the block walk and freezing
/// in `LivingEntity.aiStep`), before animation.
/// Kept as one continuation so an inline action can regain its owning host and
/// then run the same remaining work, in the original order.
#[inline]
pub fn finish_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> bool {
    count_fall_fly_ticks(m);
    if m.species() == Species::SulfurCube && m.alive() {
        // SulfurCube.doPush applies the body's contact damage even with NoAI.
        // Player contact keeps its existing player_push route.
        super::sulfur_cube::mob_contacts(m, w, events);
    }
    if m.species() == Species::Rabbit {
        super::rabbit::post_tick(m);
    }
    // Mob.tickHeadTurn -> BodyRotationControl.clientTick (a frozen
    // creaking's waits).
    if m.alive()
        && (!super::creaking::is_creaking(m) || super::creaking::can_move(m))
        && !super::armadillo::scared(m)
    {
        m.body_rotation_tick();
    }
    events.append(&mut m.outbox);
    m.update_walk_animation();
    super::axolotl::tick_animation(m);
    m.anim.end_tick(m.ai.swinging);
    tick_swing(m);
    flower_contact_effects(m, w);
    // The species' own tick after the living tick, with or without AI.
    match m.species() {
        // CopperGolem.tick: updateWeathering.
        Species::CopperGolem => {
            super::copper_golem::update_weathering(m, w, level.random(), events)
        }
        Species::Strider => super::strider::float(m, w),
        Species::Bat => super::bat::tick(m),
        Species::Bee => super::bee::tick(m, level.random()),
        Species::Allay => super::allay::tick(m, w, events),
        Species::Panda => super::panda::tick(m, w, events),
        Species::Villager => super::villager::tick(m, w.game_time),
        // Vex.tick: gravity off on every tick, AI or not.
        Species::Vex => m.no_gravity = true,
        _ => {}
    }
    aquatic::school_tick(m, w, level.random());
    // Dolphin.tick: after super.tick, NoAI restores the species maximum.
    if m.no_ai && m.species() == Species::Dolphin {
        m.air = m.max_air_supply();
    }
    // TraderLlama.maybeDespawn discards the llama in its aiStep: it leaves the
    // level's entity list in this tick, after the rest of its tick ran.
    !(m.removed && m.species() == Species::TraderLlama)
}
/// `LivingEntity.aiStep` after `travel`, with or without AI: the block walk
/// (`applyEffectsFromBlocks`), then the freezing and its hurt, both before
/// `pushEntities` (cramming) and the water's hurt (#1760). `isInPowderSnow`
/// was cleared by `Entity.baseTick`; the walk's FREEZE effect sets it for a
/// powder snow cell on the tick's path, so a hurt that teleports the mob
/// during the walk keeps it.
#[cfg(test)]
fn walk_and_freeze(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    effects_from_blocks(m, w, level, events);
    if m.alive() {
        freeze_tick(m, w, level, events);
    }
}
/// `LivingEntity.aiStep` -> `Entity.applyEffectsFromBlocks` for the blocks
/// that hurt, hold or slide a mob: the cactus (`CactusBlock.entityInside`, 1),
/// the cobweb (stuck at 0.25, 0.05, 0.25), the sweet berry bush
/// (`SweetBerryBushBlock.entityInside`: stuck at 0.8, 0.75, 0.8, and a grown bush pricks for 1 when the mob moved at least 0.003 on X
/// or Z this tick) and the honey block (`honey_slide`). The blocks are those
/// vanilla's own sweep reaches along the mob's move this tick, on the shared
/// inside walk; a part's contact fires mid-walk there.
#[cfg(test)]
pub(crate) fn effects_from_blocks(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    let mut walk = start_block_walk(m, w, level, events);
    walk.run(m, w, level, events, false);
    flush_mob_inside_hostless(m, w, level, events, walk.effects);
    walk.temperature.finish(m, w, events);
}

fn flush_mob_inside_hostless(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    mut effects: mod_api::InsideEffects,
) {
    if effects.is_empty() {
        return;
    }
    let mut access = MobInsideAccess {
        m,
        w,
        level,
        events,
    };
    effects.apply_and_clear(&mut access);
}

fn start_block_walk(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> MobBlockWalk {
    // Entity.applyEffectsFromBlocks fixes the tick's movements before stepOn:
    // a hurt below that teleports an enderman or a shulker leaves the scan on
    // the path it had. A stepOn teleport (snapTo) has rewritten the mob's
    // live fields by the time the walk below reads them, so the walk reads
    // these.
    let (tick_from, tick_to) = (m.previous, m.position);
    // Block.stepOn for the block under a mob on the ground (`getOnPosLegacy`):
    // MagmaBlock.stepOn's hot floor, 1 damage (a mob never sneaks).
    if m.ground.on_ground && m.alive() {
        // 0.2 below the feet, or the supporting block when the centre is over
        // its edge.
        let [x, y, z] = m.block_on_legacy(w);
        let carefully = matches!(m.species(), Species::Cat | Species::Ocelot)
            && m.extra.pose == super::sync::CROUCHING;
        let state = w.world.block(x, y, z);
        if !carefully
            && w.block_name(x, y, z) == "minecraft:slime_block"
            && let Some(scale) = crate::entities::slime_step_on_scale(m.velocity.y)
        {
            m.velocity.x *= scale;
            m.velocity.z *= scale;
        }
        if !carefully && w.block_name(x, y, z) == "minecraft:magma_block" {
            m.damage(level, 1.0, Damage::of(DamageKind::HotFloor));
            // Enderman.hurtServer finishes before stepOn returns; the scan
            // below still uses the movements captured before that hurt.
            hurt_teleports(m, w, events);
        }
        // TurtleEggBlock.stepOn: unless it steps carefully, an egg under
        // it breaks one time in a hundred (`destroyEgg`), unless it is a turtle
        // or a bat or mob griefing is off (`canDestroyEgg`); the roll is the
        // level's and comes only after those checks.
        if !carefully
            && w.block_name(x, y, z) == "minecraft:turtle_egg"
            && !matches!(m.species(), Species::Turtle | Species::Bat)
            && w.can_grief(m.id)
            && level.random().next_int_bound(100).unwrap_or(1) == 0
        {
            events.push(MobEvent::CrackTurtleEgg { pos: [x, y, z] });
        }
        if crate::block_contact::step_listens(state) {
            use crate::block_contact::{Contact, StepContact, StepLevel};
            let step = StepContact {
                contact: Contact {
                    entity: m.id as i32,
                    kind: &m.kind.id,
                    state,
                    position: sunrise_core::position::BlockPos::new(x, y, z),
                },
                carefully,
                bounds: m.bounds_at(m.position),
            };
            let view =
                if w.loop_context.side() == sunrise_world::native_load_context::LoopSide::Server {
                    StepLevel::Server(level)
                } else {
                    StepLevel::Client(crate::motion::FrictionLevel {
                        world: w.world,
                        registry: w.registry,
                        context: w.loop_context,
                        side: w.side,
                        coefficient: &|id| Some(w.movement.get(id).ok()?.coefficients.friction),
                    })
                };
            for write in crate::block_contact::step_on(view, step, &mut m.velocity) {
                if !m.alive() {
                    break;
                }
                apply_entity_write(m, w, level, events, write);
            }
        }
    }
    fn apply_entity_write(
        m: &mut Mob,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        write: mod_api::EntityWrite,
    ) {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                m.velocity = m.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => m.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                m.fall_distance = 0.0;
                m.stuck_multiplier = triple(v);
            }
            mod_api::EntityWrite::Hurt {
                damage_type,
                amount,
            } => {
                if let Some(named) =
                    super::NamedDamage::from_contact_key(w.side.damage_types(), damage_type)
                {
                    m.damage(level, amount, Damage::of(DamageKind::Named(named)));
                    // The hurt's own teleport, inside it (#1697).
                    hurt_teleports(m, w, events);
                }
            }
            mod_api::EntityWrite::ResetFallDistance => m.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => m.fire_ticks = ticks,
            mod_api::EntityWrite::AddEffect {
                effect,
                duration,
                amplifier,
            } => {
                // `LivingEntity.addEffect`, as the write's contract names: one
                // instance an effect, its attribute modifiers stored.
                if let Some(id) = mod_api::__private::effect_key_id(effect) {
                    m.add_effect(id, duration, i32::from(amplifier));
                }
            }
        }
    }
    // `Entity.checkInsideBlocks` itself, the shared walk: each axis of the
    // move in vanilla's order, a 16-iteration budget, one visited set, the
    // alive check. The mob's own per-block moments run mid-walk where vanilla
    // runs them, and the part's contact fires there too, staged. (This is the
    // parity change: the 0.2-block sampling over every box cell this replaced
    // visited blocks in another order; the walk's own commit carries it.)
    MobBlockWalk {
        movement: sunrise_world::entity_move::MovementRecord {
            from: tick_from,
            to: tick_to,
            original: None,
        },
        stationary: None,
        stationary_done: false,
        travel: Default::default(),
        visited: Default::default(),
        effects: Default::default(),
        temperature: InsideTemperature::read(m),
    }
}

#[derive(Debug)]
struct MobBlockWalk {
    movement: sunrise_world::entity_move::MovementRecord,
    stationary: Option<crate::block_contact::BoxCellSweep>,
    stationary_done: bool,
    travel: crate::block_contact::MovementSweep,
    visited: crate::fast_hash::FxHashSet<i64>,
    effects: mod_api::InsideEffects,
    temperature: InsideTemperature,
}

impl MobBlockWalk {
    fn run(
        &mut self,
        m: &mut Mob,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        pause_damage: bool,
    ) -> bool {
        let mut inside = MobInside {
            m,
            w,
            level,
            events,
        };
        let halt = |inside: &MobInside<'_, '_, '_>| pause_damage && inside.m.hurt_needs_owner();
        // Begin at the old box before sweeping the captured movement. The
        // two passes share the visited set and the collector's first step.
        if !self.stationary_done {
            let walk = self.stationary.get_or_insert_with(|| {
                let start = self.movement.from;
                let bounds = inside
                    .m
                    .bounds_at(start)
                    .deflate(Vec3::new(1e-5, 1e-5, 1e-5));
                crate::block_contact::BoxCellSweep::new(bounds, start)
            });
            if matches!(
                walk.run(&mut inside, &mut self.visited, &mut self.effects, halt),
                Ok(false)
            ) {
                return false;
            }
            self.stationary_done = true;
        }
        !matches!(
            self.travel.run(
                &mut inside,
                std::slice::from_ref(&self.movement),
                &mut self.visited,
                &mut self.effects,
                halt
            ),
            Ok(false)
        )
    }
}

/// Hostless backing for [`mod_api::InsideEntityView`] used only in standalone
/// mob unit tests without a live `Game`. Game-owned mob ticks pause and flush
/// through `Game::flush_mob_inside_live` with a live level loan.
pub(super) struct MobInsideAccess<'a, 'w, 'l> {
    pub(super) m: &'a mut Mob,
    pub(super) w: &'a MobWorld<'w>,
    pub(super) level: &'a mut DamageLevel<'l>,
    pub(super) events: &'a mut Vec<MobEvent>,
}

impl MobInsideAccess<'_, '_, '_> {
    fn in_loaded_world(&self, pos: [i32; 3]) -> bool {
        let layout = self.w.world.layout();
        pos[1] >= layout.min_y()
            && pos[1] < layout.max_y()
            && self
                .w
                .world
                .chunks
                .contains_key(&(pos[0] >> 4, pos[2] >> 4))
    }

    fn current_block(&self, pos: [i32; 3]) -> Option<sunrise_world::registry::BlockStateId> {
        if !self.in_loaded_world(pos) {
            return None;
        }
        Some(self.w.world.block(pos[0], pos[1], pos[2]))
    }
}

impl mod_api::InsideEntityAccess for MobInsideAccess<'_, '_, '_> {
    fn id(&self) -> u32 {
        self.m.id
    }

    fn kind(&self) -> &str {
        &self.m.kind.id
    }

    fn is_alive(&self) -> bool {
        self.m.alive()
    }

    fn position(&self) -> [f64; 3] {
        [self.m.position.x, self.m.position.y, self.m.position.z]
    }

    fn velocity(&self) -> [f64; 3] {
        [self.m.velocity.x, self.m.velocity.y, self.m.velocity.z]
    }

    fn fire_ticks(&self) -> i32 {
        self.m.fire_ticks
    }

    fn is_on_fire(&self) -> bool {
        self.m.is_on_fire()
    }

    fn fall_distance(&self) -> f64 {
        self.m.fall_distance
    }

    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        self.current_block(pos)
            .and_then(crate::block_contact::state_to_api)
    }

    fn push(&mut self, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.m.velocity = self
            .m
            .velocity
            .add_vector(Vec3::new(delta[0], delta[1], delta[2]));
        true
    }

    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.m.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }

    fn make_stuck_in_block(&mut self, speed_multiplier: [f64; 3]) -> bool {
        if !speed_multiplier.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.m.fall_distance = 0.0;
        self.m.stuck_multiplier = Vec3::new(
            speed_multiplier[0],
            speed_multiplier[1],
            speed_multiplier[2],
        );
        true
    }

    fn reset_fall_distance(&mut self) {
        self.m.fall_distance = 0.0;
    }

    fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        self.m.fire_ticks = ticks;
    }

    fn add_effect(&mut self, effect: mod_api::EffectKey, duration: i32, amplifier: u8) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)
        else {
            return false;
        };
        self.m
            .try_add_effect(def.id, duration, i32::from(amplifier))
    }

    fn hurt_with_source(
        &mut self,
        damage_type: &str,
        amount: f32,
        direct: Option<mod_api::EntityRef>,
        causing: Option<mod_api::EntityRef>,
    ) -> bool {
        if !amount.is_finite() || amount < 0.0 || direct.is_some() || causing.is_some() {
            return false;
        }
        let Some(kind) = crate::health::resolve_damage_kind(None, damage_type) else {
            return false;
        };
        let hurt = self.m.damage(self.level, amount, Damage::of(kind));
        // The hurt's own teleport, inside it (#1697).
        hurt_teleports(self.m, self.w, self.events);
        hurt
    }

    fn set_block(&mut self, _pos: [i32; 3], _state: mod_api::BlockState) -> bool {
        false
    }

    fn destroy_block(&mut self, _pos: [i32; 3], _drop: bool) -> bool {
        false
    }

    fn apply_effect(&mut self, effect: mod_api::InsideEffect) {
        match effect {
            mod_api::InsideEffect::Freeze => {
                self.m.in_powder_snow = true;
                if can_freeze(self.m, self.w) {
                    self.m.ticks_frozen = (self.m.ticks_frozen + 1)
                        .min(crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
                }
            }
            mod_api::InsideEffect::ClearFreeze => {
                self.m.ticks_frozen = 0;
            }
            mod_api::InsideEffect::FireIgnite => {
                self.m.fire_ignite_contact();
            }
            mod_api::InsideEffect::LavaIgnite => {
                if !self.m.fire_immune() {
                    self.m.ignite_for_ticks(300);
                }
            }
            mod_api::InsideEffect::Extinguish => {
                self.m.fire_ticks = self.m.fire_ticks.min(0);
            }
        }
    }

    fn apply_builtin(&mut self, action: mod_api::__private::BuiltinInsideAction) {
        match action {
            mod_api::__private::BuiltinInsideAction::PowderSnowMelt(pos) => {
                if self.m.is_on_fire() && self.w.can_grief(self.m.id) {
                    self.events.push(MobEvent::BreakBlock {
                        pos,
                        drop: false,
                        by: None,
                        breaker: None,
                    });
                }
            }
            mod_api::__private::BuiltinInsideAction::LayeredCauldronExtinguish { .. } => {}
            mod_api::__private::BuiltinInsideAction::FireHurt(damage) => {
                self.hurt_with_source("minecraft:in_fire", damage, None, None);
            }
            mod_api::__private::BuiltinInsideAction::LavaHurt => {
                if !self.m.fire_immune()
                    && self.hurt_with_source("minecraft:lava", 4.0, None, None)
                    && !self.m.silent
                {
                    self.events.push(MobEvent::Sound(
                        "entity.generic.burn",
                        self.m.position.components(),
                        0.4,
                        2.0 + self.m.random.next_float() * 0.4,
                    ));
                }
            }
        }
    }
}

/// The mob's view of the shared inside walk: its box, its life, the level's
/// blocks, its own `entityInside` moments (kept as their own code, run
/// mid-walk where vanilla runs them), and the part's contact, fired mid-walk
/// over the level random host the mob's tick already holds.
struct MobInside<'a, 'w, 'l> {
    m: &'a mut Mob,
    w: &'a MobWorld<'w>,
    level: &'a mut DamageLevel<'l>,
    events: &'a mut Vec<MobEvent>,
}

impl crate::block_contact::InsideWalk for MobInside<'_, '_, '_> {
    type Effects = mod_api::InsideEffects;
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> sunrise_core::geometry::Aabb {
        self.m.bounds_at(at)
    }
    fn is_alive(&self) -> bool {
        self.m.alive()
    }
    fn block(&self, at: sunrise_core::position::BlockPos) -> sunrise_world::registry::BlockStateId {
        self.w.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        self.w.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        at: sunrise_core::position::BlockPos,
    ) -> crate::block_contact::InsideShape {
        self.w
            .registry
            .state(state)
            .map_or(crate::block_contact::InsideShape::Full, |def| {
                crate::block_contact::cauldron_inside_shape(self.w.collision, state, def, at)
            })
    }
    fn collided_with_fluid(
        &self,
        at: sunrise_core::position::BlockPos,
        from: Vec3,
        to: Vec3,
    ) -> bool {
        crate::entities::swept_fluid_contact(
            self.w.world,
            self.w.fluid_tables,
            at,
            self.bounds_at(from),
            to.subtract(from),
        )
    }
    fn fluid_inside(
        &mut self,
        at: sunrise_core::position::BlockPos,
        _: i32,
        effects: &mut mod_api::InsideEffects,
    ) -> Result<(), ()> {
        crate::block_contact::queue_fluid_effects(
            effects,
            self.w.fluid_tables.fluid(self.block(at)).kind(),
        );
        Ok(())
    }
    fn entity_inside(
        &mut self,
        at: sunrise_core::position::BlockPos,
        state: sunrise_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let (x, y, z) = (at.x, at.y, at.z);
        let p = [x, y, z];
        // The parts' contacts, mid-walk: generated dispatch isolates each part
        // into its own staged collector and absorbs it only on normal return.
        if let Some(hooks) = crate::block_contact::HOOKS.get()
            && let Some(dimension) = crate::block_contact::current_level()
            && (hooks.inside_listens)(state)
        {
            let id = self.m.id as i32;
            (hooks.inside_fire)(
                self.level,
                dimension,
                id,
                &self.m.kind.id,
                state,
                at,
                precise,
                effects,
            );
        }
        let name = self.w.block_name(x, y, z);
        crate::block_contact::queue_block_effects(effects, name, state, at);
        match name {
            "minecraft:cactus" => {
                self.m
                    .damage(self.level, 1.0, Damage::of(DamageKind::Cactus));
                // The hurt's own teleport, inside it, as magma's (#1697).
                hurt_teleports(self.m, self.w, self.events);
            }
            // CampfireBlock.entityInside: a lit campfire hurts a
            // living entity by its fire damage (1, soul 2).
            name @ ("minecraft:campfire" | "minecraft:soul_campfire")
                if self.w.prop(x, y, z, "lit") == Some("true") =>
            {
                let damage = if name == "minecraft:campfire" {
                    1.0
                } else {
                    2.0
                };
                self.m
                    .damage(self.level, damage, Damage::of(DamageKind::Campfire));
                // The hurt's own teleport, inside it, as magma's (#1697).
                hurt_teleports(self.m, self.w, self.events);
            }
            // WebBlock.entityInside -> makeStuckInBlock, which a
            // spider ignores for cobwebs and a wither always.
            "minecraft:cobweb"
                if !matches!(
                    self.m.species(),
                    Species::Spider | Species::CaveSpider | Species::Wither
                ) =>
            {
                self.m.fall_distance = 0.0;
                self.m.stuck_multiplier = if self.m.has_effect("minecraft:weaving") {
                    Vec3::new(0.5, 0.25, 0.5)
                } else {
                    Vec3::new(0.25, f64::from(0.05_f32), 0.25)
                };
            }
            "minecraft:sweet_berry_bush"
                if !matches!(self.m.species(), Species::Fox | Species::Bee) =>
            {
                if self.m.species() != Species::Wither {
                    self.m.fall_distance = 0.0;
                    self.m.stuck_multiplier =
                        Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32));
                }
                let moved = self.m.previous.subtract(self.m.position);
                if self.w.prop(x, y, z, "age") != Some("0")
                    && moved.horizontal_distance_squared() > 0.0
                    && (moved.x.abs() >= f64::from(0.003_f32)
                        || moved.z.abs() >= f64::from(0.003_f32))
                {
                    self.m
                        .damage(self.level, 1.0, Damage::of(DamageKind::SweetBerryBush));
                    // The hurt's own teleport, inside it, as magma's (#1697).
                    hurt_teleports(self.m, self.w, self.events);
                }
            }
            // PowderSnowBlock.entityInside: a living entity is stuck
            // (and forgets its fall) only while the block at its
            // own feet is powder snow.
            "minecraft:powder_snow"
                if self.w.block_name(
                    self.m.position.x.floor() as i32,
                    self.m.position.y.floor() as i32,
                    self.m.position.z.floor() as i32,
                ) == "minecraft:powder_snow" =>
            {
                if self.m.species() != Species::Wither {
                    self.m.fall_distance = 0.0;
                    self.m.stuck_multiplier =
                        Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32));
                }
                let known_movement = self
                    .w
                    .controlling_player_movement
                    .unwrap_or(self.m.velocity);
                if let Some(velocity) =
                    powder_snow_particle_velocity(known_movement, self.level.random())
                {
                    self.events.push(MobEvent::PowderSnowParticle {
                        at: [self.m.position.x, f64::from(y + 1), self.m.position.z],
                        velocity,
                    });
                }
            }
            "minecraft:honey_block" => honey_slide(self.m, p, self.level, self.events),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod powder_snow_particle_tests {
    use super::*;

    /// A ridden mount uses the controlling player's known movement for the
    /// powder-snow roll, even when the mount's own delta is zero.
    #[test]
    #[ignore = "daily"]
    fn a_controlled_mount_uses_its_riders_known_movement() {
        let seed = (0..1000)
            .find(|seed| {
                let mut random = sunrise_core::random::LegacyRandom::new(*seed);
                random.next_boolean()
            })
            .unwrap();
        let player_movement = Vec3::new(0.2, 0.0, 0.0);
        let mut expected_random = sunrise_core::random::LegacyRandom::new(seed);
        let expected =
            powder_snow_particle_velocity(player_movement, &mut expected_random).unwrap();

        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                for cx in -1..1 {
                    for cz in -1..1 {
                        world.chunks.insert(
                            (cx, cz),
                            std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                        );
                    }
                }
                world
                    .set(
                        0,
                        64,
                        0,
                        registry.default_state("minecraft:powder_snow").unwrap(),
                    )
                    .unwrap();
            },
            |w, types| {
                let mut w = w.clone();
                w.controlling_player_movement = Some(player_movement);
                let kind = std::sync::Arc::new(types.types["minecraft:horse"].clone());
                let mut horse = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                horse.extra.set_saddled(true);
                horse.rider = Some(crate::mobs::equine::Rider::default());
                let mut level = crate::health::TestLevel::new(seed);
                let mut events = Vec::new();

                effects_from_blocks(&mut horse, &w, &mut level.level(), &mut events);

                assert!(events.iter().any(|event| matches!(
                    event,
                    MobEvent::PowderSnowParticle { velocity, .. } if *velocity == expected
                )));
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }
}

/// `HoneyBlock.entityInside` for a mob: a falling mob beside the honey block at
/// `p` slides down it (`isSlidingDown`, `doSlideMovement`, `maybeDoSlideEffects`).
fn honey_slide(m: &mut Mob, p: [i32; 3], level: &mut DamageLevel<'_>, events: &mut Vec<MobEvent>) {
    let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
    let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
    let [x, y, z] = p.map(f64::from);
    let pos = m.position;
    if m.ground.on_ground || pos.y > y + 0.9375 - 1.0e-7 || old_delta_y(m.velocity.y) >= -0.08 {
        return;
    }
    let dx = (x + 0.5 - pos.x).abs();
    let dz = (z + 0.5 - pos.z).abs();
    let overlap = 0.4375 + f64::from(m.width()) / 2.0;
    if dx + 1.0e-7 <= overlap && dz + 1.0e-7 <= overlap {
        return;
    }
    // doSlideMovement
    let v = m.velocity;
    let old = old_delta_y(v.y);
    m.velocity = if old < -0.13 {
        let factor = -0.05 / old;
        Vec3::new(v.x * factor, new_delta_y(-0.05), v.z * factor)
    } else {
        Vec3::new(v.x, new_delta_y(-0.05), v.z)
    };
    m.fall_distance = 0.0;
    // maybeDoSlideEffects: the slide sound (one time in five, unless silent),
    // then the server's entity event 53 (one time in five).
    let sound = level.random().next_int_bound(5).unwrap_or(1) == 0;
    if sound && !m.silent {
        events.push(MobEvent::Sound(
            "block.honey_block.slide",
            [pos.x, pos.y, pos.z],
            1.0,
            1.0,
        ));
    }
    if level.random().next_int_bound(5).unwrap_or(1) == 0 {
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 53,
            at: [pos.x, pos.y, pos.z],
        });
    }
}
/// LivingEntity.updateSwingTime.
pub fn tick_swing(m: &mut Mob) {
    let duration = m.swing_duration();
    if m.ai.swinging {
        m.ai.swing_time += 1;
        if m.ai.swing_time >= duration {
            m.ai.swing_time = 0;
            m.ai.swinging = false;
            m.ai.swing_duration = 0;
            m.extra.swing_right_arm = None;
            m.extra.swing_stab = false;
        }
    } else {
        m.ai.swing_time = 0;
    }
    m.anim.attack_anim = m.ai.swing_time as f32 / duration as f32;
}
/// Whether the mob's box, shrunk by 1e-5, touches powder snow. It lives here, beside
/// `sun_burn_tick`, so a replacement copy of that function can name it.
pub(crate) fn in_powder_snow(m: &Mob, w: &MobWorld) -> bool {
    let b = m.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let (min, max) = (b.min_position(), b.max_position());
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for y in min.y.floor() as i32..=max.y.floor() as i32 {
            for z in min.z.floor() as i32..=max.z.floor() as i32 {
                if &*w.facts.get(w.world.block(x, y, z)).name == "minecraft:powder_snow" {
                    return true;
                }
            }
        }
    }
    false
}
/// `Entity.isInRain`: rain at the block position, or at the top of the box.
pub(super) fn in_rain(m: &Mob, w: &MobWorld) -> bool {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    let top = (m.position.y + f64::from(m.height())).floor() as i32;
    w.raining_at(x, y, z) || w.raining_at(x, top, z)
}
/// `LivingEntity.baseTick` refills a breather's air by 4, then
/// `Axolotl.handleAirSupply` (a mob with AI): water or rain fills it, anywhere
/// else it runs down from the pre-tick value, and at -20 the axolotl dries out.
fn axolotl_air_step(m: &Mob, w: &MobWorld) -> AirStep {
    let breathes = m.fluids.in_water || in_rain(m, w);
    AirStep::own(m.air, super::axolotl::MAX_AIR, breathes)
}
/// `Nautilus.baseTick`: `LivingEntity.baseTick` refills the air of this
/// water breather, then `Nautilus.handleAirSupply` (a mob with AI) takes it
/// back out of water, where the nautilus dries out.
fn nautilus_air_step(m: &mut Mob) -> Option<AirStep> {
    const MAX_AIR: i32 = 300;
    if !m.alive() {
        m.air = MAX_AIR;
        return None;
    }
    Some(AirStep::own(m.air, MAX_AIR, m.fluids.in_water))
}
/// Apply `breath` to the air `step` decided on, and what running out of air does.
fn finish_air(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    step: AirStep,
    damage: AirDamage,
    breath: Breath,
) {
    let Some(drowning) = step.finish(w.loop_context, m.id, &m.kind.id, &mut m.air, breath) else {
        return;
    };
    match damage {
        AirDamage::Generic => {
            events.push(MobEvent::Drown {
                mob: m.id,
                at: [m.position.x, m.position.y, m.position.z],
                bubbles: drowning.bubbles,
            });
            if let Some(amount) = drowning.damage {
                m.damage(level, amount, Damage::of(DamageKind::Drown));
                hurt_teleports(m, w, events);
            }
        }
        AirDamage::Drown | AirDamage::DryOut => {
            if let Some(amount) = drowning.damage {
                let kind = if matches!(damage, AirDamage::Drown) {
                    DamageKind::Drown
                } else {
                    DamageKind::DryOut
                };
                m.damage(level, amount, Damage::of(kind));
            }
        }
    }
}
/// The thin box at a body's eyes `Entity.isInWall` tests: 0.8 of the width wide (a float
/// multiplication) and 1e-6 high.
fn eye_bounds(width: f32, eye: [f64; 3]) -> Aabb {
    let side = f64::from(width * 0.8_f32);
    let half = [side / 2.0, 1.0e-6 / 2.0, side / 2.0];
    let lo = [0, 1, 2].map(|i| eye[i] - half[i]);
    let hi = [0, 1, 2].map(|i| eye[i] + half[i]);
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}
/// `Entity.isInWall` (minus the noPhysics early-out, which the caller applies):
/// visit every cell covered by the thin eye box and ask whether a suffocating
/// collision shape actually intersects it. Width multiplication is a float.
fn in_wall(
    width: f32,
    eye: [f64; 3],
    mut intersects: impl FnMut(i32, i32, i32, &Aabb) -> bool,
) -> bool {
    let bounds = eye_bounds(width, eye);
    let lo = [
        bounds.min(Axis::X),
        bounds.min(Axis::Y),
        bounds.min(Axis::Z),
    ];
    let hi = [
        bounds.max(Axis::X),
        bounds.max(Axis::Y),
        bounds.max(Axis::Z),
    ];
    let c0 = lo.map(|v| v.floor() as i32);
    let c1 = hi.map(|v| v.floor() as i32);
    (c0[0]..=c1[0])
        .any(|x| (c0[1]..=c1[1]).any(|y| (c0[2]..=c1[2]).any(|z| intersects(x, y, z, &bounds))))
}
/// The state read after stepOn, just before `Entity.checkInsideBlocks`.
#[derive(Debug)]
struct InsideTemperature {
    burning: bool,
    freezing: bool,
    fire_ticks: i32,
}

impl InsideTemperature {
    fn read(m: &Mob) -> Self {
        Self {
            burning: m.is_on_fire(),
            freezing: m.ticks_frozen > 0,
            fire_ticks: m.fire_ticks,
        }
    }

    fn finish(self, m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
        if in_rain(m, w) {
            m.fire_ticks = m.fire_ticks.min(0);
        }
        if w.loop_context.side() == sunrise_world::native_load_context::LoopSide::Server {
            if self.burning && !m.is_on_fire() || self.freezing && m.ticks_frozen == 0 {
                events.push(MobEvent::Sound(
                    "entity.generic.extinguish_fire",
                    m.position.components(),
                    0.7,
                    1.6 + (m.random.next_float() - m.random.next_float()) * 0.4,
                ));
            }
            if !m.is_on_fire() && m.fire_ticks <= self.fire_ticks {
                m.fire_ticks = 0;
            }
        }
    }
}

/// `LivingEntity.canBreatheUnderwater`: the live tag, plus the separate
/// overrides for a baby happy ghast or a sulfur cube carrying a block.
fn can_breathe_under_water(m: &Mob, w: &MobWorld<'_>) -> bool {
    w.entity_tag_bit(
        &m.kind,
        crate::items::EntityTypeTags::CAN_BREATHE_UNDER_WATER,
    ) || m.species() == Species::HappyGhast && m.is_baby()
        || m.species() == Species::SulfurCube && crate::mobs::sulfur_cube::has_body(m)
}

/// `IronGolem.decreaseAirSupply` hands back the air it is given (`iload_1;
/// ireturn`), so a golem under water is in `baseTick`'s drowning branch with a
/// consume of 0: it never loses a bubble, never reaches the -20 that hurts, and
/// never refills either, though the tag does not list its type.
fn keeps_its_air(m: &Mob) -> bool {
    m.species() == Species::IronGolem
}

/// `LivingEntity.decreaseAirSupply` starts with `getAttribute(OXYGEN_BONUS)`, and
/// `AttributeMap.getInstance` makes the instance, which `AttributeMap.pack` saves from then on.
fn make_oxygen_bonus(m: &mut Mob, w: &MobWorld) {
    if m.extra.oxygen_bonus_made {
        return;
    }
    m.extra.oxygen_bonus_made = true;
    let seed: Vec<_> = if m.extra.live.built().is_none() {
        crate::entity_nbt::attribute_entries(m)
            .into_iter()
            .map(|(_, entry)| entry)
            .collect()
    } else {
        Vec::new()
    };
    let kind = m.kind.clone();
    m.extra.live.edit(
        w.attributes,
        &kind.id,
        "minecraft:oxygen_bonus",
        true,
        move || seed,
        |_, _| Ok(()),
    );
}

/// `base_tick` finished, or stopped at its air update for the owner to ask
/// the mods (`living_breathe`) with a host.
enum BaseTick {
    Done,
    Air(AirPause),
    AfterDamage { next: BaseStage },
}

fn base_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> BaseTick {
    if m.first_tick {
        super::villager::refresh_sleeping_position(m, w);
    }
    // Entity.canSpawnSprintParticle / spawnSprintParticle (a dashing cat or
    // ocelot): a server level drops the particle, but its spot is drawn
    // from the mob's random first.
    // `IronGolem.canSpawnSprintParticle`: moving, and a one in five draw, sprinting or not.
    let spawn = if m.species() == Species::IronGolem {
        let horizontal = m.velocity.x * m.velocity.x + m.velocity.z * m.velocity.z;
        horizontal > f64::from(2.500_000_3e-7_f32) && m.random.next_int_bound(5).unwrap_or(1) == 0
    } else {
        m.extra.sprinting && !m.fluids.in_water && !m.fluids.in_lava && m.alive()
    };
    if spawn {
        let at = m.block_on_legacy(w);
        let state = w.world.block(at[0], at[1], at[2]);
        if w.facts.get(state).render_shape != crate::blocks::RenderShape::Invisible {
            let _ = (m.random.next_double(), m.random.next_double());
        }
    }
    // Entity.baseTick: `wasInPowderSnow` keeps last tick's walk, and
    // `isInPowderSnow` waits for this tick's walk to set it.
    m.was_in_powder_snow = m.in_powder_snow;
    m.in_powder_snow = false;
    m.was_eye_in_water = m.fluids.eye_in_water;
    m.update_fluids(w, events);
    if m.species() == Species::Drowned {
        super::undead::drowned_update_swimming(m, w);
    }
    // Entity.baseTick counts burn damage and the timer before a wet mob is put out.
    if m.fire_ticks > 0 && m.fire_immune() {
        m.fire_ticks = 0;
    }
    let burning = m.fire_ticks > 0;
    // `Entity.isInLava` is false on the first tick, so a mob loaded in lava burns then.
    if burning && m.fire_ticks % 20 == 0 && (m.first_tick || !m.fluids.in_lava) {
        m.damage(level, 1.0, Damage::of(DamageKind::OnFire));
        hurt_teleports(m, w, events);
        if m.hurt_needs_owner() {
            return BaseTick::AfterDamage {
                next: BaseStage::AfterBurn,
            };
        }
    }
    continue_base_tick(m, w, level, events, BaseStage::AfterBurn)
}

fn continue_base_tick(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    mut next: BaseStage,
) -> BaseTick {
    loop {
        next = match next {
            BaseStage::AfterBurn => {
                if m.fire_ticks > 0 {
                    m.fire_ticks -= 1;
                }
                // Entity.checkBelowWorld: LivingEntity.onBelowWorld hurts 4 a tick.
                if m.position.y < f64::from(w.world.bottom_y() - 64) {
                    m.damage(level, 4.0, Damage::of(DamageKind::OutOfWorld));
                    hurt_teleports(m, w, events);
                    if m.hurt_needs_owner() {
                        return BaseTick::AfterDamage {
                            next: BaseStage::AfterVoid,
                        };
                    }
                }
                BaseStage::AfterVoid
            }
            BaseStage::AfterVoid => {
                m.first_tick = false;
                if !m.alive() {
                    next = BaseStage::Effects;
                    continue;
                }
                // LivingEntity.baseTick: a suffocating shape intersects the eye. Entity.isInWall
                // is false under noPhysics, which Vex.tick holds while it ticks.
                let mut scene = crate::collisions::SceneCollisions {
                    world: w.world,
                    binding: w.collision,
                    subject: None,
                };
                // A collider box at the eyes is a wall as well: the crush a moving collider gives.
                let in_collider = m.species() != Species::Vex
                    && sunrise_world::moving_colliders::overlaps(
                        sunrise_world::entity_obstruction::EntityIdentity(u64::from(m.id)),
                        eye_bounds(m.width(), [m.position.x, m.eye_y(), m.position.z]),
                    );
                if in_collider
                    || m.species() != Species::Vex
                        && in_wall(
                            m.width(),
                            [m.position.x, m.eye_y(), m.position.z],
                            |x, y, z, eye_bounds| {
                                let p = [x, y, z];
                                let state = w.world.block(x, y, z);
                                if !w
                                    .suffocation
                                    .is_suffocating(&mut scene, state, p)
                                    .unwrap_or(false)
                                {
                                    return false;
                                }
                                w.collision
                                    .collision_shape(&mut scene, state, p)
                                    .ok()
                                    .is_some_and(|shape| {
                                        let Ok(eye_shape) = VoxelShape::from_box(*eye_bounds)
                                        else {
                                            return false;
                                        };
                                        shape
                                            .moved(Vec3::new(
                                                f64::from(x),
                                                f64::from(y),
                                                f64::from(z),
                                            ))
                                            .join_is_not_empty(&eye_shape, ShapeOp::AND)
                                            .unwrap_or(false)
                                    })
                            },
                        )
                {
                    m.damage(level, 1.0, Damage::of(DamageKind::InWall));
                    hurt_teleports(m, w, events);
                    if m.hurt_needs_owner() {
                        return BaseTick::AfterDamage {
                            next: BaseStage::Air,
                        };
                    }
                }
                BaseStage::Air
            }
            BaseStage::Air => {
                // Air supply and drowning (WaterAnimal.handleAirSupply for aquatic mobs).
                // Axolotl.baseTick and Nautilus.baseTick: NoAI skips only their own
                // handleAirSupply, so a NoAI one takes LivingEntity.baseTick's path below,
                // under the liveness check made before the in-wall hurt.
                let step = if m.species() == Species::Axolotl && !m.no_ai {
                    Some((axolotl_air_step(m, w), AirDamage::DryOut))
                } else if m.species() == Species::Nautilus && !m.no_ai {
                    nautilus_air_step(m).map(|step| (step, AirDamage::DryOut))
                } else if m.kind.flags.needs_water && m.species() != Species::Nautilus {
                    Some((
                        AirStep::own(m.air, 300, m.fluids.in_water),
                        AirDamage::Drown,
                    ))
                } else {
                    // LivingEntity.baseTick / MobEffectUtil: bubble columns and breathing
                    // effects protect submerged mobs. Nautilus alone holds air rather
                    // than refilling it; Water Breathing or Conduit Power still refill.
                    let submerged = m.fluids.eye_in_water
                        && w.block_name(
                            m.position.x.floor() as i32,
                            m.eye_y().floor() as i32,
                            m.position.z.floor() as i32,
                        ) != "minecraft:bubble_column";
                    let water_breathing = m.has_effect("minecraft:water_breathing")
                        || m.has_effect("minecraft:conduit_power");
                    let nautilus = m.has_effect("minecraft:breath_of_the_nautilus");
                    let drowns = submerged
                        && !can_breathe_under_water(m, w)
                        && !water_breathing
                        && !nautilus;
                    if drowns && !keeps_its_air(m) {
                        make_oxygen_bonus(m, w);
                    }
                    let held = submerged && nautilus && !water_breathing;
                    let max = m.max_air_supply();
                    let vanilla = Breath {
                        can_breathe: !drowns,
                        consume: if keeps_its_air(m) { 0 } else { 1 },
                        // `increaseAirSupply`: four a tick, except the dolphin's own, which
                        // hands back the whole 4800 maximum however little air it has.
                        refill: if drowns || held {
                            0
                        } else if m.species() == Species::Dolphin {
                            max.saturating_sub(m.air)
                        } else {
                            4
                        },
                    };
                    Some((AirStep::generic(m.air, max, vanilla), AirDamage::Generic))
                };
                if let Some((step, damage)) = step {
                    // The owner asks `living_breathe` with its host between deciding and applying.
                    if crate::breathing::heard() {
                        return BaseTick::Air(AirPause {
                            mob: m.id,
                            step,
                            join: m.join,
                            damage,
                        });
                    }
                    finish_air(m, w, level, events, step, damage, step.vanilla);
                    if m.hurt_needs_owner() {
                        return BaseTick::AfterDamage {
                            next: BaseStage::Effects,
                        };
                    }
                }
                BaseStage::Effects
            }
            BaseStage::Effects | BaseStage::EffectCountdown(_) => {
                let cursor = match next {
                    BaseStage::EffectCountdown(cursor) => cursor,
                    _ => {
                        if m.species() == Species::Illusioner {
                            super::illusioner::tick(m, events);
                        }
                        if matches!(m.species(), Species::Evoker | Species::Illusioner) {
                            super::illager::cast_particles(m, events);
                        }
                        level.tick_player_memory(m);
                        if let Some((id, _)) = m.last_hurt_mob {
                            let alive = if id == m.id {
                                m.alive()
                            } else if let Some(other) = w.live_mobs.and_then(|live| live.find(id)) {
                                other.alive()
                            } else {
                                w.target_info(Who::Mob(id)).map_or_else(
                                    || w.side.living_target_alive(id),
                                    |target| target.alive,
                                )
                            };
                            if !alive {
                                m.last_hurt_mob = None;
                            }
                        }
                        super::hurt_by::base_tick(m, w);
                        EffectPass::new(m)
                    }
                };
                // LivingEntity.baseTick runs tickEffects after the timers count down.
                if let Some(cursor) = tick_effects_until_hurt(
                    m,
                    w.loop_context,
                    level,
                    w.mod_effects,
                    &mut |m| hurt_teleports(m, w, events),
                    true,
                    cursor,
                ) {
                    return BaseTick::AfterDamage {
                        next: BaseStage::EffectCountdown(cursor),
                    };
                }
                BaseStage::Finish
            }
            BaseStage::Finish => {
                if !m.alive() {
                    m.death_time += 1;
                }
                // Hurt sounds of the damage above come before the ambient sound.
                events.append(&mut m.outbox);
                // Mob.baseTick: ambient sounds.
                if m.alive() {
                    let roll = m.random.next_int_bound(1000).unwrap_or(999);
                    let t = m.ambient_timer;
                    // `ambientSoundTime++` and `-getAmbientSoundInterval()`, in int arithmetic.
                    m.ambient_timer = m.ambient_timer.wrapping_add(1);
                    if roll < t {
                        m.ambient_timer = m.ambient_interval().wrapping_neg();
                        super::entity_sounds::ambient(m, w, events);
                    }
                }
                // Entity.baseTick ends with Leashable.tickLeash.
                super::leash::tick(m, w, events);
                return BaseTick::Done;
            }
        };
    }
}
/// `LivingEntity.aiStep`'s freezing, after movement and the block walk and
/// before cramming and the water's hurt, on `isInPowderSnow` as the tick's
/// block walk left it (`m.in_powder_snow`).
fn freeze_tick(m: &mut Mob, w: &MobWorld, level: &mut DamageLevel<'_>, events: &mut Vec<MobEvent>) {
    let in_snow = m.in_powder_snow;
    if !in_snow && m.ticks_frozen == 0 {
        return;
    }
    let can_freeze = can_freeze(m, w);
    if !in_snow || !can_freeze {
        m.ticks_frozen = m.ticks_frozen.wrapping_sub(2).max(0);
    }
    if m.age.is_multiple_of(40) && m.is_fully_frozen() && can_freeze {
        m.damage(level, 1.0, Damage::of(DamageKind::Freeze));
        // Enderman.hurtServer and Shulker.hurtServer teleport inside the hurt (#1697).
        hurt_teleports(m, w, events);
    }
}
/// `Mob.playAmbientSound` on demand: a spawn egg or a mob bucket just placed `m`.
pub fn ambient_sound(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    super::entity_sounds::ambient(m, w, events);
}
pub(super) fn play_ambient(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if super::axolotl::is_playing_dead(m) {
        return;
    }
    let sound = match m.species() {
        // Warden.getAmbientSound: by anger, silent while roaring or digging.
        Species::Warden => match super::warden::ambient_sound(m) {
            Some(sound) => sound,
            None => return,
        },
        Species::Zombie | Species::Husk if m.is_baby() => m.kind.sounds[0],
        Species::Drowned if m.fluids.in_water => "entity.drowned.ambient_water",
        // Nautilus/ZombieNautilus.getAmbientSound: by age, under water or on land.
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::sounds(m)[0],
        Species::Enderman if m.ai.target.is_some() => "entity.enderman.scream",
        // Wolf.isAngry reads the persistent anger timer, not the current target.
        Species::Wolf => {
            play_wolf_ambient(m, w.side.variant_tables(), events);
            return;
        }
        Species::Pig | Species::Cow | Species::Chicken => {
            play_farm_ambient(m, w.side.variant_tables(), events);
            return;
        }
        Species::Parrot => super::parrot::ambient(m, w),
        // Cat.getAmbientSound: purr in love, purreow one time in four, else ambient (stray_ambient when wild).
        Species::Cat => {
            play_cat_ambient(m, w.side.variant_tables(), events);
            return;
        }
        // Fox.getAmbientSound: a sleeping fox makes its sleep sound.
        Species::Fox if super::fox::flag(m, super::fox::SLEEPING) => "entity.fox.sleep",
        // Fox.playAmbientSound: the screech at volume 2. Anything else goes to
        // Mob.playAmbientSound, which asks getAmbientSound again (a second draw).
        Species::Fox => {
            if super::fox::awake_ambient(m, w) == super::fox::SCREECH {
                let pitch = m.voice_pitch();
                let at = [m.position.x, m.position.y, m.position.z];
                events.push(MobEvent::Sound(super::fox::SCREECH, at, 2.0, pitch));
                return;
            }
            super::fox::awake_ambient(m, w)
        }
        // Turtle.getAmbientSound: the land ambient only on land and grown up, else silent.
        Species::Turtle if m.fluids.in_water || !m.ground.on_ground || m.is_baby() => return,
        Species::PolarBear if m.is_baby() => "entity.polar_bear.ambient_baby",
        // Turtle.getAmbientSound: only an adult on land makes a sound.
        Species::Turtle if turtle_ambient_silent(m) => return,
        Species::Dolphin if !m.fluids.in_water => "entity.dolphin.ambient",
        Species::Guardian if !m.fluids.in_water => "entity.guardian.ambient_land",
        Species::ElderGuardian if !m.fluids.in_water => "entity.elder_guardian.ambient_land",
        Species::Axolotl if !m.fluids.in_water => "entity.axolotl.idle_air",
        // Villager.getAmbientSound: silent while asleep, the trade sound
        // while a player has its menu open.
        Species::Villager if super::villager::sleeping(m) => return,
        Species::Villager if m.extra.flag2 => "entity.villager.trade",
        // WanderingTrader.getAmbientSound: only the trade branch, it never sleeps.
        Species::WanderingTrader if m.extra.flag2 => "entity.wandering_trader.trade",
        Species::Bat if m.extra.flag && m.random.next_int_bound(4).unwrap_or(1) != 0 => return,
        Species::Allay if m.extra.main_hand_id().is_some() => "entity.allay.ambient_with_item",
        // Strider.getAmbientSound: quiet while panicking or tempted.
        Species::Strider if m.ai.panicking || m.ai.goals.is_running("Tempt") => return,
        // HappyGhast: its own voice; `Mob::voice` plays it at the feet
        // (`Entity.playSound`), never for a Silent ghast, at pitch 1.0.
        Species::HappyGhast => super::happy_ghast::sounds(m)[0],
        // Sniffer.getAmbientSound.
        Species::Sniffer => match super::sniffer::ambient(m) {
            Some(s) => s,
            None => return,
        },
        // Breeze.playAmbientSound: a local sound, nothing drawn.
        Species::Breeze => {
            if let Some(s) = super::breeze::ambient(m) {
                let at = [m.position.x, m.position.y, m.position.z];
                events.push(MobEvent::LocalSound(s, at, 1.0, 1.0));
            }
            return;
        }
        // Creaking.getAmbientSound: silent while awake.
        Species::Creaking if m.extra.flag => return,
        // Shulker.playAmbientSound: silent while closed.
        Species::Shulker if m.extra.counter == 0 => return,
        Species::ZombifiedPiglin if m.ai.target.is_some() => "entity.zombified_piglin.angry",
        Species::Piglin => super::piglin::piglin_ambient(m, w),
        Species::Hoglin => super::piglin::hoglin_ambient(m, w),
        // Zoglin.getAmbientSound: with an attack target, the angry sound.
        Species::Zoglin if m.ai.target.is_some() => "entity.zoglin.angry",
        _ => super::entity_sounds::voices(m)[0],
    };
    let _ = w;
    if !sound.is_empty() {
        m.voice(sound, events);
    }
}
/// `Turtle.getAmbientSound`: `entity.turtle.ambient_land` only when the turtle
/// is out of water, on the ground and not a baby; otherwise `null` (silent).
fn turtle_ambient_silent(m: &Mob) -> bool {
    m.fluids.in_water || !m.ground.on_ground || m.is_baby()
}
/// `Animal.aiStep`: age-based love clearing and countdown, following the age step.
/// Both the normal and NoAI paths run it after the age step.
fn animal_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.alive() {
        if m.extra.age != 0 {
            m.extra.in_love = 0;
        }
        if m.extra.in_love > 0 {
            m.extra.in_love -= 1;
            if m.extra.in_love % 10 == 0 {
                // The heart's speed and getRandomX/Y/Z, drawn even on the server.
                let (w, h) = (f64::from(m.width()), f64::from(m.height()));
                let r = &mut m.random;
                let velocity = [
                    r.next_gaussian() * 0.02,
                    r.next_gaussian() * 0.02,
                    r.next_gaussian() * 0.02,
                ];
                let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
                let y = m.position.y + h * r.next_double() + 0.5;
                let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
                events.push(MobEvent::ParticleAt {
                    kind: "heart",
                    at: [x, y, z],
                    velocity,
                });
            }
        }
    }
}
/// `AgeableMob.aiStep` runs even when `Mob.serverAiStep` is disabled.
fn ageable_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    // AgeableMob.aiStep: a forced age-up's particles (`sendParticles`, so the
    // players are sent them), then the age.
    if m.extra.forced_age_timer > 0 {
        if m.extra.forced_age_timer % 4 == 0 {
            let (w, h) = (f64::from(m.width()), f64::from(m.height()));
            let r = &mut m.random;
            let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
            let y = m.position.y + h * r.next_double() + 0.5;
            let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
            // sendParticles(HAPPY_VILLAGER, x, y, z, 1, 0, 0, 0, 0): exactly there, at rest.
            events.push(MobEvent::LevelParticles {
                kind: "happy_villager",
                at: [x, y, z],
                spread: [0.0; 3],
                speed: 0.0,
                count: 1,
            });
        }
        m.extra.forced_age_timer -= 1;
    }
    // AgeableMob.aiStep: an age counts only for its subclasses (a tadpole
    // counts its own age up, a zombie's baby age never moves). `canAgeUp`:
    // a baby grows unless an age lock holds it; a breeding pause counts down.
    if m.alive() && super::finalize::ageable(m.species()) {
        if can_age_up(m) {
            let (bw, bh, _) = m.dims();
            // AgeableMob.setAge(getAge() + 1): the crossing is where a species'
            // ageBoundaryReached runs (a goat's and a hoglin's attack damage, a
            // sulfur cube's size), which the age write used to skip.
            let next = m.extra.age + 1;
            super::finalize::set_age(m, next, Some(w));
            // setAge at the age boundary: refreshDimensions.
            if m.extra.age >= 0 {
                m.fudge_position_after_size_change(w, bw, bh);
                if m.species() == Species::Villager {
                    super::villager_brain::age_boundary_reached(m, w, level, events);
                }
                // Turtle.ageBoundaryReached: a grown turtle drops a scute.
                if m.species() == Species::Turtle {
                    events.push(MobEvent::GiftLoot {
                        mob: m.id,
                        table: "minecraft:gameplay/turtle_grow",
                        at: [m.position.x, m.position.y, m.position.z],
                    });
                }
            }
        } else if m.extra.age > 0 {
            m.extra.age -= 1;
        }
    }
    if super::finalize::ageable(m.species()) {
        make_age_locked_particle(m, events);
    }
}

/// `LivingEntity.tick` after `aiStep`: the glide's tick count.
fn count_fall_fly_ticks(m: &mut Mob) {
    m.extra.fall_fly_ticks = if m.extra.fall_flying {
        m.extra.fall_fly_ticks + 1
    } else {
        0
    };
}

/// `LivingEntity.aiStep`: both flight effects discard accumulated fall before
/// travel, even when NoAI skips the server AI step.
fn reset_effect_fall_distance(m: &mut Mob) {
    if m.has_effect("minecraft:slow_falling") || m.has_effect("minecraft:levitation") {
        m.fall_distance = 0.0;
    }
}

/// LivingEntity.aiStep (and Mob/AgeableMob/Animal additions).
fn ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    pause_movement: bool,
) -> ControlFlow<AiPause> {
    if m.species() == Species::Parrot {
        super::parrot::ai_step(m, w, level.random(), events);
    }
    if m.species() == Species::Fox {
        super::fox::pre_ai_step(m);
    }
    // Blaze.aiStep slows the fall before super.aiStep(), so before travel.
    if m.species() == Species::Blaze {
        super::nether::blaze_ai_step(m);
    }
    // AbstractHorse.aiStep: the tail swishes now and then (moveTail).
    if super::equine::is_equine(m.species()) {
        super::equine::tail_tick(m);
    }
    // Monster.aiStep: updateNoActionTime (Raider: always +2).
    if is_monster(m.species())
        && (super::raider::is_raider(m.species()) || super::hostile::magic_value(m, w) > 0.5)
    {
        // A Java `int`: past `i32::MAX` it wraps, as there.
        m.ai.no_action_time = m.ai.no_action_time.wrapping_add(2);
    }
    if m.ai.no_jump_delay > 0 {
        m.ai.no_jump_delay -= 1;
    }
    drop_tiny_motion(m);
    m.ai.xxa *= 0.98;
    m.ai.zza *= 0.98;
    // LivingEntity.isImmobile, no serverAiStep or input: a stunned ravager, a rearing horse, a paused mob.
    let immobile = super::ai_pause::hold(m, w)
        || (m.species() == Species::Ravager && super::ravager::immobile(m))
        || super::equine::immobile(m)
        || m.species() == Species::Mannequin;
    if matches!(m.species(), Species::Piglin | Species::Hoglin) && m.alive() {
        super::piglin::before_goals(m, w, level.random(), events);
    }
    if m.alive() && !immobile {
        server_ai_step(m, w, level, events, pause_movement)?;
    } else {
        m.ai.jumping = false;
        m.ai.xxa = 0.0;
        m.ai.zza = 0.0;
    }
    ai_step_after_server_ai(m, w, level, events, pause_movement)
}

fn ai_step_after_server_ai(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    pause_movement: bool,
) -> ControlFlow<AiPause> {
    m.handle_jumping(w);
    let input = Vec3::new(
        f64::from(m.ai.xxa),
        f64::from(m.ai.yya),
        f64::from(m.ai.zza),
    );
    update_fall_flying(m, w, events);
    reset_effect_fall_distance(m);
    if super::equine::moved_by_client(m) {
        // LivingEntity.travelRidden: the rider's client moves it
        // (handleMoveVehicle), so the server only stops it.
        m.velocity = Vec3::ZERO;
    } else if !(m.species() == Species::Mannequin && m.extra.immovable) {
        // Mannequin.isEffectiveAi: an immovable one never travels.
        m.travel_until_pause(w, level, input, events, pause_movement)
            .map_break(|travel| AiPause::Movement(MovementPhase::Travel(travel)))?;
        // A fall's hurt (checkFallDamage) teleports at once.
        hurt_teleports(m, w, events);
    }
    ControlFlow::Continue(())
}
/// The end of `LivingEntity.aiStep`: a mob that water hurts (an enderman, a
/// blaze, a snow golem) takes 1 drowning damage in water or rain
/// (`isSensitiveToWater`, `isInWaterOrRain`). An enderman's teleport is part of
/// that hurt (`Enderman.hurtServer`), so it happens now, not next tick.
fn hurt_by_water(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    if m.kind.flags.water_sensitive && (m.fluids.in_water || in_rain(m, w)) {
        m.damage(level, 1.0, Damage::of(DamageKind::Drown));
        hurt_teleports(m, w, events);
    }
}
/// A hurt the mob's own tick deals (`Entity.baseTick`, `tickEffects`, a
/// block's touch or a mod part's queued hurt, freezing, cramming), with the
/// teleport `Enderman.hurtServer` or `Shulker.hurtServer` makes inside it: a
/// second hurt of the same tick draws after the first one's teleport and sees
/// where it landed.
#[cfg(test)]
fn hurt(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>, amount: f32, source: Damage) {
    m.damage(
        &mut crate::health::TestLevel::new(0).level(),
        amount,
        source,
    );
    hurt_teleports(m, w, events);
}
/// The teleport `Enderman.hurtServer` or `Shulker.hurtServer` makes inside a
/// hurt, after the hurt's sound; with or without AI.
fn hurt_teleports(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.has_protected_hurt() {
        return;
    }
    if m.species() == Species::Enderman && m.extra.counter2 > 0 {
        events.append(&mut m.outbox);
        super::enderman::run_pending_teleports(m, w, events);
    }
    if m.species() == Species::Shulker && m.extra.flag {
        events.append(&mut m.outbox);
        super::shulker::hurt_teleport(m, w, events);
    }
}
impl Mob {
    /// LivingEntity.isPushable (and the push it takes from `Entity.push(Entity)`):
    /// bats and shulkers are never moved by others, nor is a mob on a climbable
    /// block (`onClimbable`, cached by the last tick), nor a ridden horse
    /// (`AbstractHorse.isPushable` is `!isVehicle`).
    pub fn pushable(&self) -> bool {
        self.pushable_on(self.on_climbable)
    }

    fn pushable_on(&self, climbable: bool) -> bool {
        self.alive()
            && !matches!(self.species(), Species::Bat | Species::Shulker)
            && !climbable
            && (self.species() != Species::Creaking || super::creaking::can_move(self))
            && (self.species() != Species::Warden || !super::warden::digging_or_emerging(self))
            && !(super::equine::is_equine(self.species()) && super::equine::is_vehicle(self))
    }
}
/// LivingEntity.onClimbable for the pusher (Spider.onClimbable: climbing a wall).
fn on_climbable(m: &Mob, w: &MobWorld) -> bool {
    m.on_climbable(w)
}
/// `LivingEntity.aiStep`, with or without AI: a mob's motion below 0.003 on an
/// axis stops.
fn drop_tiny_motion(m: &mut Mob) {
    let v = m.velocity;
    m.velocity = Vec3::new(
        if v.x.abs() < 0.003 { 0.0 } else { v.x },
        if v.y.abs() < 0.003 { 0.0 } else { v.y },
        if v.z.abs() < 0.003 { 0.0 } else { v.z },
    );
}
/// LivingEntity.pushEntities: every pushable entity overlapping the box is
/// pushed apart from this one (`doPush(e)` = `e.push(this)`); pushes of other
/// mobs are `Push` events their tick picks up.
#[cfg(test)]
fn push_entities(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    push_entities_until_hurt(m, w, level, events, false);
}

fn push_entities_until_hurt(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    can_pause: bool,
) -> Option<PushPhase> {
    m.on_climbable = on_climbable(m, w);
    if m.species() == Species::Bat {
        return None;
    }
    let bounds = m.bounds();
    // Selection (including the unmounted count) stays fixed across cramming hurt.
    let mut targets = super::world::take_push_targets();
    if let Some(p) = w.selected_player()
        // Entity.push: passengers of the same vehicle never push each other.
        && m.rider.is_none()
        && p.alive
        && !p.spectator
        && p.bounds.intersects(bounds)
    {
        targets.push((Who::Player, p.passenger, None));
    }
    for o in w.others_near(bounds) {
        if o.id != m.id && o.alive && o.pushable && o.bounds.intersects(bounds) {
            targets.push((Who::Mob(o.id), o.passenger, None));
        }
    }
    // EntitySelector.pushableBy(this): the teams' collision rules.
    let own = w.teams.of(Who::Mob(m.id));
    targets.retain(|(who, ..)| crate::scoreboard::pushable_by(own, w.teams.of(*who), true));
    // Cramming: a quarter of the ticks spent with `maxEntityCramming` or more
    // others around that are not riding anything hurts it by 6.
    let limit = w.max_entity_cramming;
    if limit > 0
        && targets.len() as i64 > i64::from(limit) - 1
        && m.random.next_int_bound(4).unwrap_or(1) == 0
        && targets.iter().filter(|t| !t.1).count() as i64 > i64::from(limit) - 1
    {
        m.damage(level, 6.0, Damage::of(DamageKind::Cramming));
        // The hurt's own teleport, inside it (#1697).
        hurt_teleports(m, w, events);
        if can_pause && m.hurt_needs_owner() {
            targets.retain_mut(|(who, _, join)| match who {
                Who::Mob(id) => {
                    *join = w
                        .live_mobs
                        .and_then(|mobs| mobs.find(*id))
                        .map(|mob| mob.join);
                    join.is_some()
                }
                Who::Player => w.selected_player_identity().is_some(),
            });
            return Some(PushPhase {
                targets,
                player_before_hurt: w.selected_player_identity(),
            });
        }
    }
    apply_entity_pushes(m, w, events, targets, false, None);
    None
}

fn apply_entity_pushes(
    m: &mut Mob,
    w: &MobWorld,
    events: &mut Vec<MobEvent>,
    targets: Vec<super::world::PushTarget>,
    resumed: bool,
    player_before_hurt: Option<([i32; 4], u32)>,
) {
    if resumed {
        m.on_climbable = on_climbable(m, w);
        if m.no_physics {
            super::world::give_push_targets(targets);
            return;
        }
    }
    let self_pushable = m.pushable() && !super::equine::is_vehicle(m);
    let root = root_vehicle(w, m.id, m.extra.mount);
    for &(who, _, join) in &targets {
        let (at, sleeping, same, pushable) = match who {
            Who::Player => {
                let Some(player) = w.selected_player() else {
                    continue;
                };
                let identity = w.selected_player_identity();
                if resumed && identity != player_before_hurt {
                    continue;
                }
                let same = identity
                    .is_some_and(|(_, id)| root_vehicle(w, id, player.riding_vehicle) == root);
                (
                    player.position,
                    player.sleeping,
                    same,
                    !player.remote && player.alive && !player.spectator,
                )
            }
            Who::Mob(id) => {
                let Some(peer) = w.seen(id).filter(|peer| !peer.removed) else {
                    continue;
                };
                let pushable = if let Some(join) = join {
                    let Some(live) = w
                        .live_mobs
                        .and_then(|mobs| mobs.find(id))
                        .filter(|mob| mob.join == join && !mob.removed && !mob.no_physics)
                    else {
                        continue;
                    };
                    live.pushable_on(on_climbable(live, w))
                } else {
                    peer.pushable
                };
                (
                    peer.position,
                    peer.sleeping,
                    root_vehicle(w, id, peer.vehicle) == root,
                    pushable && !peer.is_vehicle,
                )
            }
        };
        // Warden.doPush: touching it angers it, then the push itself.
        if m.species() == Species::Warden {
            super::warden::do_push(m, w, who, events);
        }
        // IronGolem.doPush: a monster that is not a creeper becomes its target
        // one time in 20, and the draw is made for those alone, then the push.
        if m.species() == Species::IronGolem
            && let Who::Mob(id) = who
            && w.seen(id).is_some_and(super::golem::golem_prey)
            && m.random.next_int_bound(20).unwrap_or(1) == 0
        {
            m.set_ai_target(w, Some(who));
        }
        // doPush calls the other living entity's push; sleeping vetoes both impulses.
        if sleeping {
            continue;
        }
        // Entity.push: passengers of the same vehicle (a rider and its mount
        // among them) never push each other.
        if same {
            continue;
        }
        // Entity.push(Entity) with `this` the other entity.
        let (mut xa, mut za) = (m.position.x - at.x, m.position.z - at.z);
        let mut dd = xa.abs().max(za.abs());
        if dd < f64::from(0.01_f32) {
            continue;
        }
        dd = dd.sqrt();
        xa /= dd;
        za /= dd;
        let pow = (1.0 / dd).min(1.0);
        xa *= pow;
        za *= pow;
        xa *= f64::from(0.05_f32);
        za *= f64::from(0.05_f32);
        // A guest's server-side push never reaches its client (ServerPlayer
        // motion is synced only when hurt): its client pushes it itself.
        if pushable {
            let velocity = [-xa, 0.0, -za];
            if let (Who::Mob(id), Some(join)) = (who, join) {
                events.push(MobEvent::PushJoined { id, join, velocity });
            } else {
                events.push(w.route_player_event(MobEvent::Push {
                    target: who,
                    velocity,
                }));
            }
        }
        if self_pushable {
            m.velocity = Vec3::new(m.velocity.x + xa, m.velocity.y, m.velocity.z + za);
        }
    }
    super::world::give_push_targets(targets);
}
/// `Mob.aiStep` -> `Mob.burnUndead`: mobs in `minecraft:burn_in_daylight`
/// can burn in daylight even with `NoAI`.
fn burn_undead(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if w.entity_tag_bit(&m.kind, crate::items::EntityTypeTags::BURN_IN_DAYLIGHT)
        && m.alive()
        && sun_burn_tick(m, w)
    {
        if m.item_by_slot(m.sun_protection_slot()).is_some() {
            events.push(MobEvent::SunProtectionWear { mob: m.id });
        } else {
            m.ignite_for_ticks(160);
        }
    }
}
/// `Mob.isSunBurnTick`: whether the sun reaches a mob that burns in daylight this tick. On
/// `true` the caller applies the exposure, which may wear its protective equipment instead of
/// igniting it (`burn_undead`). A native mod answers here through `[[native.mixin]]`, where a
/// Java mod's mixin on `isSunBurnTick` spares one mob from the sun (#4678).
#[sunrise_mod_api_macros::replaceable]
fn sun_burn_tick(m: &mut Mob, w: &MobWorld) -> bool {
    // Mob.isSunBurnTick: the monsters_burn attribute, not the sky darkness.
    if m.extra.client_side || !w.monsters_burn {
        return false;
    }
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.eye_y().floor() as i32,
        m.position.z.floor() as i32,
    );
    let br = w.brightness(x, y, z);
    let wet = m.fluids.in_water || in_rain(m, w) || in_powder_snow(m, w) || m.was_in_powder_snow;
    br > 0.5 && m.random.next_float() * 30.0 < (br - 0.4) * 2.0 && !wet && w.can_see_sky(x, y, z)
}
// Keep the actual selectors (including partially updated goal state) attached to
// their mob even when a goal callback unwinds. Drop does not invoke goal code.
struct BorrowedGoalSelectors<'a> {
    mob: &'a mut Mob,
    targets: GoalSelector,
    goals: GoalSelector,
}

impl<'a> BorrowedGoalSelectors<'a> {
    fn new(mob: &'a mut Mob) -> Self {
        Self {
            targets: std::mem::take(&mut mob.ai.targets),
            goals: std::mem::take(&mut mob.ai.goals),
            mob,
        }
    }
}

impl Drop for BorrowedGoalSelectors<'_> {
    fn drop(&mut self) {
        self.mob.ai.targets = std::mem::take(&mut self.targets);
        self.mob.ai.goals = std::mem::take(&mut self.goals);
    }
}

#[cfg(test)]
mod selector_restore_tests {
    use super::super::goals::Goal;
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct TrackedGoal {
        progress: i32,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for TrackedGoal {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Goal for TrackedGoal {
        #[inline]
        fn flags(&self) -> u8 {
            0
        }
        #[inline]
        fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
            false
        }
        #[inline]
        fn box_clone(&self) -> Box<dyn Goal> {
            panic!("selectors must not clone goals")
        }
        #[inline]
        fn set_start_delay(&mut self, ticks: i32) {
            self.progress = ticks;
        }
        #[inline]
        fn name(&self) -> &'static str {
            if self.progress == 7 {
                "progress"
            } else {
                "initial"
            }
        }
    }

    // Exercise the production acquisition site, not just the restoration helper.
    // This is an engine Goal callback, not native Game integration coverage.
    #[test]
    #[ignore = "daily"]
    fn server_ai_step_restores_live_selectors_after_goal_unwind() {
        use crate::mobs::world::with_resource_test_world;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::sync::{Mutex, atomic::AtomicBool};
        #[derive(Debug)]
        struct Probe {
            calls: Arc<Mutex<Vec<(usize, u32)>>>,
            panic: Arc<AtomicBool>,
            ticks: u32,
        }
        impl Goal for Probe {
            #[inline]
            fn flags(&self) -> u8 {
                0
            }
            #[inline]
            fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
                true
            }
            #[inline]
            fn every_tick(&self) -> bool {
                true
            }
            #[inline]
            fn box_clone(&self) -> Box<dyn Goal> {
                panic!("live goals must not clone")
            }
            #[inline]
            fn tick(&mut self, _: &mut Mob, _: &mut Cx) {
                self.ticks += 1;
                self.calls
                    .lock()
                    .unwrap()
                    .push((self as *const Self as usize, self.ticks));
                if self.panic.load(Ordering::SeqCst) {
                    std::panic::resume_unwind(Box::new(73_u32));
                }
            }
        }
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("set SUNRISE_TEST_RESOURCES"),
        );
        assert!(
            with_resource_test_world(
                &root,
                &[],
                true,
                true,
                |_, _| {},
                |world, _| {
                    for panic_at in [None, Some(0), Some(1)] {
                        // Full selector tick and the alternate running-only production path.
                        for age in [0, 2] {
                            let kind = crate::mobs::sensor_scan_tests::kind();
                            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
                            mob.age = 0;
                            mob.ai.targets.clear();
                            mob.ai.goals.clear();
                            let calls = [
                                Arc::new(Mutex::new(Vec::new())),
                                Arc::new(Mutex::new(Vec::new())),
                            ];
                            let panics = [
                                Arc::new(AtomicBool::new(false)),
                                Arc::new(AtomicBool::new(false)),
                            ];
                            let mut identities = [0; 2];
                            for (i, selector) in [&mut mob.ai.targets, &mut mob.ai.goals]
                                .into_iter()
                                .enumerate()
                            {
                                let goal = Box::new(Probe {
                                    calls: calls[i].clone(),
                                    panic: panics[i].clone(),
                                    ticks: 0,
                                });
                                identities[i] = &*goal as *const Probe as usize;
                                selector.add_boxed(1, goal);
                            }
                            let mut level = crate::health::TestLevel::new(0);
                            let mut events = Vec::new();
                            // Start both live goals before testing either production branch.
                            let _ = server_ai_step(
                                &mut mob,
                                world,
                                &mut level.level(),
                                &mut events,
                                false,
                            );
                            mob.age = age;
                            if let Some(i) = panic_at {
                                panics[i].store(true, Ordering::SeqCst);
                            }
                            let result = catch_unwind(AssertUnwindSafe(|| {
                                let _ = server_ai_step(
                                    &mut mob,
                                    world,
                                    &mut level.level(),
                                    &mut events,
                                    false,
                                );
                            }));
                            if panic_at.is_some() {
                                assert_eq!(*result.unwrap_err().downcast::<u32>().unwrap(), 73);
                            } else {
                                assert!(result.is_ok());
                            }
                            assert_eq!(
                                mob.ai.targets.entries().count(),
                                1,
                                "actual target selector lost"
                            );
                            assert_eq!(
                                mob.ai.goals.entries().count(),
                                1,
                                "actual action selector lost"
                            );
                            for panic in &panics {
                                panic.store(false, Ordering::SeqCst);
                            }
                            let _ = server_ai_step(
                                &mut mob,
                                world,
                                &mut level.level(),
                                &mut events,
                                false,
                            );
                            for i in 0..2 {
                                let expected_ticks =
                                    if panic_at == Some(0) && i == 1 { 2 } else { 3 };
                                let expected: Vec<_> = (1..=expected_ticks)
                                    .map(|tick| (identities[i], tick))
                                    .collect();
                                assert_eq!(
                                    *calls[i].lock().unwrap(),
                                    expected,
                                    "live goal identity/progress {i}"
                                );
                            }
                        }
                    }
                }
            ),
            "production-site proof requires prepared resources"
        );
    }

    // Lower-layer ownership test, not a native callback integration test.
    #[test]
    fn selector_ownership_survives_normal_return_and_unwind() {
        for unwind in [false, true] {
            let mut mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
            mob.ai.targets.clear();
            mob.ai.goals.clear();
            let drops = Arc::new(AtomicUsize::new(0));
            for selector in [&mut mob.ai.targets, &mut mob.ai.goals] {
                selector.add(
                    1,
                    TrackedGoal {
                        progress: 0,
                        drops: drops.clone(),
                    },
                );
            }
            let payload = Arc::new(17);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut borrowed = BorrowedGoalSelectors::new(&mut mob);
                borrowed.targets.set_start_delay("initial", 7);
                borrowed.goals.set_start_delay("initial", 7);
                borrowed.targets.disabled = 3;
                borrowed.goals.disabled = 5;
                if unwind {
                    std::panic::resume_unwind(Box::new(payload.clone()));
                }
            }));
            if unwind {
                let caught = result.unwrap_err().downcast::<Arc<i32>>().unwrap();
                assert!(Arc::ptr_eq(&caught, &payload));
            } else {
                assert!(result.is_ok());
            }
            assert!(mob.ai.targets.has("progress"));
            assert!(mob.ai.goals.has("progress"));
            assert_eq!(mob.ai.targets.disabled, 3);
            assert_eq!(mob.ai.goals.disabled, 5);
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(mob);
            assert_eq!(drops.load(Ordering::SeqCst), 2);
        }
    }
}

// LivingEntity.aiStep travelRidden: a player rider still drives a NoAI mount.
fn ridden_without_ai(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    pause_movement: bool,
) -> ControlFlow<AiPause> {
    if !m.alive() || !super::equine::controlled(m) {
        return ControlFlow::Continue(());
    }
    super::equine::ridden_step(m, w.loop_context);
    m.handle_jumping(w);
    let input = Vec3::new(
        f64::from(m.ai.xxa),
        f64::from(m.ai.yya),
        f64::from(m.ai.zza),
    );
    if super::equine::moved_by_client(m) {
        m.velocity = Vec3::ZERO;
    } else {
        m.travel_until_pause(w, level, input, events, pause_movement)
            .map_break(|travel| AiPause::Movement(MovementPhase::Travel(travel)))?;
        hurt_teleports(m, w, events);
    }
    ControlFlow::Continue(())
}

/// The replacement slot the staging pass writes at the head of `server_ai_step`, for tests: a
/// plain build has no slot (`#[replaceable]` does nothing there), so a test installs its copy
/// of the function here, on its own thread, and the head check above calls it as the staged
/// head check calls an installed native row's copy.
#[cfg(test)]
mod server_ai_slot {
    use super::*;
    use std::cell::Cell;

    pub(super) type Copy = fn(
        &mut Mob,
        &MobWorld<'_>,
        &mut DamageLevel<'_>,
        &mut Vec<MobEvent>,
        bool,
    ) -> ControlFlow<AiPause>;

    std::thread_local! {
        static SLOT: Cell<Option<Copy>> = const { Cell::new(None) };
    }

    pub(super) fn installed() -> Option<Copy> {
        SLOT.get()
    }

    /// Run `f` with `copy` installed in the slot on this thread.
    pub(super) fn with_copy<R>(copy: Copy, f: impl FnOnce() -> R) -> R {
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                SLOT.set(None);
            }
        }
        SLOT.set(Some(copy));
        let _restore = Restore;
        f()
    }
}

#[cfg(test)]
mod server_ai_brain_tests {
    use super::super::piglin::{Activity, angry_at, set_angry_at};
    use super::*;
    use crate::mobs::world::{test_player, with_built_test_world, with_test_world};
    use std::sync::Arc;

    /// What `server_ai_step` alone leaves on a piglin whose target lies dead (its
    /// celebration ticks and spot, and its `ADMIRING_DISABLED` time to live) and on a brute
    /// (its `ANGRY_AT` time to live).
    fn brain_after_one_step(
        w: &MobWorld,
        types: &super::super::MobTypes,
    ) -> (i32, bool, Option<i32>, Option<i32>) {
        let mut world = w.clone();
        let mut player = test_player(Vec3::new(3.5, 64.0, 0.5));
        player.alive = false;
        world.player = Some(player);
        let mut level = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        let kind = |name: &str| Arc::new(types.types[name].clone());
        let mut piglin = Mob::new(1, kind("minecraft:piglin"), Vec3::new(0.5, 64.0, 0.5), 0.0);
        piglin.ai.target = Some(Who::Player);
        piglin.extra.piglin.admire_disabled.set(5);
        let _ = server_ai_step(&mut piglin, &world, &mut level.level(), &mut events, false);
        let mut brute = Mob::new(
            2,
            kind("minecraft:piglin_brute"),
            Vec3::new(0.5, 64.0, 2.5),
            0.0,
        );
        set_angry_at(&mut brute, 5);
        let _ = server_ai_step(&mut brute, &world, &mut level.level(), &mut events, false);
        (
            piglin.extra.scratch[1],
            piglin.extra.home.is_some(),
            piglin.extra.piglin.admire_disabled.left(),
            angry_at(&brute),
        )
    }

    /// `Piglin` and `PiglinBrute.customServerAiStep` run the brain inside `Mob.serverAiStep`:
    /// a killed target starts the piglin's celebration, its memories run down, and the
    /// brute's `ANGRY_AT` runs down, all within `server_ai_step`, so a mod's row that skips
    /// the step skips them too.
    #[test]
    #[ignore = "daily"]
    fn the_piglin_brain_runs_inside_server_ai_step() {
        let mut outcome = None;
        let ran = with_test_world(&[], |w, types| {
            outcome = Some(brain_after_one_step(w, types))
        });
        assert!(ran, "vanilla data is missing");
        let (celebration, spot, admire_disabled, anger) = outcome.unwrap();
        assert!(celebration > 0 && spot, "the celebration starts");
        assert_eq!((admire_disabled, anger), (Some(4), Some(4)));
    }

    /// A mob of `name` at `at`, one tick from converting (`timeInOverworld` at 300), its
    /// sensors due this tick (`timeToTick` 1), a repellent remembered that isn't there, an
    /// `ANGRY_AT` with 3 ticks left and, for a piglin, a seen item's `ADMIRING_ITEM` with 3
    /// ticks left and a held gold ingot whose `ADMIRING_ITEM` runs out this tick.
    fn about_to_convert(types: &super::super::MobTypes, name: &str) -> Mob {
        let mut m = Mob::new(
            1,
            Arc::new(types.types[name].clone()),
            Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        m.extra.counter2 = 300;
        m.extra.piglin.sensor_wait = 1;
        m.extra.repellent = Some([7, 7, 7]);
        set_angry_at(&mut m, 3);
        if m.species() == Species::Piglin {
            m.extra.piglin.seen_admire.set(3);
            m.extra.set_slot_id(
                sunrise_items::equipment::OFFHAND,
                Some("minecraft:gold_ingot".into()),
            );
            m.extra.using_item = 1;
        }
        m
    }

    /// What one whole tick of `m` leaves: whether it converted or finished admiring, its
    /// zombification count, its sensors' wait and remembered repellent, its `ANGRY_AT`, its
    /// seen item's `ADMIRING_ITEM` and its held item's.
    #[derive(Debug, PartialEq)]
    struct After {
        converted: bool,
        finished_admiring: bool,
        conversion: i32,
        sensor_wait: i32,
        repellent: Option<[i32; 3]>,
        angry_at: Option<i32>,
        seen_admire: Option<i32>,
        held_admire: i32,
    }

    fn one_tick(w: &MobWorld, mut m: Mob) -> After {
        let mut level = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        tick(&mut m, w, &mut level.level(), &mut events);
        After {
            converted: events.iter().any(|e| matches!(e, MobEvent::Convert { .. })),
            finished_admiring: events
                .iter()
                .any(|e| matches!(e, MobEvent::FinishAdmiring { .. })),
            conversion: m.extra.counter2,
            sensor_wait: m.extra.piglin.sensor_wait,
            repellent: m.extra.repellent,
            angry_at: angry_at(&m),
            seen_admire: m.extra.piglin.seen_admire.left(),
            held_admire: m.extra.using_item,
        }
    }

    /// The copy a native row with a cancelling `HEAD` on `Mob.serverAiStep` builds: its
    /// helper (the `lower` fixture's `server_ai`, cancelling) returns `Break(value)`, and the
    /// copy returns `value` before the body.
    fn cancelling_row(
        m: &mut Mob,
        _: &MobWorld<'_>,
        _: &mut DamageLevel<'_>,
        _: &mut Vec<MobEvent>,
        _: bool,
    ) -> ControlFlow<AiPause> {
        fn server_ai(m: &Mob) -> ControlFlow<ControlFlow<AiPause>> {
            let _ = m;
            ControlFlow::Break(ControlFlow::Continue(()))
        }
        match server_ai(&*m) {
            ControlFlow::Break(value) => value,
            ControlFlow::Continue(()) => unreachable!("the row cancels at HEAD"),
        }
    }

    /// A native row that cancels `Mob.serverAiStep` at `HEAD`, installed in the step's slot,
    /// skips all of the piglin's, brute's and hoglin's brain and conversion with it: no memory
    /// runs out or finishes a barter, the sensors neither count down nor scan, and the
    /// zombification neither counts nor converts. The same tick without the row does all of it.
    #[test]
    #[ignore = "daily"]
    fn a_row_cancelling_server_ai_step_skips_the_brain_and_the_conversion() {
        let ran = with_test_world(&[], |w, types| {
            let mut world = w.clone();
            world.player = None;
            for name in [
                "minecraft:piglin",
                "minecraft:piglin_brute",
                "minecraft:hoglin",
            ] {
                let piglin = name == "minecraft:piglin";
                let hoglin = name == "minecraft:hoglin";
                let skipped = server_ai_slot::with_copy(cancelling_row, || {
                    one_tick(&world, about_to_convert(types, name))
                });
                assert_eq!(
                    skipped,
                    After {
                        converted: false,
                        finished_admiring: false,
                        conversion: 300,
                        sensor_wait: 1,
                        repellent: Some([7, 7, 7]),
                        angry_at: Some(3),
                        seen_admire: piglin.then_some(3),
                        held_admire: i32::from(piglin),
                    },
                    "{name}, step cancelled by the row"
                );
                let ran = one_tick(&world, about_to_convert(types, name));
                assert!(ran.converted, "{name} converts when its step runs");
                assert_eq!(ran.angry_at, Some(2), "{name}'s ANGRY_AT runs down");
                assert_eq!(ran.finished_admiring, piglin, "{name} finishes admiring");
                if piglin || hoglin {
                    assert_eq!(
                        (ran.sensor_wait, ran.repellent),
                        (20, None),
                        "{name}'s sensors scan"
                    );
                }
            }
        });
        assert!(ran, "vanilla data is missing");
    }

    /// A stone floor wide enough to walk 16 blocks away on, and `block` at (0, 64, 0).
    fn repellent_floor(
        block: &'static str,
    ) -> impl FnOnce(&mut crate::world::World, &sunrise_world::registry::BlockRegistry) {
        move |world, registry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            for cx in -3..3 {
                for cz in -3..3 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -48..48 {
                for z in -48..48 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            world
                .set(0, 64, 0, registry.default_state(block).unwrap())
                .unwrap();
        }
    }

    /// As [`a_repellent_scanned_this_tick_is_avoided_this_tick`], on a tick where
    /// `Mob.serverAiStep`'s goal selectors only tick the goals already running (an odd `tickCount + id` past the first ticks): the avoid behaviour is the
    /// brain's, which `Brain.startEachNonRunningBehavior` tries on every tick, so it still
    /// starts on the scan's tick.
    #[test]
    #[ignore = "daily"]
    fn a_repellent_scanned_on_a_running_goals_only_tick_is_avoided_that_tick() {
        for (name, block, goal) in [
            (
                "minecraft:piglin",
                "minecraft:soul_lantern",
                "PiglinAvoidRepellent",
            ),
            (
                "minecraft:hoglin",
                "minecraft:warped_fungus",
                "HoglinAvoidRepellent",
            ),
        ] {
            let ran = with_built_test_world(&[], repellent_floor(block), |w, types| {
                let mut world = w.clone();
                world.player = None;
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut m = Mob::new(
                    2,
                    Arc::new(types.types[name].clone()),
                    Vec3::new(3.5, 64.0, 0.5),
                    0.0,
                );
                // `tickCount` 5, id 2: 7 is odd, so only the running goals tick.
                m.age = 5;
                m.extra.piglin.sensor_wait = 1;
                m.ground.on_ground = true;
                let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
                assert_eq!(m.extra.repellent, Some([0, 64, 0]), "{name} scans");
                assert!(
                    m.ai.goals.is_running(goal),
                    "{name}: {:?}",
                    m.ai.goals.running_names()
                );
            });
            assert!(ran, "vanilla data is missing");
        }
    }

    /// `Brain.tick` runs the sensors before the behaviours: on the tick `PiglinSpecificSensor`
    /// finds a soul lantern 3 blocks away, `PiglinAi.avoidRepellent` (the idle activity's
    /// `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, ..)`) already walks away from it, as
    /// `HoglinSpecificSensor` and the hoglin's for a warped fungus.
    #[test]
    #[ignore = "daily"]
    fn a_repellent_scanned_this_tick_is_avoided_this_tick() {
        for (name, block, goal) in [
            (
                "minecraft:piglin",
                "minecraft:soul_lantern",
                "PiglinAvoidRepellent",
            ),
            (
                "minecraft:hoglin",
                "minecraft:warped_fungus",
                "HoglinAvoidRepellent",
            ),
        ] {
            let ran = with_built_test_world(
                &[],
                move |world: &mut crate::world::World,
                      registry: &sunrise_world::registry::BlockRegistry| {
                    // A stone floor wide enough to walk 16 blocks away on, and the
                    // repellent at (0, 64, 0).
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    for cx in -3..3 {
                        for cz in -3..3 {
                            world
                                .chunks
                                .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                        }
                    }
                    for x in -48..48 {
                        for z in -48..48 {
                            world.set(x, 63, z, stone).unwrap();
                        }
                    }
                    world
                        .set(0, 64, 0, registry.default_state(block).unwrap())
                        .unwrap();
                },
                |w, types| {
                    let mut world = w.clone();
                    world.player = None;
                    let mut level = crate::health::TestLevel::new(0);
                    let mut events = Vec::new();
                    let mut m = Mob::new(
                        2,
                        Arc::new(types.types[name].clone()),
                        Vec3::new(3.5, 64.0, 0.5),
                        0.0,
                    );
                    m.extra.piglin.sensor_wait = 1;
                    // Standing on the floor, as after a tick there.
                    m.ground.on_ground = true;
                    let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
                    assert_eq!(m.extra.repellent, Some([0, 64, 0]), "{name} scans");
                    assert!(
                        m.ai.goals.is_running(goal),
                        "{name}: {:?}",
                        m.ai.goals.running_names()
                    );
                },
            );
            assert!(ran, "vanilla data is missing");
        }
    }

    /// `Brain.tick` forgets an `ANGRY_AT` whose time to live is already 0 before the
    /// behaviours and `PiglinAi.updateActivity`: without it, a piglin's target that wears gold
    /// is no longer valid and goes, and the piglin stays idle. One tick earlier the memory,
    /// with 0 ticks left, is still there, and the piglin fights.
    #[test]
    #[ignore = "daily"]
    fn an_angry_at_that_runs_out_drops_a_gold_wearer_before_the_activity() {
        let ran = with_test_world(&[], |w, types| {
            let mut world = w.clone();
            let mut player = test_player(Vec3::new(4.5, 64.0, 0.5));
            player.gold_armor = true;
            world.player = Some(player);
            let fight = |anger: i32| {
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types["minecraft:piglin"].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                m.ai.target = Some(Who::Player);
                m.extra.anger = anger;
                let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
                (m.ai.target, m.extra.piglin.activity, angry_at(&m))
            };
            // `ANGRY_AT` with 1 tick left: it counts down to 0 and stays.
            assert_eq!(
                fight(2),
                (Some(Who::Player), Activity::Fight, Some(0)),
                "the last angry tick"
            );
            // `ANGRY_AT` with 0 ticks left: it goes first.
            assert_eq!(fight(1), (None, Activity::Idle, None), "the memory ran out");
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `Piglin.customServerAiStep` is `Brain.tick`, then `PiglinAi.updateActivity`. In
    /// `Brain.tick`, `forgetOutdatedMemories` clears a memory only once its time to live is
    /// already 0 (`MemorySlot.tick`) and otherwise counts it down, so a seen item's
    /// `ADMIRING_ITEM` with one tick left is still there for that tick's `updateActivity`: the
    /// piglin admires through that tick and goes idle on the next.
    #[test]
    #[ignore = "daily"]
    fn a_piglin_admires_through_the_last_tick_of_its_memory() {
        let ran = with_test_world(&[], |w, types| {
            let mut world = w.clone();
            world.player = None;
            let mut level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:piglin"].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            m.extra.piglin.seen_admire.set(1);
            m.extra.piglin.activity = Activity::Admire;
            let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
            assert_eq!(
                (m.extra.piglin.activity, m.extra.piglin.seen_admire.left()),
                (Activity::Admire, Some(0)),
                "the last tick still admires"
            );
            let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
            assert_eq!(m.extra.piglin.activity, Activity::Idle);
        });
        assert!(ran, "vanilla data is missing");
    }

    /// A piglin admiring a held gold ingot: its `ADMIRING_ITEM` (`admireGoldItem`) runs out in
    /// `Brain.tick`, where `StopHoldingItemIfNoLongerAdmiring` finishes it (the game barters
    /// the ingot) before `updateActivity`, which then finds the piglin idle. On a tick it also
    /// converts, the finish comes first: the ingot isn't dropped at its feet as an
    /// interrupted admiring would be, and the zombified piglin doesn't carry it.
    #[test]
    #[ignore = "daily"]
    fn a_held_items_admiring_ends_in_the_brain_before_the_activity_and_the_conversion() {
        let ran = with_test_world(&[], |w, types| {
            let mut world = w.clone();
            world.player = None;
            let piglin = |held: i32, conversion: i32| {
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types["minecraft:piglin"].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                m.extra.set_slot_id(
                    sunrise_items::equipment::OFFHAND,
                    Some("minecraft:gold_ingot".into()),
                );
                m.extra.using_item = held;
                m.extra.counter2 = conversion;
                m.extra.piglin.activity = Activity::Admire;
                m
            };
            let finished = |events: &[MobEvent]| {
                events
                    .iter()
                    .position(|e| matches!(e, MobEvent::FinishAdmiring { mob: 1 }))
            };
            let mut level = crate::health::TestLevel::new(0);
            // `using_item` keeps the memory as `Ttl` does: at 2, it counts down to its
            // last tick (time to live 0) and stays; at 1, it runs out.
            let mut m = piglin(2, 0);
            let mut events = Vec::new();
            let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
            assert_eq!(m.extra.piglin.activity, Activity::Admire, "the last tick");
            assert_eq!(finished(&events), None);
            // Ran out: finished in the brain, and idle in the same tick.
            let mut events = Vec::new();
            let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
            assert!(finished(&events).is_some(), "the brain finishes it");
            assert_eq!(m.extra.piglin.activity, Activity::Idle);
            // Running out on the conversion tick.
            let mut m = piglin(1, 300);
            let mut events = Vec::new();
            let _ = server_ai_step(&mut m, &world, &mut level.level(), &mut events, false);
            let convert = events
                .iter()
                .position(|e| matches!(e, MobEvent::Convert { .. }))
                .expect("the piglin converts");
            let finish = finished(&events).expect("the brain finishes the admiring");
            assert!(finish < convert, "finished before the conversion");
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, MobEvent::DropItem { .. })),
                "the ingot is not dropped: {events:?}"
            );
            // What the zombified piglin carries is the game's to settle, once it has handled
            // the `FinishAdmiring` (`game::piglin_conversion_tests`).
        });
        assert!(ran, "vanilla data is missing");
    }

    /// A piglin converting while it walks onto a slime block: `Mob.convertTo` takes its place
    /// where it stood before `travel`, and discards it, so the discarded piglin's block
    /// effects (`Entity.applyEffectsFromBlocks`: `isAffectedByBlocks` is false once removed)
    /// don't run: `SlimeBlock.stepOn` doesn't slow it, as `TurtleEggBlock.stepOn` wouldn't
    /// crack an egg under it. A piglin that doesn't convert is slowed. (A hot floor can't
    /// show it: `isInvulnerableToBase` already spares a removed mob.)
    #[test]
    #[ignore = "daily"]
    fn a_converted_piglin_takes_no_block_effects_after_its_conversion() {
        // Where the zombified piglin took its place, and the discarded piglin's sideways speed.
        let walk = |under: &'static str, conversion: i32| {
            let mut outcome = None;
            let ran = with_built_test_world(
                &[],
                super::hurt_teleport_contact_tests::build(under, "minecraft:air"),
                |w, types| {
                    let mut world = w.clone();
                    world.player = None;
                    let mut level = crate::health::TestLevel::new(0);
                    let mut events = Vec::new();
                    let mut m = Mob::new(
                        1,
                        Arc::new(types.types["minecraft:piglin"].clone()),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    );
                    // Walking, and pressed onto the floor so that the move lands it there.
                    m.velocity = Vec3::new(0.3, -0.08, 0.0);
                    m.extra.counter2 = conversion;
                    tick(&mut m, &world, &mut level.level(), &mut events);
                    let converted_at = events.iter().find_map(|e| match e {
                        MobEvent::Convert { from, .. } => Some(from.position),
                        _ => None,
                    });
                    outcome = Some((converted_at, [m.velocity.x, m.velocity.z]));
                },
            );
            assert!(ran, "vanilla data is missing");
            outcome.unwrap()
        };
        let (converted_at, on_slime) = walk("minecraft:slime_block", 300);
        assert_eq!(
            converted_at,
            Some(Vec3::new(0.5, 64.0, 0.5)),
            "the zombified piglin takes its place before travel"
        );
        let (_, on_stone) = walk("minecraft:stone", 300);
        assert_eq!(
            on_slime, on_stone,
            "the discarded piglin's stepOn doesn't run"
        );
        let (_, staying_on_slime) = walk("minecraft:slime_block", 0);
        let (_, staying_on_stone) = walk("minecraft:stone", 0);
        assert_ne!(
            staying_on_slime, staying_on_stone,
            "a piglin that doesn't convert is slowed by the slime block"
        );
    }

    /// `PiglinBrute.customServerAiStep` runs `updateActivity` and `maybePlayActivitySound`
    /// inside `Mob.serverAiStep`, which `LivingEntity.aiStep` calls before `travel`: a moving
    /// brute that starts fighting growls where it stood before it moved.
    #[test]
    #[ignore = "daily"]
    fn a_moving_brutes_fight_growl_comes_from_before_travel() {
        let ran = with_test_world(&[], |w, types| {
            let mut world = w.clone();
            world.player = Some(test_player(Vec3::new(4.5, 64.0, 0.5)));
            let mut level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:piglin_brute"].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            m.ai.target = Some(Who::Player);
            m.extra.piglin.activity = Activity::Idle;
            m.velocity = Vec3::new(0.3, 0.0, 0.0);
            let before = [m.position.x, m.position.y, m.position.z];
            tick(&mut m, &world, &mut level.level(), &mut events);
            assert_ne!(
                [m.position.x, m.position.y, m.position.z],
                before,
                "it moved"
            );
            let growls: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound("entity.piglin_brute.angry", at, ..) => Some(*at),
                    _ => None,
                })
                .collect();
            assert!(!growls.is_empty(), "the fight starts with a growl");
            assert!(
                growls.iter().all(|at| *at == before),
                "{growls:?} != {before:?}"
            );
        });
        assert!(ran, "vanilla data is missing");
    }
}

/// `Mob.serverAiStep`, which `ai_step` calls for a living mob that isn't immobile. With
/// `pause_movement`, it pauses on goal movement, live breeding birth, or a bee's own hurt
/// (`Break`), and the rest of the step resumes later without coming back through here. A native
/// mod takes the step here through `[[native.mixin]]`, where a Java mod's mixin on `serverAiStep`
/// skips it for a tick (#4678): a cancelling `HEAD` makes it return `Continue(())`, and `ai_step`
/// goes on as after the step.
#[sunrise_mod_api_macros::replaceable]
fn server_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    pause_movement: bool,
) -> ControlFlow<AiPause> {
    // The staged build's head check: when a native row's copy of this function is
    // installed, the copy runs instead of this body. A test installs its copy here.
    #[cfg(test)]
    if let Some(copy) = server_ai_slot::installed() {
        return copy(m, w, level, events, pause_movement);
    }
    // A Java `int`: past `i32::MAX` it wraps, as there.
    m.ai.no_action_time = m.ai.no_action_time.wrapping_add(1);
    // The start of the piglin's, brute's and hoglin's `Brain.tick`: memories that
    // run out, then the sensors, before the goals that stand in for its behaviours.
    if matches!(
        m.species(),
        Species::Piglin | Species::PiglinBrute | Species::Hoglin
    ) {
        super::piglin::brain_start(m, w, events);
    }
    // The piglin brain's `StartCelebratingIfTargetDead`, before the target goals
    // drop the dead target.
    if m.species() == Species::Piglin {
        super::piglin::celebrate(m, w);
    }
    // A happy ghast set up for its age (ageBoundaryReached runs at once).
    if m.species() == Species::HappyGhast {
        super::happy_ghast::ensure_setup(m, Some(w));
    }
    // A saddled mount follows its rider instead of its goals.
    if super::equine::controlled(m) {
        super::equine::ridden_step(m, w.loop_context);
        return ControlFlow::Continue(());
    }
    super::riders::drive(m, w);
    let tick_count = i64::from(m.age);
    let id_based = tick_count + i64::from(m.id);
    let pause_birth = pause_movement && w.asks_live_birth(m.species());
    {
        let BorrowedGoalSelectors {
            mob,
            targets,
            goals,
        } = &mut BorrowedGoalSelectors::new(m);
        let mut cx = Cx { w, level, events };
        let force = !(id_based % 2 != 0 && tick_count > 1);
        // Native mods can install BreathAir in either selector, not just on dolphins.
        for (selector, phase, breed) in [
            (targets, SelectorPhase::Targets, false),
            (goals, SelectorPhase::Goals, pause_birth),
        ] {
            if pause_movement {
                if let Some(paused) = selector.tick_until_pause(mob, &mut cx, force, breed) {
                    return ControlFlow::Break(goal_pause_to_ai(phase, paused));
                }
            } else if force {
                selector.tick(mob, &mut cx);
            } else {
                selector.tick_running(mob, &mut cx, false);
            }
        }
    }
    finish_server_ai_step_from(m, w, level, events, None, pause_movement, pause_birth)
}

fn finish_server_ai_step_from(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    stage: Option<BrainStage>,
    pause_owner: bool,
    pause_birth: bool,
) -> ControlFlow<AiPause> {
    // A creaking looked at stands still: its navigation, move, look and jump
    // controls wait (CreakingPathNavigation and the Creaking*Control).
    let frozen = super::creaking::is_creaking(m) && !super::creaking::can_move(m);
    let animal_brain_active = m.species() == Species::Sniffer
        || m.species() == Species::HappyGhast && m.is_baby()
        || m.species() == Species::CopperGolem
        || m.species() == Species::Axolotl
        || m.species() == Species::Tadpole;
    if stage.is_none() {
        // The bee's navigation waits while it pollinates.
        if !frozen && !super::bee::pollinating(m) {
            m.nav_tick(w);
        }
        if pause_owner {
            if super::monster_brain::has_brain(m.species()) {
                super::monster_brain::ensure_brain(m);
            }
            if m.species() == Species::Villager {
                super::villager_brain::ensure_brain(m, w.game_time, w.day_time);
            }
            if animal_brain_active {
                super::animal_brain::ensure_brain(m);
            }
            if m.staged_parts.pending_behaviors.is_some() {
                return ControlFlow::Break(AiPause::RegisterBehaviors(BrainStage::BeforeBrain));
            }
        }
    }
    if !matches!(stage, Some(BrainStage::AfterVillagerBrain)) {
        if super::monster_brain::has_brain(m.species()) {
            // Breeze/Creaking.customServerAiStep: the brain, then updateActivity.
            super::monster_brain::tick(m, w, level, events);
        }
        if m.species() == Species::Villager {
            // Villager.customServerAiStep: the brain, then the raid sound roll.
            super::villager::tick_brain(m, w, level, events);
            if pause_owner && m.staged_parts.pending_behaviors.is_some() {
                return ControlFlow::Break(AiPause::RegisterBehaviors(
                    BrainStage::AfterVillagerBrain,
                ));
            }
        }
        if m.species() == Species::Warden {
            // Warden.customServerAiStep: the brain and its bookkeeping.
            super::warden::server_ai_step(m, w, level, events);
        }
        // Sniffer and HappyGhast.customServerAiStep: the brain (a happy ghast's
        // only while it is a ghastling), then updateActivity.
        if animal_brain_active {
            super::animal_brain::tick(m, w, level, events);
        }
    }
    if m.species() == Species::Villager {
        if let Some(player) = super::villager::ext_mut(m).last_traded_player.take() {
            events.push(MobEvent::VillagerTrade { mob: m.id, player });
        }
        let _ = m.random.next_int_bound(100);
    }
    if m.species() == Species::Warden {
        // Warden.customServerAiStep: the brain and its bookkeeping.
        super::warden::server_ai_step(m, w, level, events);
    }
    // Sniffer and HappyGhast.customServerAiStep: the brain (a happy ghast's
    // only while it is a ghastling), then updateActivity.
    if animal_brain_active
        && let Some((event, resume)) =
            super::animal_brain::tick_until_birth(m, w, level, events, pause_birth)
    {
        return ControlFlow::Break(AiPause::Breed(event, BreedingPhase::Brain(resume)));
    }
    server_ai_controls(m, w, level, events, frozen)
}

fn server_ai_controls(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    frozen: bool,
) -> ControlFlow<AiPause> {
    if m.species() == Species::Bee {
        return bee_server_ai_step(m, w, level, events, super::bee::ServerAiStage::Start)
            .map_break(AiPause::BeeDamage);
    }
    custom_server_ai_step(m, &mut Cx { w, level, events });
    tick_ai_controls(m, w, events, frozen);
    ControlFlow::Continue(())
}

/// A bee rejoins `Mob.serverAiStep` at the controls only after its own step is done.
fn bee_server_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
    stage: super::bee::ServerAiStage,
) -> ControlFlow<super::bee::ServerAiStage> {
    super::bee::server_ai_step(m, w, level, stage)?;
    tick_ai_controls(m, w, events, false);
    ControlFlow::Continue(())
}

fn tick_ai_controls(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>, frozen: bool) {
    if frozen {
        m.ai.jumping = false;
        return;
    }
    if !super::axolotl::is_playing_dead(m) {
        m.move_control_tick(w, events);
        // BeeLookControl: still while angry.
        if !(m.species() == Species::Bee && super::bee::angry(m, w)) {
            m.look_control_tick();
        }
    }
    m.jump_control_tick();
}
/// `Armadillo.pickNextScuteDropTime`: 5 to 10 minutes.
pub(super) fn next_scute_time(m: &mut Mob) -> i32 {
    m.random.next_int_bound(20 * 60 * 5).unwrap_or(0) + 20 * 60 * 5
}
/// `Armadillo.customServerAiStep`: the scute timer keeps counting down and is
/// only reset once `shouldDropLoot` (not a baby, `mob_drops` on) lets it shed.
fn armadillo_server_ai_step(m: &mut Mob, mob_drops: bool, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    m.extra.egg_time -= 1;
    if m.extra.egg_time <= 0 && !m.is_baby() && mob_drops {
        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
        events.push(MobEvent::ShedScute {
            mob: m.id,
            at: [m.position.x, m.position.y, m.position.z],
            pitch,
        });
        m.extra.egg_time = next_scute_time(m);
    }
}
fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    match m.species() {
        Species::Rabbit => {
            let target = m.target_info(cx.w).map(|t| t.position);
            super::rabbit::server_ai_step(m, target)
        }
        Species::Cat | Species::Ocelot => super::tame::feline_server_ai_step(m),
        // Armadillo.customServerAiStep: the brain and its activity, then the scute.
        Species::Armadillo => {
            super::armadillo::server_ai_step(m, cx);
            armadillo_server_ai_step(m, cx.w.mob_drops, cx.events);
        }
        Species::Sheep => {}
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::Skeleton
        | Species::Stray
        | Species::Bogged
        | Species::Parched
        | Species::WitherSkeleton
        | Species::ZombifiedPiglin => undead::custom_server_ai_step(m, cx),
        Species::Vindicator => super::illager::vindicator_server_ai_step(m),
        Species::Enderman => super::enderman::custom_server_ai_step(m, cx),
        Species::Blaze => super::nether::blaze_server_ai_step(m, cx),
        Species::ElderGuardian => super::guardian::elder_server_ai_step(m, cx),
        Species::Wither if m.alive() => super::wither::custom_server_ai_step(m, cx.w, cx.events),
        // SpellcasterIllager.customServerAiStep.
        Species::Evoker | Species::Illusioner if m.extra.counter > 0 => m.extra.counter -= 1,
        Species::Bat => super::bat::server_ai_step(m, cx.w, cx.events),
        Species::Frog => super::frog::server_ai_step(m, cx.w),
        // Hoglin, Piglin and PiglinBrute.customServerAiStep: the brain, then updateActivity,
        // then (AbstractPiglin.customServerAiStep for both piglins) the zombification.
        Species::Piglin | Species::Hoglin => {
            super::piglin::brain_tick(m, cx.w, cx.level.random(), cx.events);
            super::piglin::brain_step(m, cx);
            super::piglin::zombification_tick(m, cx.w, cx.events);
        }
        Species::PiglinBrute => {
            super::piglin::brain_tick(m, cx.w, cx.level.random(), cx.events);
            super::piglin::zombification_tick(m, cx.w, cx.events);
        }
        Species::Zoglin => super::piglin::zoglin_step(m, cx),
        _ => {}
    }
}
fn species_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) -> AiStage {
    if family(m.species()) == Family::Aquatic && aquatic::ai_step(m, w, level, events) {
        return AiStage::DolphinHop;
    }
    match m.species() {
        Species::Enderman => super::enderman::ai_step(m, w, events),
        // Endermite.aiStep: `counter` is the life; an endermite not required
        // to persist is discarded at 2400.
        Species::Endermite => super::hostile::endermite_lifetime(m),
        Species::Wolf => super::tame::ai_step(m, w, events),
        Species::Cat => super::tame::cat_ai_step(m, w, events),
        Species::IronGolem => super::golem::iron_ai_step(m, w),
        Species::Piglin | Species::PiglinBrute | Species::Hoglin => super::piglin::ai_step(m),
        Species::Zoglin if m.extra.anim_ticks > 0 => m.extra.anim_ticks -= 1,
        s if super::equine::is_equine(s) => super::equine::ai_step(m, w, events),
        Species::Phantom => super::phantom::flap_sound(m, events),
        Species::Warden => super::warden::ai_step(m, events),
        Species::Guardian | Species::ElderGuardian => super::guardian::ai_step(m, w, events),
        Species::Vex => super::vex::ai_step(m, w, level),
        Species::Ravager => super::ravager::ai_step(m, w, events),
        Species::Wither => super::wither::ai_step(m, w, level.random(), events),
        Species::Allay => super::allay::ai_step(m, w),
        Species::Bee => super::bee::ai_step(m, w),
        Species::PolarBear => super::polar_bear::ai_step(m, w),
        Species::Breeze => super::breeze::ai_step(m),
        Species::Sniffer => super::sniffer::ai_step(m, events),
        Species::HappyGhast => super::happy_ghast::ai_step(m, w),
        Species::Villager => super::villager::ai_step(m, w),
        Species::WanderingTrader => {
            super::villager::ai_step(m, w);
            super::trader::ai_step(m, w);
        }
        Species::Shulker => {
            super::shulker::tick(m, w, events);
            super::shulker::ai_step(m, w, level.random(), events);
        }
        Species::SnowGolem => super::golem::snow_ai_step(m, w, events),
        Species::CopperGolem => super::copper_golem::ai_step(m, w, events),
        Species::Slime | Species::MagmaCube => super::cube::ai_step(m, w, events),
        Species::SulfurCube => {
            super::cube::ai_step(m, w, events);
            super::sulfur_cube::ai_step(m, w, events);
        }
        Species::Nautilus | Species::ZombieNautilus => {
            super::nautilus::ai_step(m, w, level.random(), events)
        }
        Species::Chicken => farm::chicken_ai_step(m, w, events),
        Species::Parrot => super::parrot::calculate_flapping(m),
        Species::Armadillo => super::armadillo::tick(m),
        Species::Frog => super::frog::tick(m),
        Species::Goat => super::goat::tick(m),
        Species::Fox => super::fox::finish_ai_step(m, w, events),
        Species::Turtle => super::turtle::ai_step(m, w, events),
        Species::Sheep if m.extra.anim_ticks > 0 => {
            m.extra.anim_ticks -= 1;
        }
        _ => {}
    }
    AiStage::Finish
}
/// `Entity.checkDespawn` alone, for a mob outside the entity-ticking
/// chunks. Returns false when the mob is gone.
pub fn despawn_check(m: &mut Mob, w: &MobWorld) -> bool {
    if m.removed {
        return false;
    }
    if !check_despawn(m, w) {
        m.removed = true;
        return false;
    }
    true
}
/// `Mob.requiresCustomPersistence` and its overrides: a passenger or a leashed
/// mob, a raider in a raid, an enderman carrying a block, a tame nautilus, a
/// sulfur cube holding a block or let out of a bucket, a fish or axolotl let
/// out of a bucket and every tadpole (`AbstractFish`/`Axolotl`: `fromBucket()`).
/// The spawn caps do not count these mobs either (`NaturalSpawner.createState`).
pub fn requires_custom_persistence(m: &Mob) -> bool {
    super::riders::keeps_alive(m)
        || super::aquatic::from_bucket(m)
        || m.extra.leash.is_some()
        || super::raider::state(m).is_some_and(|s| s.raid_id.is_some())
        || m.species() == Species::Enderman && m.extra.carried.is_some()
        || super::nautilus::is_nautilus(m.species()) && m.extra.tame
        || m.species() == Species::SulfurCube
            && (super::sulfur_cube::has_body(m) || m.extra.scratch[3] != 0)
}
/// Mob.checkDespawn, before the tick (players are the only despawn
/// reference). Returns false when the mob is discarded.
fn check_despawn(m: &mut Mob, w: &MobWorld) -> bool {
    // A mannequin is a LivingEntity, which never despawns.
    if m.species() == Species::Mannequin {
        return true;
    }
    // `MobDespawnEvent` comes first in NeoForge's `Mob.checkDespawn`, so an answer
    // other than `Default` skips vanilla's checks and their `nextInt(800)` draw.
    match std::mem::take(&mut m.despawn) {
        mod_api::Despawn::Allow => return false,
        mod_api::Despawn::Deny => return true,
        _ => {}
    }
    // Mob.checkDespawn discards disallowed types even when persistent or NoAI.
    if w.difficulty.effective == 0.0 && !m.kind.allowed_in_peaceful() {
        return false;
    }
    let Some(p) = w.nearest_player else {
        return true;
    };
    let d = m.distance_sqr(p);
    let persistent = m.persistent || requires_custom_persistence(m);
    // removeWhenFarAway: animals, villagers, golems, patrols and raiders,
    // zombie villagers being cured and ridden mobs stay, bar the two felines.
    let feline_goes = feline_removed_when_far(m.species(), m.extra.tame || m.extra.trusting, m.age);
    let removable = !(never_removed_when_far(m.species())
        || super::aquatic::named_keeps_it(m)
        || m.kind.category == "creature" && m.kind.flags.animal && !m.extra.jockey && !feline_goes
        || super::raider::keeps_alive(m, d)
        || super::zombie_villager::keeps_alive(m)
        || super::riders::keeps_alive(m));
    // MobCategory.getDespawnDistance / getNoDespawnDistance
    let despawn = if m.kind.category == "water_ambient" {
        64.0
    } else {
        128.0
    };
    if !persistent && d > despawn * despawn && removable {
        return false;
    }
    if !persistent
        && m.ai.no_action_time > 600
        && m.random.next_int_bound(800).unwrap_or(1) == 0
        && d > 32.0 * 32.0
        && removable
    {
        return false;
    }
    if d < 32.0 * 32.0 {
        m.ai.no_action_time = 0;
    }
    true
}
/// `Cat.removeWhenFarAway` and `Ocelot.removeWhenFarAway`: an untamed cat or an
/// untrusting ocelot is removed once it is older than 2400 ticks. That is
/// `Entity.tickCount`, not `AgeableMob.age`, which only counts a baby up to
/// adulthood. `Animal.removeWhenFarAway` keeps every other animal.
fn feline_removed_when_far(s: Species, tamed: bool, ticks: u32) -> bool {
    matches!(s, Species::Cat | Species::Ocelot) && !tamed && ticks > 2400
}
/// The types whose `removeWhenFarAway` is always false (`Villager`,
/// `WanderingTrader`, `AbstractGolem`: iron, snow and copper golems and
/// shulkers; `Allay`, `Warden`), and the wither, whose `checkDespawn` never
/// removes it for distance.
fn never_removed_when_far(s: Species) -> bool {
    matches!(
        s,
        Species::Villager
            | Species::WanderingTrader
            | Species::IronGolem
            | Species::SnowGolem
            | Species::CopperGolem
            | Species::Shulker
            | Species::Allay
            | Species::Warden
            | Species::Wither
    )
}
/// The ambient, hurt and death sounds of species that pick them by state
/// (`getHurtSound` / `getDeathSound` overrides); the type's otherwise.
/// `Goat.getAmbientSound`, `Goat.getHurtSound`, and `Goat.getDeathSound`
/// select the screaming voices from the goat's saved flag.
pub(super) fn inherited_voice_sounds(m: &Mob) -> [&'static str; 3] {
    match m.species() {
        Species::Goat if m.extra.screaming => [
            "entity.goat.screaming.ambient",
            "entity.goat.screaming.hurt",
            "entity.goat.screaming.death",
        ],
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::sounds(m),
        Species::SulfurCube => super::sulfur_cube::sounds(m),
        Species::Armadillo => super::armadillo::sounds(m),
        // HappyGhast.getHurtSound and getDeathSound: the ghastling's by age.
        Species::HappyGhast => super::happy_ghast::sounds(m),
        // Panda.getAmbientSound.
        Species::Panda => {
            let ambient = if super::panda::is_aggressive(m) {
                "entity.panda.aggressive_ambient"
            } else if super::panda::is_worried(m) {
                "entity.panda.worried_ambient"
            } else {
                m.kind.sounds[0]
            };
            [ambient, m.kind.sounds[1], m.kind.sounds[2]]
        }
        // Horse.getAmbientSound, getHurtSound and getDeathSound by age.
        Species::Horse if m.is_baby() => [
            "entity.baby_horse.ambient",
            "entity.baby_horse.hurt",
            "entity.baby_horse.death",
        ],
        // Drowned, Guardian and ElderGuardian: the hurt and death sounds by
        // water or land.
        Species::Drowned if m.fluids.in_water => [
            "entity.drowned.ambient_water",
            "entity.drowned.hurt_water",
            "entity.drowned.death_water",
        ],
        Species::Guardian if !m.fluids.in_water => [
            m.kind.sounds[0],
            "entity.guardian.hurt_land",
            "entity.guardian.death_land",
        ],
        Species::ElderGuardian if !m.fluids.in_water => [
            m.kind.sounds[0],
            "entity.elder_guardian.hurt_land",
            "entity.elder_guardian.death_land",
        ],
        // Slime and MagmaCube.getHurtSound and getDeathSound: tiny ones.
        Species::Slime if m.extra.size <= 1 => [
            m.kind.sounds[0],
            "entity.slime.hurt_small",
            "entity.slime.death_small",
        ],
        Species::MagmaCube if m.extra.size <= 1 => [
            m.kind.sounds[0],
            "entity.magma_cube.hurt_small",
            "entity.magma_cube.death_small",
        ],
        // Shulker.getHurtSound: a closed shell.
        Species::Shulker if m.extra.counter == 0 => [
            m.kind.sounds[0],
            "entity.shulker.hurt_closed",
            m.kind.sounds[2],
        ],
        // Creaking.getHurtSound: bound to a heart it sways, unbound it hands
        // the blow to `Monster.getHurtSound`. `Monster`'s own sounds are the
        // hostile pair, which the giant keeps too.
        Species::Creaking => [
            m.kind.sounds[0],
            if super::creaking::heart_bound(m) {
                "entity.creaking.sway"
            } else {
                "entity.hostile.hurt"
            },
            m.kind.sounds[2],
        ],
        Species::Giant => [
            m.kind.sounds[0],
            "entity.hostile.hurt",
            "entity.hostile.death",
        ],
        // Turtle.getHurtSound and getDeathSound by age.
        Species::Turtle if m.is_baby() => [
            m.kind.sounds[0],
            "entity.turtle.hurt_baby",
            "entity.turtle.death_baby",
        ],
        // SkeletonHorse.getAmbientSound: its own under water.
        Species::SkeletonHorse if m.fluids.eye_in_water => [
            "entity.skeleton_horse.ambient_water",
            m.kind.sounds[1],
            m.kind.sounds[2],
        ],
        _ => m.kind.sounds,
    }
}
/// `Wolf.getSoundSet`: resolves `kind` (`ambient`, `death`, `growl`, `hurt`,
/// `pant`, `whine`, `step`) from the world's `minecraft:wolf_sound_variant` table.
pub(super) fn wolf_sound<'a>(
    m: &Mob,
    variants: Option<&'a super::variants::VariantTables>,
    kind: &str,
) -> Option<&'a str> {
    let variants = variants?;
    let r = m.wolf_sound_variant(&variants.wolf_sound)?;
    variants.wolf_sound.get(r)?.sound(m.is_baby(), kind)
}
/// `Pig.getSoundSet`: resolves `kind` (`ambient`, `hurt`, `death`, `step`, `eat`)
/// from the world's `minecraft:pig_sound_variant` table.
pub(super) fn pig_sound<'a>(
    m: &Mob,
    variants: Option<&'a super::variants::VariantTables>,
    kind: &str,
) -> Option<&'a str> {
    let variants = variants?;
    let r = m.pig_sound_variant(&variants.pig_sound)?;
    variants.pig_sound.get(r)?.sound(m.is_baby(), kind)
}
/// `Cow.getSoundSet`: resolves `kind` (`ambient`, `hurt`, `death`, `step`)
/// from the world's `minecraft:cow_sound_variant` table.
pub(super) fn cow_sound<'a>(
    m: &Mob,
    variants: Option<&'a super::variants::VariantTables>,
    kind: &str,
) -> Option<&'a str> {
    let variants = variants?;
    let r = m.cow_sound_variant(&variants.cow_sound)?;
    variants.cow_sound.get(r)?.sound(kind)
}
/// `Chicken.getSoundSet`: resolves `kind` (`ambient`, `hurt`, `death`, `step`)
/// from the world's `minecraft:chicken_sound_variant` table.
pub(super) fn chicken_sound<'a>(
    m: &Mob,
    variants: Option<&'a super::variants::VariantTables>,
    kind: &str,
) -> Option<&'a str> {
    let variants = variants?;
    let r = m.chicken_sound_variant(&variants.chicken_sound)?;
    variants.chicken_sound.get(r)?.sound(m.is_baby(), kind)
}
/// `Pig`/`Cow`/`Chicken.getSoundSet` for `kind` (`ambient`, `hurt`, `death`): the
/// sound variant every such mob holds, which `Mob::pig_sound_variant` and its
/// siblings resolve to the registry default when the mob stores none.
pub(super) fn farm_sound<'a>(
    m: &Mob,
    variants: Option<&'a super::variants::VariantTables>,
    kind: &str,
) -> Option<&'a str> {
    match m.species() {
        Species::Pig => pig_sound(m, variants, kind),
        Species::Cow => cow_sound(m, variants, kind),
        Species::Chicken => chicken_sound(m, variants, kind),
        _ => None,
    }
}
/// `Pig`/`Cow`/`Chicken.getAmbientSound` + `Mob.playAmbientSound` over the world's
/// sound-variant tables.
pub(super) fn play_farm_ambient(
    m: &mut Mob,
    variants: Option<&super::variants::VariantTables>,
    events: &mut Vec<MobEvent>,
) {
    if let Some(sound) = farm_sound(m, variants, "ambient") {
        m.voice_id(sound, events);
    }
}
/// `Wolf.getAmbientSound` + `Mob.playAmbientSound` over the world's sound-variant table.
pub(super) fn play_wolf_ambient(
    m: &mut Mob,
    variants: Option<&super::variants::VariantTables>,
    events: &mut Vec<MobEvent>,
) {
    let kind = if super::tame::angry(m) {
        "growl"
    } else if m.random.next_int_bound(3).unwrap_or(1) == 0 {
        if m.extra.tame && m.health < 20.0 {
            "whine"
        } else {
            "pant"
        }
    } else {
        "ambient"
    };
    if let Some(sound) = wolf_sound(m, variants, kind) {
        m.voice_id(sound, events);
    }
}
/// `Cat.getSoundSet`: resolves `kind` (`ambient`, `stray_ambient`, `hiss`,
/// `hurt`, `death`, `eat`, `beg_for_food`, `purr`, `purreow`) from the world's
/// `minecraft:cat_sound_variant` table.
pub fn cat_sound<'a>(
    m: &Mob,
    variants: Option<&'a super::variants::VariantTables>,
    kind: &str,
) -> Option<&'a str> {
    let variants = variants?;
    let r = m.cat_sound_variant(&variants.cat_sound)?;
    variants.cat_sound.get(r)?.sound(m.is_baby(), kind)
}
/// `Cat.getAmbientSound` + `Mob.playAmbientSound` over the world's sound-variant table.
pub(super) fn play_cat_ambient(
    m: &mut Mob,
    variants: Option<&super::variants::VariantTables>,
    events: &mut Vec<MobEvent>,
) {
    let kind = if m.extra.tame {
        if m.extra.in_love > 0 {
            "purr"
        } else if m.random.next_int_bound(4).unwrap_or(1) == 0 {
            "purreow"
        } else {
            "ambient"
        }
    } else {
        "stray_ambient"
    };
    if let Some(sound) = cat_sound(m, variants, kind) {
        m.voice_id(sound, events);
    }
}
/// `Animal.mobInteract` with its food in hand: an adult that can fall in
/// love does, a baby grows up by a tenth, and either plays its eating sound
/// (`Strider.mobInteract` plays its own afterwards). `None` when the animal
/// does not take the food; otherwise the sound to play, if any.
pub fn feed_animal(
    m: &mut Mob,
    level: &mut impl RandomSource,
    love_cause: Option<[i32; 4]>,
) -> Option<Option<(&'static str, f32, f32)>> {
    let sound = feed_animal_with_variants(m, level, love_cause, None)?;
    Some(sound.and_then(|(s, v, p)| match s {
        std::borrow::Cow::Borrowed(s) => Some((s, v, p)),
        std::borrow::Cow::Owned(_) => None,
    }))
}
/// [`feed_animal`] resolving registry-backed sound variants against `variants`.
pub fn feed_animal_with_variants(
    m: &mut Mob,
    level: &mut impl RandomSource,
    love_cause: Option<[i32; 4]>,
    variants: Option<&super::variants::VariantTables>,
) -> Option<Option<(std::borrow::Cow<'static, str>, f32, f32)>> {
    // Armadillo.mobInteract: a ball takes no food.
    if super::armadillo::scared(m) {
        return None;
    }
    if !takes_food(m) {
        return None;
    }
    if m.extra.age == 0 {
        m.extra.in_love = 600;
        m.extra.love_player = true;
        // Animal.setInLove: the player who fed it, for a fox baby to trust.
        m.extra.love_cause = love_cause;
    } else {
        // AgeableMob.ageUp(getSpeedUpSecondsWhenFeeding(-age), true): a tenth.
        let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
        age_up(m, secs, true);
    }
    let mut sound = eating_sound_with_variants(m, level, variants);
    if m.species() == Species::Strider && !m.silent {
        let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
        sound = Some((std::borrow::Cow::Borrowed("entity.strider.eat"), 1.0, pitch));
    }
    Some(sound)
}
/// `AgeableMob.canAgeUp`: a baby that no age lock holds back. `SkeletonHorse.canAgeUp` and
/// `ZombieHorse.canAgeUp` are `false`: an undead foal never grows.
pub fn can_age_up(m: &Mob) -> bool {
    m.is_baby()
        && !m.extra.age_locked
        && !matches!(m.species(), Species::SkeletonHorse | Species::ZombieHorse)
}
/// `AgeableMob.canUseGoldenDandelion`: a golden dandelion in hand, a baby
/// whose particle timer has run out, and a type outside
/// `#minecraft:cannot_be_age_locked`.
pub fn can_use_golden_dandelion(
    item: &str,
    baby: bool,
    cooldown: i32,
    cannot_be_age_locked: bool,
) -> bool {
    item == "minecraft:golden_dandelion" && baby && cooldown == 0 && !cannot_be_age_locked
}
/// `AgeableMob.setAgeLockedData`: a golden dandelion flips the lock, puts the
/// age back to a baby's starting age (`getBabyStartAge`) and starts the
/// particle timer (40 ticks).
pub fn set_age_locked_data(m: &mut Mob) {
    m.extra.age_locked = !m.extra.age_locked;
    let start = super::finalize::baby_start_age(m.species());
    super::finalize::set_age(m, start, None);
    m.extra.age_lock_particle_timer = 40;
}
/// `AgeableMob.makeAgeLockedParticle`: while the golden dandelion's timer
/// runs, every other tick one pause (locked) or reset (unlocked) growth
/// particle rises above the mob (`sendParticles`, so the players are sent it).
pub fn make_age_locked_particle(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.extra.age_lock_particle_timer <= 0 {
        return;
    }
    if m.extra.age_lock_particle_timer % 2 == 0 {
        let locked = m.extra.age_locked;
        let (w, h) = (f64::from(m.width()), f64::from(m.height()));
        let r = &mut m.random;
        // getRandomX(1.0), getRandomY(0.2) + getBbHeight() + 0.2F or 0, getRandomZ(1.0).
        let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
        let y = m.position.y
            + h * ((2.0 * r.next_double() - 1.0) * 0.2)
            + h
            + if locked { f64::from(0.2_f32) } else { 0.0 };
        let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
        events.push(MobEvent::LevelParticles {
            kind: if locked {
                "pause_mob_growth"
            } else {
                "reset_mob_growth"
            },
            at: [x, y, z],
            spread: [0.0; 3],
            speed: 0.0,
            count: 1,
        });
    }
    m.extra.age_lock_particle_timer -= 1;
}
/// `AgeableMob.ageUp(seconds, forced)`: both writes go through `setAge`, whose
/// crossing is where a species' `ageBoundaryReached` runs (a goat's and a
/// hoglin's attack damage, a sulfur cube's size); a field write skipped it.
pub fn age_up(m: &mut Mob, seconds: i32, forced: bool) {
    let age = m.extra.age;
    let grown = (age + seconds * 20).min(0);
    super::finalize::set_age(m, grown, None);
    if forced {
        m.extra.forced_age += grown - age;
        if m.extra.forced_age_timer == 0 {
            m.extra.forced_age_timer = 40;
        }
    }
    if m.extra.age == 0 {
        let forced_age = m.extra.forced_age;
        super::finalize::set_age(m, forced_age, None);
    }
}
/// `Animal.playEatingSound` (empty) and its overrides, as (sound, volume,
/// pitch); `level` is the level's random. Pigs and cats play their sound
/// variant's set.
pub fn eating_sound(
    m: &mut Mob,
    level: &mut impl RandomSource,
) -> Option<(&'static str, f32, f32)> {
    let (s, v, p) = eating_sound_with_variants(m, level, None)?;
    match s {
        std::borrow::Cow::Borrowed(s) => Some((s, v, p)),
        std::borrow::Cow::Owned(_) => None,
    }
}
/// [`eating_sound`] resolving registry-backed sound variants against `variants`.
pub fn eating_sound_with_variants(
    m: &mut Mob,
    level: &mut impl RandomSource,
    variants: Option<&super::variants::VariantTables>,
) -> Option<(std::borrow::Cow<'static, str>, f32, f32)> {
    use std::borrow::Cow;
    // Mth.randomBetween(level.getRandom(), 0.8, 1.2).
    let mut between = || level.next_float() * 0.4 + 0.8;
    let (sound, volume, pitch, entity_sound) = match m.species() {
        // makeSound: getSoundVolume, getVoicePitch.
        Species::Armadillo | Species::Pig | Species::SulfurCube => {
            let sound: Cow<'static, str> = match m.species() {
                Species::Armadillo => Cow::Borrowed("entity.armadillo.eat"),
                Species::Pig => pig_sound(m, variants, "eat")
                    .map(|s| Cow::Owned(s.to_owned()))
                    .unwrap_or(Cow::Borrowed("entity.pig.eat")),
                _ => Cow::Borrowed("entity.small_sulfur_cube.eat"),
            };
            let pitch = m.voice_pitch();
            (sound, sound_volume(m), pitch, true)
        }
        // playSound(sound, 1, 1).
        Species::Cat => {
            let sound = Cow::Owned(cat_sound(m, variants, "eat")?.to_owned());
            (sound, 1.0, 1.0, true)
        }
        Species::Fox => (Cow::Borrowed("entity.fox.eat"), 1.0, 1.0, true),
        // level().playSound(null, this, sound, NEUTRAL, volume, pitch).
        Species::Frog => (Cow::Borrowed("entity.frog.eat"), 2.0, 1.0, false),
        Species::Goat if m.extra.screaming => (
            Cow::Borrowed("entity.goat.screaming.eat"),
            1.0,
            between(),
            false,
        ),
        Species::Goat => (Cow::Borrowed("entity.goat.eat"), 1.0, between(), false),
        Species::Sniffer => (Cow::Borrowed("entity.sniffer.eat"), 1.0, between(), false),
        _ => return None,
    };
    // Entity.playSound is skipped for a silent entity.
    if entity_sound && m.silent {
        return None;
    }
    Some((sound, volume, pitch))
}
/// Whether `playEatingSound` binds the sound to the animal: the frog's, the
/// goat's and the sniffer's are `level().playSound(null, this, ..., NEUTRAL,
/// ...)`, the others `Entity.playSound` at its position.
pub fn eating_sound_is_bound(species: Species) -> bool {
    matches!(species, Species::Frog | Species::Goat | Species::Sniffer)
}
/// `LivingEntity.getSoundVolume`: a content type's declared `sounds.volume`, else its template's.
pub fn sound_volume(m: &Mob) -> f32 {
    if let Some(volume) = m.kind.sound_overrides.volume {
        return volume;
    }
    match m.species() {
        Species::Ghast => 5.0,
        Species::HappyGhast => super::happy_ghast::volume(m),
        Species::Warden => 4.0,
        Species::Slime | Species::MagmaCube | Species::SulfurCube => {
            0.4 * m.extra.size.max(1) as f32
        }
        Species::Bat => 0.1,
        Species::Squid
        | Species::GlowSquid
        | Species::Cow
        | Species::Mooshroom
        | Species::Bee
        | Species::Allay
        | Species::Wolf => 0.4,
        s if super::equine::is_equine(s) => 0.8,
        _ => 1.0,
    }
}
/// PathfinderMob.getWalkTargetValue (Animal and Monster overrides).
pub fn walk_target_value(m: &Mob, w: &MobWorld, p: [i32; 3]) -> f32 {
    let light = || w.brightness(p[0], p[1], p[2]) - 0.5;
    // Guardian.getWalkTargetValue: water first.
    if family(m.species()) == Family::Guardian && w.fluid(p[0], p[1], p[2]) == Fluid::Water {
        return 10.0 + light();
    }
    // HappyGhast.getWalkTargetValue: open air, best just above the ground.
    if m.species() == Species::HappyGhast {
        let air = |y: i32| w.facts.is_air(w.world.block(p[0], y, p[2]));
        return if !air(p[1]) {
            0.0
        } else if air(p[1] - 1) && !air(p[1] - 2) {
            10.0
        } else {
            5.0
        };
    }
    if m.species() == Species::Strider {
        return super::strider::walk_target_value(m, w, p);
    }
    if m.species() == Species::Turtle {
        return super::turtle::walk_target_value(m, w, p, light());
    }
    // Hoglin.getWalkTargetValue: nowhere near a repellent, crimson nylium best.
    if m.species() == Species::Hoglin {
        let near = m.extra.repellent.is_some_and(|r| {
            let d = |i: usize| i64::from(r[i] - p[i]);
            d(0) * d(0) + d(1) * d(1) + d(2) * d(2) < 64
        });
        return if near {
            -1.0
        } else if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:crimson_nylium" {
            10.0
        } else {
            0.0
        };
    }
    // Bee.getWalkTargetValue: any air block.
    if m.species() == Species::Bee {
        return if w.facts.is_air(w.world.block(p[0], p[1], p[2])) {
            10.0
        } else {
            0.0
        };
    }
    // AbstractNautilus, Axolotl and Creaking.getWalkTargetValue: everywhere alike.
    if matches!(
        m.species(),
        Species::Nautilus | Species::ZombieNautilus | Species::Axolotl | Species::Creaking
    ) {
        return 0.0;
    }
    // Silverfish.getWalkTargetValue: a block sitting on an infested host is
    // the best place to walk to, so it goes back into the stone.
    if m.species() == Species::Silverfish
        && super::hostile::is_infested_host(w.block_name(p[0], p[1] - 1, p[2]))
    {
        return 10.0;
    }
    // Enderman.getWalkTargetValue, Pillager's and Warden's: every position scores
    // the same, so their strolls pick one of the ten at random.
    if matches!(
        m.species(),
        Species::Enderman | Species::Pillager | Species::Warden
    ) {
        return 0.0;
    }
    if m.hostile()
        && !matches!(
            m.species(),
            Species::Slime | Species::MagmaCube | Species::Ghast | Species::Phantom
        )
    {
        return -light();
    }
    // MushroomCow.getWalkTargetValue replaces Animal's: mycelium is the best
    // ground and any other is scored by the light, grass or not.
    if m.species() == Species::Mooshroom {
        return if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:mycelium" {
            10.0
        } else {
            light()
        };
    }
    if m.kind.flags.animal {
        if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:grass_block" {
            return 10.0;
        }
        return light();
    }
    0.0
}
/// Species damage filters (LivingEntity.hurtServer overrides).
pub fn accepts_damage(
    m: &mut Mob,
    source: &Damage,
    _amount: f32,
    attack: &crate::health::Attack,
) -> bool {
    match m.species() {
        Species::Breeze => super::breeze::accepts_damage(m, source),
        Species::Allay => !attack
            .player_uuid
            .is_some_and(|uuid| super::allay::is_liked_player(m, source.attacker, uuid)),
        Species::Wither => super::wither::accepts(m, source, attack, false),
        Species::Warden => super::warden::accepts_damage(m, source),
        Species::Shulker => super::shulker::accepts(m, source, attack),
        // Bat.hurtServer: a hit wakes it.
        Species::Bat => {
            super::bat::hurt(m);
            true
        }
        Species::Bee => {
            super::bee::hurt(m);
            true
        }
        // Wolf.hurtServer, Parrot.hurtServer: a hit it isn't invulnerable to
        // stands it up before LivingEntity.hurtServer, even one the hurt
        // frames then turn away.
        Species::Wolf | Species::Parrot => {
            super::tame::stand_when_hurt(m);
            true
        }
        // Panda.hurtServer: up first, whatever the damage.
        Species::Panda => {
            super::panda::hurt(m);
            true
        }
        Species::Silverfish => {
            super::hostile::silverfish_hurt(m, source);
            true
        }
        Species::Enderman => super::enderman::on_damage(m, source),
        Species::WitherSkeleton | Species::Blaze | Species::MagmaCube | Species::Strider
            if source.is_fire() =>
        {
            false
        }
        _ => true,
    }
}
/// Reactions after taking damage (sounds, anger).
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    match m.species() {
        Species::Armadillo => super::armadillo::after_hurt(m, source),
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::after_hurt(m, source),
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin => super::undead::after_hurt(m, source),
        // PiglinAi.wasHurtBy: drop what is being admired, no admiring for
        // a while after a player's hit, and anger.
        Species::Piglin if source.attacker == Some(Who::Player) => {
            super::piglin::stop_admiring(m);
            m.extra.egg_time = 400;
            super::piglin::set_angry_at(m, 600);
        }
        _ => {}
    }
}
/// Mob.doHurtTarget: attack damage (+ species extras) to the target.
pub use super::combat::do_hurt_target;
/// Mob.ate: `gameEvent(GameEvent.EAT)`, then (Sheep.ate) the wool grows back and a baby
/// grows.
pub fn ate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::GameEvent {
        event: "minecraft:eat",
        at: [m.position.x, m.position.y, m.position.z],
        source: m.id,
        state: sunrise_world::registry::BlockStateId(0),
    });
    if m.species() == Species::Sheep {
        m.extra.sheared = false;
        // Sheep.ate: only a baby `canAgeUp` (no age lock holds it) grows.
        if can_age_up(m) {
            // AgeableMob.ageUp(60)
            age_up(m, 60, false);
        }
    }
}
/// `NeutralMob.playerDied` for a mob that is neutral: the anger at the
/// player who died ends. The caller checks `forgive_dead_players`.
pub fn forgives_dead_player(m: &Mob) -> bool {
    match m.species() {
        Species::Bee => super::bee::get(m).and_then(|bee| bee.angry_at) == Some(Who::Player),
        Species::PolarBear => m.extra.flag2,
        Species::Enderman | Species::ZombifiedPiglin => m.ai.target == Some(Who::Player),
        _ => m.extra.angry_at == Some(Who::Player),
    }
}

/// Apply forgiveness after the caller has dispatched the target change. Anger
/// bookkeeping still occurs if a listener cancels the target change.
pub fn forgive_dead_player_with_change(m: &mut Mob, change: super::TargetChange) {
    if !forgives_dead_player(m) {
        return;
    }
    let previous = m.ai.target;
    forgive_dead_player(m);
    match change {
        super::TargetChange::Keep => m.ai.target = previous,
        super::TargetChange::Set(target) => m.set_target(target),
    }
}

pub fn forgive_dead_player(m: &mut Mob) {
    match m.species() {
        Species::Bee => super::bee::forgive_dead_player(m),
        Species::PolarBear => super::polar_bear::forgive_dead_player(m),
        // The enderman's and zombified piglin's persistent anger is the
        // target they hold (the stare goal, the hurt-by path): vanilla's
        // `NeutralMob.playerDied` ends it through `stopBeingAngry` when it
        // is the player who died, the last hurt by with it.
        Species::Enderman | Species::ZombifiedPiglin => {
            if m.ai.target == Some(Who::Player) {
                m.ai.forget_hurt_by(m.age as i32);
                m.ai.target = None;
            }
        }
        // The iron golem keeps its anger's target as a wolf does
        // (`tame::update_persistent_anger`): `playerDied` ends the anger at
        // that player whether or not it is the golem's target now.
        _ => super::tame::forgive_dead_player(m),
    }
}
/// Run the effect pass without pausing for a damage owner, for the effect tests.
/// A hurt it deals leaves its teleport pending.
#[cfg(any(test, feature = "test-support"))]
pub fn tick_effects(
    m: &mut Mob,
    context: &LoopContext,
    level: &mut DamageLevel<'_>,
    mods: Option<&dyn super::ModEffectTicks>,
) {
    tick_effects_hurting(m, context, level, mods, &mut |_| {});
}

#[cfg(any(test, feature = "test-support"))]
fn tick_effects_hurting(
    m: &mut Mob,
    context: &LoopContext,
    level: &mut DamageLevel<'_>,
    mods: Option<&dyn super::ModEffectTicks>,
    after_effect: &mut dyn FnMut(&mut Mob),
) {
    let pass = EffectPass::new(m);
    tick_effects_until_hurt(m, context, level, mods, after_effect, false, pass);
}

#[derive(Debug)]
struct EffectPass {
    index: usize,
    end: usize,
    max_absorption: f32,
    interrupted: Option<SuspendedEffect>,
    stop_after_current: bool,
}

impl EffectPass {
    fn new(m: &Mob) -> Self {
        Self {
            index: 0,
            end: m.extra.effects.len(),
            max_absorption: super::armor::max_absorption(m),
            interrupted: None,
            stop_after_current: false,
        }
    }

    fn suspend(mut self, m: &mut Mob, i: usize, end: usize, mut tick: EffectTick) -> Self {
        let effects = &mut m.extra.effects;
        // Removing the active effect does not stop its in-flight tickServer.
        // Keep the expiry inputs in case there is no longer a live row to finish.
        if let Some(active) = effects.get(i).filter(|e| e.id == tick.effect) {
            tick.detached_expiry = active.registry_index.and_then(|index| {
                if tick.expired {
                    return Some((index, active.amplifier));
                }
                if active.duration != 1 {
                    return None;
                }
                match m.extra.effect_chains.iter().find(|e| e.id == active.id) {
                    Some(hidden) if hidden.duration == -1 || hidden.duration > 1 => None,
                    Some(hidden) => Some((index, hidden.amplifier)),
                    None => Some((index, active.amplifier)),
                }
            });
        }
        // A stamp belongs to the instance, not the index where the vector stores it.
        for (index, effect) in effects.iter_mut().enumerate() {
            effect.pass_slot = index + 1;
        }
        self.index = i;
        self.end = end;
        self.interrupted = Some(SuspendedEffect {
            instance: (i < effects.len()).then_some(i + 1),
            instances: effects.len(),
            revision: m.extra.effects_revision,
            tick,
        });
        self
    }
}

#[derive(Debug)]
struct SuspendedEffect {
    instance: Option<usize>,
    instances: usize,
    revision: u64,
    tick: EffectTick,
}

#[derive(Debug)]
struct EffectTick {
    effect: &'static str,
    expired: bool,
    changes: std::vec::IntoIter<super::MobEffectAction>,
    detached_expiry: Option<(usize, i32)>,
}
/// `LivingEntity.tickEffects`: each active effect's `MobEffectInstance.tickServer`.
/// `after_effect` runs after each effect's tick, before its countdown, so a
/// hurt the tick dealt finishes there: `Enderman.hurtServer` teleports inside
/// the hurt, before the next effect's hurt draws (#1608).
fn tick_effects_until_hurt(
    m: &mut Mob,
    context: &LoopContext,
    level: &mut DamageLevel<'_>,
    mods: Option<&dyn super::ModEffectTicks>,
    after_effect: &mut dyn FnMut(&mut Mob),
    pause_hurt: bool,
    mut pass: EffectPass,
) -> Option<EffectPass> {
    // LivingEntity.tickEffects ticks each active effect with the mob's list
    // still in place: the damage a Poison or Wither tick deals reads the
    // list it finds (Resistance softens the hit, Infested spawns silverfish,
    // a totem adds its effects to it). An effect a tick appends ticks from
    // the next tick on.
    let mut first = pass.end;
    // Finish expiry before the next callback reads the holder. A cancelled expiry
    // keeps even a zero-duration instance; hidden promotion happens before this.
    let finish_expiry = |m: &mut Mob, registered: Option<(usize, i32)>, remove: Option<&str>| {
        let mut cancel = false;
        if let (Some(mods), Some((index, amplifier))) = (mods, registered) {
            mods.effect_expired(m.id, index, amplifier, &mut cancel);
        }
        // NeoForge posts Expired before iterator.remove(), even if removal
        // will fail because a callback already invalidated the iterator.
        if let Some(id) = remove.filter(|_| !cancel) {
            m.retain_effects(|effect| effect.id != id);
            true
        } else {
            false
        }
    };
    let mut i = pass.index;
    while pass.interrupted.is_some() || (i < m.extra.effects.len() && i < first) {
        let tick = if let Some(SuspendedEffect {
            instance,
            instances,
            revision,
            tick,
        }) = pass.interrupted.take()
        {
            let effects = &m.extra.effects;
            // LivingEntity catches the iterator's concurrent-modification exception.
            // The interrupted instance finishes first, but a structurally edited list
            // is not advanced. An in-place update does not invalidate the iterator.
            pass.stop_after_current |= m.extra.effects_revision != revision
                || effects.len() != instances
                || effects
                    .iter()
                    .enumerate()
                    .any(|(index, effect)| effect.pass_slot != index + 1);
            i = instance
                .and_then(|slot| effects.iter().position(|effect| effect.pass_slot == slot))
                .unwrap_or(effects.len());
            tick
        } else {
            let (effect, amp) = (m.extra.effects[i].id, m.extra.effects[i].amplifier);
            // MobEffectInstance.tickServer: an infinite (-1) effect times its
            // ticks by the mob's tickCount and never counts down.
            let infinite = m.extra.effects[i].duration == -1;
            // MobEffectInstance.tickServer: without remaining duration the instance
            // ticks nothing; resolve its expiry before the next instance.
            if !infinite && m.extra.effects[i].duration <= 0 {
                let registered = m.extra.effects[i].registry_index.map(|index| (index, amp));
                if finish_expiry(m, registered, Some(effect)) {
                    first -= 1;
                } else {
                    i += 1;
                }
                continue;
            }
            let count = if infinite {
                m.age as i32
            } else {
                m.extra.effects[i].duration
            };
            let interval = |base: i32| base.wrapping_shr(amp as u32).max(1);
            // A refused mod effect expires before the next effect ticks.
            let mut expired = false;
            let mut changes = Vec::new();
            match effect {
                "minecraft:poison"
                    if count % interval(25) == 0 && !m.kind.flags.undead && m.health > 1.0 =>
                {
                    m.damage(level, 1.0, Damage::of(DamageKind::Magic));
                }
                "minecraft:wither" if count % interval(40) == 0 => {
                    // WitherMobEffect.applyEffectTick: damageSources().wither().
                    m.damage(level, 1.0, Damage::of(DamageKind::Wither));
                }
                // InstantenousMobEffect: applied on every tick it has time left
                // (`/effect give` keeps the effect listed for its few ticks; the hit
                // frames let a harming one through once).
                "minecraft:instant_damage" | "minecraft:instant_health" if count >= 1 => {
                    m.apply_heal_or_harm(
                        context,
                        level,
                        effect == "minecraft:instant_damage",
                        m.kind.flags.undead,
                        amp,
                        1.0,
                    );
                }
                "minecraft:regeneration"
                    if count % interval(50) == 0 && !m.kind.flags.undead && m.alive() =>
                {
                    m.heal(context, &|| level.reads(), 1.0);
                }
                // AbsorptionMobEffect.applyEffectTick (asked on every tick): false
                // once the absorption is used up, so tickServer drops the effect
                // before it counts down.
                "minecraft:absorption" if m.absorption <= 0.0 => {
                    expired = true;
                }
                id if sunrise_game_data::potions::is_mod_effect(id) => {
                    if let Some(mods) = mods
                        && let Some(index) = m.extra.effects[i].registry_index
                    {
                        let tick = mods.tick_mod_effect(index, m, count, amp);
                        expired = !tick.keep;
                        changes = tick.actions;
                    }
                }
                _ => {}
            }
            after_effect(m);
            let tick = EffectTick {
                effect,
                expired,
                changes: changes.into_iter(),
                detached_expiry: None,
            };
            if pause_hurt && m.hurt_needs_owner() {
                return Some(pass.suspend(m, i, first, tick));
            }
            tick
        };
        let EffectTick {
            effect,
            expired,
            mut changes,
            detached_expiry,
        } = tick;
        while let Some(change) = changes.next() {
            match change {
                super::MobEffectAction::Heal(amount) => m.heal(context, &|| level.reads(), amount),
                super::MobEffectAction::Hurt(kind, amount) => {
                    m.damage(level, amount, Damage::of(kind));
                    after_effect(m);
                    if pause_hurt && m.hurt_needs_owner() {
                        return Some(pass.suspend(
                            m,
                            i,
                            first,
                            EffectTick {
                                effect,
                                expired,
                                changes,
                                detached_expiry,
                            },
                        ));
                    }
                }
            }
        }
        if m.extra.effects.get(i).is_some_and(|e| e.id == effect) {
            if !expired {
                if let Some(hidden) = m.extra.effect_chains.iter_mut().find(|e| e.id == effect) {
                    hidden.tick_down();
                }
                if m.extra.effects[i].duration != -1 {
                    m.extra.effects[i].duration -= 1;
                }
            } else if !pass.stop_after_current {
                m.extra.effects[i].duration = 0;
            }
            if !expired
                && m.extra.effects[i].duration == 0
                && let Some(chain) = m.extra.effect_chains.iter().position(|e| e.id == effect)
            {
                let mut hidden = m.extra.effect_chains.remove(chain);
                if effect == "minecraft:absorption" {
                    m.absorption = m.absorption.min(4.0 * (hidden.amplifier + 1) as f32);
                }
                m.extra.effects[i].update_from_instance(&hidden);
                // `tickServer`'s onExpire: `onEffectUpdated(instance, true, null)`,
                // the modifiers of the amplifier that is active now.
                let def = m.extra.effects[i]
                    .registry_index
                    .and_then(sunrise_game_data::potions::effect_by_index);
                m.set_effect_modifiers(effect, def, Some(hidden.amplifier));
                m.extra.ambient_effects.retain(|e| *e != effect);
                if hidden.ambient {
                    m.extra.ambient_effects.push(effect);
                }
                m.extra.hidden_effects.retain(|e| *e != effect);
                if !hidden.visible {
                    m.extra.hidden_effects.push(effect);
                }
                if let Some(next) = hidden.hidden.take() {
                    m.extra.effect_chains.push(*next);
                }
                m.effects_dirty = true;
            }
        }
        let removed = if let Some(active) = m.extra.effects.get(i).filter(|e| e.id == effect) {
            if expired || (active.duration <= 0 && active.duration != -1) {
                let registered = active.registry_index.map(|index| (index, active.amplifier));
                finish_expiry(m, registered, (!pass.stop_after_current).then_some(effect))
            } else {
                // LivingEntity.tickEffects refreshes a surviving effect every 600 ticks.
                m.effects_dirty |= active.duration > 0 && active.duration % 600 == 0;
                false
            }
        } else {
            finish_expiry(m, detached_expiry, None)
        };
        if removed {
            // Removal shifts the next original effect into this slot. Shrink the
            // original-pass boundary too, so newly appended effects still wait.
            first -= 1;
        } else {
            i += 1;
        }
        if pass.stop_after_current {
            break;
        }
    }
    if m.effects_dirty {
        let invisible = m.has_effect("minecraft:invisibility");
        m.needs_sync |= m.invisible != invisible;
        m.invisible = invisible;
        m.effects_dirty = false;
    }

    let max_absorption = super::armor::max_absorption(m);
    if max_absorption != pass.max_absorption {
        m.absorption = m.absorption.min(max_absorption);
    }
    None
}

/// `Entity.getMovementEmission` is not NONE: only a shulker moves without
/// emitting anything at all, neither events nor sounds.
pub fn emits_movement(m: &Mob) -> bool {
    !matches!(m.species(), Species::Shulker)
}
/// `Entity.getMovementEmission` includes sounds (`MovementEmission.ALL`): a
/// silverfish, an endermite, a bat, a squid and a glow squid, a guardian, an
/// elder guardian and a breeze answer `EVENTS`, which keeps their footfalls and
/// swim strokes quiet while `Entity.vibrationAndSoundEffectsFromBlock` still
/// sends the step game event. Every other type is `ALL`.
pub fn movement_sounds(m: &Mob) -> bool {
    !matches!(
        m.species(),
        Species::Silverfish
            | Species::Endermite
            | Species::Bat
            | Species::Squid
            | Species::GlowSquid
            | Species::Guardian
            | Species::ElderGuardian
            | Species::Breeze
    )
}
/// `getSwimSound`. A skeleton horse's own override counts the steps it takes
/// through the water, so it needs the mob, not just its species.
pub fn swim_sound(m: &mut Mob) -> &'static str {
    if m.species() == Species::SkeletonHorse {
        return super::equine::skeleton_horse_swim_sound(m);
    }
    match m.species() {
        Species::Drowned => "entity.drowned.swim",
        Species::Axolotl => "entity.axolotl.swim",
        Species::Dolphin => "entity.dolphin.swim",
        Species::Cod
        | Species::Salmon
        | Species::TropicalFish
        | Species::Pufferfish
        | Species::Tadpole => "entity.fish.swim",
        Species::Nautilus if m.is_baby() => "entity.baby_nautilus.swim",
        Species::Nautilus => "entity.nautilus.swim",
        Species::ZombieNautilus => "entity.zombie_nautilus.swim",
        Species::Turtle => "entity.turtle.swim",
        // Monster.getSwimSound, and Hoglin's own override of it.
        s if s == Species::Hoglin || is_monster(s) => "entity.hostile.swim",
        _ => "entity.generic.swim",
    }
}

#[cfg(test)]
mod bucket_axolotl_tests {
    use super::*;
    #[test]
    fn bucket_axolotl_uses_aquatic_not_generic_land_ai() {
        assert_eq!(family(Species::Axolotl), Family::Aquatic);
    }
}
#[cfg(test)]
mod despawn_tests {
    use super::*;
    /// Village villagers are placed without PersistenceRequired (unlike the
    /// village's iron golem), so they must never despawn for distance.
    #[test]
    fn villagers_traders_and_golems_never_despawn_for_distance() {
        for s in [
            Species::Villager,
            Species::WanderingTrader,
            Species::IronGolem,
            Species::SnowGolem,
            Species::CopperGolem,
            Species::Shulker,
        ] {
            assert!(never_removed_when_far(s), "{s:?}");
        }
        for s in [Species::Zombie, Species::ZombieVillager, Species::Pillager] {
            assert!(!never_removed_when_far(s), "{s:?}");
        }
    }

    /// `Cat.removeWhenFarAway` (`!isTame() && tickCount > 2400`) and
    /// `Ocelot.removeWhenFarAway` (`!isTrusting() && tickCount > 2400`): the two
    /// animals vanilla lets go, on `Entity.tickCount` and never on
    /// `AgeableMob.age`, which an adult keeps at 0.
    #[test]
    fn an_untamed_feline_goes_once_it_is_old() {
        for s in [Species::Cat, Species::Ocelot] {
            assert!(!feline_removed_when_far(s, false, 2400), "{s:?}");
            assert!(feline_removed_when_far(s, false, 2401), "{s:?}");
            assert!(!feline_removed_when_far(s, true, 12000), "{s:?}");
        }
        // Animal.removeWhenFarAway: every other animal stays at any age.
        for s in [Species::Cow, Species::Sheep, Species::Wolf] {
            assert!(!feline_removed_when_far(s, false, 12000), "{s:?}");
        }
    }
    /// `AbstractFish.removeWhenFarAway` and `Axolotl.removeWhenFarAway`: a fish or
    /// an axolotl with a custom name is kept however far it swims, while the name
    /// saves no other mob — a monster still goes.
    #[test]
    #[ignore = "daily"]
    fn a_named_fish_or_axolotl_never_despawns() {
        crate::mobs::world::with_test_world(&[], |w, types| {
            let far = MobWorld {
                side: crate::motion::dry_side(),
                // 200 blocks from the player: past the 128 a monster loses.
                nearest_player: Some(Vec3::new(200.0, 64.0, 0.0)),
                ..w.clone()
            };
            let checked = |id: &str, name: Option<&str>| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
                m.custom_name = name.map(str::to_owned);
                despawn_check(&mut m, &far)
            };
            for fish in ["minecraft:cod", "minecraft:salmon", "minecraft:axolotl"] {
                assert!(checked(fish, Some("Bubbles")), "{fish} keeps its name");
                assert!(!checked(fish, None), "{fish} without one goes");
            }
            // A name is not persistence for anything else.
            assert!(!checked("minecraft:zombie", Some("Bubbles")));
        });
    }
}

#[cfg(test)]
mod frog_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use crate::mobs::control::MoveKind;
    use std::sync::Arc;

    /// `Frog()`: `new SmoothSwimmingMoveControl<>(this, 85, 10, 0.02F, 0.1F, true)`.
    /// The frog's movement speed is 1.0, so the controller's 0.1 outside-water
    /// speed is what keeps a walking frog at vanilla's pace. The plain ground
    /// controller walks it ten times too fast, which is the jitter, the
    /// teleporting and the flying of #2213.
    #[test]
    fn a_frog_walks_at_the_vanilla_speed() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:frog").unwrap().clone());
        assert_eq!(kind.speed, 1.0, "the frog's movement speed attribute");
        let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        register(&mut m);
        assert_eq!(
            m.ai.mov.kind,
            MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.02,
                outside: 0.1,
                gravity: true,
            }
        );
    }
}

#[cfg(test)]
mod poison_effect_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use std::sync::Arc;

    #[test]
    fn poison_effect_damages_living_mobs_but_not_undead() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();

        // Pig is not undead: should take damage from poison.
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("poison", 100, 0);
        assert_eq!(pig.extra.effects[0].id, "minecraft:poison");
        let initial_pig_health = pig.health;
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(
            pig.health,
            initial_pig_health - 1.0,
            "living mob should take 1.0 damage from poison"
        );

        // Zombie is undead: immune to poison damage.
        let zombie_kind = Arc::new(data.types.get("minecraft:zombie").unwrap().clone());
        let mut zombie = Mob::new(2, zombie_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        zombie.add_effect("poison", 100, 0);
        let initial_zombie_health = zombie.health;
        tick_effects(
            &mut zombie,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(
            zombie.health, initial_zombie_health,
            "undead mobs are immune to poison"
        );
    }

    /// `LivingEntity.addEffect` runs `AbsorptionMobEffect.onEffectStarted` for
    /// every effect it accepts: the same Absorption given again refills the
    /// shield, and a weaker one still tops it up to its own level (#11386).
    #[test]
    fn absorption_refills_on_every_accepted_add() {
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:cow").unwrap().clone());
        let mut cow = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        cow.add_effect("absorption", 600, 0);
        assert_eq!(cow.absorption, 4.0);
        // A hit used it up.
        cow.absorption = 0.0;
        cow.add_effect("absorption", 600, 0);
        assert_eq!(cow.absorption, 4.0, "the same effect again refills it");
        cow.add_effect("absorption", 600, 2);
        assert_eq!(cow.absorption, 12.0);
        cow.absorption = 2.0;
        cow.add_effect("absorption", 600, 0);
        assert_eq!(cow.absorption, 4.0, "a weaker one tops it up to its level");
    }

    /// Instant health and harm given at amplifier 32 shift by 0, as Java's
    /// `<<` does (#11386).
    #[test]
    fn instant_effects_shift_like_java_on_mobs() {
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:iron_golem").unwrap().clone());
        let mut golem = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        golem.attrs.max_health = 100.0;
        golem.health = 60.0;
        golem.add_effect("instant_health", 6, 32);
        tick_effects(
            &mut golem,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
        );
        assert_eq!(golem.health, 64.0);
    }

    /// A mob's effects act on it: instant health and harm (inverted for the
    /// undead), Resistance taking 20% a level off a hit, Absorption soaking
    /// damage before health, Slow Falling capping gravity while it falls.
    #[test]
    fn mob_effects_heal_harm_resist_absorb_and_slow_the_fall() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let make = |name: &str| {
            let kind = Arc::new(data.types.get(name).unwrap().clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
            m.attrs.max_health = 100.0;
            m.health = 60.0;
            m
        };
        // instant_health 1 heals 8 a tick, instant_damage 0 hurts 6 once (hit frames).
        let mut golem = make("minecraft:iron_golem");
        golem.add_effect("instant_health", 6, 1);
        tick_effects(
            &mut golem,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(golem.health, 68.0);
        let mut golem = make("minecraft:iron_golem");
        golem.add_effect("instant_damage", 6, 0);
        tick_effects(
            &mut golem,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(golem.health, 54.0);
        // The undead are healed by harm.
        let mut zombie = make("minecraft:zombie");
        zombie.add_effect("instant_damage", 6, 0);
        tick_effects(
            &mut zombie,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(zombie.health, 64.0);
        // Resistance V (amplifier 4) takes it all, III takes 80%.
        let mut golem = make("minecraft:iron_golem");
        golem.health = 100.0;
        golem.add_effect("resistance", 100, 4);
        golem.damage(level, 5.0, Damage::of(DamageKind::Generic));
        assert_eq!(golem.health, 100.0);
        let mut golem = make("minecraft:iron_golem");
        golem.health = 100.0;
        golem.add_effect("resistance", 100, 2);
        golem.damage(level, 10.0, Damage::of(DamageKind::Generic));
        assert_eq!(golem.health, 96.0);
        // Absorption II: 8 hearts-halves, 5 damage leaves 3 and costs no health.
        let mut golem = make("minecraft:iron_golem");
        golem.health = 100.0;
        golem.add_effect("absorption", 100, 1);
        assert_eq!(golem.absorption, 8.0);
        golem.damage(level, 5.0, Damage::of(DamageKind::Generic));
        assert_eq!((golem.health, golem.absorption), (100.0, 3.0));
        // Slow Falling: gravity floors at 0.01 while falling, not while rising.
        let mut pig = make("minecraft:pig");
        let full = pig.effective_gravity();
        pig.add_effect("slow_falling", 100, 0);
        assert!(full > 0.01);
        assert_eq!(pig.effective_gravity(), 0.01);
        pig.velocity.y = 0.5;
        assert_eq!(pig.effective_gravity(), full);
    }

    #[test]
    fn invisibility_flag_updates_only_when_effects_change() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("invisibility", 4, 0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert!(pig.is_invisible());
        pig.invisible = false;
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert!(!pig.is_invisible());
        pig.add_effect("speed", 10, 0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert!(pig.is_invisible());
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert!(!pig.is_invisible());
    }

    #[test]
    fn effect_refresh_every_600_ticks_resets_manual_visibility() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("speed", 602, 0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert!(!pig.is_invisible());
        pig.invisible = true;
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(pig.extra.effects[0].duration, 600);
        assert!(
            !pig.is_invisible(),
            "LivingEntity's periodic effect update clears the manual flag at duration 600"
        );
    }

    /// `LivingEntity.tickEffects` ticks each effect with the mob's list in
    /// place: a Poison hit reads Resistance there (0.8 of 1.0 at level I).
    #[test]
    fn effect_ticks_find_the_other_effects() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        // A pig always holds a sound variant, so a hurt pig always makes its
        // sound and draws the voice pitch before `onMobHurt` rolls Infested.
        let tables = crate::mobs::variants::VariantTables::load_prepared(&root);
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level().with_variants(&tables);
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind.clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.health = 20.0;
        pig.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:poison", 25, 0));
        pig.extra.effects.push(crate::mobs::StoredEffect::new(
            "minecraft:resistance",
            -1,
            0,
        ));
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(pig.health, 19.2);
        // `onMobHurt` finds Infested in the same in-place list: the hurt mob
        // leaves a silverfish call in the outbox.
        let spawned = (0..100).any(|seed| {
            let mut pig = Mob::new(2, pig_kind.clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
            pig.random = sunrise_core::random::LegacyRandom::new(seed);
            pig.extra
                .effects
                .push(crate::mobs::StoredEffect::new("minecraft:poison", 25, 0));
            pig.extra
                .effects
                .push(crate::mobs::StoredEffect::new("minecraft:infested", -1, 0));
            tick_effects(
                &mut pig,
                crate::test_resources::server_loop_context(),
                level,
                None,
            );
            pig.outbox
                .iter()
                .any(|e| matches!(e, MobEvent::Infested { .. }))
        });
        assert!(spawned, "a hurt while infested pushes Infested");
    }

    /// `MobEffectInstance.hasRemainingDuration` is false at 0 ticks, so an effect
    /// loaded with `duration: 0` is over on its first tick instead of counting
    /// down to -1, the value that means infinite.
    #[test]
    fn a_zero_duration_effect_is_dropped_not_made_infinite() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.health = 20.0;
        pig.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:poison", 0, 0));
        let mut test_level = crate::health::TestLevel::new(0);
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            &mut test_level.level(),
            None,
        );
        assert!(pig.extra.effects.is_empty());
        // A poison with no remaining duration does not get its first tick either.
        assert_eq!(pig.health, 20.0);
    }

    /// `checkTotemDeathProtection` inside a tick's damage adds the totem's
    /// effects to the live list: they stay on the mob after the tick.
    #[test]
    fn a_totem_fired_by_effect_damage_keeps_its_effects() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.extra
            .set_main_hand_stack(Some(crate::mobs::totem_tests::totem_with_components()));
        pig.extra.effects.push(crate::mobs::StoredEffect::new(
            "minecraft:instant_damage",
            5,
            4,
        ));
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(pig.health, 1.0);
        for effect in [
            "minecraft:regeneration",
            "minecraft:absorption",
            "minecraft:fire_resistance",
        ] {
            assert!(
                pig.extra
                    .effects
                    .iter()
                    .any(|e| e.id == effect && e.duration > 0),
                "{effect} kept after the totem fired"
            );
        }
    }

    /// `WitherMobEffect.applyEffectTick` hurts with `wither`, not `magic`:
    /// the damage the tick deals keeps its kind.
    #[test]
    fn the_wither_effect_hurts_with_wither() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.extra
            .effects
            .push(crate::mobs::StoredEffect::new("minecraft:wither", 40, 0));
        tick_effects(
            &mut pig,
            crate::test_resources::server_loop_context(),
            level,
            None,
        );
        assert_eq!(
            pig.last_damage.map(|(d, _)| d.kind),
            Some(DamageKind::Wither)
        );
    }
}

#[cfg(test)]
mod hit_effect_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    /// The effects of a cave spider's bite, a bare-handed husk's blow and a
    /// wither skeleton's hit go to the `LivingEntity` the hit landed on:
    /// `doHurtTarget` tests `target instanceof LivingEntity`, not `Player`.
    /// Biting a mob carries the effect with the hit — poison for 140 ticks on
    /// normal, hunger for 140 (effective difficulty 1.5), wither for 200 —
    /// where the mob is hit 2. Without the fix each of these is a plain
    /// `AttackMob` and the mob keeps no effect at all (0 ticks).
    #[test]
    #[ignore = "daily"]
    fn a_melee_effect_rides_the_hit_onto_a_mob() {
        let cases = [
            ("minecraft:cave_spider", "minecraft:poison", 140),
            ("minecraft:husk", "minecraft:hunger", 140),
            ("minecraft:wither_skeleton", "minecraft:wither", 200),
        ];
        let mut carried: Vec<(&str, Option<(u32, &'static str, i32, i32)>)> = Vec::new();
        let mut strays: Vec<(&str, bool, bool)> = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            for (kind, _, _) in cases {
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                let level = &mut test_level.level();
                let mut cx = Cx {
                    w,
                    level,
                    events: &mut events,
                };
                do_hurt_target(&mut m, Who::Mob(2), &mut cx);
                carried.push((
                    kind,
                    events.iter().find_map(|e| match e {
                        MobEvent::AttackMobWithEffect {
                            target,
                            effect,
                            ticks,
                            amplifier,
                            ..
                        } => Some((*target, *effect, *ticks, *amplifier)),
                        _ => None,
                    }),
                ));
                strays.push((
                    kind,
                    events
                        .iter()
                        .any(|e| matches!(e, MobEvent::EffectPlayer { .. })),
                    events
                        .iter()
                        .any(|e| matches!(e, MobEvent::AttackMob { .. })),
                ));
            }
        });
        if !ran {
            return;
        }
        let expected: Vec<(&str, Option<(u32, &'static str, i32, i32)>)> = cases
            .iter()
            .map(|(kind, effect, ticks)| (*kind, Some((2, *effect, *ticks, 0))))
            .collect();
        assert_eq!(carried, expected);
        for (kind, on_player, effectless) in strays {
            assert!(!on_player, "{kind} left its effect on the player");
            assert!(!effectless, "{kind} sent a second, effectless hit");
        }
    }

    /// On easy the cave spider's poison timer is 0, so nothing rides the hit:
    /// the mob it bites takes the damage alone (`AttackMob`, no effect).
    #[test]
    #[ignore = "daily"]
    fn an_easy_cave_spiders_bite_carries_no_effect() {
        let mut out: Option<Vec<String>> = None;
        let ran = with_test_world(&[], |w, types| {
            let mut easy = w.clone();
            easy.difficulty = super::super::spawning::LocalDifficulty::new(1, 18000, 0, 0.0);
            let mut m = Mob::new(
                1,
                Arc::new(types.types["minecraft:cave_spider"].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            let mut cx = Cx {
                w: &easy,
                level,
                events: &mut events,
            };
            do_hurt_target(&mut m, Who::Mob(2), &mut cx);
            out = Some(
                events
                    .iter()
                    .filter(|e| {
                        matches!(
                            e,
                            MobEvent::AttackMob { .. } | MobEvent::AttackMobWithEffect { .. }
                        )
                    })
                    .map(|e| match e {
                        MobEvent::AttackMob { target, .. } => format!("plain {target}"),
                        MobEvent::AttackMobWithEffect { effect, .. } => format!("effect {effect}"),
                        _ => unreachable!(),
                    })
                    .collect(),
            );
        });
        if !ran {
            return;
        }
        assert_eq!(out, Some(vec!["plain 2".to_owned()]));
    }
}

#[cfg(test)]
mod attack_sound_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    /// The sounds `do_hurt_target` raises for `kind` on a blow that landed,
    /// with `silent` set on the attacker; `None` without the extracted data.
    fn attack_sounds(kind: &str, silent: bool) -> Option<Vec<(&'static str, f32, f32)>> {
        let mut heard = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            let mut m = Mob::new(
                1,
                Arc::new(types.types[kind].clone()),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            m.silent = silent;
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let mut cx = Cx {
                w,
                level: &mut test_level.level(),
                events: &mut events,
            };
            do_hurt_target(&mut m, Who::Mob(2), &mut cx);
            heard = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Sound(s, _, volume, pitch) => Some((*s, *volume, *pitch)),
                    _ => None,
                })
                .collect();
        });
        if !ran {
            return None;
        }
        Some(heard)
    }

    /// `Mob.doHurtTarget` ends with `playAttackSound`, which `Dolphin`,
    /// `Axolotl` and `Panda` override with their own bite at volume 1 and
    /// pitch 1. Without the dolphin arm its bite is silent (no sound event at
    /// all); the axolotl and the panda sit next to it as the control.
    #[test]
    #[ignore = "daily"]
    fn a_dolphins_blow_plays_its_attack_sound() {
        let Some(dolphin) = attack_sounds("minecraft:dolphin", false) else {
            return;
        };
        assert_eq!(dolphin, vec![("entity.dolphin.attack", 1.0, 1.0)]);
        for (kind, sound) in [
            ("minecraft:axolotl", "entity.axolotl.attack"),
            ("minecraft:panda", "entity.panda.bite"),
        ] {
            assert_eq!(
                attack_sounds(kind, false),
                Some(vec![(sound, 1.0, 1.0)]),
                "{kind}"
            );
        }
        // Entity.playSound skips a silent entity, so neither attack sound plays.
        for kind in ["minecraft:dolphin", "minecraft:axolotl", "minecraft:panda"] {
            assert_eq!(attack_sounds(kind, true), Some(vec![]), "{kind}");
        }
    }
}

#[cfg(test)]
mod same_vehicle_push_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    #[test]
    #[ignore = "daily"]
    fn zombie_horse_sunburn_checks_body_not_head() {
        assert!(with_test_world(&[], |world, types| {
            let mut world = world.clone();
            world.monsters_burn = true;
            let mut horse = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:zombie_horse"].clone()),
                Vec3::ZERO,
                0.0,
            );
            horse.extra.set_slot_id(
                sunrise_items::equipment::HEAD,
                Some("minecraft:iron_helmet".into()),
            );
            let mut events = Vec::new();
            for _ in 0..1000 {
                burn_undead(&mut horse, &world, &mut events);
            }
            assert!(horse.fire_ticks > 0);
            assert!(events.is_empty());
            horse.fire_ticks = 0;
            horse
                .extra
                .set_body(Some("minecraft:leather_horse_armor".into()));
            for _ in 0..1000 {
                burn_undead(&mut horse, &world, &mut events);
            }
            assert_eq!(horse.fire_ticks, 0);
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, MobEvent::SunProtectionWear { .. }))
            );
        }));
    }

    #[test]
    #[ignore = "daily"]
    fn leather_body_armor_prevents_freezing() {
        assert!(with_test_world(&[], |w, types| {
            let mut horse = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:horse"].clone()),
                Vec3::ZERO,
                0.0,
            );
            assert!(can_freeze(&horse, w));
            horse
                .extra
                .set_body(Some("minecraft:leather_horse_armor".into()));
            assert!(!can_freeze(&horse, w));
            horse
                .extra
                .set_body(Some("minecraft:iron_horse_armor".into()));
            assert!(can_freeze(&horse, w));
        }));
    }

    /// What a mob holds and wears, by `EquipmentSlot`: the hand's item, the
    /// worn armour's stack (a dyed piece keeps its colour), a horse's
    /// armour in the body slot and its saddle, and nothing else.
    #[test]
    #[ignore = "daily"]
    fn a_mobs_equipment_is_listed_by_slot() {
        assert!(
            with_test_world(&[], |_, types| {
                let mob = |name: &str| {
                    Mob::new(
                        1,
                        std::sync::Arc::new(types.types[name].clone()),
                        Vec3::new(0.5, 64.0, 0.5),
                        0.0,
                    )
                };
                let mut zombie = mob("minecraft:zombie");
                assert!(zombie.equipment_stacks().iter().all(Option::is_none));
                zombie
                    .extra
                    .set_main_hand(Some(std::sync::Arc::from("minecraft:iron_sword")));
                zombie.extra.set_slot_id(
                    sunrise_items::equipment::FEET,
                    Some(std::sync::Arc::from("minecraft:iron_boots")),
                );
                let worn = zombie.equipment_stacks();
                assert_eq!(
                    worn[0].as_ref().unwrap().item.as_ref(),
                    "minecraft:iron_sword"
                );
                assert_eq!(
                    worn[2].as_ref().unwrap().item.as_ref(),
                    "minecraft:iron_boots"
                );
                assert_eq!(worn.iter().flatten().count(), 2);
                let mut horse = mob("minecraft:horse");
                horse
                    .extra
                    .set_body(Some(std::sync::Arc::from("minecraft:iron_horse_armor")));
                horse.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
                let gear = horse.equipment_stacks();
                assert_eq!(
                    gear[6].as_ref().unwrap().item.as_ref(),
                    "minecraft:iron_horse_armor"
                );
                assert_eq!(gear[7].as_ref().unwrap().item.as_ref(), "minecraft:saddle");
                assert!(gear[5].is_none());
            }),
            "vanilla data is missing"
        );
    }

    /// `Entity.push(Entity)` returns at once for `isPassengerOfSameVehicle`
    /// (the same `getRootVehicle`): a camel and the husk on its back, seated
    /// half a block toward its front and so inside its box, never push each
    /// other (#3199). The camel still pushes a mob that only overlaps it.
    #[test]
    #[ignore = "daily"]
    fn a_camel_and_its_rider_never_push_each_other() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // The mobs the camel pushes, and its velocity after, with these mobs
        // around: a husk on its back, and a zombie standing in its back half.
        let mut push = |zombie_too: bool| {
            let mut out = None;
            let mut riders = Vec::new();
            assert!(
                with_test_world(&[], |_, types| {
                    let mob = |id: u32, name: &str, at: Vec3| {
                        Mob::new(id, std::sync::Arc::new(types.types[name].clone()), at, 0.0)
                    };
                    let mut camel = mob(1, "minecraft:camel", Vec3::new(0.5, 64.0, 0.5));
                    let mut husk = mob(2, "minecraft:husk", Vec3::new(0.5, 65.5, 1.0));
                    camel.extra.rider = Some(husk.id);
                    husk.extra.mount = Some(camel.id);
                    riders.push(husk.seen(None, &crate::items::Items::empty()));
                    if zombie_too {
                        riders.push(
                            mob(3, "minecraft:zombie", Vec3::new(0.5, 64.0, 0.0))
                                .seen(None, &crate::items::Items::empty()),
                        );
                    }
                    out = Some(camel);
                }),
                "vanilla data is missing"
            );
            let mut camel = out?;
            let mut events = Vec::new();
            assert!(
                with_test_world(&riders, |w, _| {
                    push_entities(&mut camel, w, level, &mut events)
                }),
                "vanilla data is missing"
            );
            let pushed: Vec<u32> = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Push {
                        target: Who::Mob(id),
                        ..
                    } => Some(*id),
                    _ => None,
                })
                .collect();
            Some((pushed, camel.velocity))
        };
        let Some((pushed, velocity)) = push(false) else {
            return;
        };
        assert_eq!(pushed, Vec::<u32>::new(), "the camel pushes its own rider");
        assert_eq!(velocity, Vec3::ZERO, "its rider pushes the camel");
        let (pushed, _) = push(true).unwrap();
        assert_eq!(pushed, vec![3], "the camel pushes the zombie");
    }

    #[test]
    #[ignore = "daily"]
    fn sleeping_villager_vetoes_only_pushes_received_during_other_mobs_ticks() {
        let mut pair = None;
        let ran = with_test_world(&[], |_, types| {
            let make = |id, name: &str, x| {
                Mob::new(
                    id,
                    std::sync::Arc::new(types.types[name].clone()),
                    Vec3::new(x, 64.0, 0.5),
                    0.0,
                )
            };
            let mut villager = make(1, "minecraft:villager", 0.5);
            villager.extra.ints[1] = 1;
            pair = Some((villager, make(2, "minecraft:cow", 0.9)));
        });
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        let Some((mut villager, mut cow)) = pair else {
            return;
        };
        let mut events = Vec::new();
        let ran = with_test_world(
            &[villager.seen(None, &crate::items::Items::empty())],
            |w, _| {
                push_entities(
                    &mut cow,
                    w,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut events,
                );
            },
        );
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.is_empty());
        assert_eq!(cow.velocity, Vec3::ZERO);

        let ran = with_test_world(&[cow.seen(None, &crate::items::Items::empty())], |w, _| {
            push_entities(
                &mut villager,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
        });
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.iter().any(|e| matches!(
            e,
            MobEvent::Push {
                target: Who::Mob(2),
                ..
            }
        )));
        assert!(villager.velocity.x < 0.0);
    }

    #[test]
    #[ignore = "daily"]
    fn sleeping_fox_vetoes_pushes_received_during_other_mobs_ticks() {
        let mut pair = None;
        let ran = with_test_world(&[], |_, types| {
            let make = |id, name: &str, x| {
                Mob::new(
                    id,
                    std::sync::Arc::new(types.types[name].clone()),
                    Vec3::new(x, 64.0, 0.5),
                    0.0,
                )
            };
            let mut fox = make(1, "minecraft:fox", 0.5);
            fox.extra.ints[0] |= super::super::fox::SLEEPING;
            pair = Some((fox, make(2, "minecraft:cow", 0.9)));
        });
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        let Some((mut fox, mut cow)) = pair else {
            return;
        };
        let mut events = Vec::new();
        let ran = with_test_world(&[fox.seen(None, &crate::items::Items::empty())], |w, _| {
            push_entities(
                &mut cow,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
        });
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.is_empty());
        assert_eq!(cow.velocity, Vec3::ZERO);

        fox.extra.ints[0] &= !super::super::fox::SLEEPING;
        let ran = with_test_world(&[fox.seen(None, &crate::items::Items::empty())], |w, _| {
            push_entities(
                &mut cow,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
        });
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
        assert!(events.iter().any(|e| matches!(
            e,
            MobEvent::Push {
                target: Who::Mob(1),
                ..
            }
        )));
    }

    #[test]
    #[ignore = "daily"]
    fn a_ridden_horse_is_not_pushed_but_still_pushes() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let pair = |ridden: bool| {
            let mut out = None;
            assert!(
                with_test_world(&[], |_, types| {
                    let mob = |id: u32, name: &str, at: Vec3| {
                        Mob::new(id, std::sync::Arc::new(types.types[name].clone()), at, 0.0)
                    };
                    let mut horse = mob(1, "minecraft:horse", Vec3::new(0.5, 64.0, 0.5));
                    if ridden {
                        horse.extra.rider = Some(3);
                    }
                    let cow = mob(2, "minecraft:cow", Vec3::new(1.4, 64.0, 0.5));
                    out = Some((horse, cow));
                }),
                "vanilla data is missing"
            );
            out
        };
        let pushed = |events: &[MobEvent]| -> Vec<u32> {
            events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::Push {
                        target: Who::Mob(id),
                        ..
                    } => Some(*id),
                    _ => None,
                })
                .collect()
        };
        let Some((mut horse, cow)) = pair(true) else {
            return;
        };
        let mut events = Vec::new();
        assert!(
            with_test_world(&[cow.seen(None, &crate::items::Items::empty())], |w, _| {
                push_entities(&mut horse, w, level, &mut events)
            }),
            "vanilla data is missing"
        );
        assert_eq!(pushed(&events), vec![2], "the ridden horse pushes the cow");
        assert_eq!(
            horse.velocity,
            Vec3::ZERO,
            "the cow pushes the ridden horse"
        );

        let (horse, mut cow) = pair(true).unwrap();
        let mut events = Vec::new();
        assert!(
            with_test_world(
                &[horse.seen(None, &crate::items::Items::empty())],
                |w, _| push_entities(&mut cow, w, level, &mut events),
            ),
            "vanilla data is missing"
        );
        assert_eq!(
            pushed(&events),
            Vec::<u32>::new(),
            "the cow pushes the ridden horse"
        );
        assert_eq!(cow.velocity, Vec3::ZERO, "a ridden horse is not pushable");

        let (horse, mut cow) = pair(false).unwrap();
        let mut events = Vec::new();
        assert!(
            with_test_world(
                &[horse.seen(None, &crate::items::Items::empty())],
                |w, _| push_entities(&mut cow, w, level, &mut events),
            ),
            "vanilla data is missing"
        );
        assert_eq!(
            pushed(&events),
            vec![1],
            "the cow pushes a horse nobody rides"
        );
    }
}

#[cfg(test)]
mod warden_push_tests {
    use super::*;
    use crate::mobs::warden::{Pose, set_pose};
    use crate::mobs::world::{test_player, with_test_world};

    #[test]
    #[ignore = "daily"]
    fn a_warden_in_the_ground_does_not_take_collision_pushes() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        assert!(
            with_test_world(&[], |world, types| {
                let mob = |id, kind: &str, x| {
                    Mob::new(
                        id,
                        std::sync::Arc::new(types.types[kind].clone()),
                        Vec3::new(x, 64.0, 0.5),
                        0.0,
                    )
                };
                for pose in [Pose::Emerging, Pose::Digging, Pose::Standing] {
                    let mut warden = mob(1, "minecraft:warden", 0.5);
                    set_pose(&mut warden, pose);
                    let standing = pose == Pose::Standing;
                    assert_eq!(warden.pushable(), standing, "{pose:?}");
                    assert_eq!(
                        warden.seen(None, &crate::items::Items::empty()).pushable,
                        standing,
                        "{pose:?}"
                    );

                    // The warden's tick: an overlapping mob must not move it.
                    let mut cow = mob(2, "minecraft:cow", 1.0);
                    let others = [cow.seen(None, &crate::items::Items::empty())];
                    let mut w = world.clone();
                    w.others = &others;
                    let mut events = Vec::new();
                    push_entities(&mut warden, &w, level, &mut events);
                    assert_eq!(warden.velocity.x != 0.0, standing, "mob: {pose:?}");
                    // Non-pushable is not an early return: it still pushes others.
                    assert!(events.iter().any(|e| matches!(
                        e,
                        MobEvent::Push {
                            target: Who::Mob(2),
                            ..
                        }
                    )));

                    // The other mob's tick: the warden's Seen must exclude it.
                    let others = [warden.seen(None, &crate::items::Items::empty())];
                    w.others = &others;
                    events.clear();
                    push_entities(&mut cow, &w, level, &mut events);
                    let pushes_warden = events.iter().any(|e| {
                        matches!(
                            e,
                            MobEvent::Push {
                                target: Who::Mob(1),
                                ..
                            }
                        )
                    });
                    assert_eq!(pushes_warden, standing, "other tick: {pose:?}");

                    // The player branch of the warden's tick has its own target path.
                    w.others = &[];
                    w.player = Some(test_player(Vec3::new(1.0, 64.0, 0.5)));
                    warden.velocity = Vec3::ZERO;
                    events.clear();
                    push_entities(&mut warden, &w, level, &mut events);
                    assert_eq!(warden.velocity.x != 0.0, standing, "player: {pose:?}");
                }
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod ridden_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// #2920: a saddled horse ridden forward, ticked on the server; with
    /// `client` its rider's client moves it (`travelRidden` without
    /// `canSimulateMovement`): it only turns to the rider's look and stops.
    fn ridden(client: bool) -> Option<Mob> {
        let mut out = None;
        with_test_world(&[], |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:horse"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
            m.persistent = true;
            m.rider = Some(super::super::equine::Rider {
                forward: 1.0,
                yaw: 90.0,
                client,
                ..Default::default()
            });
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            for _ in 0..5 {
                tick(&mut m, w, level, &mut events);
            }
            out = Some(m);
        });
        out
    }

    #[test]
    #[ignore = "daily"]
    fn the_server_leaves_a_horse_to_its_riders_client() {
        let Some(moved) = ridden(false) else {
            return;
        };
        // The host's own ride: the horse walks west and falls through the air.
        assert!(moved.position.x < 0.4, "{:?}", moved.position);
        assert!(moved.position.y < 64.0, "{:?}", moved.position);
        let held = ridden(true).unwrap();
        assert_eq!(held.position, Vec3::new(0.5, 64.0, 0.5));
        assert_eq!(held.velocity, Vec3::ZERO);
        assert_eq!(held.yaw, 90.0, "tickRidden still turns it");
    }
}

#[cfg(test)]
mod bed_wake_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    #[test]
    #[ignore = "daily"]
    fn loaded_sleeping_position_refreshes_in_actual_mob_tick() {
        use sunrise_core::nbt::Tag;
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let stone = registry.default_state("minecraft:stone").unwrap();
            for x in 0..12 {
                for z in 0..12 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            let mut bed = registry
                .state(registry.default_state("minecraft:red_bed").unwrap())
                .unwrap()
                .clone();
            bed.properties.insert("facing".into(), "north".into());
            bed.properties.insert("occupied".into(), "false".into());
            for (part, z) in [("head", 0), ("foot", 1)] {
                bed.properties.insert("part".into(), part.into());
                world.set(0, 64, z, registry.id(&bed).unwrap()).unwrap();
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            assert_eq!(w.block_name(0, 64, 0), "minecraft:red_bed");
            assert_eq!(w.prop(0, 64, 0, "part"), Some("head"));
            assert_eq!(w.prop(0, 64, 0, "facing"), Some("north"));
            let types = [(
                "minecraft:villager".to_owned(),
                std::sync::Arc::new(types.types["minecraft:villager"].clone()),
            )]
            .into_iter()
            .collect();
            let input = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:villager",Pos:[0.5d,64.5625d,0.5d],Health:20f,NoAI:1b,NoGravity:1b,PersistenceRequired:1b,sleeping_pos:[I;0,64,0],Motion:[0d,0d,0d]}"#).unwrap().0;
            let mut loaded =
                crate::entity_nbt::mob_from_nbt(&input, &types, 233, Some(w.registry)).unwrap();
            assert_eq!(
                loaded.position,
                Vec3::new(0.5, 64.5625, 0.5),
                "loading alone must not perform the first-tick correction"
            );
            assert!(loaded.first_tick && loaded.no_ai && loaded.no_gravity);
            assert!(super::super::villager::sleeping(&loaded));
            assert_eq!(
                super::super::villager::sleeping_pos(&loaded),
                Some([0, 64, 0])
            );
            let mut awake_input = input.clone();
            let Tag::Compound(ref mut fields) = awake_input else {
                unreachable!()
            };
            fields.remove(&"sleeping_pos".into());
            fields.insert(
                "Pos".into(),
                Tag::List(
                    [5.5_f64, 64.0, 5.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
            let mut awake =
                crate::entity_nbt::mob_from_nbt(&awake_input, &types, 234, Some(w.registry))
                    .unwrap();
            let mut normal = awake.clone();
            normal.id = 235;
            let mut normal_events = Vec::new();
            assert!(super::super::villager::start_sleeping(
                &mut normal,
                [0, 64, 0],
                w,
                &mut normal_events
            ));
            assert_eq!(normal.position, Vec3::new(0.5, 64.6875, 0.5));
            assert!(normal_events.iter().any(|e| matches!(
                e,
                MobEvent::BedOccupied {
                    pos: [0, 64, 0],
                    occupied: true
                }
            )));
            let mut fixture = crate::health::TestLevel::new(0);
            let mut loaded_events = Vec::new();
            for n in 1..=2 {
                assert!(loaded.tick(w, &mut fixture.level(), &mut loaded_events));
                assert_eq!(loaded.age, n);
                assert!(!loaded.first_tick);
                assert_eq!(
                    loaded.position,
                    Vec3::new(0.5, 64.6875, 0.5),
                    "actual loaded sleeper at tick {n}"
                );
                assert_eq!(loaded.velocity, Vec3::ZERO);
                assert_eq!(loaded.health, 20.0);
                assert_eq!(
                    super::super::villager::sleeping_pos(&loaded),
                    Some([0, 64, 0])
                );
                assert!(awake.tick(w, &mut fixture.level(), &mut Vec::new()));
                assert_eq!(awake.position, Vec3::new(5.5, 64.0, 5.5));
                assert!(!super::super::villager::sleeping(&awake));
                assert!(normal.tick(w, &mut fixture.level(), &mut normal_events));
                assert_eq!(normal.position, Vec3::new(0.5, 64.6875, 0.5));
                assert_eq!(normal.velocity, Vec3::ZERO);
            }
            assert!(
                !loaded_events
                    .iter()
                    .any(|e| matches!(e, MobEvent::BedOccupied { .. })),
                "refresh must not re-run startSleeping side effects"
            );
        });
        assert!(ran, "prepared Mob/bed world must actually run");
    }

    /// `LivingEntity.tick` wakes a sleeper whose bed is gone before
    /// `aiStep` — `Mob.no_ai` skips the step, not the check (#7875).
    fn no_ai_sleeper() -> Option<Mob> {
        let mut out = None;
        with_test_world(&[], |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:villager"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            m.no_ai = true;
            let mut events = Vec::new();
            // The empty world is all air: the "bed" it lies in is gone.
            assert!(super::super::villager::start_sleeping(
                &mut m,
                [0, 64, 0],
                w,
                &mut events
            ));
            let mut test_level = crate::health::TestLevel::new(0);
            let level = &mut test_level.level();
            tick(&mut m, w, level, &mut events);
            out = Some(m);
        });
        out
    }

    #[test]
    #[ignore = "daily"]
    fn a_sleeper_without_ai_still_stands_when_its_bed_is_gone() {
        let Some(m) = no_ai_sleeper() else { return };
        assert!(
            !super::super::villager::sleeping(&m),
            "a missing bed wakes a no-AI sleeper"
        );
    }
}

#[cfg(test)]
mod turtle_ambient_tests {
    use super::*;
    use std::sync::Arc;

    fn turtle() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:turtle".into();
        k.species = Species::Turtle;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `Turtle.getAmbientSound`: land ambient only for an adult on the ground
    /// and out of water.
    #[test]
    fn a_turtle_is_silent_in_water_in_the_air_and_as_a_baby() {
        let mut m = turtle();
        m.ground.on_ground = true;
        assert!(!turtle_ambient_silent(&m));
        m.ground.on_ground = false;
        assert!(turtle_ambient_silent(&m));
        m.ground.on_ground = true;
        m.fluids.in_water = true;
        assert!(turtle_ambient_silent(&m));
    }
}

#[cfg(test)]
mod feed_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use std::sync::Arc;

    /// `Animal.mobInteract` (#2780): food grows a tamed baby cat or wolf
    /// (`canAgeUp`), so `Wolf.mobInteract` and `Cat.mobInteract` see the click
    /// consumed and do not make it sit. Only an adult that cannot breed now,
    /// or an age-locked baby, leaves the food to the sit toggle.
    #[test]
    fn food_grows_a_tamed_baby_cat_or_wolf() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let mut level = crate::health::TestLevel::new(0);
        for id in ["minecraft:wolf", "minecraft:cat"] {
            let kind = Arc::new(data.types.get(id).unwrap().clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
            m.extra.tame = true;
            m.extra.age = -24000;
            assert!(takes_food(&m), "{id}: a tamed baby takes its food");
            assert!(feed_animal(&mut m, level.random(), None).is_some(), "{id}");
            // getSpeedUpSecondsWhenFeeding(24000): a tenth of 1200 seconds.
            assert_eq!(m.extra.age, -24000 + 120 * 20, "{id}: grew a tenth");
            assert_eq!(m.extra.in_love, 0, "{id}: a baby does not fall in love");

            m.extra.age_locked = true;
            assert!(!takes_food(&m), "{id}: an age-locked baby cannot grow");
            m.extra.age_locked = false;

            m.extra.age = 0;
            assert!(takes_food(&m), "{id}: an adult can breed");
            m.extra.in_love = 600;
            assert!(!takes_food(&m), "{id}: already in love");
            m.extra.in_love = 0;
            m.extra.age = 6000;
            assert!(!takes_food(&m), "{id}: breeding cooldown");
        }
    }

    /// `Animal.mobInteract`'s growing branch is `canAgeUp()` — `isBaby() &&
    /// !isAgeLocked()` — and a species `canBeABaby` says is never a baby does
    /// not grow when fed, however negative its `Age` is: a frog spawned with
    /// `Age` at `getBabyStartAge` keeps its food. A cow at the same age takes
    /// it and grows a tenth.
    #[test]
    fn a_mob_that_cannot_be_a_baby_never_takes_the_food_that_grows_it() {
        let mob = |id: &str, species: Species, age: i32| {
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let k = Arc::make_mut(&mut kind);
            k.id = id.into();
            k.species = species;
            let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
            m.extra.age = age;
            m
        };
        let mut level = crate::health::TestLevel::new(0);
        let mut frog = mob("minecraft:frog", Species::Frog, -24_000);
        assert!(!frog.is_baby());
        assert!(!takes_food(&frog), "a frog is never a baby");
        assert!(feed_animal(&mut frog, level.random(), None).is_none());
        assert_eq!(frog.extra.age, -24_000, "it does not grow");

        let mut cow = mob("minecraft:cow", Species::Cow, -24_000);
        assert!(cow.is_baby());
        assert!(takes_food(&cow));
        assert!(feed_animal(&mut cow, level.random(), None).is_some());
        assert_eq!(cow.extra.age, -24_000 + 120 * 20, "it grows a tenth");
    }

    /// `Turtle.canFallInLove` (#8612): a turtle that already carries an egg
    /// cannot fall in love, so `Animal.mobInteract` refuses its food — it is
    /// neither eaten nor turned into love mode.
    #[test]
    fn a_turtle_carrying_an_egg_refuses_its_food() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let mut level = crate::health::TestLevel::new(0);
        let kind = Arc::new(data.types.get("minecraft:turtle").unwrap().clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        m.extra.age = 0;

        m.extra.flag = true;
        assert!(crate::mobs::turtle::has_egg(&m));
        assert!(!takes_food(&m), "an egg-carrying turtle refuses its food");
        assert!(
            feed_animal(&mut m, level.random(), None).is_none(),
            "and does not enter love mode"
        );
        assert_eq!(m.extra.in_love, 0);

        m.extra.flag = false;
        assert!(takes_food(&m), "a turtle without an egg takes it");
        assert!(feed_animal(&mut m, level.random(), None).is_some());
        assert_eq!(m.extra.in_love, 600);
    }
}

#[cfg(test)]
mod forgiveness_tests {
    use super::*;
    use std::sync::Arc;

    fn mob(species: Species) -> Mob {
        let mut t = (*crate::mobs::sensor_scan_tests::kind()).clone();
        t.species = species;
        Mob::new(1, Arc::new(t), Vec3::ZERO, 0.0)
    }

    #[test]
    fn forgiveness_keeps_cancelled_target_but_ends_anger() {
        let mut golem = mob(Species::IronGolem);
        golem.extra.angry_at = Some(Who::Player);
        golem.extra.anger = 300;
        golem.ai.target = Some(Who::Player);
        assert!(forgives_dead_player(&golem));
        forgive_dead_player_with_change(&mut golem, super::super::TargetChange::Keep);
        assert_eq!(golem.ai.target, Some(Who::Player));
        assert_eq!(golem.extra.anger, 0);

        golem.extra.angry_at = Some(Who::Player);
        forgive_dead_player_with_change(
            &mut golem,
            super::super::TargetChange::Set(Some(Who::Mob(7))),
        );
        assert_eq!(golem.ai.target, Some(Who::Mob(7)));
    }

    #[test]
    #[ignore = "daily"]
    fn peaceful_drops_an_iron_golems_existing_player_target() {
        crate::mobs::world::with_test_world(&[], |world, _| {
            let mut world = world.clone();
            world.player = Some(crate::mobs::world::test_player(Vec3::new(1.0, 0.0, 0.0)));
            world.difficulty = super::super::spawning::LocalDifficulty::new(0, 18000, 0, 0.0);
            let mut golem = mob(Species::IronGolem);
            golem.ai.target = Some(Who::Player);
            golem.extra.anger = 300;
            golem.ai.last_hurt_by = Some(Who::Player);
            let mut events = Vec::new();
            tick(
                &mut golem,
                &world,
                &mut crate::health::TestLevel::new(0).level(),
                &mut events,
            );
            assert_eq!(golem.ai.target, None);
            assert_eq!(golem.extra.anger, 0);
            assert_eq!(golem.ai.last_hurt_by, None);
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, MobEvent::AttackPlayer(..)))
            );
        });
    }

    /// The enderman's and zombified piglin's persistent anger is the target
    /// they hold: a dying player they are angry at takes it (and the last
    /// hurt by) with them, as `NeutralMob.playerDied` does through
    /// `stopBeingAngry`.
    #[test]
    fn a_target_anger_at_the_dying_player_ends() {
        for species in [Species::Enderman, Species::ZombifiedPiglin] {
            let mut m = mob(species);
            m.ai.target = Some(Who::Player);
            m.ai.last_hurt_by = Some(Who::Player);
            forgive_dead_player(&mut m);
            assert_eq!(m.ai.target, None, "{species:?}");
            assert_eq!(m.ai.last_hurt_by, None, "{species:?}");
            // Anger at something else is not the dying player's to end.
            let mut m = mob(species);
            m.ai.target = Some(Who::Mob(7));
            forgive_dead_player(&mut m);
            assert_eq!(m.ai.target, Some(Who::Mob(7)), "{species:?}");
        }
    }

    /// The iron golem's anger is a timer and its target: the dying player
    /// it is angry at ends both (`NeutralMob.playerDied`), even once the golem
    /// has lost sight of them, and anger at another mob stays.
    #[test]
    fn an_iron_golems_timer_and_target_end_together() {
        for target in [Some(Who::Player), None] {
            let mut golem = mob(Species::IronGolem);
            golem.extra.anger = 300;
            golem.extra.angry_at = Some(Who::Player);
            golem.ai.target = target;
            golem.ai.last_hurt_by = Some(Who::Player);
            forgive_dead_player(&mut golem);
            assert_eq!(golem.extra.anger, 0, "{target:?}");
            assert_eq!(golem.extra.angry_at, None, "{target:?}");
            assert_eq!(golem.ai.target, None, "{target:?}");
            assert_eq!(golem.ai.last_hurt_by, None, "{target:?}");
        }

        let mut golem = mob(Species::IronGolem);
        golem.extra.anger = 300;
        golem.extra.angry_at = Some(Who::Mob(7));
        golem.ai.target = Some(Who::Mob(7));
        forgive_dead_player(&mut golem);
        assert_eq!(golem.extra.anger, 300, "anger at another mob stays");
        assert_eq!(golem.ai.target, Some(Who::Mob(7)));
    }
}

#[cfg(test)]
mod endermite_goal_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// `Endermite.registerGoals` is the silverfish's family list without the
    /// stone tricks: a powder-snow climb, a stroll at 1.0 and a
    /// `HurtByTargetGoal` that alerts the others. The generic monster block
    /// an unlisted species gets has none of those (stroll 0.8, no alert).
    #[test]
    #[ignore = "daily"]
    fn an_endermite_takes_the_hostile_goal_list() {
        let ran = with_test_world(&[], |_, types| {
            let m = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:endermite"].clone()),
                Vec3::ZERO,
                0.0,
            );
            assert_eq!(family(m.species()), Family::Hostile);
            assert!(
                m.ai.goals.has("ClimbOnTopOfPowderSnow"),
                "ClimbOnTopOfPowderSnowGoal"
            );
            assert!(m.ai.goals.has("Stroll"), "WaterAvoidingRandomStrollGoal");
            assert!(m.ai.targets.has("HurtByTarget"));
            assert!(m.ai.targets.has("NearestTarget"));
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod villager_ambient_tests {
    use super::*;
    use crate::mobs::world::with_test_world;

    /// The ambient sound one tick of `play_ambient` makes, after `setup`.
    fn sound(
        w: &MobWorld<'_>,
        types: &crate::mobs::MobTypes,
        kind: &str,
        setup: impl FnOnce(&mut Mob),
    ) -> Option<&'static str> {
        let mut m = Mob::new(
            1,
            std::sync::Arc::new(types.types[kind].clone()),
            Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        setup(&mut m);
        let mut events = Vec::new();
        play_ambient(&mut m, w, &mut events);
        events.into_iter().find_map(|e| match e {
            MobEvent::Sound(s, ..) => Some(s),
            _ => None,
        })
    }

    /// `Villager.getAmbientSound`: silent while asleep, the trade sound while a
    /// player has its menu open, ambient otherwise. `WanderingTrader` only has
    /// the trade branch.
    #[test]
    #[ignore = "daily"]
    fn a_villager_is_silent_asleep_and_noisy_while_trading() {
        assert!(
            with_test_world(&[], |w, types| {
                assert_eq!(
                    sound(w, types, "minecraft:villager", |_| {}),
                    Some("entity.villager.ambient")
                );
                assert_eq!(
                    sound(w, types, "minecraft:villager", |m| m.extra.flag2 = true),
                    Some("entity.villager.trade")
                );
                assert_eq!(
                    sound(w, types, "minecraft:villager", |m| m.extra.ints[1] = 1),
                    None,
                    "asleep"
                );
                assert_eq!(
                    sound(w, types, "minecraft:villager", |m| {
                        m.extra.ints[1] = 1;
                        m.extra.flag2 = true;
                    }),
                    None,
                    "sleeping wins over trading"
                );
                let trading = |m: &mut Mob| m.extra.flag2 = true;
                assert_eq!(
                    sound(w, types, "minecraft:wandering_trader", trading),
                    Some("entity.wandering_trader.trade")
                );
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod fox_screech_tests {
    use super::*;
    use crate::mobs::PlayerView;
    use crate::mobs::world::with_test_world;

    fn player_at(x: f64) -> PlayerView {
        crate::mobs::world::test_player(Vec3::new(x, 64.0, 0.5))
    }

    /// The (sound, volume) of 200 ambient sounds from one fox in `w`.
    fn sounds(w: &MobWorld<'_>, types: &crate::mobs::MobTypes) -> Vec<(&'static str, f32)> {
        let kind = std::sync::Arc::new(types.types["minecraft:fox"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        let mut events = Vec::new();
        for _ in 0..200 {
            play_ambient(&mut m, w, &mut events);
        }
        events
            .into_iter()
            .filter_map(|e| match e {
                MobEvent::Sound(s, _, volume, _) => Some((s, volume)),
                _ => None,
            })
            .collect()
    }

    /// `Fox.getAmbientSound` and `playAmbientSound`: at night with nobody
    /// within 16 blocks, about one call in ten screeches at volume 2; by day,
    /// in the End, or with a player near, never.
    #[test]
    #[ignore = "daily"]
    fn a_fox_screeches_only_at_night_with_no_player_near() {
        assert!(
            with_test_world(&[], |w, types| {
                let mut night = w.clone();
                night.daylight = false;
                let alone = sounds(&night, types);
                assert_eq!(alone.len(), 200);
                let screeches: Vec<_> = alone
                    .iter()
                    .filter(|s| s.0 == "entity.fox.screech")
                    .collect();
                assert!(!screeches.is_empty(), "no screech in 200 night calls");
                assert!(screeches.len() < 60, "{} screeches", screeches.len());
                // Played by playAmbientSound itself at volume 2...
                assert!(screeches.iter().any(|s| s.1 == 2.0));
                // ...or picked again through Mob.playAmbientSound (makeSound, volume 1).
                assert!(screeches.iter().all(|s| s.1 == 2.0 || s.1 == 1.0));
                assert!(
                    alone
                        .iter()
                        .all(|s| s.0 == "entity.fox.screech" || s.0 == "entity.fox.ambient")
                );

                let by_day = sounds(w, types);
                assert!(by_day.iter().all(|s| s.0 == "entity.fox.ambient"));

                let mut end = night.clone();
                end.daylight = true;
                end.dimension_type =
                    sunrise_world::dimension_type::DimensionFacts::vanilla("minecraft:the_end")
                        .unwrap();
                assert!(
                    sounds(&end, types)
                        .iter()
                        .any(|s| s.0 == "entity.fox.screech")
                );

                let mut near = night.clone();
                near.player = Some(player_at(16.0));
                assert!(
                    sounds(&near, types)
                        .iter()
                        .all(|s| s.0 == "entity.fox.ambient")
                );

                let mut far = night.clone();
                far.player = Some(player_at(18.0));
                assert!(
                    sounds(&far, types)
                        .iter()
                        .any(|s| s.0 == "entity.fox.screech")
                );

                let mut spectator = night.clone();
                let mut p = player_at(2.0);
                p.spectator = true;
                spectator.player = Some(p);
                assert!(
                    sounds(&spectator, types)
                        .iter()
                        .any(|s| s.0 == "entity.fox.screech")
                );
            }),
            "vanilla data is missing"
        );
    }
}

#[cfg(test)]
mod age_up_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn sleep_answer_is_applied_with_or_without_ai() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            for no_ai in [false, true] {
                for may_continue in [false, true] {
                    let kind = Arc::new(types.types["minecraft:villager"].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(4.0, 220.0, 4.0), 0.0);
                    m.no_ai = no_ai;
                    m.no_gravity = true;
                    m.persistent = true;
                    m.extra.ints[1] = 1;
                    let ext = super::super::villager::ext_mut(&mut m);
                    ext.sleeping_pos = Some([4, 220, 4]);
                    ext.may_continue_sleeping = Some(may_continue);
                    tick(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut Vec::new(),
                    );
                    assert_eq!(
                        super::super::villager::sleeping(&m),
                        may_continue,
                        "NoAI={no_ai}"
                    );
                }
            }
        });
        if !ran {
            eprintln!("sleep regression needs prepared vanilla resources");
        }
    }

    /// Official Java 26.3: NoAI leaves both age clocks running. AgeLocked
    /// holds a baby's age, but a positive breeding cooldown still expires.
    /// An undead horse's `canAgeUp` is false, so its foal's age never moves.
    #[test]
    #[ignore = "daily"]
    fn age_clocks_tick_with_no_ai_and_respect_baby_age_lock() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            for species in [
                "pig",
                "cow",
                "horse",
                "turtle",
                "skeleton_horse",
                "zombie_horse",
            ] {
                for no_ai in [true, false] {
                    for locked in [false, true] {
                        for age in [-2, 2] {
                            let kind =
                                Arc::new(types.types[&format!("minecraft:{species}")].clone());
                            let mut m = Mob::new(1, kind, Vec3::new(4.0, 220.0, 4.0), 0.0);
                            m.no_ai = no_ai;
                            m.no_gravity = true;
                            m.persistent = true;
                            m.extra.age_locked = locked;
                            super::super::finalize::set_age(&mut m, age, None);
                            let (mut test_level, mut events) =
                                (crate::health::TestLevel::new(0), Vec::new());
                            let level = &mut test_level.level();
                            let expected = if age > 0 {
                                [1, 0, 0, 0]
                            } else if locked || matches!(species, "skeleton_horse" | "zombie_horse")
                            {
                                [-2, -2, -2, -2]
                            } else {
                                [-1, 0, 0, 0]
                            };
                            for (elapsed, expected_age) in expected.into_iter().enumerate() {
                                let previous_age = m.extra.age;
                                let previous_position = m.position;
                                tick(&mut m, w, level, &mut events);
                                assert_eq!(
                                    m.extra.age,
                                    expected_age,
                                    "{species}: NoAI={no_ai}, AgeLocked={locked}, initial Age={age}, tick={}",
                                    elapsed + 1
                                );
                                if no_ai {
                                    assert_eq!(m.velocity, Vec3::ZERO);
                                    // Growing dimensions can reposition the mob at adulthood.
                                    if previous_age != -1 || expected_age != 0 {
                                        assert_eq!(m.position, previous_position);
                                    }
                                }
                            }
                            if species == "turtle" && age < 0 && !locked {
                                assert_eq!(
                                    events
                                        .iter()
                                        .filter(|event| matches!(
                                            event,
                                            MobEvent::GiftLoot {
                                                table: "minecraft:gameplay/turtle_grow",
                                                ..
                                            }
                                        ))
                                        .count(),
                                    1,
                                    "growing up drops one scute"
                                );
                            }
                        }
                    }
                }
            }
        });
        if !ran {
            eprintln!("age clock regression needs prepared vanilla resources");
        }
    }

    /// A baby goat `age` ticks from adulthood, from no vanilla data.
    fn baby_goat(age: i32) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:goat".into();
        k.species = Species::Goat;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        // setAge(-24000) as finalizeSpawn does, whose own crossing is where
        // `Goat.ageBoundaryReached` gives the newborn its 1.0.
        super::super::finalize::set_age(&mut m, age, None);
        m
    }

    /// `AgeableMob.ageUp` ends in `setAge`, and `setAge` is where
    /// `Goat.ageBoundaryReached` sets the attack damage to 2.0 for an adult:
    /// feeding a baby goat its last two meals makes it a 2.0 goat, and the
    /// meal that leaves it one tick short keeps the baby's 1.0.
    #[test]
    fn feeding_a_baby_goat_past_its_boundary_sets_the_adult_attack_damage() {
        let mut m = baby_goat(-21);
        assert_eq!(m.attrs.attack_damage, 1.0, "a newborn goat hits for 1");

        // 20 ticks of growth: age -1, still a baby.
        age_up(&mut m, 1, true);
        assert!(m.is_baby());
        assert_eq!(m.attrs.attack_damage, 1.0);

        // The next 20 ticks cross the boundary.
        age_up(&mut m, 1, true);
        assert!(!m.is_baby());
        assert_eq!(m.attrs.attack_damage, 2.0);
    }

    /// The same crossing on the tick path: `AgeableMob.aiStep` also ages the
    /// mob through `setAge`, so a goat on its last baby tick grows up with the
    /// adult damage rather than keeping the baby's.
    #[test]
    #[ignore = "daily"]
    fn a_goat_that_grows_up_on_its_tick_gets_the_adult_attack_damage() {
        let mut out: Option<(bool, f32)> = None;
        crate::mobs::world::with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:goat"].clone());
            let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
            // As a natural spawn leaves it, then on its last baby tick.
            super::super::finalize::goat_spawn(
                &mut m,
                &mut sunrise_core::random::LegacyRandom::new(0),
            );
            super::super::finalize::set_age(&mut m, -1, None);
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            tick(&mut m, w, level, &mut events);
            out = Some((m.is_baby(), m.attrs.attack_damage));
        });
        let Some((baby, damage)) = out else {
            return;
        };
        assert!(!baby, "the tick that reaches 0 grows it up");
        assert_eq!(damage, 2.0);
    }
}

#[cfg(test)]
mod in_wall_tests {
    use super::{Aabb, ShapeOp, Vec3, VoxelShape, in_wall};

    #[test]
    fn a_suffocating_partial_shape_must_reach_the_eye() {
        let slab = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let intersects = |x, y, z, eye: &Aabb| {
            (x, y, z) == (0, 0, 0)
                && slab
                    .join_is_not_empty(&VoxelShape::from_box(*eye).unwrap(), ShapeOp::AND)
                    .unwrap()
        };
        assert!(in_wall(0.6, [0.5, 0.25, 0.5], intersects));
        assert!(!in_wall(0.6, [0.5, 0.75, 0.5], intersects));
    }

    /// `Entity.isInWall` has no width guard (#5872): a hoglin (1.3965 wide)
    /// whose eye is in a solid block, or whose 1.117-wide eye box reaches a
    /// neighbouring wall column, takes in-wall damage like a narrow mob.
    #[test]
    fn wide_mobs_suffocate_by_the_eye_box() {
        let wall = |x: i32, y: i32, z: i32, eye: &Aabb| {
            x >= 0
                && VoxelShape::block()
                    .moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)))
                    .join_is_not_empty(&VoxelShape::from_box(*eye).unwrap(), ShapeOp::AND)
                    .unwrap()
        };
        // Eye inside the wall: hoglin and zombie (0.6) alike.
        assert!(in_wall(1.3965, [0.5, 64.5, 0.5], wall));
        assert!(in_wall(0.6, [0.5, 64.5, 0.5], wall));
        // Eye in air at x=-0.2: the zombie's 0.48 box (-0.44..0.04) clips
        // x=0, and the hoglin's wider box does too.
        assert!(in_wall(0.6, [-0.2, 64.5, 0.5], wall));
        // At x=-0.3 the zombie's box (-0.54..-0.06) clears the wall; the
        // hoglin's half-width 0.5586 (-0.8586..0.2586) does not.
        assert!(!in_wall(0.6, [-0.3, 64.5, 0.5], wall));
        assert!(in_wall(1.3965, [-0.3, 64.5, 0.5], wall));
        // Open air.
        assert!(!in_wall(1.3965, [-3.0, 64.5, 0.5], wall));
        // A box edge exactly on the wall face only touches it.
        assert!(!in_wall(1.0, [-0.4, 64.5, 0.5], wall));
    }
}

#[cfg(test)]
mod burn_and_rose_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_world::registry::BlockRegistry;

    fn build_with(block: &'static str) -> impl FnOnce(&mut crate::world::World, &BlockRegistry) {
        move |world, registry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let state = registry.default_state(block).unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            world.set(0, 64, 0, state).unwrap();
        }
    }

    /// Vanilla 26.3 witness recorded by `tools/test_first_tick_lava_burn_parity.py`: a NoAI pig in lava or dry air on its first and later ticks.
    #[test]
    #[ignore = "run by tools/test_first_tick_lava_burn_parity.py"]
    fn a_mob_loaded_in_lava_burns_on_its_first_tick() {
        let path = std::env::var("SUNRISE_FIRST_TICK_LAVA_BURN_WITNESS").expect(
            "run through tools/test_first_tick_lava_burn_parity.py, which records the witness",
        );
        let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "missing witness {path} ({err}); run through tools/test_first_tick_lava_burn_parity.py"
            )
        });
        struct Row {
            in_lava: bool,
            initial_fire: i32,
            initial_absorption_bits: u32,
            ticks: usize,
            hurt_time: i32,
            fire: i32,
            health_bits: u32,
            absorption_bits: u32,
        }
        let mut rows = Vec::new();
        for line in witness.lines() {
            if line.is_empty() || line.starts_with('#') || line.starts_with("in_lava\t") {
                continue;
            }
            let mut cols = line.split('\t');
            let in_lava = match cols.next().unwrap() {
                "0" => false,
                "1" => true,
                other => panic!("unexpected in_lava flag {other} in {line}"),
            };
            let initial_fire: i32 = cols.next().unwrap().parse().unwrap();
            let initial_absorption_bits: u32 =
                u32::from_str_radix(cols.next().unwrap(), 16).unwrap();
            let ticks: usize = cols.next().unwrap().parse().unwrap();
            let hurt_time: i32 = cols.next().unwrap().parse().unwrap();
            let fire: i32 = cols.next().unwrap().parse().unwrap();
            let health_bits: u32 = u32::from_str_radix(cols.next().unwrap(), 16).unwrap();
            let absorption_bits: u32 = u32::from_str_radix(cols.next().unwrap(), 16).unwrap();
            assert!(
                cols.next().is_none(),
                "unexpected trailing column in {line}"
            );
            rows.push(Row {
                in_lava,
                initial_fire,
                initial_absorption_bits,
                ticks,
                hurt_time,
                fire,
                health_bits,
                absorption_bits,
            });
        }
        assert!(!rows.is_empty(), "empty 26.3 witness");
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:lava"),
            |w, types| {
                for row in &rows {
                    let kind = Arc::new(types.types["minecraft:pig"].clone());
                    let pos = if row.in_lava {
                        Vec3::new(0.5, 64.0, 0.5)
                    } else {
                        Vec3::new(2.5, 64.0, 2.5)
                    };
                    let mut m = Mob::new(1, kind, pos, 0.0);
                    register(&mut m);
                    m.no_ai = true;
                    m.persistent = true;
                    m.fire_ticks = row.initial_fire;
                    m.absorption = f32::from_bits(row.initial_absorption_bits);
                    assert!(m.first_tick);
                    let (mut lr, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                    for _ in 0..row.ticks {
                        events.clear();
                        assert!(tick(&mut m, w, &mut lr.level(), &mut events));
                    }
                    assert_eq!(
                        m.hurt_time, row.hurt_time,
                        "lava:{} Fire:{} abs:{:08x} ticks:{}",
                        row.in_lava, row.initial_fire, row.initial_absorption_bits, row.ticks
                    );
                    assert_eq!(
                        m.fire_ticks, row.fire,
                        "lava:{} Fire:{} abs:{:08x} ticks:{}",
                        row.in_lava, row.initial_fire, row.initial_absorption_bits, row.ticks
                    );
                    assert_eq!(
                        m.health.to_bits(),
                        row.health_bits,
                        "lava:{} Fire:{} abs:{:08x} ticks:{}",
                        row.in_lava,
                        row.initial_fire,
                        row.initial_absorption_bits,
                        row.ticks
                    );
                    assert_eq!(
                        m.absorption.to_bits(),
                        row.absorption_bits,
                        "lava:{} Fire:{} abs:{:08x} ticks:{}",
                        row.in_lava,
                        row.initial_fire,
                        row.initial_absorption_bits,
                        row.ticks
                    );
                }
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `LavaCauldronBlock.entityInside`: a pig in a lava cauldron is set alight and takes lava's
    /// 4, as a cooking chicken farm needs; one standing on the rim is not inside (#455).
    #[test]
    #[ignore = "daily"]
    fn a_mob_in_a_lava_cauldron_burns() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                build_with("minecraft:lava_cauldron")(world, registry);
                // A second cauldron with powder snow on top of it.
                let cauldron = registry.default_state("minecraft:lava_cauldron").unwrap();
                let snow = registry.default_state("minecraft:powder_snow").unwrap();
                world.set(-3, 64, -3, cauldron).unwrap();
                world.set(-3, 65, -3, snow).unwrap();
            },
            |w, types| {
                for (y, hurt) in [(64.25, true), (65.0, false)] {
                    let kind = Arc::new(types.types["minecraft:pig"].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(0.5, y, 0.5), 0.0);
                    register(&mut m);
                    m.no_ai = true;
                    m.no_gravity = true;
                    let start = m.health;
                    let (mut lr, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                    assert!(tick(&mut m, w, &mut lr.level(), &mut events));
                    let expected = if hurt { start - 4.0 } else { start };
                    assert_eq!(m.health, expected, "at y {y}");
                    assert_eq!(m.fire_ticks > 0, hurt, "at y {y}");
                }

                let kind = Arc::new(types.types["minecraft:pig"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.96875, 0.5), 0.0);
                register(&mut m);
                crate::mobs::finalize::set_baby(&mut m, true);
                m.previous = m.position;
                let start = m.health;
                let (mut lr, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                // A baby fits between the walls: above the fill but within
                // the block bounds is still outside the inside shape.
                effects_from_blocks(&mut m, w, &mut lr.level(), &mut events);
                assert_eq!(m.fire_ticks, 0);
                assert_eq!(m.health, start);

                // Neither box overlaps the cauldron at the ends of the move,
                // but the sweep crosses it and ignites, thaws and hurts once.
                m.previous = Vec3::new(-0.5, 64.5, 0.5);
                m.position = Vec3::new(1.5, 64.5, 0.5);
                m.ticks_frozen = 100;
                effects_from_blocks(&mut m, w, &mut lr.level(), &mut events);
                assert_eq!(m.fire_ticks, 300);
                assert_eq!(m.health, start - 4.0);
                assert_eq!(m.ticks_frozen, 0);

                // The hurt runs after the ignition: a pig the lava kills is
                // already burning when it dies.
                let kind = Arc::new(types.types["minecraft:pig"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.25, 0.5), 0.0);
                register(&mut m);
                m.previous = m.position;
                m.health = 1.0;
                effects_from_blocks(&mut m, w, &mut lr.level(), &mut events);
                assert!(!m.alive());
                assert_eq!(m.fire_ticks, 300);

                // CLEAR_FREEZE thaws a fire-immune mob, which the lava neither
                // lights nor hurts.
                let kind = Arc::new(types.types["minecraft:blaze"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.25, 0.5), 0.0);
                register(&mut m);
                m.previous = m.position;
                m.ticks_frozen = 100;
                let (start, fire) = (m.health, m.fire_ticks);
                effects_from_blocks(&mut m, w, &mut lr.level(), &mut events);
                assert_eq!((m.ticks_frozen, m.fire_ticks, m.health), (0, fire, start));

                // Feet in the lava and back in the powder snow above, in one
                // step: the ignition comes first, so the burning pig melts the
                // snow before EXTINGUISH puts it out.
                let kind = Arc::new(types.types["minecraft:pig"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(-2.5, 64.25, -2.5), 0.0);
                register(&mut m);
                m.previous = m.position;
                let start = m.health;
                events.clear();
                effects_from_blocks(&mut m, w, &mut lr.level(), &mut events);
                assert_eq!(m.health, start - 4.0);
                assert!(!m.is_on_fire());
                assert!(events.iter().any(|event| matches!(
                    event,
                    MobEvent::BreakBlock {
                        pos: [-3, 65, -3],
                        ..
                    }
                )));
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    fn golem(types: &crate::mobs::MobTypes) -> Mob {
        let kind = Arc::new(types.types["minecraft:iron_golem"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        register(&mut m);
        m
    }

    fn zombie(types: &crate::mobs::MobTypes) -> Mob {
        let kind = Arc::new(types.types["minecraft:zombie"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        register(&mut m);
        m.no_ai = true;
        m.random = sunrise_core::random::LegacyRandom::new(3);
        m
    }

    /// `Mob.isSunBurnTick`: `isInWaterOrRain() || isInPowderSnow || wasInPowderSnow`
    /// is not burnable, so a zombie standing in powder snow only freezes. The
    /// draws are the mob's own, so the answer over a run of them is fixed.
    #[test]
    #[ignore = "daily"]
    fn powder_snow_keeps_a_zombie_from_burning_in_daylight() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:powder_snow"),
            |w, types| {
                let mut day = w.clone();
                day.monsters_burn = true;
                let mut m = zombie(types);
                let burns = |m: &mut Mob| (0..500).any(|_| sun_burn_tick(m, &day));
                assert!(!burns(&mut m), "in the powder snow at (0, 64, 0)");
                // The same mob out of it, in plain daylight: the same draws burn it.
                m.position = Vec3::new(3.5, 64.0, 3.5);
                assert!(!m.in_powder_snow);
                assert!(burns(&mut m), "in plain daylight");
                // Having been in the powder snow last tick (`wasInPowderSnow`).
                m.was_in_powder_snow = true;
                assert!(!burns(&mut m), "was in the powder snow");
                // It waits that one tick before the sun reaches it.
                m.was_in_powder_snow = false;
                assert!(burns(&mut m), "the tick after it stepped out");
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `PowderSnowBlock.entityInside`: a mob that is already burning destroys
    /// the powder snow (`mobGriefing`) and is put out, once, and then no longer
    /// catches fire there in daylight.
    #[test]
    #[ignore = "daily"]
    fn a_burning_zombie_in_powder_snow_breaks_the_block_and_is_put_out() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:powder_snow"),
            |w, types| {
                let mut day = w.clone();
                day.monsters_burn = true;
                let mut m = zombie(types);
                m.ignite_for_ticks(100);
                assert!(m.is_on_fire());
                let (mut lr, mut events) = (crate::health::TestLevel::new(7), Vec::new());
                assert!(tick(&mut m, &day, &mut lr.level(), &mut events));
                let broken: Vec<([i32; 3], bool)> = events
                    .iter()
                    .filter_map(|e| match e {
                        MobEvent::BreakBlock { pos, drop, .. } => Some((*pos, *drop)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(broken, vec![([0, 64, 0], false)], "the powder snow breaks");
                assert_eq!(m.fire_ticks, 0, "and puts the zombie out");
                events.clear();
                for _ in 0..200 {
                    tick(&mut m, &day, &mut lr.level(), &mut events);
                }
                assert!(!m.removed, "the zombie stays");
                assert_eq!(m.fire_ticks, 0, "no sun burn while it stands in it");
                assert!(
                    !events
                        .iter()
                        .any(|e| matches!(e, MobEvent::BreakBlock { .. })),
                    "nothing left to destroy"
                );
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `WitherRoseBlock.entityInside` tests the world difficulty and
    /// `isInvulnerableTo(wither)`, not the regional difficulty of a fresh
    /// chunk and not the hit timer: a golem that was just hurt still gets it.
    #[test]
    #[ignore = "daily"]
    fn wither_rose_applies_wither_whatever_the_hit_timer_and_local_difficulty() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:wither_rose"),
            |w, types| {
                let mut m = golem(types);
                m.invulnerable = 15;
                assert_eq!(w.difficulty.base, 2);
                flower_contact_effects(&mut m, w);
                assert!(m.has_effect("minecraft:wither"), "normal difficulty");

                let mut peaceful = w.clone();
                peaceful.difficulty = crate::mobs::spawning::LocalDifficulty::new(0, 18000, 0, 0.0);
                let mut m = golem(types);
                flower_contact_effects(&mut m, &peaceful);
                assert!(!m.has_effect("minecraft:wither"), "peaceful");
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `Entity.checkInsideBlocks` runs `WitherRoseBlock.entityInside` for every
    /// block the box sweeps between the tick's start and end: a zombie falling
    /// three blocks in one tick (about terminal speed) passes through the
    /// rose's cell between the two boxes and gets wither, while one standing
    /// beside it does not.
    #[test]
    #[ignore = "daily"]
    fn a_mob_falling_past_a_wither_rose_gets_wither_and_one_beside_it_does_not() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:wither_rose"),
            |w, types| {
                // Boxes 65..67 and 62..64: the rose's cell 64 is between them.
                let mut falling = zombie(types);
                falling.previous = Vec3::new(0.5, 65.0, 0.5);
                falling.position = Vec3::new(0.5, 62.0, 0.5);
                flower_contact_effects(&mut falling, w);
                assert!(
                    falling.has_effect("minecraft:wither"),
                    "inside the rose on the way past it"
                );

                let mut beside = zombie(types);
                beside.previous = Vec3::new(3.5, 64.0, 0.5);
                beside.position = Vec3::new(3.5, 64.0, 0.5);
                flower_contact_effects(&mut beside, w);
                assert!(!beside.has_effect("minecraft:wither"), "beside it");
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// `applyEffectsFromBlocks` runs after `baseTick` counted the fire and
    /// the invulnerable time down: standing in lava a golem reads Fire 300
    /// (not 299) after its first tick, and the hit that tick starts has its
    /// full invulnerable time.
    #[test]
    #[ignore = "daily"]
    fn lava_ignites_to_the_full_time_and_hits_after_the_timers_count_down() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            build_with("minecraft:lava"),
            |w, types| {
                let mut m = golem(types);
                m.no_ai = true;
                m.no_gravity = true;
                let mut events = Vec::new();
                let start = m.health;
                assert!(tick(&mut m, w, level, &mut events));
                assert_eq!(m.health, start - 4.0, "one lava hit");
                assert_eq!(m.fire_ticks, 300, "fire after the first tick");
                assert_eq!(m.invulnerable, 20, "the hit's own invulnerable time");
                for _ in 0..10 {
                    assert!(tick(&mut m, w, level, &mut events));
                }
                assert_eq!(m.health, start - 8.0, "second hit on tick 11");
                assert_eq!(m.fire_ticks, 300);
            },
        );
        assert!(ran, "the vanilla test resources must be available");
    }
}

#[cfg(test)]
mod wither_block_slowdown_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world;
    use std::sync::Arc;
    use sunrise_world::registry::BlockRegistry;

    fn build_with(block: &'static str) -> impl FnOnce(&mut crate::world::World, &BlockRegistry) {
        move |world, registry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let state = registry.default_state(block).unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world
                        .chunks
                        .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 63, z, stone).unwrap();
                }
            }
            world.set(0, 64, 0, state).unwrap();
        }
    }

    fn mob(types: &crate::mobs::MobTypes, id: &str) -> Mob {
        let kind = Arc::new(types.types[id].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        register(&mut m);
        m
    }

    /// `SweetBerryBushBlock.entityInside` -> `makeStuckInBlock`: a regular mob
    /// (e.g. cow) is slowed down and its fall distance is reset to 0.
    #[test]
    #[ignore = "daily"]
    fn cow_gets_stuck_in_sweet_berry_bush() {
        let ran =
            with_built_test_world(&[], build_with("minecraft:sweet_berry_bush"), |w, types| {
                let mut m = mob(types, "minecraft:cow");
                m.fall_distance = 4.5;
                m.stuck_multiplier = Vec3::ZERO;
                let mut test_level = crate::health::TestLevel::new(42);
                let mut level = test_level.level();
                let mut events = Vec::new();
                effects_from_blocks(&mut m, w, &mut level, &mut events);
                assert_eq!(m.fall_distance, 0.0, "cow fall distance reset");
                assert_eq!(
                    m.stuck_multiplier,
                    Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32)),
                    "cow stuck multiplier applied"
                );
            });
        assert!(ran, "vanilla data is missing");
    }

    /// `PowderSnowBlock.entityInside` -> `makeStuckInBlock`: a regular mob
    /// (e.g. cow) in powder snow is slowed down and its fall distance is reset to 0.
    #[test]
    #[ignore = "daily"]
    fn cow_gets_stuck_in_powder_snow() {
        let ran = with_built_test_world(&[], build_with("minecraft:powder_snow"), |w, types| {
            let mut m = mob(types, "minecraft:cow");
            m.fall_distance = 4.5;
            m.stuck_multiplier = Vec3::ZERO;
            let mut test_level = crate::health::TestLevel::new(42);
            let mut level = test_level.level();
            let mut events = Vec::new();
            effects_from_blocks(&mut m, w, &mut level, &mut events);
            assert_eq!(m.fall_distance, 0.0, "cow fall distance reset");
            assert_eq!(
                m.stuck_multiplier,
                Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)),
                "cow powder snow stuck multiplier applied"
            );
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `WitherBoss.makeStuckInBlock` is a NO-OP: a wither inside sweet berry bush
    /// does not receive stuck multiplier and preserves its fall distance.
    #[test]
    #[ignore = "daily"]
    fn wither_does_not_get_stuck_in_sweet_berry_bush() {
        let ran =
            with_built_test_world(&[], build_with("minecraft:sweet_berry_bush"), |w, types| {
                let mut m = mob(types, "minecraft:wither");
                m.fall_distance = 4.5;
                m.stuck_multiplier = Vec3::ZERO;
                let mut test_level = crate::health::TestLevel::new(42);
                let mut level = test_level.level();
                let mut events = Vec::new();
                effects_from_blocks(&mut m, w, &mut level, &mut events);
                assert_eq!(m.fall_distance, 4.5, "wither fall distance preserved");
                assert_eq!(m.stuck_multiplier, Vec3::ZERO, "wither not slowed");
            });
        assert!(ran, "vanilla data is missing");
    }

    /// `WitherBoss.makeStuckInBlock` is a NO-OP: a wither inside powder snow
    /// does not receive stuck multiplier and preserves its fall distance.
    #[test]
    #[ignore = "daily"]
    fn wither_does_not_get_stuck_in_powder_snow() {
        let ran = with_built_test_world(&[], build_with("minecraft:powder_snow"), |w, types| {
            let mut m = mob(types, "minecraft:wither");
            m.fall_distance = 4.5;
            m.stuck_multiplier = Vec3::ZERO;
            let mut test_level = crate::health::TestLevel::new(42);
            let mut level = test_level.level();
            let mut events = Vec::new();
            effects_from_blocks(&mut m, w, &mut level, &mut events);
            assert_eq!(m.fall_distance, 4.5, "wither fall distance preserved");
            assert_eq!(m.stuck_multiplier, Vec3::ZERO, "wither not slowed");
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `SweetBerryBushBlock.entityInside`: Wither is NOT exempt from damage
    /// when moving through a mature bush (age > 0).
    #[test]
    #[ignore = "daily"]
    fn wither_takes_damage_when_moving_in_mature_sweet_berry_bush() {
        let ran = with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let stone = registry.default_state("minecraft:stone").unwrap();
                let default_bush = registry
                    .default_state("minecraft:sweet_berry_bush")
                    .unwrap();
                let mut state = registry.state(default_bush).unwrap().clone();
                state.properties.insert("age".into(), "3".into());
                let mature_bush = registry.id(&state).unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                world.set(0, 64, 0, mature_bush).unwrap();
            },
            |w, types| {
                let mut m = mob(types, "minecraft:wither");
                let initial_health = m.health;
                m.previous = Vec3::new(0.5, 64.0, 0.5);
                m.position = Vec3::new(0.52, 64.0, 0.5);
                let mut test_level = crate::health::TestLevel::new(42);
                let mut level = test_level.level();
                let mut events = Vec::new();
                effects_from_blocks(&mut m, w, &mut level, &mut events);
                assert_eq!(
                    m.health,
                    initial_health - 0.86,
                    "wither takes sweet berry bush damage through armor"
                );
                assert_eq!(m.stuck_multiplier, Vec3::ZERO, "wither still not slowed");
            },
        );
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod silverfish_walk_target_tests {
    use super::*;
    use crate::mobs::world::with_built_test_world;

    /// `Silverfish.getWalkTargetValue`: a block standing on an infested host
    /// (a stone-family block) is worth 10, and everywhere else falls through
    /// to the monster's light value. Sunrise treated stone like any other
    /// block, so a silverfish had no reason to walk back into the wall.
    #[test]
    #[ignore = "daily"]
    fn a_silverfish_prefers_a_block_on_an_infested_host() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            for cx in -1..1 {
                for cz in -1..1 {
                    world.chunks.insert(
                        (cx, cz),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                    );
                }
            }
            for x in -4..4 {
                for z in -4..4 {
                    world.set(x, 64, z, stone).unwrap();
                }
            }
            world.set(2, 64, 0, dirt).unwrap();
        };
        let ran = with_built_test_world(&[], build, |w, types| {
            let m = Mob::new(
                1,
                std::sync::Arc::new(types.types["minecraft:silverfish"].clone()),
                Vec3::new(0.5, 65.0, 0.5),
                0.0,
            );
            assert_eq!(
                walk_target_value(&m, w, [0, 65, 0]),
                10.0,
                "standing on stone"
            );
            let dirt = walk_target_value(&m, w, [2, 65, 0]);
            assert!(dirt < 10.0, "standing on dirt falls through, got {dirt}");
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod iron_golem_push_tests {
    use super::*;
    use crate::mobs::world::with_test_world;
    use std::sync::Arc;

    fn mob(id: u32, species: Species, category: &str) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = species;
        kind.category = category.into();
        Mob::new(id, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// `IronGolem.doPush`: a monster that is not a creeper and bumps the golem
    /// becomes its target one time in 20, and the draw is made for those alone,
    /// so a creeper or an animal leaves the golem's random alone.
    #[test]
    #[ignore = "daily"]
    fn an_iron_golem_targets_the_monster_that_bumps_it() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // CI has no prepared 26.3 data, so a world that does not build skips
        // the whole check rather than asserting the data exists.
        let mut hits = |victim: Mob| -> Option<u32> {
            let seen = victim.seen(None, &crate::items::Items::empty());
            let mut hits = 0;
            let ran = with_test_world(&[seen], |w, _| {
                let mut golem = mob(1, Species::IronGolem, "monster");
                for _ in 0..400 {
                    let mut events = Vec::new();
                    push_entities(&mut golem, w, level, &mut events);
                    if golem.ai.target.is_some() {
                        hits += 1;
                        golem.ai.target = None;
                    }
                }
            });
            ran.then_some(hits)
        };
        let Some(zombie) = hits(mob(2, Species::Zombie, "monster")) else {
            return;
        };
        assert!(
            zombie > 0,
            "a zombie that bumps the golem is taken as its target"
        );
        assert_eq!(
            hits(mob(3, Species::Creeper, "monster")),
            Some(0),
            "IronGolem.doPush never targets a creeper"
        );
        assert_eq!(
            hits(mob(4, Species::Cow, "creature")),
            Some(0),
            "a pushed animal is not an Enemy"
        );
    }
    /// #7309: a mob's melee hit is its attack damage attribute plus the
    /// `attack_damage` modifier of the held item (`Mob.doHurtTarget`), for
    /// every mob that holds a weapon. The modifiers are vanilla's, from the
    /// items' components in the data reports (iron axe 8, iron sword 5,
    /// diamond sword 6, stone sword 4, golden axe 6, trident 8, bow none).
    #[test]
    #[ignore = "daily"]
    fn a_held_weapon_adds_its_attack_damage_to_the_melee_hit() {
        // (mob, main hand, the item's attack_damage modifier)
        let cases: [(&str, Option<&str>, f32); 9] = [
            ("minecraft:vindicator", Some("minecraft:iron_axe"), 8.0),
            ("minecraft:vindicator", None, 0.0),
            ("minecraft:zombie", Some("minecraft:iron_sword"), 5.0),
            ("minecraft:zombie", Some("minecraft:diamond_sword"), 6.0),
            ("minecraft:zombie", None, 0.0),
            (
                "minecraft:wither_skeleton",
                Some("minecraft:stone_sword"),
                4.0,
            ),
            ("minecraft:skeleton", Some("minecraft:bow"), 0.0),
            ("minecraft:piglin_brute", Some("minecraft:golden_axe"), 6.0),
            ("minecraft:drowned", Some("minecraft:trident"), 8.0),
        ];
        let mut got = Vec::new();
        let ran = with_test_world(&[], |w, types| {
            for (kind, hand, _) in cases {
                let mut m = Mob::new(
                    1,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                m.extra.set_main_hand(hand.map(Arc::from));
                let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                let mut cx = Cx {
                    w,
                    level: &mut test_level.level(),
                    events: &mut events,
                };
                do_hurt_target(&mut m, Who::Player, &mut cx);
                got.push((
                    m.attrs.attack_damage,
                    events.iter().find_map(|e| match e {
                        MobEvent::AttackPlayer(amount, ..)
                        | MobEvent::AttackPlayerWithEffect { amount, .. } => Some(*amount),
                        _ => None,
                    }),
                ));
            }
        });
        if !ran {
            return;
        }
        for ((kind, hand, modifier), (base, hit)) in cases.iter().zip(got) {
            assert_eq!(hit, Some(base + modifier), "{kind} holding {hand:?}");
        }
    }
}

#[cfg(test)]
mod allay_healing_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn allays_heal_on_tenth_ticks_without_enabling_noai_work() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            for no_ai in [true, false] {
                for initial in [10.0_f32, 19.5, 20.0] {
                    let kind = Arc::new(types.types["minecraft:allay"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(16.5, 101.0, 16.5), 0.0);
                    mob.no_ai = no_ai;
                    mob.no_gravity = true;
                    mob.persistent = true;
                    mob.health = initial;
                    mob.extra.ints[2] = 37;
                    mob.extra.allay.get_or_insert_with(Default::default);
                    super::super::allay::set_pickup_cooldown(&mut mob, Some(41));
                    mob.extra.egg_time = 30;
                    assert_eq!(mob.attrs.max_health, 20.0);
                    assert_eq!(mob.age, 0);
                    let mut test_level = crate::health::TestLevel::new(0);
                    let mut level = test_level.level();
                    let mut events = Vec::new();
                    for elapsed in 1..=20 {
                        assert!(tick(&mut mob, w, &mut level, &mut events));
                        assert_eq!(mob.age, elapsed);
                        assert_eq!(
                            mob.health,
                            (initial + (elapsed / 10) as f32).min(20.0),
                            "NoAI={no_ai}, initial={initial}, tick={elapsed}"
                        );
                        assert_eq!(mob.extra.egg_time, 30 - elapsed as i32);
                        let skipped = if no_ai { 0 } else { elapsed as i32 };
                        assert_eq!(mob.extra.ints[2], 37 - skipped);
                        assert_eq!(
                            super::super::allay::pickup_cooldown(&mob),
                            Some(41 - skipped)
                        );
                    }
                }
            }
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; Allay tick coverage did not run");
        }
    }
}

#[cfg(test)]
mod noai_ridden_tests {
    use super::*;
    use std::sync::Arc;

    const WITNESS_ENV: &str = "SUNRISE_NOAI_RIDDEN_TRAVEL_WITNESS";

    struct WitnessRow<'a> {
        case: &'a str,
        saddled: bool,
        no_ai: bool,
        client_auth: bool,
        init_vx: f64,
        init_vy: f64,
        init_vz: f64,
        forward: f32,
        strafe: f32,
        rider_yaw: f32,
        rider_pitch: f32,
        tick: u32,
        px: u64,
        py: u64,
        pz: u64,
        vx: u64,
        vy: u64,
        vz: u64,
        yaw: u32,
        pitch: u32,
        body_yaw: u32,
        head_yaw: u32,
        on_ground: bool,
    }

    struct ActiveCase<'a> {
        case: &'a str,
        mob: Mob,
        level: crate::health::TestLevel,
        events: Vec<MobEvent>,
        last_tick: u32,
    }

    fn parse_bool(s: &str) -> bool {
        match s {
            "0" => false,
            "1" => true,
            other => panic!("unexpected bool token {other:?}"),
        }
    }

    fn parse_f64_bits(s: &str) -> u64 {
        u64::from_str_radix(s, 16).unwrap_or_else(|e| panic!("bad f64 hex {s:?}: {e}"))
    }

    fn parse_f32_bits(s: &str) -> u32 {
        u32::from_str_radix(s, 16).unwrap_or_else(|e| panic!("bad f32 hex {s:?}: {e}"))
    }

    fn parse_witness(text: &str) -> Vec<WitnessRow<'_>> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let c: Vec<&str> = line.split('\t').collect();
                assert_eq!(c.len(), 23, "unexpected column count: {line}");
                WitnessRow {
                    case: c[0],
                    saddled: parse_bool(c[1]),
                    no_ai: parse_bool(c[2]),
                    client_auth: parse_bool(c[3]),
                    init_vx: f64::from_bits(parse_f64_bits(c[4])),
                    init_vy: f64::from_bits(parse_f64_bits(c[5])),
                    init_vz: f64::from_bits(parse_f64_bits(c[6])),
                    forward: f32::from_bits(parse_f32_bits(c[7])),
                    strafe: f32::from_bits(parse_f32_bits(c[8])),
                    rider_yaw: f32::from_bits(parse_f32_bits(c[9])),
                    rider_pitch: f32::from_bits(parse_f32_bits(c[10])),
                    tick: c[11].parse().unwrap(),
                    px: parse_f64_bits(c[12]),
                    py: parse_f64_bits(c[13]),
                    pz: parse_f64_bits(c[14]),
                    vx: parse_f64_bits(c[15]),
                    vy: parse_f64_bits(c[16]),
                    vz: parse_f64_bits(c[17]),
                    yaw: parse_f32_bits(c[18]),
                    pitch: parse_f32_bits(c[19]),
                    body_yaw: parse_f32_bits(c[20]),
                    head_yaw: parse_f32_bits(c[21]),
                    on_ground: parse_bool(c[22]),
                }
            })
            .collect()
    }

    /// Vanilla 26.3 parity (#3040): a ridden NoAI donkey matches the vanilla 26.3 witness
    /// `tools/test_noai_ridden_travel_parity.py` records bit-for-bit.
    #[test]
    #[ignore = "run by tools/test_noai_ridden_travel_parity.py"]
    fn a_ridden_noai_donkey_travels_and_turns_with_its_rider() {
        let witness_path = std::env::var(WITNESS_ENV).unwrap_or_else(|_| {
            panic!(
                "{WITNESS_ENV} is not set; run through tools/test_noai_ridden_travel_parity.py, which records the witness"
            )
        });
        let witness_file = {
            let path = std::path::PathBuf::from(&witness_path);
            if path.is_dir() {
                path.join("noai_ridden_travel.tsv")
            } else {
                path
            }
        };
        let witness = std::fs::read_to_string(&witness_file).unwrap_or_else(|err| {
            panic!(
                "could not read {} (from {WITNESS_ENV}): {err}; run through tools/test_noai_ridden_travel_parity.py",
                witness_file.display()
            )
        });
        let rows = parse_witness(&witness);
        assert!(!rows.is_empty(), "26.3 witness must not be empty");
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                let stone = registry.default_state("minecraft:stone").unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
            },
            |w, types| {
                let kind = Arc::new(types.types["minecraft:donkey"].clone());
                let mut active: Option<ActiveCase<'_>> = None;
                for row in &rows {
                    if active.as_ref().is_none_or(|a| a.case != row.case) {
                        assert_eq!(row.tick, 1, "{}: first tick must be 1", row.case);
                        let mut mob = Mob::new(1, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0);
                        mob.no_ai = row.no_ai;
                        mob.persistent = true;
                        mob.extra.tame = true;
                        if row.saddled {
                            mob.set_saddle(Some(crate::items::ItemStack::new(
                                "minecraft:saddle",
                                1,
                            )));
                        }
                        mob.velocity = Vec3::new(row.init_vx, row.init_vy, row.init_vz);
                        active = Some(ActiveCase {
                            case: row.case,
                            mob,
                            level: crate::health::TestLevel::new(0),
                            events: Vec::new(),
                            last_tick: 0,
                        });
                    }
                    let state = active.as_mut().unwrap();
                    assert_eq!(
                        row.tick,
                        state.last_tick + 1,
                        "{}: contiguous ticks",
                        row.case
                    );
                    state.last_tick = row.tick;
                    state.mob.rider = Some(super::super::equine::Rider {
                        forward: row.forward,
                        strafe: row.strafe,
                        yaw: row.rider_yaw,
                        pitch: row.rider_pitch,
                        client: row.client_auth,
                        ..Default::default()
                    });
                    assert!(tick(
                        &mut state.mob,
                        w,
                        &mut state.level.level(),
                        &mut state.events
                    ));
                    let ctx = format!("{} tick {}", row.case, row.tick);
                    assert_eq!(state.mob.position.x.to_bits(), row.px, "{ctx}: px");
                    assert_eq!(state.mob.position.y.to_bits(), row.py, "{ctx}: py");
                    assert_eq!(state.mob.position.z.to_bits(), row.pz, "{ctx}: pz");
                    assert_eq!(state.mob.velocity.x.to_bits(), row.vx, "{ctx}: vx");
                    assert_eq!(state.mob.velocity.y.to_bits(), row.vy, "{ctx}: vy");
                    assert_eq!(state.mob.velocity.z.to_bits(), row.vz, "{ctx}: vz");
                    assert_eq!(state.mob.yaw.to_bits(), row.yaw, "{ctx}: yaw");
                    assert_eq!(state.mob.head_pitch.to_bits(), row.pitch, "{ctx}: pitch");
                    assert_eq!(
                        state.mob.body_yaw.to_bits(),
                        row.body_yaw,
                        "{ctx}: body_yaw"
                    );
                    assert_eq!(
                        state.mob.head_yaw.to_bits(),
                        row.head_yaw,
                        "{ctx}: head_yaw"
                    );
                    assert_eq!(
                        state.mob.ground.on_ground, row.on_ground,
                        "{ctx}: on_ground"
                    );
                }
            },
        );
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod noai_species_step_tests {
    use super::*;
    use std::sync::Arc;

    /// `Bee.aiStep`, `WanderingTrader.aiStep`, `Guardian.aiStep` and `Camel.tick` run without
    /// AI: one tick counts a NoAI bee's `CannotEnterHiveTicks` and a trader's `DespawnDelay`
    /// down, flops a guardian on land, and stands a camel sitting in water up (vanilla 26.3,
    /// #2658).
    #[test]
    #[ignore = "daily"]
    fn noai_mobs_still_run_their_own_ai_step() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                for cx in 0..2 {
                    world
                        .chunks
                        .insert((cx, 0), Arc::new(crate::chunk::Chunk::empty(cx, 0)));
                }
                let stone = registry.default_state("minecraft:stone").unwrap();
                let water = registry.default_state("minecraft:water").unwrap();
                for x in 0..32 {
                    for z in 0..16 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                world.set(24, 64, 8, water).unwrap();
            },
            |w, types| {
                let mob = |id: &str, x: f64| {
                    let kind = Arc::new(types.types[id].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(x, 64.0, 8.5), 0.0);
                    m.no_ai = true;
                    m.persistent = true;
                    m.ground.on_ground = true;
                    m
                };
                let (mut level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                let mut bee = mob("minecraft:bee", 2.5);
                super::super::bee::st(&mut bee).stay_out_of_hive = 20;
                assert!(tick(&mut bee, w, &mut level.level(), &mut events));
                assert_eq!(super::super::bee::get(&bee).unwrap().stay_out_of_hive, 19);

                let mut trader = mob("minecraft:wandering_trader", 8.5);
                trader.extra.ints[5] = 20;
                assert!(tick(&mut trader, w, &mut level.level(), &mut events));
                assert_eq!(trader.extra.ints[5], 19);

                let mut guardian = mob("minecraft:guardian", 14.5);
                assert!(tick(&mut guardian, w, &mut level.level(), &mut events));
                assert_eq!(guardian.velocity.y, 0.5);
                assert!(!guardian.ground.on_ground);

                let mut camel = mob("minecraft:camel", 24.5);
                camel.extra.camel_sitting = true;
                assert!(tick(&mut camel, w, &mut level.level(), &mut events));
                assert!(camel.fluids.in_water);
                assert!(!super::super::camel::sitting(&camel));
            },
        );
        assert!(ran, "vanilla data is missing");
    }

    /// A saved NoAI bee with an extreme hive and anger deadline ticks without overflowing.
    #[test]
    #[ignore = "daily"]
    fn a_saved_noai_bee_with_extreme_hive_and_anger_ticks() {
        use sunrise_core::nbt::Tag;
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:bee"].clone());
            let fresh = Mob::new(1, kind.clone(), Vec3::new(2.5, 64.0, 8.5), 0.0);
            let mut saved = crate::entity_nbt::mob_to_nbt(
                &fresh,
                [1, 2, 3, 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let Tag::Compound(map) = &mut saved else {
                panic!("a mob saves a compound");
            };
            map.insert("NoAI".into(), Tag::Byte(1));
            map.insert(
                "hive_pos".into(),
                Tag::IntArray(vec![i32::MIN, 64, i32::MIN]),
            );
            map.insert("anger_end_time".into(), Tag::Long(i64::MAX));
            let mob_types = std::collections::HashMap::from([(kind.id.clone(), kind)]);
            let mut bee = crate::entity_nbt::mob_from_nbt(&saved, &mob_types, 1, None).unwrap();
            assert!(bee.no_ai);
            bee.age = 19;
            let mut w = w.clone();
            w.game_time = -1;
            assert!(super::super::bee::angry(&bee, &w));
            let (mut level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            assert!(tick(&mut bee, &w, &mut level.level(), &mut events));
            let b = super::super::bee::get(&bee).unwrap();
            assert_eq!(b.hive, None, "a hive 2^31 blocks away is too far");
            assert_eq!(b.anger_end, i64::MAX);
        });
        assert!(ran, "vanilla data is missing");
    }

    /// A saved `TicksFrozen` at an int extreme thaws with Java's wrapping `Math.max(0, ticks - 2)`.
    #[test]
    #[ignore = "daily"]
    fn a_saved_noai_bee_with_extreme_ticks_frozen_thaws_like_vanilla() {
        use sunrise_core::nbt::Tag;
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let kind = Arc::new(types.types["minecraft:bee"].clone());
            let mob_types = std::collections::HashMap::from([(kind.id.clone(), kind.clone())]);
            for (saved_ticks, thawed) in [(i32::MIN, i32::MAX - 1), (i32::MAX, i32::MAX - 2)] {
                let fresh = Mob::new(1, kind.clone(), Vec3::new(2.5, 64.0, 8.5), 0.0);
                let mut saved = crate::entity_nbt::mob_to_nbt(
                    &fresh,
                    [1, 2, 3, 4],
                    None,
                    None,
                    |_| None,
                    &Default::default(),
                );
                let Tag::Compound(map) = &mut saved else {
                    panic!("a mob saves a compound");
                };
                map.insert("NoAI".into(), Tag::Byte(1));
                map.insert("TicksFrozen".into(), Tag::Int(saved_ticks));
                let mut bee = crate::entity_nbt::mob_from_nbt(&saved, &mob_types, 1, None).unwrap();
                assert!(bee.no_ai);
                assert_eq!(bee.ticks_frozen, saved_ticks);
                let (mut level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                tick(&mut bee, w, &mut level.level(), &mut events);
                assert_eq!(bee.ticks_frozen, thawed);
            }
        });
        assert!(ran, "vanilla data is missing");
    }
}

#[cfg(test)]
mod ravager_noai_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn ravager_counters_tick_without_ai_and_stun_starts_a_full_roar() {
        crate::mobs::world::with_test_world(&[], |w, types| {
            for no_ai in [false, true] {
                for initial in [(20, 30, 5), (0, 0, 0), (-2, -3, -4), (2, 2, 0)] {
                    let kind = Arc::new(types.types["minecraft:ravager"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(10.5, 200.0, 10.5), 0.0);
                    mob.no_ai = no_ai;
                    mob.no_gravity = true;
                    mob.persistent = true;
                    mob.extra.counter = initial.0;
                    mob.extra.counter2 = initial.1;
                    mob.extra.anim_ticks = initial.2;
                    let mut test_level = crate::health::TestLevel::new(0);
                    let mut events = Vec::new();
                    for elapsed in 1..=22 {
                        tick(&mut mob, w, &mut test_level.level(), &mut events);
                        let attack = if initial.0 > 0 {
                            (initial.0 - elapsed).max(0)
                        } else {
                            initial.0
                        };
                        let stun = if initial.1 > 0 {
                            (initial.1 - elapsed).max(0)
                        } else {
                            initial.1
                        };
                        let roar = match initial {
                            (20, 30, 5) => (5 - elapsed).max(0),
                            (2, 2, 0) if elapsed >= 2 => (22 - elapsed).max(0),
                            _ => initial.2,
                        };
                        assert_eq!(
                            (mob.extra.counter, mob.extra.counter2, mob.extra.anim_ticks),
                            (attack, stun, roar),
                            "NoAI={no_ai}, initial={initial:?}, tick={elapsed}"
                        );
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod air_tests {
    use super::*;

    /// A mob of `kind` in a pool, its eyes under, after `ticks` base ticks.
    fn air_after(kind: &str, air: i32, ticks: u32) -> Option<i32> {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let mut out = None;
        crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types[kind].clone());
            let mut m = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
            m.air = air;
            let mut events = Vec::new();
            let mut test_level = crate::health::TestLevel::new(0);
            for _ in 0..ticks {
                base_tick(&mut m, w, &mut test_level.level(), &mut events);
            }
            out = Some(m.air);
        });
        out
    }

    /// `LivingEntity.canBreatheUnderWater` on the drown path: the
    /// `can_breathe_under_water` tag (a guardian and a copper golem are in it, an
    /// iron golem is not), with the baby happy ghast and the sulfur cube holding a
    /// block on top of it and the golem's own `decreaseAirSupply` holding its air.
    /// 300 ticks take the air from 300 to 0 and 20 more deal the drown's 2.
    #[test]
    #[ignore = "daily"]
    fn the_types_the_jar_lists_breathe_under_water() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let drowned = |w: &MobWorld, id: &str, baby: bool, body: Option<&str>| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
                if baby {
                    m.extra.age = -24000;
                }
                m.extra.set_body(body.map(Into::into));
                let start = m.health;
                let mut events = Vec::new();
                let mut test_level = crate::health::TestLevel::new(0);
                for _ in 0..320 {
                    base_tick(&mut m, w, &mut test_level.level(), &mut events);
                }
                (start - m.health, m.air)
            };
            assert_eq!(drowned(w, "minecraft:guardian", false, None), (0.0, 300));
            assert_eq!(
                drowned(w, "minecraft:copper_golem", false, None),
                (0.0, 300)
            );
            // Not in the tag, and still no drowning damage: its decreaseAirSupply
            // is the identity, so its air stays at 300 and it never refills.
            assert_eq!(drowned(w, "minecraft:iron_golem", false, None), (0.0, 300));
            assert_eq!(drowned(w, "minecraft:happy_ghast", false, None), (2.0, 0));
            assert_eq!(drowned(w, "minecraft:happy_ghast", true, None), (0.0, 300));
            assert_eq!(drowned(w, "minecraft:sulfur_cube", false, None), (2.0, 0));
            assert_eq!(
                drowned(w, "minecraft:sulfur_cube", false, Some("minecraft:stone")),
                (0.0, 300)
            );

            // A replacement swaps which types breathe, without rebuilding their types.
            let mut items = w.items.unwrap().clone();
            items.set_entity_type_tags(crate::items::EntityTypeTags::from_ordered(
                [(
                    "minecraft:can_breathe_under_water".into(),
                    vec!["minecraft:cow".into()],
                )]
                .into(),
                items.entity_type_tags.ids().clone(),
            ));
            let w = MobWorld {
                items: Some(&items),
                ..w.clone()
            };
            assert_eq!(drowned(&w, "minecraft:guardian", false, None), (2.0, 0));
            assert_eq!(drowned(&w, "minecraft:cow", false, None), (0.0, 300));
        });
        // Without the extracted vanilla data the world cannot be built.
        assert!(ran, "vanilla data is missing");
    }

    /// `Dolphin.handleAirSupply` is empty and the dolphin isn't in
    /// `#can_breathe_under_water`: under water it loses a breath a tick from
    /// its 4800, as `LivingEntity.baseTick` takes it, so it must surface.
    #[test]
    #[ignore = "daily"]
    fn a_dolphin_holds_its_breath_under_water() {
        let Some(air) = air_after("minecraft:dolphin", 4800, 10) else {
            return;
        };
        assert_eq!(air, 4790);
    }

    /// `Dolphin.increaseAirSupply` hands back the whole 4800 maximum, so under Water
    /// Breathing a submerged dolphin is at 4800 on the next tick. An ordinary mob
    /// still climbs four at a time to 300, and a fish's own handler fills it to
    /// 300 in water. Air above the maximum is left alone.
    #[test]
    #[ignore = "daily"]
    fn a_dolphin_refills_to_its_own_maximum() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 0..=8 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let after = |id: &str, start: i32, breathing: bool| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
                if breathing {
                    m.add_effect("minecraft:water_breathing", 100, 0);
                }
                m.air = start;
                let mut events = Vec::new();
                let mut test_level = crate::health::TestLevel::new(0);
                base_tick(&mut m, w, &mut test_level.level(), &mut events);
                (m.air, m.health)
            };
            assert_eq!(after("minecraft:dolphin", 100, true), (4800, 10.0));
            assert_eq!(after("minecraft:dolphin", 4800, false).0, 4799);
            assert_eq!(after("minecraft:dolphin", 5000, true).0, 5000);
            assert_eq!(after("minecraft:zombie", 100, true).0, 104);
            assert_eq!(after("minecraft:zombie", 299, true).0, 300);
            assert_eq!(after("minecraft:cod", 100, false).0, 300);
        });
        assert!(ran, "vanilla data is missing");
    }

    /// `WaterAnimal.handleAirSupply` asks `shouldTakeDrowningDamage`, which is
    /// `getAirSupply() <= -20`: a cod out of water with `{Air:-25}` still drowns, is
    /// reset to 0 and takes the 2.
    #[test]
    #[ignore = "daily"]
    fn a_water_creature_below_minus_twenty_still_drowns() {
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let drowned = |start: i32| {
                let kind = std::sync::Arc::new(types.types["minecraft:cod"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                m.air = start;
                let health = m.health;
                let mut events = Vec::new();
                let mut test_level = crate::health::TestLevel::new(0);
                base_tick(&mut m, w, &mut test_level.level(), &mut events);
                (health - m.health, m.air)
            };
            assert_eq!(drowned(-25), (2.0, 0));
            assert_eq!(drowned(-20), (2.0, 0));
        });
        assert!(ran, "vanilla data is missing");
    }

    /// A copper golem is in `#can_breathe_under_water`: its air stays full.
    #[test]
    #[ignore = "daily"]
    fn a_copper_golem_breathes_under_water() {
        let Some(air) = air_after("minecraft:copper_golem", 300, 10) else {
            return;
        };
        assert_eq!(air, 300);
    }
}

#[cfg(test)]
mod flight_effect_fall_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn flight_effects_reset_accumulated_fall_distance_even_without_ai() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, _| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            },
            |world, types| {
                let mut failures = Vec::new();
                let mut rows = Vec::new();
                for effect in [
                    None,
                    Some("minecraft:levitation"),
                    Some("minecraft:slow_falling"),
                ] {
                    for no_ai in [false, true] {
                        for velocity_y in [0.0, 0.2, -0.2] {
                            let kind = Arc::new(types.types["minecraft:cow"].clone());
                            let mut mob = Mob::new(1, kind, Vec3::new(2.5, 64.0, 2.5), 0.0);
                            mob.no_ai = no_ai;
                            mob.no_gravity = true;
                            mob.persistent = true;
                            mob.fall_distance = 12.0;
                            mob.velocity.y = velocity_y;
                            if let Some(effect) = effect {
                                mob.add_effect(effect, 100, 0);
                            }
                            let mut test_level = crate::health::TestLevel::new(0);
                            assert!(tick(
                                &mut mob,
                                world,
                                &mut test_level.level(),
                                &mut Vec::new()
                            ));
                            // The pinned Java aiStep resets before travel. A
                            // downward move then adds 0.2F of NEW fall distance;
                            // NoAI skips movement but must still erase the old fall.
                            let expected = match (effect.is_some(), no_ai, velocity_y < 0.0) {
                                (false, false, true) => 12.200000002980232,
                                (true, false, true) => 0.20000000298023224,
                                (false, _, _) => 12.0,
                                (true, _, _) => 0.0,
                            };
                            let name = effect.unwrap_or("none").trim_start_matches("minecraft:");
                            rows.push(format!(
                                "{name} {no_ai} {:016x} {:016x}",
                                velocity_y.to_bits(),
                                mob.fall_distance.to_bits()
                            ));
                            if mob.fall_distance != expected {
                                failures.push((effect, no_ai, velocity_y, mob.fall_distance));
                            }
                        }
                    }
                }
                if let Some(path) = std::env::var_os("SUNRISE_MOB_FALL_RESET_OUTPUT") {
                    std::fs::write(path, rows.join("\n") + "\n").unwrap();
                }
                assert!(failures.is_empty(), "fall distance differs: {failures:?}");
            },
        );
        assert!(ran, "prepared resources must run the integration fixture");
    }
}

#[cfg(test)]
mod breathing_effect_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn breathing_effects_and_bubble_columns_preserve_mob_air() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let ran =
            crate::mobs::world::with_resource_test_world(
                &root,
                &[],
                true,
                true,
                |world, registry| {
                    world
                        .chunks
                        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                    for (x0, name) in [(6, "minecraft:water"), (10, "minecraft:bubble_column")] {
                        let state = registry.default_state(name).unwrap();
                        for x in x0..=x0 + 1 {
                            for z in 2..=3 {
                                for y in 63..=67 {
                                    world.set(x, y, z, state).unwrap();
                                }
                            }
                        }
                    }
                    // Mixed columns distinguish the eye-block query from feet position.
                    for (x0, bubble_at_eye) in [(6, true), (10, false)] {
                        for x in x0..=x0 + 1 {
                            for z in 6..=7 {
                                for y in 63..=67 {
                                    let bubble = (y >= 65) == bubble_at_eye;
                                    let name = if bubble {
                                        "minecraft:bubble_column"
                                    } else {
                                        "minecraft:water"
                                    };
                                    world
                                        .set(x, y, z, registry.default_state(name).unwrap())
                                        .unwrap();
                                }
                            }
                        }
                    }
                },
                |world, types| {
                    let kind = Arc::new(types.types["minecraft:cow"].clone());
                    assert!(!kind.flags.needs_water);
                    assert!(!world.entity_tag_bit(
                        &kind,
                        crate::items::EntityTypeTags::CAN_BREATHE_UNDER_WATER
                    ));
                    let mut rows = Vec::new();
                    let mut failures = Vec::new();
                    for (scene, x, z) in [
                        (0, 2.5, 2.5),
                        (1, 6.5, 2.5),
                        (2, 10.5, 2.5),
                        (3, 6.5, 6.5),
                        (4, 10.5, 6.5),
                    ] {
                        for mask in 0..8 {
                            for air in [301, 300, 10, 0, -19, -20, -21] {
                                let mut mob = Mob::new(1, kind.clone(), Vec3::new(x, 64.0, z), 0.0);
                                mob.air = air;
                                for (bit, effect) in [
                                    (1, "minecraft:water_breathing"),
                                    (2, "minecraft:conduit_power"),
                                    (4, "minecraft:breath_of_the_nautilus"),
                                ] {
                                    if mask & bit != 0 {
                                        mob.add_effect(effect, 100, 0);
                                    }
                                }
                                base_tick(
                                    &mut mob,
                                    world,
                                    &mut crate::health::TestLevel::new(0).level(),
                                    &mut Vec::new(),
                                );
                                assert_eq!(
                                    mob.fluids.eye_in_water,
                                    scene != 0,
                                    "invalid fixture immersion"
                                );
                                rows.push(format!(
                                    "{scene} {mask} {air} {} {:08x}",
                                    mob.air,
                                    mob.health.to_bits()
                                ));
                                // Selected independent goldens from the pinned Cow.baseTick
                                // oracle. The parity harness compares all 280 air/health rows.
                                let expected = match (scene, mask, air) {
                                    (1, 1, 10)
                                    | (1, 2, 10)
                                    | (1, 5, 10)
                                    | (2, 0, 10)
                                    | (2, 4, 10)
                                    | (0, 4, 10)
                                    | (3, 0, 10) => Some((14, 10.0)),
                                    (1, 4, 10) | (4, 4, 10) => Some((10, 10.0)),
                                    (4, 0, 10) => Some((9, 10.0)),
                                    (1, 0, -19) => Some((0, 8.0)),
                                    _ => None,
                                };
                                if let Some(expected) = expected
                                    && (mob.air, mob.health) != expected
                                {
                                    failures.push((scene, mask, air, mob.air, mob.health));
                                }
                            }
                        }
                    }
                    if let Some(path) = std::env::var_os("SUNRISE_MOB_BREATHING_OUTPUT") {
                        std::fs::write(path, rows.join("\n") + "\n").unwrap();
                    }
                    assert!(failures.is_empty(), "mob air/health differs: {failures:?}");
                },
            );
        assert!(
            ran,
            "prepared resources must run the production breathing fixture"
        );
    }
}

#[cfg(test)]
mod dolphin_noai_air_tests {
    use super::*;
    use std::sync::Arc;

    fn check(water: bool, no_ai: bool, initial: i32, expected: [i32; 2]) {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            if water {
                let water = registry.default_state("minecraft:water").unwrap();
                for x in 0..=4 {
                    for z in 0..=4 {
                        for y in 0..=8 {
                            world.set(x, y, z, water).unwrap();
                        }
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = Arc::new(types.types["minecraft:dolphin"].clone());
            let mut mob = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
            mob.no_ai = no_ai;
            mob.no_gravity = true;
            mob.persistent = true;
            mob.attrs.speed = 0.0;
            mob.air = initial;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            assert_eq!(mob.health, 10.0);
            assert_eq!(mob.max_air_supply(), 4800);
            for (index, expected) in expected.into_iter().enumerate() {
                assert!(tick(&mut mob, w, &mut test_level.level(), &mut events));
                assert_eq!(mob.age, (index + 1) as u32);
                assert_eq!(mob.fluids.eye_in_water, water);
                assert_eq!(mob.health, 10.0);
                assert_eq!(mob.no_ai, no_ai);
                assert_eq!(
                    mob.air,
                    expected,
                    "water={water}, NoAI={no_ai}, initial={initial}, tick={}",
                    index + 1
                );
            }
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; Dolphin full-tick coverage did not run");
        }
    }

    /// `Dolphin.tick` restores max air after super.tick for a NoAI dolphin,
    /// even when that superclass tick refilled or consumed loaded air.
    #[test]
    #[ignore = "daily"]
    fn noai_dolphins_restore_air_after_the_full_mob_tick() {
        for water in [false, true] {
            for initial in [4800, 100] {
                check(water, true, initial, [4800, 4800]);
            }
        }
    }

    /// The NoAI exception must not suppress normal underwater consumption.
    #[test]
    #[ignore = "daily"]
    fn normal_dolphins_keep_their_underwater_countdown() {
        check(true, false, 4800, [4799, 4798]);
    }
}

#[cfg(test)]
mod oxygen_bonus_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;

    #[derive(serde::Deserialize)]
    struct Witness {
        minecraft: String,
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        entity: String,
        medium: String,
        effect: Option<String>,
        stored_oxygen_bonus_bits: Option<String>,
        ticks: u32,
        attributes: Vec<AttrRow>,
    }

    #[derive(serde::Deserialize)]
    struct AttrRow {
        id: String,
        base_bits: String,
    }

    fn saved_attributes(case: &Case) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let medium = case.medium.as_str();
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                let block = match medium {
                    "air" => None,
                    "water" => Some("minecraft:water"),
                    "bubble_column" => Some("minecraft:bubble_column"),
                    other => panic!("unexpected medium {other}"),
                };
                if let Some(name) = block {
                    let state = registry.default_state(name).unwrap();
                    for x in 1..=3 {
                        for z in 1..=3 {
                            for y in 3..=8 {
                                world.set(x, y, z, state).unwrap();
                            }
                        }
                    }
                }
            },
            |w, types| {
                let kind = Arc::new(types.types[case.entity.as_str()].clone());
                let kind_map = HashMap::from([(case.entity.clone(), kind)]);
                let mut entries: Vec<(&str, Tag)> = vec![
                    ("id", Tag::String(case.entity.as_str().into())),
                    (
                        "Pos",
                        Tag::List(vec![
                            Tag::Double(2.5_f64.to_bits()),
                            Tag::Double(3.0_f64.to_bits()),
                            Tag::Double(2.5_f64.to_bits()),
                        ]),
                    ),
                    ("NoAI", Tag::Byte(1)),
                    ("NoGravity", Tag::Byte(1)),
                    ("PersistenceRequired", Tag::Byte(1)),
                ];
                if let Some(effect) = &case.effect {
                    entries.push((
                        "active_effects",
                        Tag::List(vec![Tag::compound([
                            ("id", Tag::String(effect.as_str().into())),
                            ("duration", Tag::Int(100)),
                            ("amplifier", Tag::Byte(0)),
                        ])]),
                    ));
                }
                if let Some(bits) = &case.stored_oxygen_bonus_bits {
                    let raw = u64::from_str_radix(bits, 16).unwrap();
                    entries.push((
                        "attributes",
                        Tag::List(vec![Tag::compound([
                            ("id", Tag::String("minecraft:oxygen_bonus".into())),
                            ("base", Tag::Double(raw)),
                        ])]),
                    ));
                }
                let input = Tag::compound(entries);
                let mut m = crate::entity_nbt::mob_from_nbt(&input, &kind_map, 1, None).unwrap();
                let (mut level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
                for _ in 0..case.ticks {
                    assert!(tick(&mut m, w, &mut level.level(), &mut events));
                }
                if case.ticks > 0 {
                    assert_eq!(m.fluids.eye_in_water, medium != "air", "{}", case.name);
                }
                let tag = crate::entity_nbt::mob_to_nbt(
                    &m,
                    [1, 2, 3, 4],
                    None,
                    None,
                    |_| None,
                    &Default::default(),
                );
                let Some(Tag::List(list)) = tag.get("attributes") else {
                    panic!("no attributes list for {}", case.name);
                };
                for entry in list {
                    let Some(Tag::String(name)) = entry.get("id") else {
                        panic!("attribute entry missing id for {}", case.name);
                    };
                    let Some(Tag::Double(base_bits)) = entry.get("base") else {
                        panic!("attribute entry missing double base for {}", case.name);
                    };
                    out.push((name.to_string().unwrap(), *base_bits));
                }
            },
        );
        assert!(ran, "vanilla data is missing");
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Saved `attributes` against the vanilla 26.3 witness `tools/test_mob_oxygen_bonus_parity.py` records (#2679).
    #[test]
    #[ignore = "run by tools/test_mob_oxygen_bonus_parity.py"]
    fn a_mob_losing_air_under_water_saves_oxygen_bonus() {
        let path = std::env::var_os("SUNRISE_MOB_OXYGEN_BONUS_WITNESS").unwrap_or_else(|| {
            panic!(
                "SUNRISE_MOB_OXYGEN_BONUS_WITNESS is unset; run tools/test_mob_oxygen_bonus_parity.py, which records the witness"
            )
        });
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read SUNRISE_MOB_OXYGEN_BONUS_WITNESS ({}): {err}; run tools/test_mob_oxygen_bonus_parity.py",
                std::path::Path::new(&path).display()
            )
        });
        let witness: Witness = serde_json::from_str(&raw).unwrap();
        assert_eq!(witness.minecraft, "26.3");
        assert!(!witness.cases.is_empty());
        for case in &witness.cases {
            let expected: Vec<(String, u64)> = case
                .attributes
                .iter()
                .map(|row| {
                    (
                        row.id.clone(),
                        u64::from_str_radix(&row.base_bits, 16).unwrap(),
                    )
                })
                .collect();
            assert_eq!(saved_attributes(case), expected, "{}", case.name);
        }
    }
}

#[cfg(test)]
mod noai_breather_air_tests {
    use super::*;
    use std::sync::Arc;

    fn witness_26_3_rows() -> Vec<String> {
        let raw = std::env::var("SUNRISE_NOAI_BREATHER_AIR_WITNESS").expect(
            "run through tools/test_noai_breather_air_parity.py, which records the 26.3 witness",
        );
        let mut path = std::path::PathBuf::from(&raw);
        if path.is_dir() {
            path.push("noai_breather_air.txt");
        }
        let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read {} (run tools/test_noai_breather_air_parity.py to record the 26.3 witness): {err}",
                path.display()
            )
        });
        witness
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect()
    }

    fn check(water: bool, no_ai: bool, initial: i32, expected: [i32; 2]) {
        check_species("minecraft:nautilus", water, no_ai, initial, expected);
    }

    fn check_species(id: &str, water: bool, no_ai: bool, initial: i32, expected: [i32; 2]) {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            if water {
                let state = registry.default_state("minecraft:water").unwrap();
                for x in 0..=4 {
                    for z in 0..=4 {
                        for y in 0..=8 {
                            world.set(x, y, z, state).unwrap();
                        }
                    }
                }
            }
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = Arc::new(types.types[id].clone());
            if id == "minecraft:nautilus" {
                assert!(kind.flags.needs_water);
            }
            assert!(
                w.entity_tag_bit(&kind, crate::items::EntityTypeTags::CAN_BREATHE_UNDER_WATER,)
            );
            let mut mob = Mob::new(1, kind, Vec3::new(2.5, 3.0, 2.5), 0.0);
            mob.no_ai = no_ai;
            mob.no_gravity = true;
            mob.persistent = true;
            mob.attrs.speed = 0.0;
            mob.health = 10.0;
            mob.air = initial;
            let mut test_level = crate::health::TestLevel::new(0);
            let mut events = Vec::new();
            for (index, expected) in expected.into_iter().enumerate() {
                assert!(tick(&mut mob, w, &mut test_level.level(), &mut events));
                assert_eq!(mob.age, (index + 1) as u32);
                assert_eq!(mob.fluids.eye_in_water, water);
                assert_eq!(mob.health, 10.0);
                assert_eq!(mob.no_ai, no_ai);
                assert_eq!(
                    mob.air,
                    expected,
                    "{id} water={water}, NoAI={no_ai}, initial={initial}, tick={}",
                    index + 1
                );
            }
        });
        assert!(ran, "prepared mob data must run the {id} full-tick fixture");
    }

    /// `Nautilus.baseTick`: NoAI skips only handleAirSupply; super.baseTick
    /// still refills a breathing mob by four, capped at300.
    #[test]
    #[ignore = "daily"]
    fn noai_nautilus_keeps_superclass_refill_in_both_media() {
        for water in [false, true] {
            check(water, true, 300, [300, 300]);
            check(water, true, 100, [104, 108]);
        }
    }

    /// Vanilla 26.3 (witness recorded by `tools/test_noai_breather_air_parity.py`): `LivingEntity.baseTick`
    /// refills only while the air is below the maximum, so a NoAI axolotl or nautilus
    /// loaded above it keeps its air across full mob ticks, and one just below tops up to it.
    #[test]
    #[ignore = "run by tools/test_noai_breather_air_parity.py"]
    fn noai_breathers_keep_air_above_their_maximum() {
        let rows = witness_26_3_rows();
        let mut checked = 0usize;
        for (species, max) in [("axolotl", 6000), ("nautilus", 300)] {
            let id = format!("minecraft:{species}");
            for (water, scene) in [(false, "0"), (true, "1")] {
                for initial in [max + 1, 32767, max - 6] {
                    let initial_str = initial.to_string();
                    let row = rows
                        .iter()
                        .find(|line| {
                            let mut cols = line.split_ascii_whitespace();
                            cols.next() == Some(species)
                                && cols.next() == Some(scene)
                                && cols.next() == Some("0")
                                && cols.next() == Some(initial_str.as_str())
                        })
                        .unwrap_or_else(|| {
                            panic!(
                                "missing 26.3 witness row for {species} scene={scene} air={initial}"
                            )
                        });
                    let cols: Vec<&str> = row.split_ascii_whitespace().collect();
                    let expected = [
                        cols[6].parse::<i32>().unwrap(),
                        cols[9].parse::<i32>().unwrap(),
                    ];
                    check_species(&id, water, true, initial, expected);
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 12);
    }

    /// With AI, handleAirSupply still uses the pre-super air on land and
    /// writes300 in water. The NoAI exception must not alter that policy.
    #[test]
    #[ignore = "daily"]
    fn normal_nautilus_keeps_its_custom_handler() {
        check(false, false, 300, [299, 298]);
        check(false, false, 100, [99, 98]);
        check(true, false, 300, [300, 300]);
        check(true, false, 100, [300, 300]);
    }

    /// Vanilla 26.3 (`VanillaNoAiBreatherAirOracle`, `tools/test_noai_breather_air_parity.py`):
    /// a NoAI axolotl or nautilus loaded with a signed-short `Air`, ticked, saved, reloaded and
    /// ticked again, over dry air, water, bubble columns at the eye and feet, the breathing
    /// effects and a lethal in-wall hurt.
    #[test]
    #[ignore = "run by tools/test_noai_breather_air_parity.py"]
    fn noai_breathers_match_the_pinned_oracle() {
        use std::collections::HashMap;
        use sunrise_core::nbt::Tag;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let water = |scene: i32| (1..=4).contains(&scene);
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, registry| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
                for scene in 0..6 {
                    let x = 2 * scene;
                    let (eye, feet) = match scene {
                        1 => ("minecraft:water", "minecraft:water"),
                        2 => ("minecraft:bubble_column", "minecraft:bubble_column"),
                        3 => ("minecraft:bubble_column", "minecraft:water"),
                        4 => ("minecraft:water", "minecraft:bubble_column"),
                        5 => ("minecraft:stone", "minecraft:air"),
                        _ => ("minecraft:air", "minecraft:air"),
                    };
                    world
                        .set(x, 65, 2, registry.default_state(eye).unwrap())
                        .unwrap();
                    world
                        .set(x, 64, 2, registry.default_state(feet).unwrap())
                        .unwrap();
                }
            },
            |world, types| {
                let types: HashMap<String, Arc<crate::mobs::MobType>> = types
                    .types
                    .iter()
                    .map(|(id, kind)| (id.clone(), Arc::new(kind.clone())))
                    .collect();
                let saved_air = |tag: &Tag| match tag.get("Air") {
                    Some(Tag::Short(air)) => i32::from(*air),
                    other => panic!("no short Air: {other:?}"),
                };
                let mut rows = Vec::new();
                for (species, max) in [("axolotl", 6000), ("nautilus", 300)] {
                    let id = format!("minecraft:{species}");
                    let probe = Mob::new(1, types[&id].clone(), Vec3::ZERO, 0.0);
                    // The eye in block 65, the feet in block 64.
                    let y = 65.05 - probe.eye_height();
                    for scene in 0..6 {
                        for mask in 0..8 {
                            if scene == 5 && mask != 0 {
                                continue;
                            }
                            for air in [10, max - 6, max - 1, max, max + 1, 32767] {
                                let health = if scene == 5 { ",Health:1f" } else { "" };
                                let snbt = format!(
                                    "{{id:\"{id}\",Pos:[{}.5d,{y:?}d,2.5d],NoAI:1b,Air:{air}s{health}}}",
                                    2 * scene
                                );
                                let tag = sunrise_core::nbt::parse_snbt(&snbt).unwrap().0;
                                let mut first =
                                    crate::entity_nbt::mob_from_nbt(&tag, &types, 1, None).unwrap();
                                for (bit, effect) in [
                                    (1, "minecraft:water_breathing"),
                                    (2, "minecraft:conduit_power"),
                                    (4, "minecraft:breath_of_the_nautilus"),
                                ] {
                                    if mask & bit != 0 {
                                        first.add_effect(effect, 100, 0);
                                    }
                                }
                                let loaded = first.air;
                                let tick = |mob: &mut Mob| {
                                    base_tick(
                                        mob,
                                        world,
                                        &mut crate::health::TestLevel::new(0).level(),
                                        &mut Vec::new(),
                                    );
                                    assert_eq!(
                                        mob.fluids.eye_in_water,
                                        water(scene),
                                        "fixture immersion"
                                    );
                                    crate::entity_nbt::mob_to_nbt(
                                        mob,
                                        [1, 2, 3, 4],
                                        None,
                                        None,
                                        |_| None,
                                        &Default::default(),
                                    )
                                };
                                let saved = tick(&mut first);
                                let ticked = first.air;
                                let mut second =
                                    crate::entity_nbt::mob_from_nbt(&saved, &types, 1, None)
                                        .unwrap();
                                let reloaded = second.air;
                                let again = tick(&mut second);
                                rows.push(format!(
                                    "{species} {scene} {mask} {air} {:016x} {loaded} {ticked} {} {reloaded} {} {:08x}",
                                    y.to_bits(),
                                    saved_air(&saved),
                                    saved_air(&again),
                                    first.health.to_bits()
                                ));
                            }
                        }
                    }
                }
                if let Some(path) = std::env::var_os("SUNRISE_NOAI_BREATHER_AIR_OUTPUT") {
                    std::fs::write(path, rows.join("\n") + "\n").unwrap();
                }
                let expected = witness_26_3_rows();
                assert_eq!(expected.len(), 2 * (5 * 8 + 1) * 6);
                assert_eq!(rows.len(), expected.len());
                for (index, (actual, expected)) in rows.iter().zip(expected.iter()).enumerate() {
                    assert_eq!(
                        actual.as_str(),
                        expected.as_str(),
                        "26.3 NoAI breather air row {index} diverged"
                    );
                }
            },
        );
        assert!(ran, "prepared resources must run the NoAI breather fixture");
    }
}

#[cfg(test)]
mod freeze_tests {
    use super::*;
    use std::sync::Arc;

    /// `LivingEntity.aiStep`'s freezing, through `freeze_tick`: powder snow
    /// builds a mob's frozen ticks towards the 140 that freeze it, and at 140 a
    /// tick that falls on the 40-tick damage interval hurts it by 1. A skeleton
    /// answers `canFreeze` false, so it takes neither, and so does the stray (by
    /// tag), while the bogged, parched, wither skeleton and a zombie freeze.
    #[test]
    #[ignore = "daily"]
    fn a_skeleton_never_freezes() {
        crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                let stone = registry.default_state("minecraft:stone").unwrap();
                let snow = registry.default_state("minecraft:powder_snow").unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                // The mob stands in this block.
                world.set(0, 64, 0, snow).unwrap();
            },
            |w, types| {
                let frozen_with = |id: &str, ticks_frozen: i32, body: Option<&str>| {
                    let kind = Arc::new(types.types[id].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
                    m.ticks_frozen = ticks_frozen;
                    m.extra.set_body(body.map(Into::into));
                    // The freeze damage falls on a tick that is a multiple of 40.
                    m.age = 40;
                    let start = m.health;
                    // The block walk's FREEZE effect for the snow it stands in,
                    // followed by aiStep's freezing check.
                    walk_and_freeze(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut Vec::new(),
                    );
                    (m.ticks_frozen, start - m.health)
                };
                let frozen = |id: &str, ticks_frozen: i32| frozen_with(id, ticks_frozen, None);
                // Powder snow builds no frozen ticks on a skeleton, and one
                // already frozen melts two ticks a tick instead of taking the
                // damage a fully frozen mob takes at 140.
                for skeleton in ["minecraft:skeleton", "minecraft:stray"] {
                    assert_eq!(frozen(skeleton, 0), (0, 0.0), "{skeleton}");
                    assert_eq!(frozen(skeleton, 140), (138, 0.0), "{skeleton} frozen");
                }
                // The bogged, parched and wither skeleton extend AbstractSkeleton,
                // not Skeleton, and freeze like a zombie.
                for sibling in [
                    "minecraft:bogged",
                    "minecraft:parched",
                    "minecraft:wither_skeleton",
                ] {
                    assert_eq!(frozen(sibling, 0), (1, 0.0), "{sibling}");
                    assert_eq!(frozen(sibling, 139), (140, 1.0), "{sibling} frozen");
                }
                // A zombie in the same block freezes, and at 140 takes the 1.
                assert_eq!(frozen("minecraft:zombie", 139), (140, 1.0));
                // SulfurCube.canFreeze: a cube carrying a block in its body does
                // not freeze; without one it freezes and takes the 1 like a zombie.
                assert_eq!(
                    frozen_with("minecraft:sulfur_cube", 0, Some("minecraft:stone")),
                    (0, 0.0)
                );
                assert_eq!(
                    frozen_with("minecraft:sulfur_cube", 140, Some("minecraft:stone")),
                    (138, 0.0)
                );
                assert_eq!(frozen("minecraft:sulfur_cube", 0), (1, 0.0));
                assert_eq!(frozen("minecraft:sulfur_cube", 139), (140, 1.0));
            },
        );
    }
}
#[cfg(test)]
mod happy_ghast_ambient_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn natural_happy_ghast_ambient_uses_entity_position() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |world, _| {
                world
                    .chunks
                    .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            },
            |w, types| {
                for baby in [false, true] {
                    let kind = Arc::new(types.types["minecraft:happy_ghast"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(10.5, 208.0, 10.5), 0.0);
                    mob.no_ai = true;
                    mob.no_gravity = true;
                    mob.persistent = true;
                    mob.extra.age = if baby { -24000 } else { 0 };
                    let sound = super::super::happy_ghast::sounds(&mob)[0];
                    let mut test_level = crate::health::TestLevel::new(0);
                    let level = &mut test_level.level();
                    let mut heard = false;
                    // Natural scheduling: no ambient counter is forced or reset here.
                    for _ in 0..1200 {
                        let mut events = Vec::new();
                        assert!(tick(&mut mob, w, level, &mut events));
                        for event in events {
                            if let MobEvent::Sound(name, at, _, _) = event
                                && name == sound
                            {
                                assert_eq!(at, [10.5, 208.0, 10.5], "baby={baby}");
                                heard = true;
                            }
                        }
                        if heard {
                            break;
                        }
                    }
                    assert!(heard, "no natural ambient event, baby={baby}");
                }
                // Entity.playSound plays nothing for a Silent entity.
                let kind = Arc::new(types.types["minecraft:happy_ghast"].clone());
                let mut mob = Mob::new(1, kind, Vec3::new(10.5, 208.0, 10.5), 0.0);
                mob.no_ai = true;
                mob.no_gravity = true;
                mob.persistent = true;
                mob.silent = true;
                let sound = super::super::happy_ghast::sounds(&mob)[0];
                let mut test_level = crate::health::TestLevel::new(0);
                let level = &mut test_level.level();
                for _ in 0..1200 {
                    let mut events = Vec::new();
                    assert!(tick(&mut mob, w, level, &mut events));
                    assert!(
                        !events
                            .iter()
                            .any(|e| matches!(e, MobEvent::Sound(name, ..) if *name == sound)),
                        "a Silent happy ghast is heard"
                    );
                }
            },
        );
        assert!(ran, "prepared-data callback did not run");
    }
}

#[cfg(test)]
mod hidden_speed_causal_tests {
    use super::*;
    use std::sync::Arc;
    fn mob() -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        Mob::new(202, Arc::new(kind), Vec3::ZERO, 0.0)
    }
    #[test]
    fn zero_active_never_promotes_and_infinite_fallback_counts_once() {
        let mut m = mob();
        m.add_effect("speed", 200, 0);
        m.add_effect("speed", 0, 1);
        assert_eq!(m.extra.effect_chains.len(), 1);
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
        );
        assert!(m.extra.effects.is_empty() && m.extra.effect_chains.is_empty());
        m.add_effect("speed", -1, 0);
        m.add_effect("speed", 2, 1);
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", 1, 1)]
        );
        assert_eq!(m.extra.effect_chains[0].duration, -1);
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", -1, 0)]
        );
        tick_effects(
            &mut m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
        );
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", -1, 0)]
        );
        assert!(m.extra.effect_chains.is_empty());
    }
    #[test]
    fn native_cancellation_drops_the_whole_chain_before_promotion() {
        struct Cancel(std::cell::Cell<usize>);
        impl crate::mobs::ModEffectTicks for Cancel {
            fn effect_expired(&self, _: u32, _: usize, _: i32, _: &mut bool) {}
            fn tick_mod_effect(
                &self,
                _: usize,
                m: &Mob,
                count: i32,
                amp: i32,
            ) -> crate::mobs::ModEffectTick {
                let (id, mob) = (m.extra.effects[0].id, m.id);
                assert_eq!((id, mob, amp), ("h202:cancel", 202, 1));
                assert!(count == 1 || count == 17);
                self.0.set(self.0.get() + 1);
                crate::mobs::ModEffectTick {
                    keep: false,
                    actions: Vec::new(),
                }
            }
        }
        for duration in [1, -1] {
            let mut m = mob();
            m.age = 17;
            m.add_effect("h202:cancel", 200, 0);
            if duration == -1 {
                // A loaded subtree can accompany an infinite active instance.
                m.extra.effects[0].duration = -1;
                let h = crate::effects::EffectInstance {
                    id: "h202:cancel",
                    registry_index: Some(0),
                    client_blend: (false, false),
                    duration: 200,
                    amplifier: 0,
                    ambient: false,
                    visible: true,
                    show_icon: true,
                    hidden: None,
                };
                m.extra.effects[0].amplifier = 1;
                m.extra.effect_chains.push(h);
            } else {
                m.add_effect("h202:cancel", duration, 1);
            }
            // This fixture stands in for a registered mod effect at slot zero.
            m.extra.effects[0].registry_index = Some(0);
            let cancel = Cancel(std::cell::Cell::new(0));
            tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                Some(&cancel),
            );
            assert_eq!(cancel.0.get(), 1);
            assert!(m.extra.effects.is_empty() && m.extra.effect_chains.is_empty());
        }
    }
}

#[cfg(test)]
mod trader_llama_no_ai_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[ignore = "daily"]
    fn eligible_trader_llama_delay_advances_in_actual_no_ai_tick() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let mut differences = Vec::new();
        let ran = crate::mobs::world::with_resource_test_world(
            &root,
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let kind = Arc::new(types.types["minecraft:trader_llama"].clone());
                let fresh = || {
                    let mut mob = Mob::new(1, kind.clone(), Vec3::new(4.5, 200.0, 4.5), 0.0);
                    mob.no_ai = true;
                    mob.no_gravity = true;
                    mob.silent = true;
                    mob.extra.ints[0] = 5;
                    mob
                };
                // Existing eligibility stays in the shared helper. These are focused
                // model controls, not runtime claims about real leash/rider sessions.
                let mut trader = Mob::new(
                    50,
                    Arc::new(types.types["minecraft:wandering_trader"].clone()),
                    Vec3::new(30.5, 200.0, 30.5),
                    0.0,
                );
                trader.extra.ints[5] = 13;
                let cow = Mob::new(
                    51,
                    Arc::new(types.types["minecraft:cow"].clone()),
                    Vec3::new(40.5, 200.0, 40.5),
                    0.0,
                );
                let seen = [
                    trader.seen(None, &crate::items::Items::empty()),
                    cow.seen(None, &crate::items::Items::empty()),
                ];
                assert_eq!(seen[0].delay, 13);
                let view = MobWorld {
                    side: crate::motion::dry_side(),
                    others: &seen,
                    ..w.clone()
                };
                for guard in [
                    "tame",
                    "persistent",
                    "age-lock",
                    "rider",
                    "other-mob-leash",
                    "player-leash",
                ] {
                    let mut mob = fresh();
                    match guard {
                        "tame" => mob.extra.tame = true,
                        "persistent" => mob.persistent = true,
                        "age-lock" => mob.extra.age_locked = true,
                        "rider" => mob.rider = Some(super::super::equine::Rider::default()),
                        "other-mob-leash" => {
                            mob.extra.leash = Some(super::super::leash::Holder::Mob(51))
                        }
                        "player-leash" => {
                            mob.extra.leash = Some(super::super::leash::Holder::Player)
                        }
                        _ => unreachable!(),
                    }
                    super::super::caravan::trader_llama_tick(&mut mob, &view);
                    assert_eq!(mob.extra.ints[0], 5, "{guard}");
                    assert!(!mob.removed, "{guard}");
                }
                let mut linked = fresh();
                linked.extra.leash = Some(super::super::leash::Holder::Mob(50));
                super::super::caravan::trader_llama_tick(&mut linked, &view);
                assert_eq!(linked.extra.ints[0], 12, "trader delay minus one");

                for initial in [0, 5, 47999] {
                    let mut mob = fresh();
                    mob.extra.ints[0] = initial;
                    mob.health = mob.attrs.max_health - 2.0;
                    let health = mob.health;
                    let steps = if initial == 0 { 1 } else { 5 };
                    for elapsed in 1..=steps {
                        let mut expected_random = mob.random.clone();
                        expected_random.next_int_bound(1000).unwrap(); // Existing Mob.baseTick ambient draw only.
                        let mut test_level = crate::health::TestLevel::new(0);
                        let mut events = Vec::new();
                        tick(&mut mob, w, &mut test_level.level(), &mut events);
                        assert_eq!(mob.age, elapsed as u32, "actual tick reached");
                        assert_eq!(
                            mob.health, health,
                            "NoAI must not run unrelated llama healing"
                        );
                        assert_eq!(
                            mob.random.state(),
                            expected_random.state(),
                            "NoAI must not add the healing draw"
                        );
                        let expected = initial - elapsed;
                        if mob.extra.ints[0] != expected || mob.removed != (expected <= 0) {
                            differences.push((
                                initial,
                                elapsed,
                                expected,
                                mob.extra.ints[0],
                                mob.removed,
                            ));
                        }
                    }
                }
                // The normal-AI caller already reaches the same timer.
                let mut normal = fresh();
                normal.no_ai = false;
                normal.health = normal.attrs.max_health;
                let mut events = Vec::new();
                tick(
                    &mut normal,
                    w,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut events,
                );
                assert_eq!(normal.extra.ints[0], 4);
            },
        );
        assert!(
            ran,
            "prepared resources must actually admit the mob tick world"
        );
        assert!(
            differences.is_empty(),
            "NoAI trader timer mismatch: {differences:?}"
        );
    }
}

#[cfg(test)]
#[path = "aistep_order_tests.rs"]
mod aistep_order_tests;
#[cfg(test)]
#[path = "hurt_teleport_contact_tests.rs"]
mod hurt_teleport_contact_tests;

#[cfg(test)]
mod hidden_absorption_promotion_tests {
    use super::*;
    use std::sync::Arc;

    /// #11384: a promoted hidden instance goes through `onEffectUpdated(effect,
    /// true)`, whose `refreshDirtyAttributes` clamps the absorption amount to the
    /// weaker effect's `max_absorption` (Absorption II's 8 down to I's 4).
    #[test]
    fn a_promoted_weaker_absorption_clamps_the_amount() {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        let mut m = Mob::new(203, Arc::new(kind), Vec3::ZERO, 0.0);
        m.add_effect("absorption", 600, 0);
        m.add_effect("absorption", 2, 1);
        assert_eq!(m.extra.effect_chains.len(), 1, "Absorption I waits");
        assert_eq!(
            (m.absorption, super::super::armor::max_absorption(&m)),
            (8.0, 8.0)
        );
        for _ in 0..3 {
            tick_effects(
                &mut m,
                crate::test_resources::server_loop_context(),
                &mut crate::health::TestLevel::new(0).level(),
                None,
            );
        }
        assert_eq!(m.extra.effects.len(), 1);
        assert_eq!(m.extra.effects[0].amplifier, 0, "Absorption I took over");
        assert_eq!(super::super::armor::max_absorption(&m), 4.0);
        assert_eq!(m.absorption, 4.0);
    }
}

#[cfg(test)]
mod baby_speed_tests {
    use super::*;
    use crate::mobs::{finalize, sensor_scan_tests::kind};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    fn baby(species: Species, id: &str, speed: f64) -> Mob {
        let mut k = kind();
        let km = Arc::make_mut(&mut k);
        km.id = id.into();
        km.species = species;
        km.speed = speed;
        km.attributes = Arc::new(std::collections::HashMap::from([(
            "minecraft:movement_speed".to_owned(),
            speed,
        )]));
        let mut m = Mob::new(1, k, Vec3::ZERO, 0.0);
        finalize::set_baby(&mut m, true);
        m
    }

    /// #11994: movement reads the transient `minecraft:baby` modifier itself:
    /// `Zombie.SPEED_MODIFIER_BABY` +0.5 and `Piglin.SPEED_MODIFIER_BABY` +0.2
    /// (`ADD_MULTIPLIED_BASE`), and none once `/attribute ... modifier remove`
    /// took it away, until the next `setBaby`.
    #[test]
    fn a_babys_speed_follows_its_baby_modifier() {
        let mut zombie = baby(Species::Zombie, "minecraft:zombie", 0.23);
        // 0.23 + 0.23 x 0.5.
        assert_eq!(zombie.movement_speed(), 0.345_000_000_000_000_03);
        zombie
            .extra
            .attributes
            .get_or_insert_default()
            .baby_speed_removed = true;
        assert_eq!(zombie.movement_speed(), 0.23, "removed with /attribute");
        finalize::set_baby(&mut zombie, true);
        assert_eq!(
            zombie.movement_speed(),
            0.345_000_000_000_000_03,
            "setBaby adds it back"
        );
        // The piglin's is 0.2F widened to a double, not the zombie's 0.5:
        // 0.35 + 0.35 x 0.20000000298023224.
        let piglin = baby(Species::Piglin, "minecraft:piglin", 0.35);
        assert_eq!(piglin.movement_speed(), 0.420_000_001_043_081_3);
        let mut adult = baby(Species::Zombie, "minecraft:zombie", 0.23);
        finalize::set_baby(&mut adult, false);
        assert_eq!(adult.movement_speed(), 0.23);
    }
}

#[cfg(test)]
mod walk_target_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    /// `PathfinderMob.getWalkTargetValue` and its overrides, at three positions
    /// that differ only in the block below. A cow scores grass 10 and mycelium by
    /// the light; a mushroom cow, whose own override replaces Animal's, mycelium
    /// 10 and grass by the light; a monster the negated light; and an enderman, a
    /// pillager and a warden the same 0 everywhere.
    #[test]
    #[ignore = "daily"]
    fn a_stroll_scores_its_ground_the_way_the_jar_does() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                let stone = registry.default_state("minecraft:stone").unwrap();
                let grass = registry.default_state("minecraft:grass_block").unwrap();
                let mycelium = registry.default_state("minecraft:mycelium").unwrap();
                for cx in -1..1 {
                    for cz in -1..1 {
                        world
                            .chunks
                            .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                    }
                }
                for x in -4..4 {
                    for z in -4..4 {
                        world.set(x, 63, z, stone).unwrap();
                    }
                }
                world.set(-2, 63, 0, grass).unwrap();
                world.set(0, 63, 0, stone).unwrap();
                world.set(2, 63, 0, mycelium).unwrap();
            },
            |w, types| {
                let value = |id: &str, x: i32| {
                    let kind = Arc::new(types.types[id].clone());
                    let m = Mob::new(1, kind, Vec3::ZERO, 0.0);
                    walk_target_value(&m, w, [x, 64, 0])
                };
                // The light term of the middle position, which the report quotes.
                let light = w.brightness(0, 64, 0) - 0.5;
                // A cow: grass 10, mycelium and stone by the light.
                assert_eq!(value("minecraft:cow", -2), 10.0, "cow on grass");
                assert_eq!(value("minecraft:cow", 0), light, "cow on stone");
                assert_eq!(value("minecraft:cow", 2), light, "cow on mycelium");
                // A mushroom cow: mycelium 10, grass by the light, not 10.
                assert_eq!(
                    value("minecraft:mooshroom", 2),
                    10.0,
                    "mooshroom on mycelium"
                );
                assert_eq!(
                    value("minecraft:mooshroom", -2),
                    light,
                    "mooshroom on grass"
                );
                // A monster: the negated light, on grass like anywhere else.
                assert_eq!(value("minecraft:zombie", -2), -light, "zombie");
                // The three that score every position the same.
                for flat in [
                    "minecraft:enderman",
                    "minecraft:pillager",
                    "minecraft:warden",
                ] {
                    assert_eq!(value(flat, -2), 0.0, "{flat} on grass");
                    assert_eq!(value(flat, 0), 0.0, "{flat} on stone");
                    assert_eq!(value(flat, 2), 0.0, "{flat} on mycelium");
                }
            },
        );
        // Without the extracted vanilla data the world cannot be built.
        assert!(ran, "vanilla data is missing");
    }
}
