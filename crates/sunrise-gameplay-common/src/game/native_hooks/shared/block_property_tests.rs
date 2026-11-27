/// A mod's property write: none for a key or value the block lacks, the
/// state itself when it already has the value, else the new state.
#[test]
fn with_property_finds_the_state() {
    let facts = br#"{"defaults":{"minecraft:air":0,"test:crop":1},"states":[
        ["air",true,false,"","minecraft:air"],
        ["crop0",false,false,"",{"id":"test:crop","properties":{"age":"0"}}],
        ["crop1",false,false,"",{"id":"test:crop","properties":{"age":"1"}}]
    ]}"#;
    let registry = sunrise_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap();
    use sunrise_world::registry::BlockStateId;
    let (age0, age1) = (BlockStateId(1), BlockStateId(2));
    assert_eq!(
        super::with_property(&registry, age0, "age", "0"),
        Some(age0)
    );
    assert_eq!(
        super::with_property(&registry, age0, "age", "1"),
        Some(age1)
    );
    assert_eq!(super::with_property(&registry, age0, "age", "9"), None);
    assert_eq!(super::with_property(&registry, age0, "lit", "true"), None);
    // What both property setters write: nothing (false) when the
    // property already has the value, as `Level.setBlock`.
    assert_eq!(super::property_change(&registry, age0, "age", "0"), None);
    assert_eq!(
        super::property_change(&registry, age0, "age", "1"),
        Some(age1)
    );
    assert_eq!(super::property_change(&registry, age0, "age", "9"), None);
    assert_eq!(super::property_change(&registry, age0, "lit", "true"), None);
}

/// The state a mod block is placed in is built from its own default state
/// and one of its properties, its declared ones (`state_properties` in its
/// block file) included, and keeps the properties it already had.
#[test]
fn a_placement_state_is_built_from_a_blocks_own_properties() {
    let facts = br#"{"defaults":{"minecraft:air":0,"test:pipe":1},"states":[
        ["air",true,false,"","minecraft:air"],
        ["north_gas",false,false,"",{"id":"test:pipe","properties":{"carries":"gas","facing":"north"}}],
        ["east_gas",false,false,"",{"id":"test:pipe","properties":{"carries":"gas","facing":"east"}}],
        ["north_items",false,false,"",{"id":"test:pipe","properties":{"carries":"items","facing":"north"}}],
        ["east_items",false,false,"",{"id":"test:pipe","properties":{"carries":"items","facing":"east"}}]
    ]}"#;
    let registry = sunrise_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap();
    use sunrise_world::registry::BlockStateId;
    let (north_gas, east_gas, east_items) = (BlockStateId(1), BlockStateId(2), BlockStateId(4));
    // The declared property changes, and `carries` stays as it was.
    assert_eq!(
        super::with_property(&registry, north_gas, "facing", "east"),
        Some(east_gas)
    );
    assert_eq!(
        super::with_property(&registry, east_gas, "carries", "items"),
        Some(east_items)
    );
    // A value the property does not have builds nothing.
    assert_eq!(
        super::with_property(&registry, east_gas, "facing", "up"),
        None
    );
}
