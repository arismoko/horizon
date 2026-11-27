use sunrise_gameplay_common::game::remote::hold_server_fire;

/// The server's on-fire flag keeps the predicted countdown (which
/// `fluid_base_tick` runs down) from ending the fire; a clear flag leaves
/// the window's own contact fire alone.
#[test]
fn the_servers_fire_flag_holds_the_burning_state_through_the_countdown() {
    let mut ticks = 1;
    for _ in 0..5 {
        ticks = hold_server_fire(ticks - 1, true);
        assert!(ticks > 0, "still burning");
    }
    assert_eq!(hold_server_fire(300, true), 300, "a longer fire is not cut");
    assert_eq!(hold_server_fire(7, false), 7, "predicted lava fire stays");
    assert_eq!(hold_server_fire(0, false), 0);
}
