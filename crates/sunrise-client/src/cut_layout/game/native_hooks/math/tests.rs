use mod_api::{Host, MathProfile};

fn assert_platform_profile(actual: MathProfile) {
    // Independent literal expectations, including Windows ARM's precedence.
    #[cfg(target_arch = "aarch64")]
    assert_eq!(actual, MathProfile::Aarch64Fma);
    #[cfg(target_arch = "x86_64")]
    assert_eq!(actual, MathProfile::HotSpotX86_64);
    #[cfg(all(windows, not(any(target_arch = "aarch64", target_arch = "x86_64"))))]
    assert_eq!(actual, MathProfile::CorrectlyRounded);
    #[cfg(all(
        not(windows),
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    assert_eq!(actual, MathProfile::LibmRounding);
}

#[test]
fn math_profile_selection_matches_platform_and_is_stable() {
    let first = sunrise_gameplay_common::game::native_hooks::math::selected_profile();
    assert_platform_profile(first);
    for _ in 0..128 {
        assert_eq!(
            sunrise_gameplay_common::game::native_hooks::math::selected_profile(),
            first
        );
    }
}

#[test]
#[ignore = "daily"]
fn game_host_math_profile_matches_platform_and_is_stable() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let host: &dyn Host = game;
        let first = host.math_profile();
        assert_platform_profile(first);
        for _ in 0..128 {
            assert_eq!(host.math_profile(), first);
        }
    });
}
