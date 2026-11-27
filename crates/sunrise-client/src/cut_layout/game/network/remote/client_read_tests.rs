use {
    sunrise_gameplay_common::game::remote::look_at_rotation,
    sunrise_gameplay_common::game::remote::snapshot_value,
};

/// An NPC plugin's scale: a base the plugin sets, or a modifier on 1.
#[test]
fn attribute_values_add_then_multiply_as_vanilla() {
    use sunrise_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
    let modifier = |amount, operation| AttributeModifier {
        id: "minecraft:proof".into(),
        amount,
        operation,
    };
    let snapshot = |base, modifiers| AttributeSnapshot {
        attribute: 0,
        base,
        modifiers,
    };
    assert_eq!(snapshot_value(&snapshot(2.0, vec![])), 2.0);
    assert_eq!(snapshot_value(&snapshot(1.0, vec![modifier(1.0, 0)])), 2.0);
    // (1 + 1) * (1 + 0.5) from the base, then * (1 + 1) of the total.
    let all = vec![modifier(1.0, 0), modifier(0.5, 1), modifier(1.0, 2)];
    assert_eq!(snapshot_value(&snapshot(1.0, all)), 6.0);
}

/// The pinned vanilla 26.3 side is `VanillaTurnOracle`'s `look_at.txt`
/// (`tools/test_mob_turn_parity.py`, recorded into `.local/witness/26.3/mob_turn/look_at.txt`):
/// `Entity.lookAt(FEET, target)` from each place to each offset, as the yaw and pitch bits it
/// stores: the cardinal directions, the yaw wrap at -180, both pitch signs, straight up and down
/// (`Mth.atan2` stops a hair short of 90) and a target at the eye (-90 yaw, pitch -0.0).
#[test]
#[ignore = "run by tools/test_mob_turn_parity.py"]
fn look_at_matches_the_pinned_oracle() {
    run_look_at_matches_the_pinned_oracle();
}

pub(in crate::game) fn run_look_at_matches_the_pinned_oracle() {
    let witness_dir = std::env::var_os("SUNRISE_MOB_TURN_WITNESS_DIR").expect(
        "SUNRISE_MOB_TURN_WITNESS_DIR must be set; run through tools/test_mob_turn_parity.py, which records the witness",
    );
    let witness_path = std::path::Path::new(&witness_dir).join("look_at.txt");
    let witness = std::fs::read_to_string(&witness_path).unwrap_or_else(|err| {
        panic!(
            "failed to read {} ({err}); run through tools/test_mob_turn_parity.py, which records the witness",
            witness_path.display()
        )
    });
    let bits = |v: f64| format!("{:016x}", v.to_bits());
    let mut rows = Vec::new();
    for from in [[10.0, 64.0, -5.0], [0.0, 0.0, 0.0], [0.5, 64.5, 0.5]] {
        for off in [
            [0.0, 0.0, 3.0],
            [3.0, 0.0, 0.0],
            [-3.0, 0.0, 0.0],
            [0.0, 0.0, -3.0],
            [0.0, 2.0, 2.0],
            [0.0, -4.0, 4.0],
            [1.0, 0.5, 2.0],
            [0.0, 5.0, 0.0],
            [0.0, -5.0, 0.0],
            [0.0, 0.0, 0.0],
            [-2.0, 1.0, -0.0001],
            [7.0, -3.0, -1.0],
        ] {
            let to = [from[0] + off[0], from[1] + off[1], from[2] + off[2]];
            let (yaw, pitch) = look_at_rotation(from, to);
            rows.push(format!(
                "{} {} {} {} {} {} {:08x} {:08x}",
                bits(from[0]),
                bits(from[1]),
                bits(from[2]),
                bits(off[0]),
                bits(off[1]),
                bits(off[2]),
                yaw.to_bits(),
                pitch.to_bits()
            ));
        }
    }
    let actual = rows.join("\n") + "\n";
    if let Some(dir) = std::env::var_os("SUNRISE_TURN_OUTPUT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("look_at.txt"), &actual).unwrap();
    }
    assert_eq!(rows.len(), 36);
    assert_eq!(actual, witness);
}

/// `handleMovePlayer` moves the player's pitch and the model's old pitch
/// (`xRotO`) alike: a relative change keeps the ease a tick began, each
/// end clamped on its own, and an absolute one snaps both (#3844).
#[test]
fn a_position_packet_moves_both_pitch_ends() {
    use sunrise_gameplay_common::game::remote::position_packet_pitch as pitch;
    // The tick is laying the head down from 30 to 0; the packet turns 10.
    assert_eq!(
        (pitch(30.0, 10.0, true), pitch(0.0, 10.0, true)),
        (40.0, 10.0)
    );
    // Each end clamps on its own.
    assert_eq!(
        (pitch(80.0, 20.0, true), pitch(60.0, 20.0, true)),
        (90.0, 80.0)
    );
    // An absolute pitch ignores both ends.
    assert_eq!(
        (pitch(30.0, -45.0, false), pitch(0.0, -45.0, false)),
        (-45.0, -45.0)
    );
    assert_eq!(pitch(0.0, 120.0, false), 90.0);
}
