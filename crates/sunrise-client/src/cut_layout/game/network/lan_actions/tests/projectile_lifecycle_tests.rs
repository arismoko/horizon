//! Production residency regressions for projectile ownership.
#![cfg(feature = "client")]
#![cfg(test)]
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::SideGame;
use sunrise_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use sunrise_sim::{
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};

fn arrow(id: u32, at: [f64; 3]) -> Projectile {
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::new(at[0], at[1], at[2]),
        None,
    );
    p.id = id;
    p
}

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "projectile_restore",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

fn exercise_restore<'g, G: SideGame<'g>>(
    game: &mut G,
    path: usize,
    step: impl FnOnce(&mut G, [f64; 3]),
) {
    let at = [3.0, 64.0, 5.0];
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
        .unwrap();
    game.creatures.projectiles.push(arrow(71, at));
    game.creatures
        .projectiles
        .update(0, |p| p.velocity = Vec3::new(0.3, 0.0, 0.0));
    let index = Arc::new(Mutex::new(AttachmentIndex::default()));
    let mut values = Attachments::default();
    VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
    let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
    let original = game.creatures.projectiles.update(0, |p| {
        p.attachment_lifetime().bind(&mut values, &index, holder);
        p.attachment_lifetime().registration().unwrap()
    });
    let key = ("test".into(), "projectile_restore".into());
    let before = index.lock().unwrap().snapshot(&key);
    assert_eq!(before.len(), 1);
    step(game, at);
    let current = game
        .creatures
        .projectiles
        .update(0, |p| p.attachment_lifetime().registration().unwrap());
    assert!(original.same_registration(&current), "restore path {path}");
    assert_eq!(
        index.lock().unwrap().snapshot(&key).len(),
        1,
        "restore path {path}"
    );
    game.creatures.projectiles.clear();
    assert!(index.lock().unwrap().snapshot(&key).is_empty());
    assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
}

#[test]
#[ignore = "daily"]
fn projectile_restore_entries_keep_the_bound_arrow() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        exercise_restore(game, 0, |game, at| {
            game.tick_remote_projectiles(|_, at| at);
            assert!(game.creatures.projectiles[0].position.x > at[0]);
        });
    });
    for path in 1..3 {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            exercise_restore(game, path, |game, at| {
                // One takeable unbound arrow exercises the pickup branch while
                // the bound, disallowed arrow must survive the same retain.
                let mut picked = arrow(72, at);
                let state = picked.arrow.as_mut().unwrap();
                state.in_ground = true;
                state.pickup = sunrise_sim::entities::arrows::Pickup::Allowed;
                for id in [73, 74] {
                    game.creatures.projectiles.push(arrow(id, at));
                }
                game.creatures.projectiles.push(picked);
                if path == 1 {
                    sunrise_gameplay::game::entity_server::taken_pass_tests::assert_projectile_spawn_survives(
                        game,
                        |game| {
                            game.guest_arrows(0);
                        },
                    );
                } else {
                    game.player = Some(
                        PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2])).unwrap(),
                    );
                    sunrise_gameplay::game::entity_server::taken_pass_tests::assert_projectile_spawn_survives(
                        game,
                        |game| {
                            game.touch_arrows();
                        },
                    );
                }
                assert_eq!(
                    game.creatures
                        .projectiles
                        .iter()
                        .map(|p| p.id)
                        .collect::<Vec<_>>(),
                    [71, 73, 74]
                );
            });
        });
    }
}
