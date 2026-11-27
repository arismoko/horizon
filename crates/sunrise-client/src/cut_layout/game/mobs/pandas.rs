#![cfg(test)]
use sunrise_core::geometry::Vec3;
#[cfg(test)]
pub(crate) mod nested_spawn_tests {
    use crate::game::server_side::entity_server::taken_pass_tests;

    #[test]
    #[ignore = "daily"]
    fn lifted_mob_callback_keeps_nested_spawns_until_the_next_pass() {
        taken_pass_tests::in_both_dimensions(|game, balance| {
            let kind = game.res.mob_types().get("minecraft:panda").unwrap().clone();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                99,
                kind,
                sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
                0.0,
            ));
            taken_pass_tests::assert_mob_spawn_survives(game, balance, |game| {
                game.panda_sneezed(99);
            });
        });
    }
}

#[cfg(test)]
pub(crate) mod pickup_tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_sim::advancements as adv;
    use sunrise_sim::motion::PlayerMotion;
    use sunrise_world::lazy_json::LazyJson;

    const ADV: &str = "sunrise:test/panda_pickup";
    const TRIGGER: &str = "minecraft:thrown_item_picked_up_by_entity";
    const THROWER: [i32; 4] = [413, 0, 0, 1];

    /// An advancement on the pickup trigger, as a data pack would add one: no
    /// vanilla advancement tests it, so the tree needs one of its own.
    fn install_pickup_advancement(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let mut tree = std::sync::Arc::new(adv::Tree::default());
        let one = std::sync::Arc::get_mut(&mut tree).unwrap();
        one.list.push(adv::Advancement {
            id: ADV.into(),
            parent: None,
            display: None,
            criteria: vec![adv::Criterion {
                name: "pickup".into(),
                trigger: TRIGGER.into(),
                conditions: LazyJson::new(&serde_json::json!({})),
                player: None,
            }],
            requirements: vec![vec!["pickup".into()]],
            rewards: Default::default(),
        });
        one.by_id.insert(ADV.into(), 0);
        one.by_trigger.insert(TRIGGER.into(), vec![(0, 0)]);
        game.adv.tree = tree;
    }

    /// `Panda.pickUpItem` calls `onItemPickup`, which fires
    /// `CriteriaTriggers.THROWN_ITEM_PICKED_UP_BY_ENTITY` for whoever threw the
    /// bamboo, as the fox's pickup does (`LivingEntity.onItemPickup`).
    #[test]
    #[ignore = "daily"]
    fn a_panda_that_takes_a_thrown_item_credits_the_thrower() {
        let world = tempfile::tempdir().unwrap();
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.options.dedicated = None;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());
        game.level.player_uuid = Some(THROWER);
        game.level.health = 20.0;
        game.creative = false;
        game.server.world_root = world.path().to_owned();
        game.set_allow_commands(true);
        // Opening the world installs its staged advancement tree. Install the
        // custom tree afterwards so the first tick cannot replace it.
        game.open_world_datapacks(&game.level.enabled_features.clone(), None)
            .unwrap();
        install_pickup_advancement(&mut game);
        // `DistanceManager` keeps a chunk entity-ticking only while its whole
        // 5x5 neighbourhood is resident.
        for cx in -2..=2 {
            for cz in -2..=2 {
                game.world.chunks.insert(
                    (cx, cz),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(cx, cz)),
                );
            }
        }
        for command in [
            "forceload add 0 0",
            "tick freeze",
            "fill 0 99 0 15 99 15 stone",
            "summon minecraft:panda 4.5 100 6.5 {UUID:[I;9,9,9,9],PersistenceRequired:1b,CanPickUpLoot:1b}",
            "summon minecraft:item 4.5 100 6.5 {Item:{id:\"minecraft:bamboo\",count:1},Thrower:[I;413,0,0,1],PickupDelay:0s}",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        let id = game.mob_by_uuid([9, 9, 9, 9]).unwrap();
        let holding = |g: &sunrise_gameplay::game::ServerGame<'_>| {
            g.mobs
                .iter()
                .any(|m| m.id == id && m.extra.main_hand_id() == Some("minecraft:bamboo"))
        };
        for _ in 0..20 {
            if holding(&game) {
                break;
            }
            assert!(game.run_command("tick step 1"));
            game.tick_server(false).unwrap();
        }
        assert!(holding(&game), "the panda took the bamboo");
        assert!(
            game.adv.progress.has(ADV, "pickup"),
            "the thrower's advancement: {:?}",
            game.adv.progress.criteria
        );
    }
}
