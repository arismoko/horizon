//! `DamageSource.isDirect` in a death's loot: a blow the attacker dealt
//! through another entity (a thrown trident, a splash potion, primed TNT) is
//! not direct.
use sunrise_sim::mobs::{Damage, DamageKind, Who};
use sunrise_world::loot::{EntityLootParams, LootData, LootEntity, PackOverlay};

/// What a table that drops a stick only for indirect damage
/// (`damage_source_properties` with `is_direct: false`) drops for this death.
fn indirect_only_drops(damage: Damage, attacker: Option<LootEntity>) -> Vec<String> {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), r#"{"minecraft:stick":{}}"#).unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let table = serde_json::json!({"pools": [{
        "rolls": 1,
        "condition": {
            "type": "minecraft:damage_source_properties",
            "predicate": {"is_direct": false}
        },
        "entries": [{"type": "minecraft:item", "name": "minecraft:stick"}]
    }]});
    let files = vec![(
        "loot_table/test:indirect".to_owned(),
        vec![serde_json::to_vec(&table).unwrap()],
    )];
    let loot = LootData::load(root.path(), &PackOverlay::new(&files), &defaults).unwrap();
    let params = EntityLootParams {
        damage: Some(super::loot_damage(damage, attacker)),
        ..EntityLootParams::default()
    };
    let mut random = sunrise_core::random::LegacyRandom::new(1);
    loot.roll_entity("test:indirect", &mut random, &params)
        .iter()
        .map(|stack| stack.item.to_string())
        .collect()
}

#[test]
fn a_kill_through_a_thrown_trident_potion_or_tnt_is_not_direct() {
    let player = || Some(LootEntity::new("minecraft:player"));
    let mut trident = Damage::of(DamageKind::Thrown).with_direct("minecraft:trident");
    trident.attacker = Some(Who::Player);
    let potion = Damage::indirect_magic("minecraft:splash_potion", Some(Who::Player), [0.0; 3]);
    let mut tnt = Damage::of(DamageKind::Explosion).with_direct("minecraft:tnt");
    tnt.attacker = Some(Who::Player);
    // Primed TNT nobody lit: a direct entity and no attacker.
    let unlit = Damage::of(DamageKind::Explosion).with_direct("minecraft:tnt");
    for (damage, attacker) in [
        (trident, player()),
        (potion, player()),
        (tnt, player()),
        (unlit, None),
    ] {
        assert_eq!(
            indirect_only_drops(damage, attacker),
            ["minecraft:stick"],
            "{damage:?}"
        );
    }
    // The player's own blow, and a fall with no entity at all, are direct.
    assert!(indirect_only_drops(Damage::player([0.0; 3]), player()).is_empty());
    assert!(indirect_only_drops(Damage::fall(), None).is_empty());
}

/// What the damage source says to a death's loot: its causing entity's type, its direct entity's
/// type and `isDirect`.
fn seen(damage: Damage, attacker: Option<LootEntity>) -> (Option<String>, Option<String>, bool) {
    let d = super::loot_damage(damage, attacker);
    (
        d.source.map(|e| e.kind.to_string()),
        d.direct.map(|e| e.kind.to_string()),
        d.is_direct,
    )
}

/// A `/damage` of a projectile's type records no direct entity, so none is made up from the
/// type: with no source it has no entity at all, and `by` an entity names that entity both ways.
/// An ownerless arrow, thrown trident or fireball is its own cause (direct); ownerless TNT, a
/// potion's cloud or a rocket has no cause (not direct).
#[test]
fn only_the_recorded_direct_entity_counts_and_an_ownerless_projectile_is_its_own_cause() {
    let some = |kind: &str| Some(kind.to_owned());
    for kind in [
        DamageKind::Arrow,
        DamageKind::Fireball,
        DamageKind::WitherSkull,
        DamageKind::Fireworks,
    ] {
        assert_eq!(seen(Damage::of(kind), None), (None, None, true), "{kind:?}");
        let mut by = Damage::of(kind);
        by.attacker = Some(Who::Mob(7));
        assert_eq!(
            seen(by, Some(LootEntity::new("minecraft:zombie"))),
            (some("minecraft:zombie"), some("minecraft:zombie"), true),
            "{kind:?}"
        );
    }
    for (kind, projectile) in [
        (DamageKind::Arrow, "minecraft:arrow"),
        (DamageKind::Arrow, "minecraft:spectral_arrow"),
        (DamageKind::Thrown, "minecraft:trident"),
        (DamageKind::Fireball, "minecraft:fireball"),
        (DamageKind::Fireball, "minecraft:small_fireball"),
    ] {
        let ownerless = Damage::of(kind).with_direct(projectile);
        assert_eq!(
            seen(ownerless, None),
            (some(projectile), some(projectile), true),
            "{projectile}"
        );
        let mut owned = Damage::of(kind).with_direct(projectile);
        owned.attacker = Some(Who::Player);
        assert_eq!(
            seen(owned, Some(LootEntity::new("minecraft:player"))),
            (some("minecraft:player"), some(projectile), false),
            "{projectile}"
        );
    }
    for projectile in [
        "minecraft:tnt",
        "minecraft:area_effect_cloud",
        "minecraft:firework_rocket",
    ] {
        let ownerless = Damage::of(DamageKind::Explosion).with_direct(projectile);
        assert_eq!(
            seen(ownerless, None),
            (None, some(projectile), false),
            "{projectile}"
        );
    }
}

/// A real blast records its direct entity on the mob it hurts (`explosion_hurt_entities`): the
/// primed TNT that went off and an End crystal's blast as the dragon's respawn sets them off,
/// whichever path set the source up; an exploding mob that is the attacker deals it itself.
#[test]
#[ignore = "daily"]
fn a_blast_records_the_entity_that_went_off() {
    use crate::game::explosion::{BlockInteraction, ExplosionSource};
    use sunrise_sim::health::DirectEntity;
    crate::game::native_entity_refs::tests::with_game(|game| {
        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let blast = |game: &mut crate::game::ServerGame<'_>, source: ExplosionSource| {
            game.common.mobs.clear();
            let mut target = sunrise_sim::mobs::Mob::new(
                70,
                pig.clone(),
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
                0.0,
            );
            target.health = 1000.0;
            game.common.mobs.push(target);
            game.server_explode_from([0.5, 64.5, 2.0], 4.0, BlockInteraction::Keep, false, source)
                .unwrap();
            let hit = game.mobs.iter().find(|m| m.id == 70).unwrap();
            let (damage, _) = hit.last_damage.expect("the blast hurt the pig");
            seen(damage, None)
        };
        let tnt = ExplosionSource::default().with_entity(900, "minecraft:tnt", DirectEntity::Other);
        assert_eq!(
            blast(game, tnt),
            (None, Some("minecraft:tnt".into()), false)
        );
        let crystal = ExplosionSource::default().with_damage_direct(
            901,
            Some("minecraft:end_crystal"),
            DirectEntity::Other,
        );
        assert_eq!(
            blast(game, crystal),
            (None, Some("minecraft:end_crystal".into()), false)
        );
    });
}
