use sunrise_gameplay_common::game::remote::named_hand_operated;

/// A cauldron's fill level is a use this client predicts, and a guest's
/// host forwards its sound: every cauldron state counts, not only the
/// doors and levers.
#[test]
fn every_cauldron_counts_as_hand_operated() {
    for name in [
        "minecraft:cauldron",
        "minecraft:water_cauldron",
        "minecraft:lava_cauldron",
        "minecraft:powder_snow_cauldron",
    ] {
        assert!(named_hand_operated(name), "{name}");
    }
    assert!(!named_hand_operated("minecraft:water"));
}
