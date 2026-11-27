#![cfg(test)]
use sunrise_sim::mobs::Mob;
#[cfg(test)]
pub(crate) mod tests {

    use mod_api::{AgeableGroup, EquipmentSlot, Identifier, SpawnGroup, UnspawnedMobError};

    use sunrise_gameplay_common::game::server_side::native_unspawned_mob::tests::{
        KeptGoalMod, pumpkin,
    };

    use sunrise_gameplay_common::game::server_side::native_unspawned_mob::tests::AT;

    use sunrise_gameplay_common::game::server_side::native_unspawned_mob::tests::id;

    type Listen = dyn Fn(&dyn Host, &mut FinalizeSpawnEvent<'_>) -> Flow + Send + Sync;
    /// Answers `finalize_spawn` with the test's closure.
    struct Listener(Box<Listen>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn finalize_spawn(&self, host: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            (self.0)(host, event)
        }
    }
    fn listening<R>(
        listen: impl Fn(&dyn Host, &mut FinalizeSpawnEvent<'_>) -> Flow + Send + Sync + 'static,
        run: impl FnOnce() -> R,
    ) -> R {
        let listener = Listener(Box::new(listen));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("unspawned", Box::new(listener))].into(),
            run,
        )
    }

    use sunrise_gameplay::game::server_side::native_unspawned_mob::tests::spawn_pig;

    use sunrise_gameplay_common::game::commands_common::Target;

    use super::super::native_entity_refs::tests::with_game;

    use super::*;

    use mod_api::{FinalizeSpawnEvent, Flow, Host};

    use std::sync::{Arc, Mutex};

    use sunrise_sim::mobs::finalize::{self, Attr};

    /// What a listener edits through the editor is on the mob that joins the
    /// level, and the handle reads the edits back inside the call.
    #[test]
    #[ignore = "daily"]
    fn edits_through_the_editor_are_on_the_mob_that_joins() {
        type Seen = (
            Result<Identifier, UnspawnedMobError>,
            Result<f64, UnspawnedMobError>,
            Result<Option<mod_api::ItemStack>, UnspawnedMobError>,
        );
        let seen: Arc<Mutex<Option<Seen>>> = Arc::new(Mutex::new(None));
        let saw = seen.clone();
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                let mob = &mut event.mob;
                mob.set_equipment(EquipmentSlot::Head, Some(pumpkin()))
                    .unwrap();
                let mut oversized = pumpkin();
                oversized.set_count(100).unwrap();
                assert_eq!(
                    mob.set_equipment(EquipmentSlot::Head, Some(oversized)),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(mob.equipment(EquipmentSlot::Head), Ok(Some(pumpkin())));
                mob.set_equipment(EquipmentSlot::Head, None).unwrap();
                assert_eq!(mob.equipment(EquipmentSlot::Head), Ok(None));
                mob.set_equipment(EquipmentSlot::Head, Some(pumpkin()))
                    .unwrap();
                mob.set_attribute_base(&max_health, 20.0).unwrap();
                assert_eq!(mob.add_tag("test:tagged"), Ok(true));
                mob.set_custom_name(Some(mod_api::Text::literal("Bacon")))
                    .unwrap();
                *saw.lock().unwrap() = Some((
                    mob.kind(),
                    mob.attribute_base(&max_health),
                    mob.equipment(EquipmentSlot::Head),
                ));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    let head = &m.equipment_stacks()[EquipmentSlot::Head as usize];
                    assert_eq!(
                        head.as_ref().map(|s| &*s.item),
                        Some("minecraft:carved_pumpkin")
                    );
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 20.0);
                    assert_eq!(m.custom_name.as_deref(), Some("Bacon"));
                    assert_eq!(game.tags_of(Target::Mob(spawned)), ["test:tagged"]);
                });
                // Item keys resolve with the game inside this scope, so the stack is read here.
                let (kind, base, head) = seen.lock().unwrap().take().expect("the hook ran");
                assert_eq!(kind, Ok(id("minecraft:pig")));
                assert_eq!(base, Ok(20.0));
                assert_eq!(head, Ok(Some(pumpkin())));
            },
        );
    }
    /// A mod's base written through the editor stays in the attribute's range, as documented.
    #[test]
    #[ignore = "daily"]
    fn a_base_written_through_the_editor_stays_in_range() {
        let seen: Arc<Mutex<Option<Result<f64, UnspawnedMobError>>>> = Arc::new(Mutex::new(None));
        let saw = seen.clone();
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                let mob = &mut event.mob;
                mob.set_attribute_base(&max_health, 2000.0).unwrap();
                *saw.lock().unwrap() = Some(mob.attribute_base(&max_health));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 1024.0);
                });
                assert_eq!(seen.lock().unwrap().take(), Some(Ok(1024.0)));
            },
        );
    }
    /// `Flow::Cancel` skips finalization and keeps the listener's edits on
    /// the mob that still joins; `spawn_cancelled` adds no mob at all.
    #[test]
    #[ignore = "daily"]
    fn both_cancels_keep_their_contract_with_edits() {
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                event.mob.set_attribute_base(&max_health, 20.0).unwrap();
                Flow::Cancel
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).expect("a skipped finalization still spawns");
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 20.0);
                    assert!(
                        !finalize::has_modifier(
                            m,
                            Attr::FollowRange,
                            "minecraft:random_spawn_bonus"
                        ),
                        "no finalizeSpawn ran"
                    );
                });
            },
        );
        listening(
            |_, event| {
                assert_eq!(event.mob.add_tag("test:gone"), Ok(true));
                event.spawn_cancelled = true;
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let before = game.mobs.len();
                    assert_eq!(spawn_pig(game), None);
                    assert_eq!(game.mobs.len(), before, "a stopped spawn adds no mob");
                });
            },
        );
    }
    /// Every live site hands `group` over empty today; group data a listener
    /// leaves there is what finalization reads: an ageable group whose later
    /// members are all babies makes this pig one.
    #[test]
    #[ignore = "daily"]
    fn the_group_a_listener_leaves_is_what_finalization_reads() {
        with_game(|game| {
            let alone = spawn_pig(game).unwrap();
            let m = game.mobs.iter().find(|m| m.id == alone).unwrap();
            assert!(!m.is_baby(), "a pack's first member is never a baby");
        });
        let given = Arc::new(Mutex::new(Vec::new()));
        let heard = given.clone();
        listening(
            move |_, event| {
                heard.lock().unwrap().push(event.group.clone());
                let mut ageable = AgeableGroup::new(true, 1.0);
                ageable.size = 1;
                *event.group = Some(SpawnGroup::Ageable(ageable));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert!(m.is_baby());
                });
            },
        );
        assert_eq!(*given.lock().unwrap(), [None]);
    }
    /// Unwinding out of an unspawned-mob loan leaves the mob intact in place
    /// (mod goal and all, never copied or replaced) and leaves no broken state
    /// for a subsequent `finalize_spawn` call.
    #[test]
    #[ignore = "daily"]
    fn an_engine_unwind_still_ends_the_unspawned_mob_lend() {
        let installed = || vec![mod_api::ModEntry::new("kept-goal", Box::new(KeptGoalMod))];
        mod_platform::testing::with_mods(installed().into(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&installed()).unwrap();
            with_game(|game| {
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut zombie = Mob::new(70, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
                game.flush_unjoined_mob_parts(&mut zombie);
                zombie.custom_name = Some("Lent".to_owned());
                let goals = |m: &Mob| -> Vec<&'static str> {
                    m.ai.goals.entries().map(|(_, name, _)| name).collect()
                };
                assert!(
                    goals(&zombie).contains(&"Kept"),
                    "the part attached its goal"
                );
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut borrowed =
                        game.borrowed_unspawned_mob_mut("minecraft:zombie", AT, Some(&mut zombie));
                    let editor = mod_api::UnspawnedMobEditor::new(&mut borrowed);
                    assert_eq!(editor.kind(), Ok(id("minecraft:zombie")));
                    std::panic::resume_unwind(Box::new("unspawned mob fixture fault"));
                }));
                assert_eq!(
                    panicked.unwrap_err().downcast_ref::<&str>(),
                    Some(&"unspawned mob fixture fault")
                );
                assert_eq!(zombie.id, 70);
                assert_eq!(zombie.custom_name.as_deref(), Some("Lent"));
                let back = goals(&zombie);
                assert!(
                    back.contains(&"Kept"),
                    "the mod goal keeps its goal: {back:?}"
                );
                assert!(!back.contains(&"ModGoal"), "no copied mod goal: {back:?}");
            });
            sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
        });
        listening(
            |_, event| {
                assert_eq!(event.mob.add_tag("test:after_panic"), Ok(true));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    assert_eq!(game.tags_of(Target::Mob(spawned)), ["test:after_panic"]);
                });
            },
        );
    }
}
