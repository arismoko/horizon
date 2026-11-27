#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_core::geometry::Aabb;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
pub(crate) mod pass_tests {
    #[cfg(feature = "client")]
    use super::*;
    #[cfg(feature = "client")]
    use sunrise_client::mobs::Mob;
    #[cfg(feature = "client")]
    use sunrise_core::random::RandomSource;
    #[cfg(feature = "client")]
    use sunrise_core::{geometry::Vec3, random::LegacyRandom};
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::potent_sulfur_common::PassMobs;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::potent_sulfur::nearby_living_area;

    /// The mobs whose box meets `area`, by walking them all.
    #[cfg(feature = "client")]
    fn scan(game: &sunrise_gameplay::game::ServerGame<'_>, area: Aabb) -> Vec<u32> {
        (0..game.mobs.len() as u32)
            .filter(|&j| game.mobs[j as usize].bounds().intersects(area))
            .collect()
    }
    /// The same from the pass's grid.
    #[cfg(feature = "client")]
    fn from_grid(game: &mut sunrise_gameplay::game::ServerGame<'_>, area: Aabb) -> Vec<u32> {
        let mut near = Vec::new();
        assert!(
            game.mobs_in_pass(area, &mut near),
            "a small area reads the grid"
        );
        near.retain(|&j| game.mobs[j as usize].bounds().intersects(area));
        near
    }

    /// A nausea listener can change another mob's spatial membership. The next
    /// geyser in the same pass must see that write, not bless the old grid.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn nausea_listener_moving_a_mob_invalidates_the_pass_grid() {
        use mod_api::{Hooks, MobEffectApplicableEvent, ModEntry, ServerHost};

        struct MoveMob(mod_api::EntityRef);
        #[mod_api::__subscriptions]
        impl Hooks for MoveMob {
            fn mob_effect_applicable(
                &self,
                host: &mut dyn ServerHost,
                event: &mut MobEffectApplicableEvent,
            ) -> Option<mod_api::EffectApplicable> {
                if event.entity.entity().id() == 1 {
                    assert!(self.0.teleport_to(host, [33.5, 64.0, 1.5], None));
                }
                None
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            for x in [0, 2, 4] {
                game.world.chunks.insert(
                    (x, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(x, 0)),
                );
            }
            let water = game
                .pack
                .assets
                .registry
                .default_state("minecraft:water")
                .unwrap();
            // Water under each zombie's eyes; the source is the air at y=65.
            for x in [1, 33] {
                game.world.set(x, 64, 1, water).unwrap();
            }
            game.world.game_time = 10;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(1, kind.clone(), Vec3::new(1.5, 64.0, 1.5), 0.0));
            game.mobs
                .push(Mob::new(2, kind.clone(), Vec3::new(65.5, 64.0, 1.5), 0.0));
            // The six-block-wide nausea query, expanded by the zombie's width
            // plus SeenGrid's margin, spans 3 x 3 four-block grid columns.
            // Populate distinct distant columns so candidates uses the grid
            // rather than its small-population scan fallback. Keep mob 2 in slot 1.
            for i in 0..64u32 {
                game.mobs.push(Mob::new(
                    i + 3,
                    kind.clone(),
                    Vec3::new(256.5 + f64::from(i) * 8.0, 64.0, 256.5),
                    0.0,
                ));
            }
            let moved = game.entity(2).unwrap();
            let mods: mod_api::Mods =
                vec![ModEntry::new("move-on-nausea", Box::new(MoveMob(moved)))].into();
            mod_platform::testing::with_mods(mods, || {
                game.server.pass_mobs = Some(PassMobs::default());
                game.server_nausea_effect_ticker((1, 63, 1));
                assert_eq!(game.mobs[1].position, Vec3::new(33.5, 64.0, 1.5));
                assert!(game.mobs[1].extra.effects.is_empty());
                let second_area = nearby_living_area([33, 65, 1]);
                assert_eq!(
                    from_grid(game, second_area),
                    vec![1],
                    "the second geyser finds the moved mob"
                );
                game.server_nausea_effect_ticker((33, 63, 1));
                assert!(
                    !game.mobs[1].extra.effects.is_empty(),
                    "the second geyser applies nausea"
                );
            });
        });
    }

    /// The grid of a block entity pass finds every mob a scan finds, in the
    /// same order, however the mob list changed between two geysers.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn the_pass_grid_finds_what_a_scan_finds() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let kinds = ["minecraft:zombie", "minecraft:chicken", "minecraft:ravager"];
            let kinds: Vec<_> = kinds
                .iter()
                .map(|k| game.res.mob_types().get(*k).cloned().unwrap())
                .collect();
            let mut rng = LegacyRandom::new(8645);
            let place = |rng: &mut LegacyRandom| {
                let mut at = || f64::from(rng.next_int_bound(4000).unwrap()) / 100.0 - 20.0;
                Vec3::new(at(), 64.0 + at() / 8.0, at())
            };
            for id in 0..60u32 {
                let kind = kinds[id as usize % kinds.len()].clone();
                game.mobs.push(Mob::new(id + 1, kind, place(&mut rng), 0.0));
            }
            let areas: Vec<Aabb> = (0..40)
                .map(|i| {
                    let c = place(&mut rng);
                    // Geyser columns one block wide, and wider gas boxes.
                    let (w, h) = if i % 2 == 0 { (1.0, 6.0) } else { (5.0, 9.0) };
                    Aabb::new(c, Vec3::new(c.x + w, c.y + h, c.z + w))
                })
                .collect();
            game.server.pass_mobs = Some(PassMobs::default());
            let check = |game: &mut sunrise_gameplay::game::ServerGame<'_>, what: &str| {
                for &area in &areas {
                    assert_eq!(from_grid(game, area), scan(game, area), "{what}");
                }
            };
            check(game, "as built");
            game.mobs[7].position = areas[0].center();
            check(game, "a mob moved into a geyser");
            let kind = kinds[2].clone();
            game.mobs
                .insert(0, Mob::new(100, kind, areas[1].center(), 0.0));
            check(game, "a mob added in front");
            game.mobs.retain(|m| m.id % 5 != 0);
            check(game, "some removed");
            // A geyser's own write leaves the boxes as they were.
            game.mobs[3].velocity.y += 0.2;
            game.server
                .pass_mobs
                .as_mut()
                .unwrap()
                .wrote(&game.common.mobs);
            check(game, "a push");
        });
    }
}
