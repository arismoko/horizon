/// The hurt sound of `ClientboundDamageEventPacket` follows its damage
/// type (`Player.getHurtSound`: the type's effects), with the codec's
/// `hurt` default for an unknown type (#4149).
#[test]
fn the_hurt_sound_follows_the_damage_events_type() {
    use sunrise_gameplay_common::game::remote::selector_metadata::hurt_sound_for;
    use sunrise_protocol::registry::Dynamic;
    use sunrise_world::damage_source::Catalog;
    let types = [
        ("minecraft:in_fire", "burning"),
        ("minecraft:drown", "drowning"),
        ("minecraft:sweet_berry_bush", "poking"),
        ("minecraft:freeze", "freezing"),
        ("minecraft:arrow", "hurt"),
    ];
    // The types the server synchronized, in registry order.
    let mut registries = Dynamic::default();
    registries.add(sunrise_protocol::packets::configuration::RegistryData {
        registry: "minecraft:damage_type".into(),
        entries: types
            .iter()
            .map(
                |(id, _)| sunrise_protocol::packets::configuration::RegistryEntry {
                    id: (*id).into(),
                    data: None,
                },
            )
            .collect(),
    });
    // The damage catalog: one entry per type above, each with its effects.
    let bits = |f: f32| format!("{:08x}", f.to_bits());
    let catalog = Catalog::from_json(
        &serde_json::json!({
            "data_version": i64::from(sunrise_world::DATA_VERSION),
            "types": types.iter().map(|(id, effects)| serde_json::json!({
                "id": id,
                "message_id": id.trim_start_matches("minecraft:"),
                "exhaustion": bits(0.1),
                "scaling": "when_caused_by_living_non_player",
                "effects": effects,
                "death_message_type": "default",
            })).collect::<Vec<_>>(),
        }),
        &serde_json::json!({}),
    )
    .unwrap();
    for (id, (name, effects)) in types.iter().enumerate() {
        assert_eq!(
            hurt_sound_for(&registries, &catalog, id as i32),
            sunrise_sim::health::hurt_sound(effects),
            "{name}"
        );
    }
    // An unknown id falls back to the codec's default effects.
    assert_eq!(
        hurt_sound_for(&registries, &catalog, types.len() as i32),
        "minecraft:entity.player.hurt"
    );
    // A server-defined damage type with inline NBT effects needs no entry in
    // the client's built-in catalog.
    let mut custom = sunrise_core::nbt::Compound::new();
    custom.insert(
        "effects".into(),
        sunrise_core::nbt::Tag::String("burning".into()),
    );
    registries.add(sunrise_protocol::packets::configuration::RegistryData {
        registry: "minecraft:damage_type".into(),
        entries: vec![sunrise_protocol::packets::configuration::RegistryEntry {
            id: "custom:lava_splash".into(),
            data: Some(sunrise_core::nbt::Tag::Compound(custom)),
        }],
    });
    assert_eq!(
        hurt_sound_for(&registries, &catalog, 0),
        "minecraft:entity.player.hurt_on_fire"
    );
}

#[test]
fn synchronized_damage_defaults_override_a_different_local_definition() {
    let local = sunrise_world::damage_source::Catalog::from_json(&serde_json::json!({
        "data_version": sunrise_world::DATA_VERSION,
        "types": [{"id":"demo:heat", "message_id":"heat", "exhaustion":"00000000", "scaling":"never", "effects":"burning", "death_message_type":"default"}]
    }), &serde_json::json!({})).unwrap();
    let mut registries = sunrise_protocol::registry::Dynamic::default();
    registries.add(sunrise_protocol::packets::configuration::RegistryData {
        registry: "minecraft:damage_type".into(),
        entries: vec![sunrise_protocol::packets::configuration::RegistryEntry {
            id: "demo:heat".into(),
            data: Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        }],
    });
    assert_eq!(
        sunrise_gameplay_common::game::remote::selector_metadata::hurt_sound_for(
            &registries,
            &local,
            0
        ),
        "minecraft:entity.player.hurt"
    );
}
