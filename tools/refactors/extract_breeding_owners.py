#!/usr/bin/env python3
"""Extract breeding owners before the slow-shard behavioral repairs.

Run once on the parent of the repair series. This script only relocates existing
species rules, finalization, and the XP decision; behavioral changes follow in
separate commits.
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLIENT = ROOT / "crates/horizon-client/src"


def take_function(text, marker, closing):
    start = text.index(marker)
    end = text.index(closing, start) + len(closing)
    return text[:start] + text[end:], text[start:end]


path = CLIENT / "mobs.rs"
text = path.read_text()
text, variant = take_function(text, "pub fn draws_parent_variant", "\n}\n")
text, orb = take_function(text, "pub fn orb_after_child", "\n}\n")
text = text.replace(
    "pub use species::Species;",
    "pub use species::Species;\n\nmod breeding;\npub use breeding::{draws_parent_variant, orb_after_child};",
    1,
)
path.write_text(text)

path = CLIENT / "game/mobs/creatures/mod.rs"
text = path.read_text()
text, finalization = take_function(
    text,
    "    /// `Animal.finalizeSpawnChildFromBreeding` before the experience orb.",
    "\n    }\n",
)
text = text.replace("mod borrowed_tick;", "mod borrowed_tick;\nmod breeding;", 1)
path.write_text(text)
finalization = finalization.replace("    fn finalize_breeding(", "    pub(super) fn finalize_breeding(", 1)
(CLIENT / "game/mobs/creatures/breeding.rs").write_text(
    "//! Breeding credit, cooldown and heart finalization.\nuse super::*;\n\n"
    + "impl Game<'_> {\n" + finalization + "}\n"
)

path = CLIENT / "mobs/goals.rs"
text = path.read_text()
start = text.index("            let experience = if (equine.is_some() || parent_variant.is_some())")
end = text.index("            };\n", start) + len("            };\n")
old = text[start:end]
condition = "(equine.is_some() || parent_variant.is_some())"
helper = old.replace("            let experience = ", "    ", 1)
helper = helper.replace(condition, "inherited").replace("cx.w.", "w.")
helper = helper.replace("cx.w.mob_drops", "w.mob_drops")
helper = helper.replace("            };\n", "    }\n", 1)
helper = helper.replace("super::orb_after_child", "orb_after_child")
text = text[:start] + (
    "            let experience = super::breeding::experience(\n"
    "                m, cx.w, equine.is_some() || parent_variant.is_some(),\n"
    "            );\n"
) + text[end:]
path.write_text(text)
(CLIENT / "mobs/breeding.rs").write_text(
    "//! Shared breeding inheritance and parent-stream reward decisions.\n"
    "use super::{Mob, MobWorld, Species};\n\n"
    + variant + "\n" + orb
    + "\npub(super) fn experience(m: &mut Mob, w: &MobWorld, inherited: bool) -> super::BreedExperience {\n"
    + helper + "}\n"
)
