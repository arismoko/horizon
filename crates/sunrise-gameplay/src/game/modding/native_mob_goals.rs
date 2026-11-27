//! AI goals of native mod mob handles.
use super::commands::Target;
use super::native_namespaced_id::namespaced;
use mod_api::{GoalSpec, MobFilter, MobGoalEntry, MobRef};
use sunrise_sim::mobs::{
    self, Species,
    goals::{
        Avoid, AvoidEntity, BreathAir, Breed, ClimbOnTopOfPowderSnow, Cx, EatBlock, FleeSun, Float,
        FollowMob, FollowParent, Goal, HurtByTarget, LeapAtTarget, LookAtPlayer, MeleeAttack,
        NearestTarget, Panic, Prey, RandomLookAround, RestrictSun, Stroll, Tempt, TryFindWater,
    },
    nav::NavKind,
};

/// The goal's kind id ([`GoalSpec::id`]) of a registered goal, by its
/// internal name: the plain vanilla kinds map to their snake_case id, and a
/// species variant of one (a bee's `BeeHurtByOtherGoal`, a labeled stroll)
/// keeps its own vanilla name, as `goals` reads it and `remove_goal` takes
/// it to remove that one.
pub(in crate::game::server_side) fn goal_id(name: &str) -> &str {
    match name {
        "Float" => "float",
        "ClimbOnTopOfPowderSnow" => "climb_on_top_of_powder_snow",
        "Panic" => "panic",
        "Stroll" => "stroll",
        "LookAtPlayer" => "look_at_player",
        "RandomLookAround" => "random_look_around",
        "MeleeAttack" => "melee_attack",
        "LeapAtTarget" => "leap_at_target",
        "AvoidEntity" => "avoid_entity",
        "Tempt" => "tempt",
        "Breed" => "breed",
        "FollowParent" => "follow_parent",
        "FollowMobGoal" => "follow_mob",
        "RestrictSun" => "restrict_sun",
        "FleeSun" => "flee_sun",
        "EatBlock" => "eat_block",
        "BreathAir" => "breath_air",
        "TryFindWater" => "try_find_water",
        "NearestTarget" => "nearest_target",
        "HurtByTarget" => "hurt_by_target",
        other => other,
    }
}

/// The supported `Mob` classes that do not inherit `PathfinderMob`.
fn pathfinder(species: Species) -> bool {
    !matches!(species, Species::Bat | Species::Ghast | Species::Phantom)
}

impl crate::game::ServerGame<'_> {
    /// `GoalSelector.addGoal(priority, goal)` on a mob's goal selector.
    /// Refuses a stale or unavailable mob, a client, and the dragon (it
    /// keeps no selector); a goal the mob's vanilla class does not hold (a
    /// `PathfinderMob` goal for a bat, ghast or phantom, `BreedGoal` or
    /// `FollowParentGoal` for a mob that is no animal); and non-finite
    /// numbers, negative speeds and distances, a probability outside
    /// 0..=1, a negative interval, and an unknown species id (in `prey`,
    /// `what` or `only`) or empty `items`. The target selector's kinds
    /// (`nearest_target`, `hurt_by_target`) go in through
    /// `add_target_goal`; both selectors take them, as vanilla's two
    /// selectors do.
    pub fn mob_add_goal(&mut self, mob: MobRef, priority: i32, goal: &GoalSpec) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some((species, navigation)) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| (m.species(), m.ai.nav.kind))
        else {
            return false;
        };
        let Some(goal) = self.boxed_goal(species, navigation, goal) else {
            return false;
        };
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        m.ai.goals.add_boxed(priority, goal);
        true
    }

    /// `GoalSelector.addGoal` on the mob's target selector. The same rules
    /// as `add_goal`.
    pub fn mob_add_target_goal(&mut self, mob: MobRef, priority: i32, goal: &GoalSpec) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some((species, navigation)) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| (m.species(), m.ai.nav.kind))
        else {
            return false;
        };
        let Some(goal) = self.boxed_goal(species, navigation, goal) else {
            return false;
        };
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        m.ai.targets.add_boxed(priority, goal);
        true
    }

    /// `GoalSelector.removeGoal` on the goal selector, by kind: every goal
    /// whose id, as `goals` reads it, is `id` goes (`removeAllGoals`), a
    /// running one stopped first. How many went; None for a stale or
    /// unavailable mob, a client, and the dragon. Nothing in a mob's tick
    /// resolves, as its copy is held out there: add what a mod needs in
    /// `entity_join`, which fires for a loaded chunk's mobs too.
    pub(in crate::game) fn mob_remove_goal(&mut self, mob: MobRef, id: &str) -> Option<usize> {
        let Some(Target::Mob(entity)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let index = self.common.mobs.iter().position(|m| m.id == entity)?;
        Some(self.remove_from_selector(index, id, false))
    }

    /// `GoalSelector.removeGoal` on the target selector, by kind, as
    /// `remove_goal` does on the goal selector.
    pub(in crate::game) fn mob_remove_target_goal(
        &mut self,
        mob: MobRef,
        id: &str,
    ) -> Option<usize> {
        let Some(Target::Mob(entity)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let index = self.common.mobs.iter().position(|m| m.id == entity)?;
        Some(self.remove_from_selector(index, id, true))
    }

    /// `GoalSelector.getRunningGoals` read wider: every goal of the goal
    /// selector as a `MobGoalEntry` (the priority, the kind id and
    /// whether it runs), in registration order. None for a stale or
    /// unavailable mob, a client, and the dragon.
    pub(in crate::game) fn mob_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>> {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let m = self.common.mobs.iter().find(|m| m.id == id)?;
        Some(
            m.ai.goals
                .entries()
                .map(|(priority, name, running)| MobGoalEntry {
                    priority,
                    id: goal_id(name),
                    running,
                })
                .collect(),
        )
    }

    /// Every goal of the target selector, as `goals` reads the goal
    /// selector's.
    pub(in crate::game) fn mob_target_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>> {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let m = self.common.mobs.iter().find(|m| m.id == id)?;
        Some(
            m.ai.targets
                .entries()
                .map(|(priority, name, running)| MobGoalEntry {
                    priority,
                    id: goal_id(name),
                    running,
                })
                .collect(),
        )
    }

    /// One `removeGoal` pass over the mob's selector: the mob list comes
    /// out, as the mob tick takes it, so the goal's `stop` sees the world
    /// the tick's `Cx` sees; no hook fires in between. The events a stop
    /// makes land as any tick's do.
    fn remove_from_selector(&mut self, index: usize, id: &str, targets: bool) -> usize {
        let snapshot: Vec<_> = self
            .common
            .mobs
            .iter()
            .map(|m| {
                sunrise_gameplay_common::game::creatures::seen(
                    m,
                    self.common.level.player_uuid,
                    self.items(),
                )
            })
            .collect();
        let mut mobs = self.common.mobs.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let mut lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
        let mut w = self.mob_world_base(&snapshot);
        let pos = mobs[index].position;
        w.difficulty = self.common.local_difficulty([pos.x, pos.y, pos.z]);
        let mut events = Vec::new();
        let removed = {
            let mut level = sunrise_sim::health::DamageLevel::lent(
                &self.common.world,
                self.common.pack.assets.registry,
                &mut lent,
                Some(world_items!(self)),
                Some(&self.common.pack.block_registries),
                &self.common.damage_hooks,
            )
            .with_variants(world_variants!(self))
            .with_mob_uuids(&self.common.persist.uuids)
            .with_reads(crate::game::server_side::entity_server::hit_reads_of!(self));
            let m = &mut mobs[index];
            let mut selector = if targets {
                std::mem::take(&mut m.ai.targets)
            } else {
                std::mem::take(&mut m.ai.goals)
            };
            let mut cx = Cx {
                w: &w,
                level: &mut level,
                events: &mut events,
            };
            let removed = selector.remove_all(|g| goal_id(g.name()) == id, m, &mut cx);
            if targets {
                mobs[index].ai.targets = selector;
            } else {
                mobs[index].ai.goals = selector;
            }
            mobs::apply_brain_events(&mut mobs, &mut level, &mut events, 0, &w.teams);
            removed
        };
        drop(w);
        lent.give_back(&mut self.common.world);
        self.common
            .mobs
            .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        self.handle_mob_events(events).ok();
        removed
    }

    /// The goal `goal` names, built as the species code builds it, for a
    /// mob of `species`: None when the numbers are not ones vanilla's
    /// constructor takes or the mob's class does not hold the goal. The
    /// species ids of a filter resolve through the mob type table, a bare
    /// id in `minecraft:`'s, as the effects' ids read.
    pub fn boxed_goal(
        &self,
        species: Species,
        navigation: NavKind,
        goal: &GoalSpec,
    ) -> Option<Box<dyn Goal>> {
        let needs_pathfinder = !matches!(
            goal,
            GoalSpec::Float
                | GoalSpec::ClimbOnTopOfPowderSnow
                | GoalSpec::LookAtPlayer { .. }
                | GoalSpec::RandomLookAround
                | GoalSpec::LeapAtTarget { .. }
                | GoalSpec::EatBlock
                | GoalSpec::FollowMob { .. }
                | GoalSpec::NearestTarget { .. }
        );
        if needs_pathfinder && !pathfinder(species) {
            return None;
        }
        if matches!(goal, GoalSpec::FollowMob { .. })
            && !matches!(
                navigation,
                NavKind::Ground | NavKind::WallClimber | NavKind::Flying
            )
        {
            return None;
        }
        let animal_only = matches!(goal, GoalSpec::Breed { .. } | GoalSpec::FollowParent { .. });
        if animal_only && !mobs::drops::is_animal(species) {
            return None;
        }
        match goal {
            GoalSpec::Float => Some(Box::new(Float)),
            GoalSpec::ClimbOnTopOfPowderSnow => Some(Box::new(ClimbOnTopOfPowderSnow)),
            GoalSpec::Panic { speed, water_range } => {
                if !speed.is_finite() || *speed < 0.0 || water_range.is_some_and(|r| r < 0) {
                    return None;
                }
                let mut g = Panic::new(*speed);
                g.water_range = *water_range;
                Some(Box::new(g))
            }
            GoalSpec::Stroll {
                speed,
                interval,
                water_avoiding,
                swim,
                flying,
            } => {
                if !speed.is_finite()
                    || *speed < 0.0
                    || *interval < 0
                    || water_avoiding.is_some_and(|p| !p.is_finite() || !(0.0..=1.0).contains(&p))
                {
                    return None;
                }
                let mut g = Stroll::new(*speed);
                g.interval = *interval;
                g.water_avoiding = *water_avoiding;
                g.swim = *swim;
                g.flying = *flying;
                Some(Box::new(g))
            }
            GoalSpec::LookAtPlayer {
                range,
                probability,
                only_horizontal,
                mobs,
                only,
            } => {
                if !range.is_finite()
                    || *range < 0.0
                    || !probability.is_finite()
                    || !(0.0..=1.0).contains(probability)
                {
                    return None;
                }
                let only = match only {
                    None => None,
                    Some(id) => Some(self.species_of(id)?),
                };
                let mut g = if *mobs {
                    LookAtPlayer::mobs(*range)
                } else {
                    LookAtPlayer::new(*range)
                };
                g.probability = *probability;
                g.only_horizontal = *only_horizontal;
                g.only = only;
                Some(Box::new(g))
            }
            GoalSpec::RandomLookAround => Some(Box::new(RandomLookAround::default())),
            GoalSpec::MeleeAttack {
                speed,
                follow_unseen,
            } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(MeleeAttack::new(*speed, *follow_unseen)))
            }
            GoalSpec::LeapAtTarget { yd } => {
                if !yd.is_finite() {
                    return None;
                }
                Some(Box::new(LeapAtTarget::new(*yd)))
            }
            GoalSpec::AvoidEntity {
                what,
                max_dist,
                walk,
                sprint,
            } => {
                if !max_dist.is_finite()
                    || *max_dist < 0.0
                    || !walk.is_finite()
                    || *walk < 0.0
                    || !sprint.is_finite()
                    || *sprint < 0.0
                {
                    return None;
                }
                let what = match what {
                    MobFilter::Player => Avoid::Player,
                    MobFilter::Species(ids) => Avoid::Species(self.species_list(ids)?),
                    _ => unreachable!("&MobFilter: unhandled variant"),
                };
                Some(Box::new(AvoidEntity::new(what, *max_dist, *walk, *sprint)))
            }
            GoalSpec::Tempt {
                speed,
                items,
                can_scare,
            } => {
                if !speed.is_finite() || *speed < 0.0 || items.is_empty() {
                    return None;
                }
                let items: Vec<String> = items.iter().map(|id| namespaced(id)).collect::<Vec<_>>();
                Some(Box::new(Tempt::new(*speed, items.into(), *can_scare)))
            }
            GoalSpec::Breed { speed } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(Breed::new(*speed)))
            }
            GoalSpec::FollowParent { speed } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(FollowParent::new(*speed)))
            }
            GoalSpec::FollowMob { speed, stop, area } => {
                if !speed.is_finite()
                    || *speed < 0.0
                    || !stop.is_finite()
                    || *stop < 0.0
                    || !area.is_finite()
                    || *area < 0.0
                {
                    return None;
                }
                Some(Box::new(FollowMob::new(*speed, *stop, *area)))
            }
            GoalSpec::RestrictSun => Some(Box::new(RestrictSun)),
            GoalSpec::FleeSun { speed } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(FleeSun::new(*speed)))
            }
            GoalSpec::EatBlock => Some(Box::new(EatBlock::default())),
            GoalSpec::BreathAir => Some(Box::new(BreathAir::default())),
            GoalSpec::TryFindWater => Some(Box::new(TryFindWater::default())),
            GoalSpec::NearestTarget {
                prey,
                must_see,
                must_reach,
                interval,
            } => {
                if *interval < 0 {
                    return None;
                }
                let prey = match prey {
                    MobFilter::Player => Prey::Player,
                    MobFilter::Species(ids) => Prey::Species(self.species_list(ids)?),
                    _ => unreachable!("&MobFilter: unhandled variant"),
                };
                let mut g = NearestTarget::new(prey, *must_see);
                g.must_reach = *must_reach;
                g.interval = *interval;
                Some(Box::new(g))
            }
            GoalSpec::HurtByTarget { alert } => {
                let mut g = HurtByTarget::new();
                g.alert = *alert;
                Some(Box::new(g))
            }
            _ => unreachable!("&GoalSpec: unhandled variant"),
        }
    }

    /// Every species id of a filter, all of them known; None when one is
    /// not. Bare ids read as `minecraft:`'s.
    fn species_list(&self, ids: &[String]) -> Option<Vec<Species>> {
        if ids.is_empty() {
            return None;
        }
        ids.iter().map(|id| self.species_of(id)).collect()
    }

    /// The species of the mob type `id` names (`minecraft:zombie`).
    fn species_of(&self, id: &str) -> Option<Species> {
        self.common
            .res
            .mob_types()
            .get(&namespaced(id))
            .map(|t| t.species)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
