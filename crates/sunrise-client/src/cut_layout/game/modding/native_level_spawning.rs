#![cfg(test)]

#[cfg(test)]
mod tests {

    use super::super::native_entity_refs::tests::with_game;
    use mod_api::{
        EntityConstructingEvent, EntityJoinLevelEvent, EntityRef, EntitySpawn, FinalizeSpawnEvent,
        Flow, Holder, Host, HostSide, LevelRef, ServerHost, SpawnBody, SpawnFlags, SpawnLimits,
        WorldEpoch,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::spawn_selection::{Category, Counted, SpawnPlayer, SpawnState};
    use {
        crate::game::server_side::commands::peaceful_tests::with_types,
        sunrise_gameplay_common::game::server_side::commands::peaceful_tests::TRADER,
    };

    /// What a spawn told the mods, in order.
    type Heard = Arc<Mutex<Vec<String>>>;
    struct Listener(Heard);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn entity_constructing(&self, _: HostSide<'_>, event: &mut EntityConstructingEvent) {
            if let Holder::Entity(_) = event.holder {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("construct {}", event.kind));
            }
        }
        fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push(format!("finalize {} {:?}", event.kind, event.reason));
            Flow::Continue
        }
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(format!("join {}", event.kind));
            Flow::Continue
        }
    }

    /// A cow (no jockey or rider rolls) spawns through the entity type's own path: constructed,
    /// then finalized with the caller's reason, and it joins not persistent. Every refusal
    /// spawns nothing and tells no mod.
    #[test]
    #[ignore = "daily"]
    fn a_level_spawns_a_mob_as_its_entity_type_does_and_refuses_the_rest() {
        let heard = Heard::default();
        let mods = vec![mod_api::ModEntry::new(
            "level-spawning",
            Box::new(Listener(heard.clone())),
        )];
        with_types(ExecutionRole::IntegratedServer, |game| {
            mod_platform::testing::with_mods(mods.into(), || {
                let level = game.level(game.dim.id()).unwrap();
                let cow = game
                    .level_spawn_entity(level, "minecraft:cow", [8.5, 64.0, 8.25], &living())
                    .unwrap();
                assert_eq!(
                    game.entity_kind(cow)
                        .and_then(|kind| kind.id(game))
                        .as_deref(),
                    Some("minecraft:cow")
                );
                assert_eq!(game.entity_position(cow), Some([8.5, 64.0, 8.25]));
                let mob = game.mobs.iter().find(|m| m.id == cow.id()).unwrap();
                assert!(!mob.persistent);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        "construct minecraft:cow",
                        "finalize minecraft:cow Command",
                        "join minecraft:cow"
                    ]
                );

                let mobs = game.mobs.len();
                let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
                let at = [8.5, 64.0, 8.5];
                let refused: [(LevelRef, &str, [f64; 3]); 10] = [
                    (level, "minecraft:cow", [f64::NAN, 64.0, 8.5]),
                    (level, "minecraft:cow", [8.5, f64::INFINITY, 8.5]),
                    // Past i32: `as` saturates, and the bounds refuse it.
                    (level, "minecraft:cow", [1.0e300, 64.0, 8.5]),
                    (level, "minecraft:cow", [8.5, 64.0, f64::from(i32::MIN)]),
                    (level, "minecraft:cow", [f64::from(i32::MAX), 64.0, 8.5]),
                    // Level.isInSpawnableBounds: y below 20,000,000.
                    (level, "minecraft:cow", [8.5, 20_000_000.0, 8.5]),
                    (level, "minecraft:cow", [1000.5, 64.0, 8.5]),
                    (level, "minecraft:stone", at),
                    (level, "minecraft:snowball", at),
                    (stale, "minecraft:cow", at),
                ];
                for (level, kind, pos) in refused {
                    assert_eq!(
                        game.level_spawn_entity(level, kind, pos, &living()),
                        None,
                        "{kind} at {pos:?}"
                    );
                }
                // A spawn that isn't finite, a mob with an owner, and a body
                // of another family refuse as well.
                let mut moving = living();
                moving.velocity = [0.0, f64::NAN, 0.0];
                let mut turned = living();
                turned.rotation = Some([f32::INFINITY, 0.0]);
                let mut owned = living();
                owned.owner = Some(cow);
                let general = EntitySpawn::new(SpawnBody::General);
                let thrown = EntitySpawn::new(SpawnBody::Thrown {
                    item: snowballs(game, 1),
                });
                for spawn in [moving, turned, owned, general, thrown] {
                    assert_eq!(
                        game.level_spawn_entity(level, "minecraft:cow", at, &spawn),
                        None,
                        "{spawn:?}"
                    );
                }
                let next_id = game.server.next_entity;
                assert_eq!(
                    game.level_spawn_mob(level, TRADER, at, mod_api::SpawnReason::Command),
                    None
                );
                assert_eq!(game.server.next_entity, next_id);
                assert_eq!(game.mobs.len(), mobs);
                assert_eq!(heard.lock().unwrap().len(), 3, "no refusal told a mod");
            });
        });
    }

    /// A mob's spawn as `/summon` makes it, at rest and with no owner.
    fn living() -> EntitySpawn {
        EntitySpawn::new(SpawnBody::Living {
            reason: mod_api::SpawnReason::Command,
        })
    }

    /// `count` snowballs as a mod's stack.
    fn snowballs(game: &sunrise_gameplay::game::ServerGame<'_>, count: u32) -> mod_api::ItemStack {
        let snowball = mod_api::Identifier::parse("minecraft:snowball").unwrap();
        mod_api::ItemStack::new(game.item_key(&snowball).unwrap(), count).unwrap()
    }

    /// A mob spawned moving and turned starts with that velocity and facing,
    /// set before it joins.
    #[test]
    #[ignore = "daily"]
    fn a_spawned_mob_starts_with_its_velocity_and_rotation() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let mut spawn = living();
            spawn.velocity = [0.25, 0.5, -0.125];
            spawn.rotation = Some([45.0, -10.0]);
            let cow = game
                .level_spawn_entity(level, "minecraft:cow", [8.5, 64.0, 8.5], &spawn)
                .unwrap();
            let mob = game.mobs.iter().find(|m| m.id == cow.id()).unwrap();
            assert_eq!(mob.velocity, Vec3::new(0.25, 0.5, -0.125));
            assert_eq!((mob.yaw, mob.head_pitch), (45.0, -10.0));
            // The kind resolves as `/summon`'s.
            let bare = game
                .level_spawn_entity(level, "cow", [9.5, 64.0, 8.5], &living())
                .unwrap();
            assert_eq!(
                game.entity_kind(bare)
                    .and_then(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:cow")
            );
            // The ender dragon takes its own creation, facing the given yaw;
            // a velocity or a pitch, which it has no state for, refuses it.
            let mut rising = living();
            rising.velocity = [0.0, 0.5, 0.0];
            let mut pitched = living();
            pitched.rotation = Some([30.0, 10.0]);
            for refused in [rising, pitched] {
                let at = [8.5, 80.0, 8.5];
                assert_eq!(
                    game.level_spawn_entity(level, "minecraft:ender_dragon", at, &refused),
                    None
                );
            }
            assert!(game.dragon.is_none());
            let mut facing = living();
            facing.rotation = Some([30.0, 0.0]);
            let dragon = game
                .level_spawn_entity(level, "ender_dragon", [8.5, 80.0, 8.5], &facing)
                .unwrap();
            let d = game.dragon.as_ref().unwrap();
            assert_eq!((d.id, d.yaw), (dragon.id(), 30.0));
        });
    }

    /// A thrown projectile spawned with a velocity and an owner faces along
    /// its velocity, carries one of its item, keeps clear of its owner until
    /// it leaves the owner's box, and reads its owner back.
    #[test]
    #[ignore = "daily"]
    fn a_thrown_spawn_takes_its_velocity_item_and_owner() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let cow = game
                .level_spawn_entity(level, "minecraft:cow", [8.5, 64.0, 8.5], &living())
                .unwrap();
            let mut spawn = EntitySpawn::new(SpawnBody::Thrown {
                item: snowballs(game, 16),
            });
            spawn.velocity = [1.0, 0.0, 1.0];
            spawn.owner = Some(cow);
            let shot = game
                .level_spawn_entity(level, "minecraft:snowball", [8.5, 65.5, 8.5], &spawn)
                .unwrap();
            assert_eq!(game.entity_owner(shot), Some(cow));
            let p = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == shot.id())
                .unwrap();
            assert_eq!(p.velocity, Vec3::new(1.0, 0.0, 1.0));
            assert_eq!(p.position, Vec3::new(8.5, 65.5, 8.5));
            // Projectile.shoot: atan2(x, z) and atan2(y, horizontal), in degrees.
            assert!((p.yaw - 45.0).abs() < 1.0e-4 && p.pitch.abs() < 1.0e-6);
            assert_eq!(p.item.as_ref().map(|item| item.count), Some(1));
            assert_eq!(p.owner, Some(sunrise_sim::mobs::Who::Mob(cow.id())));
            assert!(!p.left_owner);
            // A type that isn't thrown, and an owner in no level, refuse.
            assert_eq!(
                game.level_spawn_entity(level, "minecraft:arrow", [8.5, 65.5, 8.5], &spawn),
                None
            );
            spawn.owner = Some(EntityRef::new(WorldEpoch::new(0), cow.id()));
            assert_eq!(
                game.level_spawn_entity(level, "minecraft:snowball", [8.5, 65.5, 8.5], &spawn),
                None
            );
            // An owner the shot has no collision identity for refuses.
            game.options.dedicated = Some(Vec::new());
            let stand = game
                .level_summon(
                    level,
                    "minecraft:armor_stand",
                    [9.5, 64.0, 8.5],
                    Default::default(),
                )
                .unwrap()
                .unwrap();
            assert_eq!(game.entity(stand.id()), Some(stand));
            spawn.owner = Some(stand);
            let projectiles = game.creatures.projectiles.len();
            assert_eq!(
                game.level_spawn_entity(level, "minecraft:snowball", [8.5, 65.5, 8.5], &spawn),
                None
            );
            assert_eq!(game.creatures.projectiles.len(), projectiles);
        });
    }

    /// One player in chunk (0, 0).
    fn player() -> Vec<SpawnPlayer> {
        vec![SpawnPlayer {
            position: [8.0, 64.0, 8.0],
            spectator: false,
        }]
    }

    /// The state and categories the level's next spawn pass reads over vanilla's 289
    /// spawnable chunks, with one loose zombie counted in chunk (0, 0).
    fn next_pass(game: &sunrise_gameplay::game::ServerGame<'_>) -> (SpawnState, Vec<Category>) {
        let zombie = [Counted {
            entity: "minecraft:zombie",
            category: Category::Monster,
            block: [8, 64, 8],
            is_mob: true,
            persistent: false,
        }];
        let level = game.natural_level();
        let state = game.natural_spawn_state(&level, 289, player(), Default::default(), &zombie);
        let categories = game.natural_categories(&state);
        (state, categories)
    }

    /// Without a change the pass is vanilla's: every spawning category under the caps. A
    /// monster limit of 1 is filled by the one zombie counted (1 * 289 / 289), so monsters
    /// drop out; 2 lets them back. The flags take categories away, and the limits hold
    /// beside them. A stale handle and a client read nothing and set nothing.
    #[test]
    #[ignore = "daily"]
    fn a_levels_spawn_policy_changes_what_its_next_pass_spawns() {
        use Category as C;
        let friendly = [
            C::Creature,
            C::Ambient,
            C::Axolotls,
            C::UndergroundWaterCreature,
            C::WaterCreature,
            C::WaterAmbient,
        ];
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // The persistent categories' pass, under vanilla's default rules.
            game.world.game_time = 400;
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(game.level_spawn_limits(level), Some(SpawnLimits::VANILLA));
            let both = SpawnFlags {
                monsters: true,
                friendlies: true,
            };
            assert_eq!(game.level_spawn_flags(level), Some(both));
            let (mut state, categories) = next_pass(game);
            let mut all = vec![C::Monster];
            all.extend(friendly);
            assert_eq!(categories, all);
            assert!(state.can_spawn_local(C::Monster, 0, 0));

            let one = SpawnLimits {
                monster: 1,
                ..SpawnLimits::VANILLA
            };
            assert!(game.level_set_spawn_limits(level, one));
            assert_eq!(game.level_spawn_limits(level), Some(one));
            let (mut state, categories) = next_pass(game);
            assert_eq!(categories, friendly);
            assert!(!state.can_spawn_local(C::Monster, 0, 0));
            let two = SpawnLimits {
                monster: 2,
                ..SpawnLimits::VANILLA
            };
            assert!(game.level_set_spawn_limits(level, two));
            assert_eq!(next_pass(game).1, all);

            let monsters_only = SpawnFlags {
                monsters: true,
                friendlies: false,
            };
            assert!(game.level_set_spawn_flags(level, monsters_only));
            assert_eq!(game.level_spawn_flags(level), Some(monsters_only));
            assert_eq!(next_pass(game).1, [C::Monster]);
            assert_eq!(
                game.level_spawn_limits(level),
                Some(two),
                "flags leave limits"
            );

            let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
            assert_eq!(game.level_spawn_limits(stale), None);
            assert_eq!(game.level_spawn_flags(stale), None);
            assert!(!game.level_set_spawn_limits(stale, SpawnLimits::VANILLA));
            assert!(!game.level_set_spawn_flags(stale, both));
            assert_eq!(game.level_can_spawn_entities_in_chunk(stale, [0, 0]), None);
            assert_eq!(game.level_spawn_limits(level), Some(two));

            // canSpawnEntitiesInChunk: the loaded chunk, not its unloaded neighbour, and the
            // ends of i32 read as any other chunk.
            assert_eq!(
                game.level_can_spawn_entities_in_chunk(level, [0, 0]),
                Some(true)
            );
            assert_eq!(
                game.level_can_spawn_entities_in_chunk(level, [1, 0]),
                Some(false)
            );
            assert_eq!(
                game.level_can_spawn_entities_in_chunk(level, [i32::MIN, i32::MAX]),
                Some(false)
            );
        });
        // Client refusal is enforced at compile time (`ClientGame` lacks server spawn-limit ops).
    }
}
