use super::*;

#[test]
fn arrow_family_belongs_to_the_actual_direct_entity_not_the_type() {
    let arrow = Damage::of(DamageKind::Arrow);
    let attack = Attack {
        projectile: Some(12),
        projectile_kind: DirectEntity::Arrow,
        ..Default::default()
    };
    assert!(attack.direct_is_arrow(&arrow));
    assert!(!Attack::default().direct_is_arrow(&arrow));
    assert!(!attack.direct_is_arrow(&Damage::of(DamageKind::Magic)));
    assert!(!attack.direct_is_arrow(&Damage::of(DamageKind::Explosion)));
    assert!(!attack.direct_is_arrow(&Damage::of(DamageKind::MobAttack)));
    assert!(
        !Attack {
            direct: Some(0),
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        !Attack {
            direct: Some(99),
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        Attack {
            direct: Some(12),
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        !Attack {
            projectile_kind: DirectEntity::Projectile,
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        !Attack {
            projectile: Some(0),
            ..attack
        }
        .direct_is_arrow(&arrow)
    );
}

#[test]
fn trident_source_identity_does_not_imply_an_abstract_arrow() {
    let trident = Damage::of(DamageKind::Named(crate::mobs::NamedDamage::new(
        "minecraft:trident",
        0,
    )));
    let attack = Attack {
        projectile: Some(12),
        projectile_kind: DirectEntity::Arrow,
        ..Default::default()
    };
    assert!(attack.direct_is_arrow(&trident));
    assert_eq!(
        mob_source(&trident, &attack).damage_type,
        "minecraft:trident"
    );
    assert_eq!(mob_source(&trident, &attack).direct, 12);
    assert!(!Attack::default().direct_is_arrow(&trident));
    assert!(
        !Attack {
            direct: Some(0),
            ..attack
        }
        .direct_is_arrow(&trident)
    );
}

#[test]
fn is_direct_compares_causing_and_direct_entities_not_damage_type() {
    let mob_projectile = Damage {
        kind: DamageKind::Named(crate::mobs::NamedDamage::new(
            "minecraft:mob_projectile",
            crate::mobs::NamedDamage::IS_PROJECTILE,
        )),
        attacker: Some(Who::Mob(5)),
        from: None,
        direct: None,
        attacker_species: None,
    };
    let melee = Damage::mob(5, [0.0; 3]);
    let same_entity = Attack {
        attacker: 5,
        direct: Some(5),
        direct_kind: Some(DirectEntity::Living),
        ..Default::default()
    };
    let different_entity = Attack {
        attacker: 5,
        direct: Some(12),
        direct_kind: Some(DirectEntity::Arrow),
        ..Default::default()
    };
    // Vary damage type with fixed entities.
    assert!(same_entity.is_direct(&mob_projectile));
    assert!(same_entity.is_direct(&melee));
    // Vary entities with fixed damage type.
    assert!(!different_entity.is_direct(&melee));
    assert!(!different_entity.is_direct(&mob_projectile));
    // A player cause with an explicit unrelated direct entity (or 0) is indirect.
    let player_indirect = Attack {
        attacker: 7,
        direct: Some(12),
        direct_kind: Some(DirectEntity::Projectile),
        ..Default::default()
    };
    assert!(!player_indirect.is_direct(&Damage::player([0.0; 3])));
    let player_direct = Attack {
        attacker: 7,
        direct: Some(7),
        direct_kind: Some(DirectEntity::Living),
        ..Default::default()
    };
    assert!(player_direct.is_direct(&Damage {
        kind: DamageKind::Arrow,
        attacker: Some(Who::Player),
        from: None,
        direct: None,
        attacker_species: None,
    }));
}

#[test]
fn direct_entity_and_large_fireball_follow_entities_not_damage_type() {
    let large_fireball = Attack {
        attacker: 7,
        direct: Some(15),
        direct_kind: Some(DirectEntity::LargeFireball),
        ..Default::default()
    };
    let small_fireball = Attack {
        attacker: 7,
        direct: Some(16),
        direct_kind: Some(DirectEntity::Projectile),
        ..Default::default()
    };
    let living = Attack {
        attacker: 7,
        direct: Some(7),
        direct_kind: Some(DirectEntity::Living),
        ..Default::default()
    };
    let tnt = Attack {
        attacker: 5,
        direct: Some(20),
        direct_kind: Some(DirectEntity::Other),
        ..Default::default()
    };
    let fireball_kind = Damage {
        kind: DamageKind::Fireball,
        attacker: Some(Who::Player),
        from: None,
        direct: None,
        attacker_species: None,
    };
    let generic_kind = Damage {
        kind: DamageKind::Generic,
        attacker: Some(Who::Player),
        from: None,
        direct: None,
        attacker_species: None,
    };
    assert!(large_fireball.direct_is_large_fireball(&generic_kind));
    assert!(large_fireball.direct_is_large_fireball(&fireball_kind));
    assert!(!small_fireball.direct_is_large_fireball(&fireball_kind));
    assert!(!living.direct_is_large_fireball(&fireball_kind));
    assert!(!Attack::default().direct_is_large_fireball(&fireball_kind));
    assert!(large_fireball.direct_entity(&generic_kind).is_projectile());
    assert!(small_fireball.direct_entity(&generic_kind).is_projectile());
    assert!(living.direct_entity(&generic_kind).is_living());
    assert!(!tnt.direct_entity(&Damage::mob(5, [0.0; 3])).is_living());
    assert!(!tnt.direct_entity(&Damage::mob(5, [0.0; 3])).is_projectile());
}
