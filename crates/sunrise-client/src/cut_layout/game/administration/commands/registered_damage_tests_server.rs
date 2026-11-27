use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_sim::mobs::{Damage, Who};

#[test]
#[ignore = "daily"]
fn live_damage_and_blast_warden_reactions_follow_source_entities_and_cooldown() {
    use mod_api::{ExplosionInteraction, ExplosionSettings};
    use sunrise_core::{geometry::Vec3, nbt::Tag, position::BlockPos};
    use sunrise_sim::{mobs::ai, projectiles::ProjectileKind};
    use {
        crate::game::server_side::native_goal_ticks::live_read_tests::read_borrowed_target,
        crate::game::server_side::native_living_flags::tests::guest,
        crate::game::server_side::native_living_flags::tests::guest_server,
        sunrise_gameplay::game::commands::data::DataRef,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::commands_server::world_cmds::registered_damage_tests::live_damage_and_blast_warden_reactions_follow_source_entities_and_cooldown",
        "SUNRISE_WARDEN_LIVE_DAMAGE_CHILD",
        &["live-damage"],
    ) {
        return;
    }
    with_game(|game| {
        setup_live_damage_world(game);
        assert!(game.run_command("summon warden 4.5 64 4.5 {UUID:[I;84,0,0,1],NoGravity:1b}"));
        let warden_id = game.mobs.last().unwrap().id;
        assert!(
            game.run_command("summon pig 14.5 64 4.5 {UUID:[I;84,0,0,2],NoAI:1b,NoGravity:1b}")
        );
        let pig_id = game.mobs.last().unwrap().id;
        assert!(
            game.run_command("summon armor_stand 14.5 64 4.5 {UUID:[I;84,0,0,3],NoGravity:1b}")
        );
        let stand_id = game.armor_stands.last().unwrap().id;
        let _arrow_id = spawn_test_projectile(
            game,
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(5.5, 64.0, 4.5),
            Some(Who::Mob(pig_id)),
            [84, 0, 0, 4],
        );

        let reset_warden = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            let w = game.mobs.iter_mut().find(|m| m.id == warden_id).unwrap();
            w.invulnerable = 0;
            w.last_hurt = 0.0;
            w.health = 500.0;
            w.extra.warden.as_mut().unwrap().anger = Default::default();
            assert!(ai::set_brain_target(w, None, None));
        };
        let warden_state = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let st = game
                .mobs
                .iter()
                .find(|m| m.id == warden_id)
                .unwrap()
                .extra
                .warden
                .as_ref()
                .unwrap();
            (st.target(), st.anger.active_anger(None))
        };

        // Vary damage type with a fixed direct mob attacker at 10 blocks: every type stays direct.
        for ty in [
            "minecraft:mob_projectile",
            "minecraft:arrow",
            "minecraft:mob_attack",
            "minecraft:generic",
        ] {
            reset_warden(game);
            assert!(
                game.run_command(&format!(
                    "damage 00000054-0000-0000-0000-000000000001 1 {ty} by 00000054-0000-0000-0000-000000000002"
                )),
                "{ty}"
            );
            let (target, anger) = warden_state(game);
            assert_eq!(target, Some(Who::Mob(pig_id)), "{ty}");
            assert_eq!(anger, 100, "{ty}");
        }

        // A direct ArmorStand sets attack_target (any LivingEntity) while canTargetEntity skips anger.
        reset_warden(game);
        assert!(game.run_command(
            "damage 00000054-0000-0000-0000-000000000001 1 minecraft:arrow by 00000054-0000-0000-0000-000000000003"
        ));
        assert_eq!(warden_state(game), (Some(Who::Mob(stand_id)), 0));

        // Vary entities with fixed minecraft:mob_attack: indirect arrow beyond 5 blocks angers Pig
        // without targeting, and skips ArmorStand completely.
        reset_warden(game);
        assert!(game.run_command(
            "damage 00000054-0000-0000-0000-000000000001 1 minecraft:mob_attack by 00000054-0000-0000-0000-000000000004 from 00000054-0000-0000-0000-000000000002"
        ));
        let (target, anger) = warden_state(game);
        assert_eq!(target, None);
        assert_eq!(anger, 100);

        reset_warden(game);
        assert!(game.run_command(
            "damage 00000054-0000-0000-0000-000000000001 1 minecraft:mob_attack by 00000054-0000-0000-0000-000000000004 from 00000054-0000-0000-0000-000000000003"
        ));
        assert_eq!(warden_state(game), (None, 0));

        // Moving Pig within 5 blocks lets the same indirect hit set attack_target.
        game.mobs
            .iter_mut()
            .find(|m| m.id == pig_id)
            .unwrap()
            .position = Vec3::new(7.5, 64.0, 4.5);
        reset_warden(game);
        assert!(game.run_command(
            "damage 00000054-0000-0000-0000-000000000001 1 minecraft:mob_attack by 00000054-0000-0000-0000-000000000004 from 00000054-0000-0000-0000-000000000002"
        ));
        let (target, anger) = warden_state(game);
        assert_eq!(target, Some(Who::Mob(pig_id)));
        assert_eq!(anger, 100);

        // Smaller hit during hurt cooldown returns false, yet still updates anger and target.
        reset_warden(game);
        assert!(
            game.run_command("damage 00000054-0000-0000-0000-000000000001 5 minecraft:generic")
        );
        let health_after_first = game.mobs.iter().find(|m| m.id == warden_id).unwrap().health;
        assert!(!game.run_command(
            "damage 00000054-0000-0000-0000-000000000001 1 minecraft:generic by 00000054-0000-0000-0000-000000000002"
        ));
        assert_eq!(
            game.mobs.iter().find(|m| m.id == warden_id).unwrap().health,
            health_after_first
        );
        let (target, anger) = warden_state(game);
        assert_eq!(target, Some(Who::Mob(pig_id)));
        assert_eq!(anger, 100);

        // A refused player melee hit also updates anger and target before any tick.
        reset_warden(game);
        assert!(
            game.run_command("damage 00000054-0000-0000-0000-000000000001 5 minecraft:generic")
        );
        let index = game.mobs.iter().position(|m| m.id == warden_id).unwrap();
        let health_after_first = game.mobs[index].health;
        let from = game.target_position(Target::Player).unwrap();
        assert!(!game.hurt_by_player(index, 1.0, Damage::player(from)).0);
        assert_eq!(game.mobs[index].health, health_after_first);
        assert_eq!(warden_state(game), (Some(Who::Player), 100));

        // Live blasts: direct mob blast at 10 blocks targets the mob; unowned TNT stays sourceless.
        game.mobs
            .iter_mut()
            .find(|m| m.id == pig_id)
            .unwrap()
            .position = Vec3::new(14.5, 64.0, 4.5);
        reset_warden(game);
        game.native_explode(
            Some((pig_id, Target::Mob(pig_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        let (target, anger) = warden_state(game);
        assert_eq!(target, Some(Who::Mob(pig_id)));
        assert_eq!(anger, 100);

        reset_warden(game);
        game.add_primed_tnt(BlockPos::new(4, 64, 4), 0.0, 80, None, None);
        let tnt_id = game.tnts.last().unwrap().id;
        game.native_explode(
            Some((tnt_id, Target::Tnt(tnt_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        assert_eq!(warden_state(game), (None, 0));

        const WARDEN: &str = "00000054-0000-0000-0000-000000000001";
        const DRAGON: &str = "00000054-0000-0000-0000-000000000007";
        let warden = game.mob(game.entity(warden_id).unwrap()).unwrap();
        let check_target = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                            expected: Option<mod_api::LivingRef>| {
            assert_eq!(warden.target(game), expected);
            // Keep guest 23 selected in the tick even when the stored target is 22.
            assert_eq!(read_borrowed_target(game, warden, pig_id, 23), expected);
            let selector = match expected.map(|living| living.entity().id()) {
                Some(22) => "@s[name=guest22]",
                Some(23) => "@s[name=guest23]",
                Some(id) if id == pig_id => "@s[type=minecraft:pig]",
                Some(_) => "@s[type=minecraft:ender_dragon]",
                None => "@s",
            };
            game.server.cmds.0.results = Some(Vec::new());
            assert!(game.run_command(&format!(
                "execute as {WARDEN} on target if entity {selector}"
            )));
            let results = game.server.cmds.0.results.take().unwrap();
            if expected.is_some() {
                assert_eq!(results, [(true, 1)], "{selector}");
            } else {
                assert!(
                    results.is_empty(),
                    "the target fork should produce no source"
                );
            }
        };
        guest_server(game, true, false);
        let (first, _first_out) = guest(game, 22);
        let (second, _second_out) = guest(game, 23);
        // DamageCommand defaults the cause to the direct entity, which the warden uses for attribution.
        let hurt = |game: &mut sunrise_gameplay::game::ServerGame<'_>, amount, direct| {
            game.hurt_target(
                Target::Mob(warden_id),
                amount,
                "minecraft:generic",
                direct,
                direct,
                None,
            )
        };
        reset_warden(game);
        assert!(hurt(game, 1.0, Some(Target::Guest(22))));
        assert!(hurt(game, 5.0, Some(Target::Guest(23))));
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .mob_players
                .get(&warden_id),
            Some(&23)
        );
        check_target(game, Some(first));

        reset_warden(game);
        assert!(hurt(game, 5.0, None));
        assert!(!hurt(game, 1.0, Some(Target::Guest(23))));
        check_target(game, Some(second));
        assert!(game.entity_set_invulnerable_time(second.entity(), 7));
        check_target(game, None);
        assert!(game.entity_set_invulnerable_time(second.entity(), 0));
        check_target(game, Some(second));

        // A read after every live edit: absence and the two unreadable shapes
        // keep the brain; a correctly typed empty map replaces it.
        let cooldown = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.data_of(&DataRef::Entity(Target::Mob(warden_id)))
                .unwrap()
                .get("Brain")
                .and_then(|tag| tag.get("memories"))
                .and_then(|tag| tag.get("minecraft:sonic_boom_cooldown"))
                .and_then(|tag| tag.get("ttl"))
                .cloned()
        };
        assert_eq!(cooldown(game), Some(Tag::Long(200)));
        for edit in [
            format!("data remove entity {WARDEN} Brain"),
            format!("data modify entity {WARDEN} Brain set value 1"),
            format!("data modify entity {WARDEN} Brain.memories set value 1"),
        ] {
            assert!(game.run_command(&edit), "{edit}");
            assert_eq!(cooldown(game), Some(Tag::Long(200)), "{edit}");
            check_target(game, Some(second));
        }
        assert!(game.run_command(&format!(
            "data modify entity {WARDEN} Brain.memories set value {{}}"
        )));
        assert_eq!(cooldown(game), None);
        check_target(game, None);

        let pig = game.living(game.entity(pig_id).unwrap()).unwrap();
        reset_warden(game);
        assert!(hurt(game, 1.0, Some(Target::Mob(pig_id))));
        game.mobs
            .iter_mut()
            .find(|mob| mob.id == pig_id)
            .unwrap()
            .invulnerable = 20;
        check_target(game, Some(pig)); // Damage cooldown is not entity immunity.
        assert!(game.entity_set_invulnerable_time(pig.entity(), 5));
        check_target(game, None);
        assert!(game.entity_set_invulnerable_time(pig.entity(), 0));
        check_target(game, Some(pig));

        assert!(game.run_command("summon ender_dragon 8.5 64 8.5 {UUID:[I;84,0,0,7]}"));
        let (dragon_id, _) = game.dragon_target().unwrap();
        let dragon = game.living(game.entity(dragon_id).unwrap()).unwrap();
        reset_warden(game);
        assert!(game.run_command(&format!("damage {WARDEN} 1 minecraft:generic by {DRAGON}")));
        check_target(game, Some(dragon));
        assert!(game.entity_set_invulnerable_time(dragon.entity(), 10));
        check_target(game, None);
        assert!(game.entity_set_invulnerable_time(dragon.entity(), 0));
        check_target(game, Some(dragon));
        assert!(game.entity_set_invulnerable(dragon.entity(), true));
        check_target(game, None);
    });
}

pub(crate) fn setup_live_damage_world(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    game.build_registries();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 64.0, 14.5)).unwrap());
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
    game.set_mode_id(0);
    game.server.next_entity = 100;
}

pub(crate) fn spawn_test_projectile(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    kind: sunrise_sim::projectiles::ProjectileKind,
    at: sunrise_core::geometry::Vec3,
    owner: Option<Who>,
    uuid: [i32; 4],
) -> u32 {
    use sunrise_sim::projectiles::Projectile;
    game.server.next_entity += 1;
    let id = game.server.next_entity;
    let mut shot = Projectile::new(kind, at, owner);
    shot.id = id;
    shot.previous = at;
    game.creatures.projectiles.push(shot);
    game.persist.uuids.insert(id, uuid);
    id
}

use sunrise_gameplay_common::game::creatures::world_view::WorldViewExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;
