/// The window's key path: only a key's printable characters go on to the
/// hook and the text box.
#[test]
fn a_key_offers_only_its_printable_characters() {
    // Backspace, Escape and the like are keys, not text.
    assert_eq!(super::chars_to_type("a\u{7f}\u{1b}b", true), "ab");
    // Nothing typed: nothing to hand over.
    assert_eq!(super::chars_to_type("", true), "");
    // Ctrl+A is a shortcut: its `a` reaches no hook and no box.
    assert_eq!(super::chars_to_type("a", false), "");
}
