#!/usr/bin/env python3
"""Keep the original engine brewing fixture and move callback contracts to API tests.

The callback implementations move to brewing_dispatch_tests.rs with owned stack
construction and key reads. No recipe matching or brewing implementation moves.
"""
from pathlib import Path

path = Path("crates/horizon-client-render/src/brewing.rs")
text = path.read_text()
start = text.index("    use mod_api::Flow;")
fixture_start = text.index("        let set = |ids: &[&str]|", start)
fixture_end = text.index("        let mut host = NoHost;", fixture_start)
end = text.index("    #[test]\n    fn infinite_effects", fixture_end)
engine_test = '''    /// Cancelled pre-hook slots replace the brew without consuming an ingredient.
    /// Uncancelled pre-hook edits are discarded before vanilla brewing runs.
    #[test]
    fn a_cancelled_brew_takes_the_mods_slots_and_an_uncancelled_one_brews() {
'''
engine_test += text[fixture_start:fixture_end]
engine_test += '''        for cancel in [true, false] {
            let mut stand = fresh();
            let mut edited = stand.items.clone();
            edited[0] = Some(ItemStack::new("minecraft:diamond", 1));
            let mut out = BrewTick::default();
            stand.brew_after_pre(Some((edited, cancel)), &brewing, &items, &mut out);
            assert_eq!(out.brewed, !cancel);
            assert_eq!(stand.items[0], Some(if cancel {
                ItemStack::new("minecraft:diamond", 1)
            } else {
                awkward.clone()
            }));
            assert!(stand.items[1].is_none());
            assert_eq!(stand.items[3].as_ref().map(|s| s.count), Some(if cancel { 2 } else { 1 }));
        }
    }

'''
path.write_text(text[:start] + engine_test + text[end:])
