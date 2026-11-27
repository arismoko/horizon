use sunrise_core::geometry::{Aabb, Vec3};
pub use sunrise_sim::thrown_trident::{HALF, ThrownTrident};

/// `Breeze.PROJECTILE_DEFLECTION`: `ProjectileDeflection.REVERSE` at power
/// `(1, 1, 1)`. Each axis is multiplied by `-0.5`, and the yaw turns by
/// `170 + nextFloat * 20`.
pub fn breeze_deflect(velocity: Vec3, yaw: f32, jitter: f32) -> (Vec3, f32) {
    (
        Vec3::new(velocity.x * -0.5, velocity.y * -0.5, velocity.z * -0.5),
        yaw + 170.0 + jitter * 20.0,
    )
}
/// The entry distance of a segment into a box, if it crosses it.
pub fn segment_box(from: Vec3, d: Vec3, b: Aabb) -> Option<f64> {
    let (lo, hi) = (b.min_position(), b.max_position());
    let (o, d, lo, hi) = (
        [from.x, from.y, from.z],
        [d.x, d.y, d.z],
        [lo.x, lo.y, lo.z],
        [hi.x, hi.y, hi.z],
    );
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for a in 0..3 {
        if d[a].abs() < 1e-12 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some(t0)
}

#[cfg(test)]
mod tests {
    use super::{ThrownTrident, breeze_deflect};
    use sunrise_core::geometry::Vec3;

    /// A trident summoned at rest still reads pitch 0 after its first tick
    /// and -17.999998 after the second, when it starts falling; one summoned
    /// with Motion [0, 0.5, 0] reads 17.999998 after its first (#13168).
    #[test]
    fn a_flying_trident_turns_a_fifth_of_the_way_each_tick() {
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        t.turn_towards_movement();
        assert_eq!((t.yaw, t.pitch), (0.0, 0.0));
        t.velocity = Vec3::new(0.0, -0.05, 0.0);
        t.turn_towards_movement();
        assert_eq!((t.yaw, t.pitch), (0.0, -17.999998));
        let mut up = ThrownTrident::remote(2, Vec3::ZERO, 0.0, 0.0);
        up.velocity = Vec3::new(0.0, 0.5, 0.0);
        up.turn_towards_movement();
        assert_eq!(up.pitch, 17.999998);
    }

    /// `AbstractArrow.tickDespawn` through `ThrownTrident.tickDespawn`: the tick
    /// that brings `life` to 1200 discards a stuck trident, one loaded past
    /// 1200 goes on its first count, and a Loyalty trident that can be picked
    /// up never counts.
    #[test]
    fn a_stuck_trident_is_discarded_when_life_reaches_1200() {
        use sunrise_sim::entities::arrows::Pickup;
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        t.life = 1198;
        assert!(!t.tick_despawn());
        assert_eq!(t.life, 1199);
        assert!(t.tick_despawn());
        assert_eq!(t.life, 1200);
        let mut loaded = ThrownTrident::remote(2, Vec3::ZERO, 0.0, 0.0);
        loaded.life = 5000;
        assert!(loaded.tick_despawn());
        // Loyalty only exempts a trident that can be picked up.
        let mut loyal = ThrownTrident::remote(3, Vec3::ZERO, 0.0, 0.0);
        (loyal.loyalty, loyal.pickup, loyal.life) = (3, Pickup::Allowed, 1199);
        assert!(!loyal.tick_despawn());
        assert_eq!(loyal.life, 1199);
        loyal.pickup = Pickup::CreativeOnly;
        assert!(loyal.tick_despawn());
        loyal.pickup = Pickup::Allowed;
        loyal.loyalty = 0;
        assert!(loyal.tick_despawn());
    }

    /// `AbstractArrow.startFalling` sets `life = 0`: a block removed under a
    /// trident that was about to despawn starts the count over.
    #[test]
    fn a_trident_that_starts_falling_restarts_its_life() {
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        (t.in_ground, t.life) = (true, 1199);
        t.start_falling();
        assert_eq!((t.in_ground, t.life), (false, 0));
    }

    /// A block registry with air and stone, for save round trips.
    fn stone_registry() -> sunrise_world::registry::BlockRegistry {
        use sunrise_core::nbt::Tag;
        use sunrise_world::registry::BlockRegistry;
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            (
                "states",
                Tag::List(vec![
                    Tag::compound([("id", Tag::String("minecraft:air".into()))]),
                    Tag::compound([("id", Tag::String("minecraft:stone".into()))]),
                ]),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                ]),
            ),
        ]))
        .unwrap()
    }

    /// A trident hits its first block with its own sound and every later one
    /// with `entity.arrow.hit`, which is what it saves and keeps across a
    /// reload (#13172).
    #[test]
    fn a_trident_hits_with_its_own_sound_once() {
        use sunrise_core::nbt::Tag;
        let registry = stone_registry();
        let saved = |t: &ThrownTrident| {
            t.to_nbt([0; 4], &registry, None, &Default::default())
                .get("SoundEvent")
                .cloned()
        };
        let sound = |id: &str| Some(Tag::String(id.into()));
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        assert_eq!(saved(&t), sound("minecraft:item.trident.hit_ground"));
        assert_eq!(t.hit_ground_sound(), "minecraft:item.trident.hit_ground");
        assert_eq!(saved(&t), sound("minecraft:entity.arrow.hit"));
        let tag = t.to_nbt([0; 4], &registry, None, &Default::default());
        let n = sunrise_sim::entity_nbt::trident_from_nbt(&tag).unwrap();
        let mut back = ThrownTrident::from_nbt(n, &tag, &registry);
        assert_eq!(back.hit_ground_sound(), "minecraft:entity.arrow.hit");
        // readAdditionalSaveData: no SoundEvent, or no identifier, is the default.
        let pos = ("Pos", Tag::List(vec![Tag::Double(0); 3]));
        for tag in [
            Tag::compound([pos.clone()]),
            Tag::compound([pos, ("SoundEvent", Tag::String("Not An Id".into()))]),
        ] {
            let n = sunrise_sim::entity_nbt::trident_from_nbt(&tag).unwrap();
            assert_eq!(n.sound_event, "minecraft:item.trident.hit_ground");
        }
    }

    /// `AbstractArrow.readAdditionalSaveData` keeps a negative `life` as it
    /// was saved (it only counts up while the trident is in a block) and reads
    /// `shake` as `& 255`, so `-1b` is 255 ticks and saves as `-1b` again.
    #[test]
    fn a_trident_keeps_a_negative_life_and_shake_across_a_reload() {
        use sunrise_core::nbt::Tag;
        let registry = stone_registry();
        let tag = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0); 3])),
            ("life", Tag::Short(-1)),
            ("shake", Tag::Byte(-1)),
        ]);
        let loaded = sunrise_sim::entity_nbt::trident_from_nbt(&tag).unwrap();
        assert_eq!((loaded.life, loaded.shake), (-1, 255));
        // `Entity.load` reads `Fire` with `getShortOr` as well: 40000 is -25536.
        let burning = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0); 3])),
            ("Fire", Tag::Int(40000)),
        ]);
        let fire = sunrise_sim::entity_nbt::trident_from_nbt(&burning)
            .unwrap()
            .fire_ticks;
        assert_eq!(fire, -25536);
        let back = ThrownTrident::from_nbt(loaded, &tag, &registry);
        // `inGroundTime` is not saved: Loyalty's count starts at 0 whatever
        // `life` was, so a negative `life` does not hold the trident back.
        assert_eq!((back.life, back.in_ground_time), (-1, 0));
        let saved = back.to_nbt([1; 4], &registry, None, &Default::default());
        assert_eq!(saved.get("life"), Some(&Tag::Short(-1)));
        assert_eq!(saved.get("shake"), Some(&Tag::Byte(-1)));
    }

    /// `AbstractArrow` saves `lastState` as `inBlockState` and reads it back.
    #[test]
    fn a_trident_round_trips_its_in_block_state() {
        use sunrise_core::nbt::Tag;
        let registry = stone_registry();
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        assert!(
            t.to_nbt([1; 4], &registry, None, &Default::default())
                .get("inBlockState")
                .is_none()
        );
        t.last_state = registry.default_state("minecraft:stone");
        let tag = t.to_nbt([1; 4], &registry, None, &Default::default());
        assert_eq!(
            tag.get("inBlockState"),
            Some(&Tag::String("minecraft:stone".into()))
        );
        let loaded = sunrise_sim::entity_nbt::trident_from_nbt(&tag).unwrap();
        let back = ThrownTrident::from_nbt(loaded, &tag, &registry);
        assert_eq!(back.last_state, t.last_state);
    }

    /// `Entity.saveWithoutId` writes the `fall_distance` `Entity.load` read:
    /// a trident keeps the one it was loaded with.
    #[test]
    fn a_trident_keeps_its_fall_distance_across_a_reload() {
        use sunrise_core::nbt::Tag;
        let registry = stone_registry();
        let fall = |tag: &Tag| match tag.get("fall_distance") {
            Some(Tag::Double(bits)) => Some(f64::from_bits(*bits)),
            _ => None,
        };
        let fresh = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        assert_eq!(
            fall(&fresh.to_nbt([1; 4], &registry, None, &Default::default())),
            Some(0.0)
        );
        let tag = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0); 3])),
            ("fall_distance", Tag::Double(5.0_f64.to_bits())),
        ]);
        let loaded = sunrise_sim::entity_nbt::trident_from_nbt(&tag).unwrap();
        let back = ThrownTrident::from_nbt(loaded, &tag, &registry);
        assert_eq!(
            fall(&back.to_nbt([1; 4], &registry, None, &Default::default())),
            Some(5.0)
        );
    }

    /// `AbstractArrow.startFalling` on `new Random(42)`: its first three
    /// `nextFloat() * 0.2F` draws (computed from Java's LCG) scale x, y, z.
    #[test]
    fn a_falling_trident_scales_its_velocity_by_three_draws() {
        let mut t = ThrownTrident::remote(42, Vec3::ZERO, 0.0, 0.0);
        // The network id does not seed the projectile's independent entity random.
        t.random = sunrise_core::random::LegacyRandom::new(42);
        t.in_ground = true;
        t.velocity = Vec3::new(1.0, 1.0, 1.0);
        t.start_falling();
        assert!(!t.in_ground);
        assert_eq!(
            t.velocity,
            Vec3::new(0.14551274478435516, 0.0109330415725708, 0.13664469122886658)
        );
    }

    /// `ProjectileDeflection.REVERSE` at power 1: half speed, the other way,
    /// and a turn of 170 degrees plus a fifth of the random draw.
    #[test]
    fn a_breeze_turns_a_trident_around_at_half_speed() {
        let (velocity, yaw) = breeze_deflect(Vec3::new(4.0, -2.0, 1.0), 10.0, 0.25);
        assert_eq!(velocity, Vec3::new(-2.0, 1.0, -0.5));
        assert_eq!(yaw, 185.0);
    }

    #[test]
    fn piston_move_releases_a_stuck_trident_with_start_falling_velocity() {
        let mut trident = ThrownTrident::remote(1, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        trident.in_ground = true;
        trident.velocity = Vec3::new(1.0, -2.0, 3.0);
        let mut random = trident.random.clone();
        let expected =
            sunrise_sim::projectiles::start_falling_velocity(trident.velocity, &mut random);

        trident.move_by_piston(Vec3::new(0.5, 0.0, 0.0));
        assert!(trident.piston_release_bounds().is_some());
        trident.release_after_piston(false);
        assert!(trident.in_ground);
        assert_eq!(trident.velocity, Vec3::new(1.0, -2.0, 3.0));

        trident.release_after_piston(true);
        assert!(!trident.in_ground);
        assert_eq!(trident.velocity, expected);
        assert!(trident.needs_sync);
        assert!(trident.piston_release_bounds().is_none());
    }
}
