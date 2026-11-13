//! An entity's `Entity.noPhysics` for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level
//! handles"): whether its moves pass through blocks. Offered only for the
//! kinds whose flag has one value between ticks; an item recomputes its own
//! inside its tick, so it reads None, as do the other kinds, a stale handle
//! and a client. The write is refused for the three kinds whose own tick
//! rewrites the flag.
use super::{Game, commands::Target};
use horizon_client::mobs::Species;
use horizon_client::projectiles::ProjectileKind;
use mod_api::EntityRef;

impl Game<'_> {
    /// `Entity.noPhysics`: a player's is whether it spectates (`Player.tick`
    /// sets it from `isSpectator`); a mob's is the flag a mod wrote, false
    /// until then (a vex sets it only for the length of its own tick); an
    /// arrow's or a trident's is its `AbstractArrow.noPhysics`, set once a
    /// loyal trident turns back. The ender dragon, an area-effect cloud, an
    /// ominous item spawner and a shulker bullet set it in their constructors
    /// and never clear it. An armor stand's is recomputed on load, so it keeps
    /// a written value only until then; a kind Horizon does not simulate keeps
    /// it in its saved compound.
    pub(super) fn entity_no_physics(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_target(entity)? {
            Target::Player => Some(self.player.as_ref()?.spectator),
            Target::Guest(conn) => Some(self.command_guest(conn)?.motion.as_deref()?.spectator),
            Target::Mob(id) => Some(self.mobs.iter().find(|m| m.id == id)?.no_physics),
            Target::Dragon(_) | Target::Cloud(_) | Target::Spawner(_) => Some(true),
            Target::Projectile(id) => {
                let p = self.creatures.projectiles.iter().find(|p| p.id == id)?;
                match (&p.kind, &p.arrow) {
                    (ProjectileKind::ShulkerBullet { .. }, _) => Some(true),
                    (_, Some(arrow)) => Some(arrow.no_physics),
                    _ => None,
                }
            }
            Target::Trident(id) => Some(self.tridents.iter().find(|t| t.id == id)?.no_physics),
            Target::Stand(id) => Some(self.armor_stands.iter().find(|s| s.id == id)?.no_physics),
            Target::Other(id) => Some(
                self.persist
                    .raw
                    .get(&id)
                    .is_some_and(|tag| horizon_client::entity_nbt::flag(tag, "NoPhysics")),
            ),
            _ => None,
        }
    }

    /// `Entity.noPhysics` for a native server mod: the value
    /// `entity_no_physics` reads. Server-only; a player, an item entity and a
    /// vex rewrite the flag every tick (`Player.tick`, `ItemEntity.tick`,
    /// `Vex.tick`), so they are refused by kind, and so is a client's write.
    /// An arrow's or a trident's write carries the synced flag clients read.
    /// An armor stand recomputes the flag on load, so its write is not
    /// persisted; an unsimulated kind keeps it in its saved compound.
    pub(super) fn entity_set_no_physics(&mut self, entity: EntityRef, no_physics: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => {
                let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                if mob.species() == Species::Vex {
                    return false;
                }
                mob.no_physics = no_physics;
                true
            }
            Some(Target::Projectile(id)) => {
                let Some(index) = self.creatures.projectiles.iter().position(|p| p.id == id) else {
                    return false;
                };
                self.creatures
                    .projectiles
                    .update(index, |p| p.set_no_physics(no_physics))
            }
            Some(Target::Trident(id)) => self
                .tridents
                .iter_mut()
                .find(|t| t.id == id)
                .is_some_and(|t| {
                    t.set_no_physics(no_physics);
                    true
                }),
            Some(Target::Stand(id)) => self
                .armor_stands
                .iter_mut()
                .find(|s| s.id == id)
                .is_some_and(|s| {
                    s.no_physics = no_physics;
                    true
                }),
            Some(Target::Other(id)) => {
                if !self.others.iter().any(|o| o.id == id) {
                    return false;
                }
                use horizon_core::nbt::Tag;
                let mut compound = match self.persist.raw.get(&id) {
                    Some(Tag::Compound(map)) => map.clone(),
                    _ => std::collections::BTreeMap::new(),
                };
                if no_physics {
                    compound.insert("NoPhysics".into(), Tag::Byte(1));
                } else {
                    compound.remove(&horizon_core::nbt::JavaString::from("NoPhysics"));
                }
                self.persist.raw.insert(id, Tag::Compound(compound));
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn no_physics_reads_supported_entities_and_refuses_others() {
        use super::super::native_entity_refs::tests::with_game;
        use super::super::trident::ThrownTrident;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_client::projectiles::{Projectile, ProjectileKind};
        use horizon_core::geometry::Vec3;
        use mod_api::WorldEpoch;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            game.player.as_mut().unwrap().spectator = false;
            assert_eq!(player.no_physics(game), Some(false));
            game.player.as_mut().unwrap().spectator = true;
            assert_eq!(player.no_physics(game), Some(true));

            let kind = game.res.mob_types().get("minecraft:vex").unwrap().clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.no_physics(game), Some(false));

            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = game.take_entity_id();
            let arrow_id = arrow.id;
            game.creatures.projectiles.push(arrow);
            let arrow = game.entity(arrow_id).unwrap();
            assert_eq!(arrow.no_physics(game), Some(false));
            game.creatures
                .projectiles
                .update(0, |p| p.arrow.as_mut().unwrap().no_physics = true);
            assert_eq!(arrow.no_physics(game), Some(true));

            let trident_id = game.take_entity_id();
            game.tridents
                .push(ThrownTrident::remote(trident_id, Vec3::ZERO, 0.0, 0.0));
            let trident = game.entity(trident_id).unwrap();
            assert_eq!(trident.no_physics(game), Some(false));
            game.tridents[0].no_physics = true;
            assert_eq!(trident.no_physics(game), Some(true));

            let mut snowball = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
            snowball.id = game.take_entity_id();
            let snowball_id = snowball.id;
            game.creatures.projectiles.push(snowball);
            let snowball = game.entity(snowball_id).unwrap();
            assert_eq!(snowball.no_physics(game), None);

            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::ZERO,
                None,
            );
            bullet.id = game.take_entity_id();
            let bullet_id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(bullet_id).unwrap();
            assert_eq!(bullet.no_physics(game), Some(true));

            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = game.take_entity_id();
            let dragon_id = dragon.id;
            game.dragon.replace(dragon);
            let dragon = game.entity(dragon_id).unwrap();
            assert_eq!(dragon.no_physics(game), Some(true));

            game.boats.push(super::super::boats::Boat::new(
                44,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(44).unwrap();
            assert_eq!(boat.no_physics(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.no_physics(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn no_physics_write_holds_for_a_marker_and_refuses_the_three_kinds() {
        use super::super::native_entity_refs::tests::with_game;
        use super::super::trident::ThrownTrident;
        use crate::game::commands::OtherEntity;
        use horizon_client::entities::ArmorStand;
        use horizon_client::items::ItemStack;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_client::projectiles::Projectile;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.others.push(OtherEntity {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 14,
                kind: "minecraft:marker".into(),
                position: Vec3::new(0.5, 64.0, 0.5),
                yaw: 0.0,
                pitch: 0.0,
                item_transformation: None,
                interaction: None,
                knot_check: None,
                eye: None,
            });
            let marker = game.entity(14).unwrap();
            assert_eq!(marker.no_physics(game), Some(false));
            // The write lands, and holds: no tick of the marker's own rewrites it.
            assert!(marker.set_no_physics(game, true));
            assert_eq!(marker.no_physics(game), Some(true));
            for _ in 0..20 {
                game.tick_mobs().unwrap();
            }
            assert_eq!(
                marker.no_physics(game),
                Some(true),
                "the write did not hold across ticks"
            );
            assert!(marker.set_no_physics(game, false));
            assert_eq!(marker.no_physics(game), Some(false));

            // A player rewrites the flag from its spectator state every tick.
            let player = game.entity(20).unwrap();
            assert!(!player.set_no_physics(game, true));
            assert_eq!(player.no_physics(game), Some(false));

            // An item entity recomputes it from the blocks it is in.
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.entity(game.dropped[0].id).unwrap();
            assert!(!item.set_no_physics(game, true));
            assert_eq!(item.no_physics(game), None);

            // A vex sets it inside its own tick.
            let vex = game.res.mob_types().get("minecraft:vex").unwrap().clone();
            game.mobs.push(Mob::new(42, vex, Vec3::new(0.5, 100.0, 0.5), 0.0));
            let vex = game.entity(42).unwrap();
            assert!(!vex.set_no_physics(game, true));
            assert_eq!(vex.no_physics(game), Some(false));

            // A mob that keeps its flag takes the write and holds it.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(43, zombie, Vec3::new(1.5, 100.0, 0.5), 0.0));
            let zombie = game.entity(43).unwrap();
            assert!(zombie.set_no_physics(game, true));
            game.tick_mobs().unwrap();
            assert_eq!(zombie.no_physics(game), Some(true));

            // An arrow and a trident write their synced flag with it.
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = game.take_entity_id();
            let arrow_id = arrow.id;
            game.creatures.projectiles.push(arrow);
            let arrow = game.entity(arrow_id).unwrap();
            assert!(arrow.set_no_physics(game, true));
            assert_eq!(arrow.no_physics(game), Some(true));
            assert!(game.creatures.projectiles[0].needs_sync);

            let trident_id = game.take_entity_id();
            game.tridents
                .push(ThrownTrident::remote(trident_id, Vec3::ZERO, 0.0, 0.0));
            let trident_id = game.tridents[0].id;
            let trident = game.entity(trident_id).unwrap();
            assert!(trident.set_no_physics(game, true));
            assert_eq!(trident.no_physics(game), Some(true));
            assert!(game.tridents[0].needs_sync);

            // An armor stand takes it, and recomputes it on a load.
            game.armor_stands
                .push(ArmorStand::new(44, Vec3::ZERO, 0.0));
            let stand = game.entity(44).unwrap();
            assert!(stand.set_no_physics(game, true));
            assert_eq!(stand.no_physics(game), Some(true));
            let saved = horizon_client::entity_nbt::armor_stand_to_nbt(
                &game.armor_stands[0],
                [1, 2, 3, 4],
                None,
            );
            let loaded = horizon_client::entity_nbt::armor_stand_from_nbt(&saved, 44).unwrap();
            assert!(
                !loaded.no_physics,
                "a stand recomputes the flag on load"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn no_physics_write_is_refused_on_a_client() {
        use super::super::native_entity_refs::tests::with_client_game;
        use mod_api::WorldEpoch;

        with_client_game(|game| {
            let entity = EntityRef::new(WorldEpoch::new(0), 1);
            assert!(!entity.set_no_physics(game, true));
            assert!(!entity.set_no_physics(game, false));
        });
    }
}
