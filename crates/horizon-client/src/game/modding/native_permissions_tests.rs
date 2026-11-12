//! Permission nodes: a permissions mod's answers and the operator fallback.
use super::native_guest_turn_tests::with_guests;
use mod_api::{Host, Hooks, PlayerPermissionCheckEvent};

/// Grants `test.granted`, denies `test.denied`, grants `test.guest12` to
/// guest 12 alone, and leaves every other node unset.
struct Provider;

#[mod_api::__subscriptions]
impl Hooks for Provider {
    fn player_permission_check(
        &self,
        _: &dyn Host,
        event: &mut PlayerPermissionCheckEvent<'_>,
    ) -> Option<bool> {
        match event.node {
            "test.granted" => Some(true),
            "test.denied" => Some(false),
            "test.guest12" if event.player == 12 && event.name == "guest12" => Some(true),
            _ => None,
        }
    }
}

fn with_provider(test: impl FnOnce(&mut super::Game<'_>)) {
    let mods = vec![mod_api::ModEntry::new(
        "permissions",
        Box::new(Provider),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_guests(test));
}

pub(super) fn set_permission(game: &mut super::Game<'_>, conn: u32, permission: u8) {
    let i = game.guest_index(conn).unwrap();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(i, |g| g.permission = permission);
}

/// `has_permission` takes a permissions mod's answer over the operator level, and
/// falls back to level 2 for a node no mod sets; `permission_value` tells unset apart.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_permissions_mod_answers_before_the_operator_level() {
    with_provider(|game| {
        let a = game.player(12).unwrap();
        let b = game.player(13).unwrap();
        set_permission(game, 12, 0);
        set_permission(game, 13, 0);
        assert!(game.player_has_permission(a, "test.granted"));
        assert!(!game.player_has_permission(a, "test.denied"));
        assert!(!game.player_has_permission(a, "test.other"));
        assert_eq!(game.player_permission_value(a, "test.granted"), Some(true));
        assert_eq!(game.player_permission_value(a, "test.denied"), Some(false));
        assert_eq!(game.player_permission_value(a, "test.other"), None);
        assert!(game.player_has_permission(a, "test.guest12"));
        assert!(!game.player_has_permission(b, "test.guest12"));
        set_permission(game, 13, 2);
        assert!(game.player_has_permission(b, "test.other"));
        assert!(!game.player_has_permission(b, "test.denied"));
        // Inside the player's own turn, as a command or a hook asks.
        game.as_guest(12, |game| {
            assert!(game.player_has_permission(a, "test.guest12"));
            assert!(!game.player_has_permission(a, "test.other"));
            assert!(game.player_has_permission(b, "test.other"));
        })
        .unwrap();
    });
}
