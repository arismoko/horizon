use mod_api::{
    Identifier, PendingChildrenRef, PendingMobSpec, SpawnFlags, SpawnLimits, UnattachedMob,
    UnspawnedMobRef, WorldEpoch,
    wire::{Reader, Writer},
};

#[test]
fn pending_child_tokens_preserve_the_entire_callback_identity() {
    let epoch = WorldEpoch::new(u64::MAX - 1);
    let collection = PendingChildrenRef::new(epoch, u64::MAX - 2, u64::MAX - 3);
    let mob = UnspawnedMobRef::pending(epoch, u64::MAX - 2, u64::MAX - 3, 255);
    let owned = UnattachedMob::new(mob);
    let mut writer = Writer::new();
    writer.put(&collection);
    let expected: Vec<u8> = [u64::MAX - 1, u64::MAX - 2, u64::MAX - 3]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
    assert_eq!(writer.bytes(), expected);
    assert_eq!(
        Reader::new(&expected).take::<PendingChildrenRef>().unwrap(),
        collection
    );
    writer.clear();
    writer.put(&owned);
    // Unspawned handles encode world, callback lease, action, then lifetime slot.
    let mut expected: Vec<u8> = [u64::MAX - 1, u64::MAX - 3, u64::MAX - 2]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
    expected.extend(255u32.to_le_bytes());
    assert_eq!(writer.bytes(), expected);
    let decoded = Reader::new(&expected).take::<UnattachedMob>().unwrap();
    assert_eq!(decoded, owned);
    assert_eq!(
        decoded.mob().pending_parts(),
        Some((u64::MAX - 2, u64::MAX - 3, 255))
    );
    for end in 0..expected.len() {
        assert!(
            Reader::new(&expected[..end])
                .take::<UnattachedMob>()
                .is_err()
        );
    }
}

#[test]
fn pending_constructor_preserves_species_dimension_and_initial_fields() {
    let mut spec = PendingMobSpec::new(
        Identifier::parse("minecraft:sulfur_cube").unwrap(),
        Identifier::parse("minecraft:the_nether").unwrap(),
        [8.5, -63.0, -20.25],
        90.0,
        true,
    );
    spec.size = Some(2);
    let mut writer = Writer::new();
    writer.put(&spec);
    let decoded = Reader::new(writer.bytes())
        .take::<PendingMobSpec>()
        .unwrap();
    assert_eq!(decoded.kind.to_string(), "minecraft:sulfur_cube");
    assert_eq!(decoded.dimension.to_string(), "minecraft:the_nether");
    assert_eq!(decoded.position, [8.5, -63.0, -20.25]);
    assert_eq!(
        (decoded.yaw, decoded.baby, decoded.size),
        (90.0, true, Some(2))
    );
}

#[test]
fn spawn_policy_crosses_wire_without_narrowing_limits_or_flags() {
    let limits = SpawnLimits {
        monster: u32::MAX,
        creature: 0,
        ambient: 1,
        axolotls: 2,
        underground_water_creature: 3,
        water_creature: 4,
        water_ambient: 5,
    };
    let mut writer = Writer::new();
    writer.put(&limits);
    let expected: Vec<u8> = [u32::MAX, 0, 1, 2, 3, 4, 5]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert_eq!(writer.bytes(), expected);
    assert_eq!(
        Reader::new(&expected).take::<SpawnLimits>().unwrap(),
        limits
    );
    for monsters in [false, true] {
        for friendlies in [false, true] {
            let flags = SpawnFlags {
                monsters,
                friendlies,
            };
            writer.clear();
            writer.put(&flags);
            assert_eq!(writer.bytes(), [u8::from(monsters), u8::from(friendlies)]);
            assert_eq!(
                Reader::new(writer.bytes()).take::<SpawnFlags>().unwrap(),
                flags
            );
        }
    }
}
