#[test]
fn replacing_mod_block_entity_payload_keeps_its_type_and_saved_identity() {
    use sunrise_level::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use sunrise_sim::save::{block_entity_encode, decode_block_entity};

    let pos = (1, 64, 1);
    let mut machine = ModBlockEntity::new("test:machine".into(), Default::default());
    let made = machine.revision();
    *machine = ModBlockEntityFields {
        data: vec![7],
        ..Default::default()
    };
    assert_eq!(machine.id(), "test:machine");
    assert_eq!(machine.data, [7]);
    assert_ne!(machine.revision(), made);
    let entity = BlockEntity::Mod(Box::new(machine));
    let saved = block_entity_encode::encode(pos, &entity, "minecraft:stone", &Default::default());
    let (saved_pos, loaded) = decode_block_entity(&saved).unwrap();
    assert_eq!(saved_pos, pos);
    assert_eq!(loaded.id(), "test:machine");
}
