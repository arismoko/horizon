//! Entity base state round trips through the server handle and vanilla save tags.
#![cfg(test)]
use crate::game::{ServerGame, native_entity_refs::tests::with_game};
use mod_api::EntityRef;
use std::sync::Arc;
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_gameplay_common::game::{
    boats_common::Boat,
    commands_common::{OtherEntity, Target},
    fishing_common::Hook,
    minecarts::{CartKind, Minecart},
    native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
};
use sunrise_level::chunk::Chunk;
use sunrise_sim::projectiles::{Projectile, ProjectileKind};

fn open_chunk(game: &mut ServerGame<'_>) {
    game.common
        .world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
}

/// Writes every base value through `entity`, and checks the reads answer them.
fn write_and_read_back(game: &mut ServerGame<'_>, entity: EntityRef) {
    assert_eq!(entity.fire_ticks(game), Some(0));
    assert!(entity.set_remaining_fire_ticks(game, 60));
    assert!(entity.set_no_gravity(game, true));
    assert!(entity.set_silent(game, true));
    assert!(entity.set_glowing(game, true));
    assert!(entity.set_invulnerable(game, true));
    assert_eq!(entity.fire_ticks(game), Some(60));
    assert_eq!(entity.is_on_fire(game), Some(true));
    assert_eq!(entity.no_gravity(game), Some(true));
    assert_eq!(entity.is_silent(game), Some(true));
    assert_eq!(entity.glowing(game), Some(true));
    assert_eq!(entity.permanently_invulnerable(game), Some(true));
    assert!(entity.set_no_gravity(game, false));
    assert_eq!(entity.no_gravity(game), Some(false));
    assert!(entity.clear_fire(game));
    assert_eq!(entity.is_on_fire(game), Some(false));
}

#[test]
#[ignore = "daily"]
fn a_boat_keeps_its_base_state() {
    with_game(|game| {
        open_chunk(game);
        game.common
            .boats
            .push(Boat::new(31, "oak".into(), Vec3::new(8.5, 65.0, 8.5), 0.0));
        let entity = game.entity(31).unwrap();
        write_and_read_back(game, entity);
        let raw = game.common.persist.raw.get(&31).unwrap();
        assert_eq!(raw.get("Silent"), Some(&Tag::Byte(1)));
        assert_eq!(raw.get("Glowing"), Some(&Tag::Byte(1)));
    });
}

#[test]
#[ignore = "daily"]
fn a_minecart_keeps_its_base_state() {
    with_game(|game| {
        open_chunk(game);
        game.common.carts.push(Minecart::new(
            32,
            CartKind::Rideable,
            Vec3::new(8.5, 65.0, 8.5),
        ));
        let entity = game.entity(32).unwrap();
        write_and_read_back(game, entity);
        let raw = game.common.persist.raw.get(&32).unwrap();
        assert_eq!(raw.get("Glowing"), Some(&Tag::Byte(1)));
        assert_eq!(raw.get("Invulnerable"), Some(&Tag::Byte(1)));
    });
}

#[test]
#[ignore = "daily"]
fn a_projectile_keeps_its_base_state() {
    with_game(|game| {
        open_chunk(game);
        let mut shot = Projectile::new(ProjectileKind::Trident, Vec3::new(8.5, 65.0, 8.5), None);
        shot.id = game.take_entity_id();
        let id = shot.id;
        game.common.creatures.projectiles.push(shot);
        let entity = game.entity(id).unwrap();
        write_and_read_back(game, entity);
        assert_eq!(entity.fire_immune(game), Some(false));
    });
}

#[test]
#[ignore = "daily"]
fn a_marker_keeps_its_base_state_in_its_saved_tags() {
    with_game(|game| {
        open_chunk(game);
        game.server.others.0.push(OtherEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            knot_check: None,
            eye: None,
            join: Default::default(),
            id: 33,
            kind: "minecraft:marker".into(),
            position: Vec3::new(8.5, 65.0, 8.5),
            yaw: 0.0,
            pitch: 0.0,
            display: None,
            interaction: None,
        });
        let entity = game.entity(33).unwrap();
        write_and_read_back(game, entity);
        let raw = game.common.persist.raw.get(&33).unwrap();
        assert_eq!(raw.get("Silent"), Some(&Tag::Byte(1)));
        assert_eq!(raw.get("NoGravity"), None);
    });
}

#[test]
#[ignore = "daily"]
fn a_fishing_hook_keeps_its_base_state_in_memory() {
    with_game(|game| {
        open_chunk(game);
        game.common.options.dedicated = Some(Vec::new());
        let id = game.take_entity_id();
        game.install_fishing_hook(Hook::remote(id, Vec3::ZERO, [1, 2, 3, 4]));
        let entity = game.entity(id).unwrap();
        write_and_read_back(game, entity);
        assert_eq!(entity.fire_immune(game), Some(false));
        // A bobber is never saved: nothing lands in a saved compound.
        assert!(!game.common.persist.raw.contains_key(&id));
    });
}

#[test]
#[ignore = "daily"]
fn a_leash_knot_keeps_its_base_state_in_its_saved_tags() {
    with_game(|game| {
        open_chunk(game);
        let id = game.tie_knot([8, 70, 8]);
        let entity = game.entity(id).unwrap();
        write_and_read_back(game, entity);
        assert_eq!(entity.fire_immune(game), Some(false));
        assert!(entity.set_silent(game, true));
        assert!(entity.set_remaining_fire_ticks(game, 40));
        let tag = game.command_target_nbt(Target::Knot(id)).unwrap();
        assert_eq!(tag.get("Silent"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("Glowing"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("Invulnerable"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("NoGravity"), None);
        assert_eq!(tag.get("Fire"), Some(&Tag::Short(40)));
    });
}

#[test]
#[ignore = "daily"]
fn an_end_crystal_keeps_the_fire_ticks_it_was_given() {
    with_game(|game| {
        open_chunk(game);
        game.summon_with_nbt("minecraft:end_crystal", [8.5, 65.0, 8.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let id = game.common.crystals[0].id;
        let entity = game.entity(id).unwrap();
        assert!(entity.set_remaining_fire_ticks(game, 40_000));
        assert_eq!(entity.fire_ticks(game), Some(40_000));
        // The save truncates to a short, as `Entity.saveWithoutId` does.
        let tag = game.command_target_nbt(Target::Crystal(id)).unwrap();
        assert_eq!(tag.get("Fire"), Some(&Tag::Short(40_000_i32 as i16)));
    });
}

/// `WitherSkull.isOnFire` and `ShulkerBullet.isOnFire` answer false though their types are not `.fireImmune()`; `END_CRYSTAL` is.
#[test]
#[ignore = "daily"]
fn a_skull_and_a_bullet_never_burn_but_are_not_fire_immune_and_a_crystal_is() {
    with_game(|game| {
        open_chunk(game);
        for kind in [
            ProjectileKind::WitherSkull { dangerous: false },
            ProjectileKind::ShulkerBullet {
                target: None,
                dir: None,
                steps: 0,
                delta: [0.0; 3],
                seed: 0,
            },
        ] {
            let mut shot = Projectile::new(kind, Vec3::new(8.5, 65.0, 8.5), None);
            shot.id = game.take_entity_id();
            let id = shot.id;
            game.common.creatures.projectiles.push(shot);
            let entity = game.entity(id).unwrap();
            assert!(entity.set_remaining_fire_ticks(game, 60));
            assert_eq!(entity.fire_ticks(game), Some(60));
            assert_eq!(entity.fire_immune(game), Some(false));
            assert_eq!(entity.is_on_fire(game), Some(false));
        }
        game.summon_with_nbt("minecraft:end_crystal", [8.5, 65.0, 8.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let id = game.common.crystals[0].id;
        assert_eq!(game.entity(id).unwrap().fire_immune(game), Some(true));
    });
}
