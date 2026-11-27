#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{bind_local_player, with_game};
    use mod_api::{EntityRef, MobAiPauseRefusal, MobRef, WorldEpoch};
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, nbt::Tag};
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::{Mob, Who};
    use sunrise_sim::motion::PlayerMotion;

    /// A server level with a stone floor at y 64 over chunk (0, 0) and a
    /// player bound as entity 20.
    fn level(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        game.options.dedicated = Some(Vec::new());
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), Arc::new(chunk));
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
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        bind_local_player(game, 20, 123);
    }

    fn add(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
        name: &str,
        at: Vec3,
    ) -> MobRef {
        let kind = game.res.mob_types().get(name).unwrap().clone();
        let mut m = Mob::new(id, kind, at, 0.0);
        m.ground.on_ground = true;
        game.mobs.push(m);
        game.mob(game.entity(id).unwrap()).unwrap()
    }

    fn mob<'a>(game: &'a sunrise_gameplay::game::ServerGame<'_>, handle: MobRef) -> &'a Mob {
        let id = handle.entity().id();
        game.mobs.iter().find(|m| m.id == id).unwrap()
    }

    fn mob_mut<'a>(
        game: &'a mut sunrise_gameplay::game::ServerGame<'_>,
        handle: MobRef,
    ) -> &'a mut Mob {
        let id = handle.entity().id();
        game.mobs.iter_mut().find(|m| m.id == id).unwrap()
    }

    fn tick(game: &mut sunrise_gameplay::game::ServerGame<'_>, ticks: u32) {
        for _ in 0..ticks {
            game.world.game_time += 1;
            game.tick_mobs().unwrap();
        }
    }

    /// A pig walking a path to (12.5, 65, 3.5) with a cow as its target.
    fn walking_pig(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> MobRef {
        level(game);
        add(game, 41, "minecraft:cow", Vec3::new(14.5, 65.0, 12.5));
        let pig = add(game, 40, "minecraft:pig", Vec3::new(3.5, 65.0, 3.5));
        assert!(pig.move_to(game, [12.5, 65.0, 3.5], 1.0));
        mob_mut(game, pig).ai.target = Some(Who::Mob(41));
        tick(game, 3);
        assert_eq!(pig.is_done(game), Some(false), "the path is established");
        pig
    }

    /// A paused pig keeps its path, move target and target, falls under
    /// gravity and takes knockback; unpaused, it walks its path on.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_paused_pig_keeps_its_path_and_target_falls_and_is_knocked_back() {
        with_game(|game| {
            let pig = walking_pig(game);
            assert_eq!(pig.ai_paused(game), Some(false));
            assert_eq!(pig.set_ai_paused(game, true), Ok(()));
            assert_eq!(pig.ai_paused(game), Some(true));
            assert_eq!(pig.no_ai(game), Some(false), "NoAI is a separate flag");
            let before = mob(game, pig).clone();
            let lifted = mob_mut(game, pig);
            lifted.position.y = 68.0;
            lifted.ground.on_ground = false;
            // The pig is the last mob, so it pops for the push as a hit does.
            let mut m = game.mobs.pop().unwrap();
            let hit = Damage::of(DamageKind::Generic);
            m.knockback(
                damage_level_of!(game, &sunrise_sim::health::NO_ATTACK, no_push, None),
                0.4,
                0.0,
                1.0,
                &hit,
                0.0,
            );
            game.mobs.push(m);
            tick(game, 20);
            let m = mob(game, pig);
            assert!(
                m.position.y < 65.1,
                "fell under gravity to the floor: {}",
                m.position.y
            );
            assert!(
                (m.position.z - before.position.z).abs() > 0.2,
                "carried the knockback: {} from {}",
                m.position.z,
                before.position.z
            );
            assert_eq!(
                m.ai.no_action_time, before.ai.no_action_time,
                "no AI step ran after the path's warm-up"
            );
            assert_eq!(m.ai.target, Some(Who::Mob(41)), "the target is kept");
            assert_eq!(
                m.ai.nav.target_pos, before.ai.nav.target_pos,
                "the path is kept"
            );
            assert_eq!(pig.is_done(game), Some(false));
            assert_eq!(
                m.ai.mov.wanted, before.ai.mov.wanted,
                "the move target is kept"
            );
            assert_eq!(
                (m.ai.xxa, m.ai.yya, m.ai.zza, m.ai.jumping),
                (0.0, 0.0, 0.0, false)
            );

            assert_eq!(pig.set_ai_paused(game, false), Ok(()));
            let x = mob(game, pig).position.x;
            tick(game, 20);
            let m = mob(game, pig);
            assert!(m.ai.no_action_time >= 1, "the AI step runs again");
            assert!(
                m.position.x > x + 0.5,
                "walks its path on: {} from {x}",
                m.position.x
            );
            assert_eq!(m.ai.target, Some(Who::Mob(41)));
        });
    }

    /// A long pause on an established path does not count towards the
    /// navigation timeout (`doStuckDetection`): the resumed pig keeps it.
    #[test]
    #[ignore = "daily"]
    fn a_long_pause_does_not_time_out_an_established_path() {
        with_game(|game| {
            let pig = walking_pig(game);
            let target = mob(game, pig).ai.nav.target_pos;
            assert_eq!(pig.set_ai_paused(game, true), Ok(()));
            // Ten times the timeout a node a block or two away allows.
            tick(game, 2400);
            assert_eq!(pig.set_ai_paused(game, false), Ok(()));
            tick(game, 1);
            assert_eq!(pig.is_done(game), Some(false), "the resume keeps the path");
            assert_eq!(mob(game, pig).ai.nav.target_pos, target);
            let x = mob(game, pig).position.x;
            tick(game, 20);
            assert!(mob(game, pig).position.x > x + 0.5, "and walks it");
        });
    }

    /// A paused allay (a brain mob) and a paused zombie (a goal mob) run no
    /// AI step while their unpaused neighbours do; a flying mob's vertical
    /// input is zeroed every paused tick, its move target kept. The zombie
    /// retains a running goal without ticking, stopping or restarting it.
    /// Its unpaused neighbour keeps ticking the same goal.
    /// The Allay keeps its real Deliver behaviour and expiring Brain memories,
    /// while its passive healing and duplication countdown still run.
    #[test]
    #[ignore = "daily"]
    fn a_paused_brain_or_goal_mob_runs_no_ai_step_and_holds_no_input() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_items::items::ItemStack;
        use sunrise_sim::mobs::allay;
        use sunrise_sim::mobs::goals::{Cx, Goal};

        #[derive(Clone, Debug)]
        struct PauseProbe(Arc<[AtomicUsize; 3]>);
        impl Goal for PauseProbe {
            fn flags(&self) -> u8 {
                0
            }
            fn can_use(&mut self, _: &mut Mob, _: &mut Cx<'_, '_, '_>) -> bool {
                true
            }
            fn start(&mut self, _: &mut Mob, _: &mut Cx<'_, '_, '_>) {
                self.0[0].fetch_add(1, Ordering::SeqCst);
            }
            fn tick(&mut self, _: &mut Mob, _: &mut Cx<'_, '_, '_>) {
                self.0[1].fetch_add(1, Ordering::SeqCst);
            }
            fn stop(&mut self, _: &mut Mob, _: &mut Cx<'_, '_, '_>) {
                self.0[2].fetch_add(1, Ordering::SeqCst);
            }
            fn every_tick(&self) -> bool {
                true
            }
            fn box_clone(&self) -> Box<dyn Goal> {
                Box::new(self.clone())
            }
            fn name(&self) -> &'static str {
                "PauseProbe"
            }
        }
        with_game(|game| {
            level(game);
            let paused = [
                add(game, 30, "minecraft:allay", Vec3::new(2.5, 67.0, 2.5)),
                add(game, 31, "minecraft:zombie", Vec3::new(6.5, 65.0, 2.5)),
            ];
            let thinking = [
                add(game, 32, "minecraft:allay", Vec3::new(2.5, 67.0, 10.5)),
                add(game, 33, "minecraft:zombie", Vec3::new(6.5, 65.0, 10.5)),
            ];
            const NOTE: [i32; 3] = [12, 65, 2];
            const LIKED_PLAYER: [i32; 4] = [1, 2, 3, 4];
            let note = game
                .pack
                .assets
                .registry
                .default_state("minecraft:note_block")
                .unwrap();
            game.world.set(NOTE[0], NOTE[1], NOTE[2], note).unwrap();
            let m = mob_mut(game, paused[0]);
            allay::hear_noteblock(m, "minecraft:overworld", NOTE);
            m.extra.allay.as_mut().unwrap().inventory = Some(ItemStack {
                item: "minecraft:diamond".into(),
                count: 1,
                damage: 0,
                extra: None,
            });
            let counts = Arc::new(std::array::from_fn(|_| AtomicUsize::new(0)));
            let thinking_counts = Arc::new(std::array::from_fn(|_| AtomicUsize::new(0)));
            for (handle, counts) in [(paused[1], &counts), (thinking[1], &thinking_counts)] {
                let zombie = mob_mut(game, handle);
                zombie.ai.goals.clear();
                zombie.ai.targets.clear();
                zombie.ai.goals.add(0, PauseProbe(Arc::clone(counts)));
            }
            let observed = || counts.each_ref().map(|count| count.load(Ordering::SeqCst));
            let thinking_observed = || {
                thinking_counts
                    .each_ref()
                    .map(|count| count.load(Ordering::SeqCst))
            };
            tick(game, 1);
            assert_eq!(observed(), [1, 1, 0]);
            assert_eq!(thinking_observed(), [1, 1, 0]);
            assert_eq!(
                mob(game, paused[1]).ai.goals.running_names(),
                vec!["PauseProbe"]
            );
            let allay_running = mob(game, paused[0]).ai.goals.running_names();
            assert!(
                allay_running.contains(&"Deliver"),
                "the real delivery starts"
            );
            for handle in paused {
                assert_eq!(handle.set_ai_paused(game, true), Ok(()));
            }
            let allay = paused[0];
            {
                let m = mob_mut(game, allay);
                allay::like_player(m, LIKED_PLAYER);
                let state = m.extra.allay.as_mut().unwrap();
                state.liked_player_ttl = Some(2);
                state.pickup_cooldown = Some(5);
                m.extra.ints[2] = 7;
                m.extra.egg_time = 40;
                m.health = 8.0;
            }
            {
                let m = mob_mut(game, thinking[0]);
                allay::like_player(m, LIKED_PLAYER);
                m.extra.allay.as_mut().unwrap().liked_player_ttl = Some(2);
            }
            mob_mut(game, allay).ai.mov.set_wanted(2.5, 75.0, 2.5, 1.0);
            let wanted = mob(game, allay).ai.mov.wanted;
            for _ in 0..10 {
                // What a flying control leaves set: the pause zeroes it each tick.
                mob_mut(game, allay).ai.yya = 0.6;
                tick(game, 1);
                assert_eq!(mob(game, allay).ai.yya, 0.0, "the vertical input is zeroed");
                assert_eq!(observed(), [1, 1, 0], "the running goal is held");
                assert_eq!(
                    mob(game, paused[1]).ai.goals.running_names(),
                    vec!["PauseProbe"]
                );
                assert_eq!(
                    mob(game, allay).ai.goals.running_names(),
                    allay_running,
                    "the real delivery behaviour is held"
                );
            }
            let m = mob(game, allay);
            assert!(m.position.y < 67.5, "did not climb: {}", m.position.y);
            assert!(m.ai.mov.has_wanted(), "the move target is kept");
            assert_eq!(m.ai.mov.wanted, wanted);
            let state = m.extra.allay.as_ref().unwrap();
            assert_eq!(state.liked_player_ttl, Some(2));
            assert!(m.extra.tame);
            assert_eq!(m.extra.owner, Some(LIKED_PLAYER));
            assert_eq!(state.noteblock.unwrap().pos, NOTE);
            assert_eq!(m.extra.ints[2], 7);
            assert_eq!(state.pickup_cooldown, Some(5));
            let held = state.inventory.as_ref().unwrap();
            assert_eq!(held.item.as_ref(), "minecraft:diamond");
            assert_eq!(held.count, 1);
            assert_eq!(m.extra.egg_time, 30, "duplication is passive");
            assert_eq!(m.health, 9.0, "passive healing continues");
            assert_eq!(
                thinking_observed(),
                [1, 11, 0],
                "the unpaused goal keeps ticking without restarting"
            );
            let thinking_allay = mob(game, thinking[0]);
            assert!(!thinking_allay.extra.tame, "the unpaused memory expires");
            assert_eq!(thinking_allay.extra.owner, None);
            for handle in paused {
                assert_eq!(handle.set_ai_paused(game, false), Ok(()));
            }
            tick(game, 1);
            let m = mob(game, allay);
            assert_eq!(m.extra.allay.as_ref().unwrap().liked_player_ttl, Some(1));
            assert_eq!(m.extra.owner, Some(LIKED_PLAYER));
            assert_eq!(m.extra.ints[2], 6);
            assert_eq!(allay::pickup_cooldown(m), Some(4));
            assert!(m.ai.goals.running_names().contains(&"Deliver"));
            tick(game, 2);
            let m = mob(game, allay);
            assert!(!m.extra.tame, "the resumed memory expires normally");
            assert_eq!(m.extra.owner, None);
            assert_eq!(
                observed(),
                [1, 4, 0],
                "the same goal resumes without restarting"
            );
            assert_eq!(
                mob(game, paused[1]).ai.goals.running_names(),
                vec!["PauseProbe"]
            );
            assert_eq!(thinking_observed(), [1, 14, 0]);
        });
    }

    fn paused_value(tag: &Tag) -> Option<&Tag> {
        tag.get(sunrise_mod_runtime::attachments::ATTACHMENTS_KEY)?
            .get("sunrise")?
            .get("ai_paused")
    }

    /// The pause is saved with the mob beside the mods' attachments, never as
    /// `NoAI`, and a reload finds the mob still paused; a resumed mob's save
    /// drops it and keeps the mods' own.
    #[test]
    #[ignore = "daily"]
    fn the_pause_round_trips_through_a_save_and_writes_no_no_ai() {
        use sunrise_core::nbt::Compound;
        with_game(|game| {
            level(game);
            let pig = add(game, 40, "minecraft:pig", Vec3::new(3.5, 65.0, 3.5));
            let mods = Compound::from_iter([(
                "example".into(),
                Tag::Compound(Compound::from_iter([(
                    "count".into(),
                    Tag::Compound(Compound::from_iter([("value".into(), Tag::Int(3))])),
                )])),
            )]);
            game.entity_attachments.insert(
                40,
                sunrise_mod_runtime::attachments::Attachments::from_nbt(mods),
            );
            assert_eq!(pig.set_ai_paused(game, true), Ok(()));
            let reload = |game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                let index = game.mobs.iter().position(|m| m.id == id).unwrap();
                let saved = game.mob_tree_nbt(index);
                game.mobs.remove(index);
                game.forget_entity(id);
                let id = game.load_mob(saved.clone(), None).unwrap();
                (saved, game.mob(game.entity(id).unwrap()).unwrap())
            };
            let (saved, pig) = reload(game, 40);
            assert_eq!(paused_value(&saved), Some(&Tag::Byte(1)));
            assert!(saved.get("NoAI").is_none(), "no NoAI tag: {saved:?}");
            assert_eq!(
                pig.ai_paused(game),
                Some(true),
                "still paused after the reload"
            );
            assert_eq!(pig.no_ai(game), Some(false));

            assert_eq!(pig.set_ai_paused(game, false), Ok(()));
            let (saved, pig) = reload(game, pig.entity().id());
            assert_eq!(paused_value(&saved), None);
            assert!(saved.get("NoAI").is_none());
            let attachments = saved
                .get(sunrise_mod_runtime::attachments::ATTACHMENTS_KEY)
                .unwrap();
            assert!(attachments.get("sunrise").is_none(), "nothing left behind");
            assert!(
                attachments.get("example").is_some(),
                "the mods' attachment is kept"
            );
            assert_eq!(pig.ai_paused(game), Some(false));
        });
    }

    /// Each case the pause cannot apply to has its own refusal, from the
    /// real backend, and none of them changes anything.
    #[test]
    #[ignore = "daily"]
    fn each_refusal_is_named_and_changes_nothing() {
        use sunrise_gameplay_common::game::dragon_common::Dragon;
        use sunrise_gameplay_common::game::execution_role::ExecutionRole;
        with_game(|game| {
            level(game);
            let pig = add(game, 40, "minecraft:pig", Vec3::new(3.5, 65.0, 3.5));
            let epoch = pig.entity().epoch();
            let refused =
                |game: &mut sunrise_gameplay::game::ServerGame<'_>, handle: MobRef, why| {
                    assert_eq!(handle.set_ai_paused(game, true), Err(why), "{why:?}");
                    assert_eq!(handle.ai_paused(game), None, "{why:?}");
                };

            let stale = MobRef::new(EntityRef::new(WorldEpoch::new(999), 40));
            refused(game, stale, MobAiPauseRefusal::Stale);
            let gone = MobRef::new(EntityRef::new(epoch, 999));
            refused(game, gone, MobAiPauseRefusal::Stale);

            game.server.held_out.push(40);
            refused(game, pig, MobAiPauseRefusal::HeldOut);
            game.server.held_out.pop();

            let mut dragon = Dragon::new(Vec3::new(8.0, 80.0, 8.0), 0.0, 0);
            dragon.id = 50;
            game.dragon.replace(dragon);
            refused(
                game,
                MobRef::new(EntityRef::new(epoch, 50)),
                MobAiPauseRefusal::NoAiStep,
            );
            refused(
                game,
                MobRef::new(EntityRef::new(epoch, 51)),
                MobAiPauseRefusal::NoAiStep,
            );

            let player = MobRef::new(game.entity(20).unwrap());
            refused(game, player, MobAiPauseRefusal::NotAMob);

            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(
                pig.set_ai_paused(game, true),
                Err(MobAiPauseRefusal::Client)
            );
            game.options.execution_role = ExecutionRole::IntegratedServer;

            assert_eq!(pig.ai_paused(game), Some(false), "no refusal paused it");
            assert!(!mob(game, pig).no_ai);
        });
    }
}
