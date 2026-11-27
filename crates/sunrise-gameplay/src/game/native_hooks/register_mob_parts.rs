use super::*;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
use sunrise_sim::{
    join_seq::JoinSeq,
    mobs::{Mob, mod_behaviors::StagedBehaviors, part_goals::StagedGoals},
};

/// Server-side native hook points that dispatch staged `EntityPart` goal and
/// behavior registration.
impl crate::game::ServerGame<'_> {
    /// `Mob.registerGoals` for entity parts attached to `goals.mob()`, with a scoped
    /// server host. Server-side.
    pub(in crate::game) fn native_register_goals(
        &mut self,
        goals: &mut mod_api::GoalRegistrar<'_>,
    ) {
        mod_api::dispatch_register_goals(self, goals);
    }

    /// `LivingEntity.makeBrain` for entity parts attached to `behaviors.mob()`, with
    /// a scoped server host. Server-side.
    pub(in crate::game) fn native_register_behaviors(
        &mut self,
        behaviors: &mut mod_api::BehaviorRegistrar<'_>,
    ) {
        mod_api::dispatch_register_behaviors(self, behaviors);
    }
}

enum MobTarget<'a> {
    Resident { id: u32, join: JoinSeq },
    Unjoined(&'a mut Mob),
}

impl MobTarget<'_> {
    fn get_mut<'b>(&'b mut self, game: &'b mut crate::game::ServerGame<'_>) -> Option<&'b mut Mob> {
        match self {
            Self::Resident { id, join } => {
                let idx = game
                    .mobs
                    .locate(*join)
                    .filter(|&k| game.mobs[k].id == *id && !game.mobs[k].removed)?;
                Some(&mut game.mobs[idx])
            }
            Self::Unjoined(mob) => (!mob.removed).then_some(&mut **mob),
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Drain staged `register_goals` and `register_behaviors` queued when mobs with
    /// pending parts entered `self.mobs`. O(1) when no new mobs were staged.
    pub(in crate::game) fn flush_pending_mob_parts(&mut self) {
        if !self.mobs.has_pending_mod_parts() {
            return;
        }
        let pending = self.mobs.take_pending_mod_parts();
        if !self.is_logical_server()
            || (!mod_api::__private::has_scoped_hook(mod_api::HookId::RegisterGoals)
                && !mod_api::__private::has_scoped_hook(mod_api::HookId::RegisterBehaviors))
        {
            return;
        }
        for (id, join) in pending {
            self.flush_resident_mob_parts(id, join);
        }
    }

    /// Flush staged `register_goals` and `register_behaviors` for the resident mob
    /// `(id, join)` in `self.mobs`. Listener writes and the selector/brain apply run
    /// in one transaction; apply refuses (never recomputes) if the mob or its target
    /// selectors/brain changed while listeners ran.
    pub fn flush_resident_mob_parts(&mut self, id: u32, join: JoinSeq) {
        self.mobs.pop_pending_mod_part(id, join);
        self.flush_mob_parts(MobTarget::Resident { id, join });
    }

    /// Flush staged `register_goals` and `register_behaviors` for an unjoined `Mob`
    /// owned by the caller (e.g. `create_mob_reason`, `finalize_with`, off-level loads).
    pub fn flush_unjoined_mob_parts(&mut self, mob: &mut Mob) {
        self.flush_mob_parts(MobTarget::Unjoined(mob));
    }

    fn flush_mob_parts(&mut self, mut target: MobTarget<'_>) {
        if !self.is_logical_server() {
            return;
        }
        self.flush_staged_goals(&mut target);
        self.flush_staged_behaviors(&mut target);
    }

    fn flush_staged_goals(&mut self, target: &mut MobTarget<'_>) {
        let Some((id, expected_gen, mob_id, goal_sel_gen, target_sel_gen)) =
            target.get_mut(self).and_then(|mob| {
                let expected_gen = mob.staged_parts.pending_goals.take()?;
                Some((
                    mob.id,
                    expected_gen,
                    mob.kind.id.clone(),
                    mob.ai.goals.generation(),
                    mob.ai.targets.generation(),
                ))
            })
        else {
            return;
        };
        if !mod_api::__private::entity_part_hooked(
            mod_api::HookId::RegisterGoals,
            "register_goals",
            &mob_id,
        ) {
            return;
        }
        let mut staged = Some(StagedGoals::new(id, mob_id));
        let _ = self.storage_transaction(|game, _| {
            if let Some(buffer) = staged.as_mut() {
                let mut registrar = mod_api::GoalRegistrar::new(buffer);
                game.native_register_goals(&mut registrar);
            }
            if !game.is_logical_server() {
                return Err(());
            }
            let Some(mob) = target.get_mut(game) else {
                return Ok(());
            };
            if mob.staged_parts.goal_generation != expected_gen
                || mob.ai.goals.generation() != goal_sel_gen
                || mob.ai.targets.generation() != target_sel_gen
            {
                return Ok(());
            }
            if let Some(buffer) = staged.take() {
                buffer.apply_to(&mut mob.ai.goals, &mut mob.ai.targets);
            }
            Ok::<(), ()>(())
        });
        if let Some(leftover) = staged {
            leftover.discard();
        }
    }

    fn flush_staged_behaviors(&mut self, target: &mut MobTarget<'_>) {
        let Some((behavior_target, mob_id)) = target.get_mut(self).and_then(|mob| {
            let behavior_target = mob.staged_parts.pending_behaviors.take()?;
            Some((behavior_target, mob.kind.id.clone()))
        }) else {
            return;
        };
        if !mod_api::__private::entity_part_hooked(
            mod_api::HookId::RegisterBehaviors,
            "register_behaviors",
            &mob_id,
        ) {
            return;
        }
        let mut staged = Some(StagedBehaviors::new(mob_id.clone(), behavior_target));
        let _ = self.storage_transaction(|game, _| {
            if let Some(buffer) = staged.as_mut() {
                let mut registrar = mod_api::BehaviorRegistrar::new(buffer, &mob_id, "", "");
                game.native_register_behaviors(&mut registrar);
            }
            if !game.is_logical_server() {
                return Err(());
            }
            let Some(mob) = target.get_mut(game) else {
                return Ok(());
            };
            if let Some(buffer) = staged.take() {
                let _ = buffer.apply_to(mob);
            }
            Ok::<(), ()>(())
        });
        if let Some(leftover) = staged {
            leftover.discard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::{
        fixture_resident_column, run_fresh_cases, with_game,
    };
    use mod_api::{
        Activity, Behavior, BlockUpdateFlags, EntityPart, Goal, GoalContext, GoalFlags, Hooks,
        ModEntry, RegisterBehaviorsEvent, RegisterGoalsEvent, Registrar, RegistrationContext,
        ServerHost,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_core::geometry::Vec3;

    struct MarkerGoal(&'static str);
    impl Goal for MarkerGoal {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            self.0
        }
        fn can_use(&mut self, _: &mut GoalContext<'_>) -> bool {
            false
        }
    }

    struct MarkerBehavior(&'static str);
    impl Behavior for MarkerBehavior {
        fn name(&self) -> &'static str {
            self.0
        }
    }

    /// `register_goals` receives a live `&mut dyn ServerHost`: a healthy listener
    /// can read and mutate the world inside the registration transaction, while a
    /// panicking listener rolls back both its own host writes and its staged goals
    /// without breaking healthy sibling listeners.
    #[test]
    #[ignore = "daily"]
    fn register_goals_lends_server_host_and_rolls_back_panicking_listener() {
        if run_fresh_cases(
            "game::native_hooks::register_mob_parts::tests::register_goals_lends_server_host_and_rolls_back_panicking_listener",
            "SUNRISE_REGISTER_GOALS_HOST_CHILD",
            &["run"],
        ) {
            return;
        }
        struct HealthyPart;
        #[mod_api::sunrise_part]
        impl EntityPart for HealthyPart {
            fn register_goals(
                &self,
                host: &mut dyn ServerHost,
                event: &mut RegisterGoalsEvent<'_, '_>,
            ) {
                let overworld = host.level("minecraft:overworld").expect("overworld");
                let stone = host.block_state_id("minecraft:stone").expect("stone");
                assert!(
                    overworld
                        .set_block(host, [2, 65, 2], stone, BlockUpdateFlags::NONE)
                        .is_ok()
                );
                event.goals.add_goal(6, MarkerGoal("HealthyGoal"));
            }
        }
        struct PanickingPart;
        #[mod_api::sunrise_part]
        impl EntityPart for PanickingPart {
            fn register_goals(
                &self,
                host: &mut dyn ServerHost,
                event: &mut RegisterGoalsEvent<'_, '_>,
            ) {
                let overworld = host.level("minecraft:overworld").expect("overworld");
                let dirt = host.block_state_id("minecraft:dirt").expect("dirt");
                assert!(
                    overworld
                        .set_block(host, [3, 65, 2], dirt, BlockUpdateFlags::NONE)
                        .is_ok()
                );
                event.goals.add_goal(5, MarkerGoal("LeakedGoal"));
                panic!("simulated register_goals failure");
            }
        }
        struct PanickingMod;
        #[mod_api::__subscriptions]
        impl Hooks for PanickingMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.entity_part("minecraft:zombie", PanickingPart);
            }
        }
        struct HealthyMod;
        #[mod_api::__subscriptions]
        impl Hooks for HealthyMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.entity_part("minecraft:zombie", HealthyPart);
            }
        }

        let entries = || {
            vec![
                ModEntry::new("reg-goals-panic", Box::new(PanickingMod)),
                ModEntry::new("reg-goals-healthy", Box::new(HealthyMod)),
            ]
        };
        mod_platform::testing::with_mods(entries().into(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&entries()).unwrap();
            with_game(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(201, kind, Vec3::new(3.5, 65.0, 3.5), 0.0);
                game.flush_unjoined_mob_parts(&mut mob);

                let names: Vec<_> = mob.ai.goals.entries().map(|(_, n, _)| n).collect();
                assert!(
                    names.contains(&"HealthyGoal"),
                    "healthy listener goal applied: {names:?}"
                );
                assert!(
                    !names.contains(&"LeakedGoal"),
                    "panicking listener goal rolled back: {names:?}"
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                assert_eq!(
                    game.world.block(2, 65, 2),
                    stone,
                    "healthy listener host write committed"
                );
                assert_eq!(
                    game.world.block(3, 65, 2),
                    game.world.air,
                    "panicking listener host write rolled back"
                );
            });
        });
    }

    /// Precondition refusal: if a `register_goals` listener discards the mob
    /// through its `ServerHost`, the listener's discard commits and staged goals
    /// are refused (never applied to a departed mob).
    #[test]
    #[ignore = "daily"]
    fn register_goals_refuses_apply_when_listener_discards_mob() {
        if run_fresh_cases(
            "game::native_hooks::register_mob_parts::tests::register_goals_refuses_apply_when_listener_discards_mob",
            "SUNRISE_REGISTER_GOALS_DISCARD_CHILD",
            &["run"],
        ) {
            return;
        }
        struct DiscardPart;
        #[mod_api::sunrise_part]
        impl EntityPart for DiscardPart {
            fn register_goals(
                &self,
                host: &mut dyn ServerHost,
                event: &mut RegisterGoalsEvent<'_, '_>,
            ) {
                event.goals.add_goal(4, MarkerGoal("ShouldNotApply"));
                if let Some(entity) = host.entity(202) {
                    assert!(entity.discard(host));
                }
            }
        }
        struct TestMod;
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.entity_part("minecraft:zombie", DiscardPart);
            }
        }

        let entries = || vec![ModEntry::new("reg-goals-discard", Box::new(TestMod))];
        mod_platform::testing::with_mods(entries().into(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&entries()).unwrap();
            with_game(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mob = Mob::new(202, kind, Vec3::new(3.5, 65.0, 3.5), 0.0);
                game.mobs.push(mob);
                let join = game.mobs.last().unwrap().join;
                game.flush_resident_mob_parts(202, join);

                let mob = game.mobs.iter().find(|m| m.id == 202);
                assert!(
                    mob.is_none_or(|m| m.removed
                        && !m.ai.goals.entries().any(|(_, n, _)| n == "ShouldNotApply")),
                    "discarded mob stays removed and does not receive refused staged goals"
                );
            });
        });
    }

    /// Production `tick_mobs` / `borrowed_tick` pauses at `Pause::RegisterBehaviors`
    /// when a villager lazily constructs its brain and again when it rebuilds via
    /// `age_boundary_reached`, giving `register_behaviors` a live `&mut dyn ServerHost`
    /// while isolating panicking sibling listeners and never breaking subsequent ticks.
    #[test]
    #[ignore = "daily"]
    fn register_behaviors_fires_with_server_host_through_production_tick_and_rebuild() {
        if run_fresh_cases(
            "game::native_hooks::register_mob_parts::tests::register_behaviors_fires_with_server_host_through_production_tick_and_rebuild",
            "SUNRISE_REGISTER_BEHAVIORS_TICK_CHILD",
            &["run"],
        ) {
            return;
        }
        struct HealthyBehaviorPart {
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::sunrise_part]
        impl EntityPart for HealthyBehaviorPart {
            fn register_behaviors(
                &self,
                host: &mut dyn ServerHost,
                event: &mut RegisterBehaviorsEvent<'_, '_>,
            ) {
                let n = self.calls.fetch_add(1, Ordering::Relaxed);
                let overworld = host.level("minecraft:overworld").expect("overworld");
                let stone = host.block_state_id("minecraft:stone").expect("stone");
                assert!(
                    overworld
                        .set_block(host, [4 + n as i32, 65, 4], stone, BlockUpdateFlags::NONE)
                        .is_ok()
                );
                event
                    .behaviors
                    .add(
                        Activity::Idle,
                        50,
                        60,
                        60,
                        &[],
                        MarkerBehavior("HealthyBehavior"),
                    )
                    .unwrap();
            }
        }
        struct PanickingBehaviorPart;
        #[mod_api::sunrise_part]
        impl EntityPart for PanickingBehaviorPart {
            fn register_behaviors(
                &self,
                host: &mut dyn ServerHost,
                event: &mut RegisterBehaviorsEvent<'_, '_>,
            ) {
                let overworld = host.level("minecraft:overworld").expect("overworld");
                let dirt = host.block_state_id("minecraft:dirt").expect("dirt");
                assert!(
                    overworld
                        .set_block(host, [9, 65, 4], dirt, BlockUpdateFlags::NONE)
                        .is_ok()
                );
                event
                    .behaviors
                    .add(
                        Activity::Idle,
                        49,
                        60,
                        60,
                        &[],
                        MarkerBehavior("LeakedBehavior"),
                    )
                    .unwrap();
                panic!("simulated register_behaviors failure");
            }
        }
        struct PanickingBehaviorMod;
        #[mod_api::__subscriptions]
        impl Hooks for PanickingBehaviorMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.entity_part("minecraft:villager", PanickingBehaviorPart);
            }
        }
        struct HealthyBehaviorMod {
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for HealthyBehaviorMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.entity_part(
                    "minecraft:villager",
                    HealthyBehaviorPart {
                        calls: self.calls.clone(),
                    },
                );
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let entries = || {
            vec![
                ModEntry::new("reg-behaviors-panic", Box::new(PanickingBehaviorMod)),
                ModEntry::new(
                    "reg-behaviors-healthy",
                    Box::new(HealthyBehaviorMod {
                        calls: calls.clone(),
                    }),
                ),
            ]
        };
        mod_platform::testing::with_mods(entries().into(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&entries()).unwrap();
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                for (cx, cz) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                    fixture_resident_column(&mut game.world, (cx, cz));
                }
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 64, z, stone).unwrap();
                    }
                }
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:villager")
                    .unwrap()
                    .clone();
                let mut villager = Mob::new(203, kind, Vec3::new(3.5, 65.0, 3.5), 0.0);
                villager.ground.on_ground = true;
                // Baby villager about to grow up on the second tick, triggering
                // `age_boundary_reached` -> `villager_brain::refresh` -> second
                // `register_behaviors` dispatch through production `tick_mobs`.
                villager.extra.age = -2;
                game.mobs.push(villager);
                game.world.game_time = 100;
                game.world.day_time = 3000;

                // Tick 1: lazy first-tick brain init pauses and dispatches `register_behaviors`.
                game.tick_mobs().unwrap();
                assert_eq!(calls.load(Ordering::Relaxed), 1, "first brain build fired");
                assert_eq!(
                    game.world.block(4, 65, 4),
                    stone,
                    "first host write committed"
                );
                assert_eq!(
                    game.world.block(9, 65, 4),
                    game.world.air,
                    "panicking host write rolled back"
                );
                let mob = game.mobs.iter().find(|m| m.id == 203).unwrap();
                let keys = sunrise_sim::mobs::villager_brain::entry_keys(mob).unwrap();
                assert_eq!(
                    keys.iter().filter(|k| k.2).count(),
                    1,
                    "only the healthy behavior joined (panicking one rolled back)"
                );

                // Tick 2: baby reaches adult boundary (`breeding_age` -1 -> 0) and rebuilds
                // the brain, pausing and firing `register_behaviors` a second time.
                game.world.game_time = 101;
                game.tick_mobs().unwrap();
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    2,
                    "brain rebuild on growth fired register_behaviors again"
                );
                assert_eq!(
                    game.world.block(5, 65, 4),
                    stone,
                    "second host write committed"
                );
                assert_eq!(
                    game.world.block(9, 65, 4),
                    game.world.air,
                    "panicking host write still rolled back"
                );
                let mob = game.mobs.iter().find(|m| m.id == 203).unwrap();
                let keys = sunrise_sim::mobs::villager_brain::entry_keys(mob).unwrap();
                assert_eq!(
                    keys.iter().filter(|k| k.2).count(),
                    1,
                    "rebuilt brain holds exactly one healthy mod behavior"
                );
            });
        });
    }
}
