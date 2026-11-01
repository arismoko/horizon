fn sample_knock(entity: i32, kind: &str, damage_type: &str, strength: &mut f64, x: &mut f64, z: &mut f64, canceled: &mut bool) {
    match kind {
        "minecraft:cow" => *strength *= 2.0,
        "minecraft:sheep" => *strength = 0.0,
        "minecraft:chicken" => *canceled = true,
        _ => {}
    }
    eprintln!("KNOCK_TEST id={entity} kind={kind} source={damage_type} strength={strength} x={x} z={z} canceled={canceled}");
}
