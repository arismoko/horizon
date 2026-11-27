use mod_api::TriState;

const ME: [i32; 4] = [1, 2, 3, 4];
const OTHER: [i32; 4] = [5, 6, 7, 8];

/// Whether `ME` takes an item with `delay` and `target` given `answer`.
fn decide(delay: i32, target: Option<[i32; 4]>, answer: TriState) -> bool {
    crate::game::ServerGame::pickup_decision(delay, target, ME, answer)
}

#[test]
fn no_opinion_keeps_vanilla_rule() {
    assert!(decide(0, None, TriState::Default));
    assert!(decide(0, Some(ME), TriState::Default));
    assert!(!decide(-5, None, TriState::Default));
    assert!(!decide(-5, Some(ME), TriState::Default));
    assert!(!decide(-5, Some(OTHER), TriState::Default));
    assert!(!decide(0, Some(OTHER), TriState::Default));
    assert!(!decide(40, None, TriState::Default));
    assert!(!decide(40, Some(ME), TriState::Default));
}

#[test]
fn allow_overrides_delay_and_target() {
    assert!(decide(-5, None, TriState::True));
    assert!(decide(40, None, TriState::True));
    assert!(decide(0, Some(OTHER), TriState::True));
    assert!(decide(32767, Some(OTHER), TriState::True));
}

#[test]
fn deny_vetoes_only_the_current_attempt() {
    assert!(!decide(0, None, TriState::False));
    assert!(!decide(3, Some(ME), TriState::False));
    assert!(!decide(40, None, TriState::False));
}
