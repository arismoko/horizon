use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::projectiles::ProjectileKind;

#[test]
#[ignore = "daily"]
fn conversion_transfers_tags_and_team() {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::{
        mobs::{Mob, conversion::Params},
        save::uuid_text,
    };

    super::super::native_entity_refs::tests::with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let from = Mob::new(100, kind, Vec3::ZERO, 0.0);
        game.mobs.push(from.clone());
        game.server.next_entity = 100;
        game.set_entity_tags(Target::Mob(100), &["tracked".into()]);
        let old = uuid_text(game.uuid_for(100));
        game.load_scoreboard();
        game.score.board.add_team("test");
        game.score.board.add_to_team(&old, "test");
        let id = game
            .convert_to(
                &from,
                "minecraft:zombie_villager",
                Params {
                    keep_equipment: false,
                    preserve_can_pick_up_loot: true,
                },
                false,
                |_, _| {},
            )
            .unwrap();
        assert_eq!(game.tags_of(Target::Mob(id)), ["tracked"]);
        let new = uuid_text(game.uuid_for(id));
        assert_eq!(game.score.board.players_team(&new).unwrap().name, "test");
        assert!(game.score.board.players_team(&old).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn ender_pearl_clears_impulse_before_next_fall() {
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::motion::{MotionWorld, PlayerMotion};
    use sunrise_sim::{
        mobs::Who,
        projectiles::{Hit, Projectile},
    };
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    let mut player = PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
    player
        .impulse
        .set_ignore_fall_damage(true, Some(player.position));
    player.fall_distance = 4.0;
    game.player = Some(player);
    let destination = Vec3::new(0.0, 80.0, 0.0);
    let mut pearl = Projectile::new(ProjectileKind::EnderPearl, destination, Some(Who::Player));
    pearl.previous = destination;
    game.projectile_hit(
        &pearl,
        Hit::Block {
            pos: [0, 80, 0],
            at: destination,
            face: [0; 3],
        },
    )
    .unwrap();
    use sunrise_gameplay_common::game::{Side as _, side_ops::tables::SideOps as _};
    let (common, state) = (&mut game.common, &game.server);
    let player = common.player.as_mut().unwrap();
    assert_eq!(player.position, destination);
    assert_eq!(player.fall_distance, 0.0);
    let level = state.level_ref(common.dim, common.native_lifecycle.epoch());
    let side = sunrise_gameplay_common::game_side!(
        common,
        (state.lan_acting_uuid(), level, state.session_lock_active()),
        state.side_level_features(common.level.enabled_features.as_arc()),
        state.side_game_tag_items(&common.res)
    );
    let damaged = MotionWorld {
        side: &side,
        entities: &[],
        player,
        scene: &mut common.world,
        assets: &common.pack.assets,
        collision_binding: common.collision,
        resources: common.motion,
        health: &common.health_resources,
        tag_items: Some(common.res.items()),
        loop_context: &common.loop_context,
        damage_hooks: &sunrise_sim::health::DamageHooks::NONE,
        block_registries: Some(&common.ui.registries),
    }
    .cause_mount_fall(10.0, 1.0)
    .unwrap();
    assert!(
        damaged,
        "a ten-block fall after the teleport must cause damage"
    );
}

#[test]
#[ignore = "daily"]
fn wolf_owner_rejoining_receives_the_kill_statistic() {
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_sim::advancements::{Advancement, Criterion, Rewards, Tree};
    use sunrise_sim::mobs::{Damage, Mob};

    for dedicated in [false, true] {
        super::super::native_guest_turn_tests::with_guests(|game| {
            if dedicated {
                game.options.dedicated = Some(Vec::new());
            }
            let mut tree = Tree::default();
            tree.list.push(Advancement {
                id: "test:catalyst".into(),
                parent: None,
                display: None,
                criteria: vec![Criterion {
                    name: "kill".into(),
                    trigger: "minecraft:kill_mob_near_sculk_catalyst".into(),
                    conditions: serde_json::json!({}).into(),
                    player: None,
                }],
                requirements: vec![vec!["kill".into()]],
                rewards: Rewards::default(),
            });
            tree.by_id.insert("test:catalyst".into(), 0);
            tree.by_trigger.insert(
                "minecraft:kill_mob_near_sculk_catalyst".into(),
                vec![(0, 0)],
            );
            game.adv.tree = Arc::new(tree);
            game.adv.loaded = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let catalyst = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sculk_catalyst")
                .unwrap();
            game.with_level(|b, level, ticks| {
                b.set_block(level, ticks, BlockPos::new(1, 0, 0), catalyst, 2 | 16);
            });
            game.sculk_prepare();
            game.sculk_sync_index();
            assert!(game.world.block_entities.contains_key(&(1, 0, 0)));
            let wolf_type = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
            let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut wolf = Mob::new(100, wolf_type, Vec3::ZERO, 0.0);
            wolf.extra.tame = true;
            wolf.extra.owner = Some(sunrise_sim::lan::playerdata::uuid_ints(12));
            game.mobs.push(wolf);
            game.mobs.push(Mob::new(101, pig_type, Vec3::ZERO, 0.0));
            {
                let value = false;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }
            game.resolve_player_responsible(101, Damage::mob(100, [0.0; 3]));
            let credit = *game.server.last_hurt_by_player.get(&101).unwrap();
            assert!(game.resolve_player_credit(credit).is_none());
            {
                let value = true;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }
            assert_eq!(game.resolve_player_credit(credit), Some(Some(12)));
            let victim = game.mobs[1].clone();
            game.mob_die(&victim);
            let stats = &game
                .server
                .lan
                .as_ref()
                .unwrap()
                .play
                .get(&12)
                .unwrap()
                .stats;
            assert_eq!(stats.get("custom", "minecraft:mob_kills"), 1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    assert!(guest.advancements.has("test:catalyst", "kill"));
                    guest.advancements = Default::default();
                });
            let mut second = victim.clone();
            second.id = 102;
            game.server.last_hurt_by_player.insert(
                102,
                sunrise_gameplay_common::game::combat_common::PlayerCredit(12),
            );
            game.as_guest(12, |game| game.mob_die(&second)).unwrap();
            let host = game.server.lan.as_ref().unwrap();
            assert_eq!(host.play[&12].stats.get("custom", "minecraft:mob_kills"), 2);
            assert!(host.hub.guests[0].advancements.has("test:catalyst", "kill"));
            {
                let value = "minecraft:the_nether".into();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            let mut third = victim.clone();
            third.id = 103;
            game.resolve_player_responsible(101, Damage::mob(100, [0.0; 3]));
            let credit = game.server.last_hurt_by_player.remove(&101).unwrap();
            game.server.last_hurt_by_player.insert(103, credit);
            game.mob_die(&third);
            let host = game.server.lan.as_ref().unwrap();
            assert_eq!(host.play[&12].stats.get("custom", "minecraft:mob_kills"), 3);
            game.mobs[0].extra.owner = None;
            game.resolve_player_responsible(101, Damage::mob(100, [0.0; 3]));
            assert!(!game.server.last_hurt_by_player.contains_key(&101));
        });
    }
}
