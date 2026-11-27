use super::*;

#[test]
fn a_projectile_blow_has_no_known_direct_entity_and_a_melee_one_is_its_causer() {
    let arrow = death_hurt_source("minecraft:arrow", 7);
    assert_eq!((arrow.causing, arrow.direct), (7, 0));
    let melee = death_hurt_source("minecraft:mob_attack", 7);
    assert_eq!((melee.causing, melee.direct), (7, 7));
    let none = death_hurt_source("minecraft:generic", 0);
    assert_eq!((none.causing, none.direct), (0, 0));
}
