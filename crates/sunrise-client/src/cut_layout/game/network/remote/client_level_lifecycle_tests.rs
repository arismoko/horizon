use sunrise_gameplay_common::game::remote::client_has_level;

/// Minecraft.tick: CLIENT_TICK_END only while a level exists. None before
/// the first Login (configuration has finished, the server has not placed
/// the player yet), and none from StartConfiguration until the next Login.
#[test]
fn tick_ends_need_the_level_the_login_makes() {
    assert!(!client_has_level(false, None), "joined, waiting for Login");
    assert!(client_has_level(true, None), "after Login");
    assert!(!client_has_level(true, Some(40)), "reconfiguring");
}
