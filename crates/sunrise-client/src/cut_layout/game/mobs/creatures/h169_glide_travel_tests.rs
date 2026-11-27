use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::creatures::{Vec3, source_damage_type};
use sunrise_gameplay_common::game::tables::GameItems;
pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_role_game(
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        test,
    );
}
#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::h169_glide_travel_tests::with_role_game;

#[test]
#[ignore = "daily"]
fn h169_registered_eligible_husk_actually_travels_as_a_glider() {
    with_game(|game| {
        game.world.game_time = 0;
        for (n, no_ai, glide, x) in [(1, 0, 1, 4.5), (2, 1, 1, 8.5), (3, 0, 0, 12.5)] {
            assert!(game.run_command(&format!(
                "summon husk {x} 300 8.5 {{UUID:[I;169,0,0,{n}],NoAI:{no_ai}b,PersistenceRequired:1b,FallFlying:{glide}b,Rotation:[0f,0f],Motion:[0d,0d,0d],equipment:{{chest:{{id:\"minecraft:elytra\",count:1}}}}}}"
            )));
        }
        let indexes: Vec<_> = (1..=3)
            .map(|n| {
                let id = game
                    .persist
                    .uuids
                    .iter()
                    .find_map(|(id, u)| (*u == [169, 0, 0, n]).then_some(*id))
                    .unwrap();
                game.mobs.iter().position(|m| m.id == id).unwrap()
            })
            .collect();
        let main = indexes[0];
        let id = game.mobs[main].id;
        assert!(game.mobs[main].extra.fall_flying);
        assert!(!game.mobs[main].no_ai && !game.mobs[main].ground.on_ground);
        let stack = sunrise_sim::mobs::finalize::equipment_stack(
            &game.mobs[main],
            sunrise_core::equipment_slot::EquipmentSlot::Chest,
        )
        .unwrap();
        assert!(sunrise_sim::item_sync::can_glide_using(
            &stack,
            "chest",
            game.items()
        ));
        game.world.game_time += 1;
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(4, 8));
        game.tick_creature_entities().unwrap();
        assert_eq!(game.mobs[main].id, id);
        assert_eq!(game.uuid_for(id), [169, 0, 0, 1]);
        assert!(
            game.mobs[main].extra.fall_flying,
            "real modeled flag remains eligible after the tick"
        );
        assert_eq!(
            game.mobs[indexes[1]].position,
            Vec3::new(8.5, 300.0, 8.5),
            "NoAI is a stationary control"
        );
        assert!(!game.mobs[indexes[2]].extra.fall_flying);
        assert_eq!(game.world.game_time, 1);
        // Independent fresh Java first-tick values, not native helper output.
        assert_eq!(
            game.mobs[main].position,
            Vec3::new(4.5, 299.9823599996567, 8.501782000017165)
        );
        assert_eq!(
            game.mobs[main].velocity,
            Vec3::new(0.0, -0.017640000343322755, 0.0017820000171661376)
        );
    });
}

// Added source/unit control, not part of the frozen dry-air Java runtime pair.
#[test]
#[ignore = "daily"]
fn h169_added_actual_wall_collision_uses_mob_damage_source() {
    with_game(|game| {
        assert!(game.run_command("fill 6 298 7 6 303 10 stone"));
        assert!(game.run_command(r#"summon husk 4.5 300 8.5 {UUID:[I;169,0,0,4],PersistenceRequired:1b,FallFlying:1b,Rotation:[-90f,0f],Motion:[2d,0d,0d],equipment:{chest:{id:"minecraft:elytra",count:1}}}"#));
        let id = game
            .persist
            .uuids
            .iter()
            .find_map(|(id, u)| (*u == [169, 0, 0, 4]).then_some(*id))
            .unwrap();
        let index = game.mobs.iter().position(|m| m.id == id).unwrap();
        assert!(
            game.mobs[index].kind.armor > 0.0,
            "real Husk armor is the bypass control"
        );
        assert_eq!(game.mobs[index].health, 20.0);
        game.world.game_time += 1;
        game.update_chunk_tickets();
        game.tick_creature_entities().unwrap();
        let m = &game.mobs[index];
        assert!(m.ground.horizontal_collision, "actual wall collision");
        assert!(m.position.x > 4.5 && m.position.x < 6.0);
        assert_eq!(m.velocity.x, 0.0, "resolved collision, no damage knockback");
        assert_eq!(m.health, 3.0, "(2 - 0) * 10 - 3, bypassing the real armor");
        let damage = m.last_damage.unwrap().0;
        assert_eq!(damage.kind, sunrise_sim::mobs::DamageKind::FlyIntoWall);
        assert!(damage.bypasses_armor() && damage.no_knockback());
        assert!(!damage.is_fire() && !damage.is_projectile());
        assert_eq!(source_damage_type(damage), "minecraft:fly_into_wall");
        assert_eq!(game.uuid_for(id), [169, 0, 0, 4]);
    });
}
