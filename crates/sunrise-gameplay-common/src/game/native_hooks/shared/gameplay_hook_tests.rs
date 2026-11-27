#[test]
fn loot_table_load_stays_hookable() {
    let source = include_str!("../../../../../sunrise-world/src/loot.rs").replace("\r\n", "\n");
    // From its doc comment's last line, so an attribute above it counts.
    let hook = source.find("pub fn native_loot_table_load(").unwrap();
    let start = source[..hook].rfind("///").unwrap();
    let end = source[start..]
        .find("\n}\n")
        .or_else(|| source[start..].find("\r\n}\r\n"))
        .unwrap();
    let body = &source[start..start + end];
    for line in body.lines() {
        assert!(
            !line.contains("cfg"),
            "a cfg in the loot hook point: {line}"
        );
        for mac in ["!(", "![", "! {", "!{"] {
            assert!(
                !line.contains(mac),
                "a macro in the loot hook point: {line}"
            );
        }
    }
}

#[test]
fn lifecycle_source_stays_hookable() {
    let source =
        include_str!("../../../../../sunrise-gameplay/src/game/modding/native_lifecycle.rs");
    let code = source.split("#[cfg(test)]").next().unwrap_or_default();
    for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
        assert!(!line.contains("cfg"), "a cfg in lifecycle source: {line}");
        for mac in ["!(", "![", "! {", "!{"] {
            assert!(!line.contains(mac), "a macro in lifecycle source: {line}");
        }
    }
}

#[test]
fn the_damage_hook_names_the_source() {
    use super::{Attack, HurtSource, Hurting, hurt_server_args, mob_source};
    use sunrise_sim::health::{NO_ATTACK, source_damage_type};
    use sunrise_sim::mobs::{Damage, DamageKind as K, Who};
    let source = |kind, attacker| {
        let damage = Damage {
            kind,
            attacker,
            from: None,
            direct: None,
            attacker_species: None,
        };
        let attack = Attack {
            attacker: 7,
            projectile: None,
            direct: None,
            player_uuid: None,
            held: Default::default(),
            ..Default::default()
        };
        let s = mob_source(&damage, &attack);
        (source_damage_type(damage), s.causing, s.direct)
    };
    let zombie = Some(Who::Mob(3));
    // A registered type of a datapack reaches the hook under its own id.
    let scorch = K::Named(sunrise_sim::mobs::NamedDamage::new("demo:scorch", 0));
    assert_eq!(source(scorch, None), ("demo:scorch", 0, 0));
    assert_eq!(source(scorch, zombie), ("demo:scorch", 3, 0));
    assert_eq!(source(K::MobAttack, zombie), ("minecraft:mob_attack", 3, 3));
    assert_eq!(source(K::Thorns, zombie), ("minecraft:thorns", 3, 3));
    assert_eq!(source(K::Arrow, zombie), ("minecraft:arrow", 3, 0));
    let during = |kind| {
        let damage = Damage {
            kind,
            attacker: zombie,
            from: None,
            direct: None,
            attacker_species: None,
        };
        let attack = Attack {
            attacker: 7,
            projectile: Some(912),
            direct: None,
            player_uuid: None,
            held: Default::default(),
            ..Default::default()
        };
        let s = mob_source(&damage, &attack);
        (source_damage_type(damage), s.direct)
    };
    // The impact's projectile is the direct entity only when the type names it.
    assert_eq!(during(K::Arrow), ("minecraft:arrow", 912));
    assert_eq!(during(K::WitherSkull), ("minecraft:wither_skull", 912));
    assert_eq!(during(K::IndirectMagic), ("minecraft:indirect_magic", 912));
    // magic() and a blast do not, even while that projectile's attack is in scope.
    assert_eq!(during(K::Magic), ("minecraft:magic", 0));
    assert_eq!(during(K::Explosion), ("minecraft:player_explosion", 0));
    let blast = source(K::Explosion, zombie);
    assert_eq!(blast, ("minecraft:player_explosion", 3, 0));
    // A player behind a mob's hit is the one whose attack the level lends; a fall has no
    // source.
    let hit = source(K::PlayerAttack, Some(Who::Player));
    assert_eq!(hit, ("minecraft:player_attack", 7, 7));
    assert_eq!(source(K::Fall, None), ("minecraft:fall", 0, 0));
    let arrow = HurtSource {
        damage_type: "minecraft:arrow",
        causing: 4,
        direct: 0,
        position: None,
        direct_type: None,
    };
    let player = (2, "minecraft:player", arrow);
    assert_eq!(
        hurt_server_args(Hurting::Player(2, arrow), &NO_ATTACK),
        player
    );
    let dragon = (5, "minecraft:ender_dragon", arrow);
    assert_eq!(
        hurt_server_args(Hurting::Dragon(5, arrow), &NO_ATTACK),
        dragon
    );
    assert_eq!(
        hurt_server_args(Hurting::Entity(6, "minecraft:item", arrow), &NO_ATTACK),
        (6, "minecraft:item", arrow)
    );
}
