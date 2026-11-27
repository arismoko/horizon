#![cfg(test)]
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod attachment_membership_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::{join_seq::Joined, mobs::Mob};
    use {
        crate::game::mod_api_host::*, sunrise_gameplay_common::game::mod_api_host::AttachmentAccess,
    };
    use {
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    fn old_membership(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> bool {
        use {
            sunrise_gameplay_common::game::entity::Entity as _,
            sunrise_gameplay_common::game::entity::entity_lists_of,
            sunrise_gameplay_common::game::entity::for_each_entity,
        };
        let mut found = false;
        for_each_entity!(entity_lists_of!(game), entity => {
            found |= entity.id() == id && !entity.is_removed();
        });
        found || game.entity_ids_off_walk().any(|e| e == id) || game.level_dragon_part(id).is_some()
    }
    fn assert_membership(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32) {
        assert_eq!(
            game.attachment_entity_exists(id),
            old_membership(game, id),
            "id {id}"
        );
    }
    #[test]
    #[ignore = "daily"]
    fn attachment_off_walk_membership_matches_old_walk() {
        with_game(|game| {
            game.reset_entity_residency();
            macro_rules! summon {
                ($name:literal, $($field:tt).+) => {{
                    game.summon_with_nbt(concat!("minecraft:", $name), [0.5, 70.0, 0.5], "{}").unwrap();
                    game.lan_assign_ids();
                    let id = game$(.$field)+.last().unwrap().id;
                    assert_membership(game, id);
                }};
            }
            summon!("pig", mobs);
            summon!("item_frame", item_frames);
            summon!("painting", paintings);
            for name in ["marker", "leash_knot"] {
                game.summon_with_nbt(&format!("minecraft:{name}"), [0.5, 70.0, 0.5], "{}")
                    .unwrap();
                game.lan_assign_ids();
                let id = game.test_last_other_id();
                assert_membership(game, id);
            }
            summon!("area_effect_cloud", breath_clouds);
            summon!("end_crystal", crystals);
            summon!("cushion", cushions);
            game.add_fangs([0.5, 70.0, 0.5], 0.0, 0, 0);
            let id = game.fangs.list[0].id;
            assert_membership(game, id);
            game.add_ominous_item_spawner(
                sunrise_items::items::ItemStack::new("minecraft:stone", 1),
                [0.5, 70.0, 0.5],
                60,
            );
            let id = game.ominous_spawners[0].id;
            assert_membership(game, id);
            let fence = [0, 70, 0];
            game.tie_knot(fence);
            let mob = game.mobs[0].id;
            assert!(game.set_leash(
                Target::Mob(mob),
                sunrise_sim::mobs::leash::Holder::Knot(fence),
                None
            ));
            let id = game.resident_entities.knots[0].id;
            assert_membership(game, id);
            // Also cover a player-tied natural knot, besides the explicitly summoned knot.
            let natural_fence = [3, 70, 3];
            assert!(game.set_leash(
                Target::Mob(mob),
                sunrise_sim::mobs::leash::Holder::Player,
                None
            ));
            assert!(game.use_knot(natural_fence, false));
            let id = game.server.knot_ids[&natural_fence];
            assert_membership(game, id);
            let mut player =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 880;
            game.player = Some(player);
            game.bind_local_player_residency();
            let player_id = game.player.as_ref().unwrap().entity_id as u32;
            assert_membership(game, player_id);
            assert!(!old_membership(game, player_id));
            game.install_fishing_hook(sunrise_gameplay_common::game::fishing_common::Hook::remote(
                900,
                Vec3::ZERO,
                [1, 2, 3, 4],
            ));
            assert_membership(game, 900);
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(900), AttachmentAccess::Existing)
                    .is_ok()
            );
            game.persist.removals.push(Target::Hook(900));
            assert_membership(game, 900);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn attachment_guest_hook_membership_matches_old_walk() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            for id in [12, 13] {
                assert_membership(game, id);
                assert!(
                    !old_membership(game, id),
                    "players are not entity attachment holders"
                );
            }
            game.as_guest(12, |game| {
                game.install_fishing_hook(
                    sunrise_gameplay_common::game::fishing_common::Hook::remote(
                        901,
                        Vec3::ZERO,
                        [5, 6, 7, 8],
                    ),
                );
                assert_membership(game, 901);
                assert!(
                    game.attachment_slot(mod_api::Holder::Entity(901), AttachmentAccess::Existing)
                        .is_ok()
                );
            })
            .unwrap();
            assert_membership(game, 901);
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(901), AttachmentAccess::Existing)
                    .is_ok()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn attachment_membership_matches_old_walk() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind, Vec3::ZERO, 0.0));
            let check = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                for id in [0, 410, 999] {
                    assert_eq!(
                        game.attachment_entity_exists(id),
                        old_membership(game, id),
                        "id {id}"
                    );
                }
            };
            check(game);
            game.paintings.push(sunrise_sim::entities::Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 420,
                pos: [0, 64, 0],
                facing: 2,
                variant: "minecraft:kebab".into(),
                asset: None,
                width: 1,
                height: 1,
                check_interval: 0,
                ticks: 0,
            });
            game.persist.removals.push(Target::Painting(420));
            assert_eq!(
                game.attachment_entity_exists(420),
                old_membership(game, 420)
            );
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(420), AttachmentAccess::Existing)
                    .is_ok()
            );
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(420), AttachmentAccess::Store)
                    .is_err()
            );
            let join = game.mobs[0].join_seq();
            let taken = game.mobs.extract(join).unwrap();
            assert!(matches!(
                game.attachment_slot(mod_api::Holder::Entity(410), AttachmentAccess::Existing),
                Err(mod_api::AttachmentError::HolderBusy)
            ));
            game.mobs.put_back(0, taken);
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 500;
            game.dragon.replace(dragon);
            for id in 500..=508 {
                assert_eq!(game.attachment_entity_exists(id), old_membership(game, id));
            }
            game.persist.removals.push(Target::Dragon(500));
            for id in 500..=508 {
                assert_eq!(game.attachment_entity_exists(id), old_membership(game, id));
            }
            game.persist.removals.push(Target::Mob(410));
            check(game);
            game.persist.destroying_vehicles.insert(Target::Mob(410));
            check(game);
            game.mobs.update(0, |mob| mob.removed = true);
            check(game);
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            check(game);
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            let holder = mod_api::Holder::Entity(999);
            game.entity_initialization.active = Some(
                sunrise_gameplay_common::game::entity_initialization_common::Construction::Holder(
                    holder,
                ),
            );
            assert!(
                game.attachment_slot(holder, AttachmentAccess::Existing)
                    .is_ok()
            );
            game.entity_initialization.active = None;
            assert!(
                game.attachment_slot(holder, AttachmentAccess::Existing)
                    .is_err()
            );
        });
    }
    #[test]
    #[ignore = "manual attachment scaling benchmark; needs SUNRISE_TEST_RESOURCES"]
    fn attachment_entity_read_bench() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for population in [300, 3000, 10000] {
                while game.mobs.len() < population {
                    let id = game.mobs.len() as u32 + 100;
                    game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
                }
                let holder = mod_api::Holder::Entity(100);
                game.attachment_slot(holder, AttachmentAccess::Existing)
                    .unwrap();
                let start = std::time::Instant::now();
                for _ in 0..100_000 {
                    std::hint::black_box(
                        game.attachment_slot(holder, AttachmentAccess::Existing)
                            .unwrap(),
                    );
                }
                println!(
                    "attachment entity population={population} ns/read={:.2}",
                    start.elapsed().as_nanos() as f64 / 100_000.0
                );
            }
        });
    }
}
