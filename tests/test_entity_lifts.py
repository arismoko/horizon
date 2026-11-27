"""Entity-list whole-pass lifts go through their observable boundary."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIELDS = (
    "mobs|dropped|orbs|boats|carts|armor_stands|item_frames|paintings|tnts|"
    "falling|rockets|tridents|crystals|creatures\\.projectiles|others|"
    "fangs\\.list|ominous_spawners|breath_clouds"
)
RAW_TAKE = re.compile(
    r"(?:std::)?mem::take\s*\(\s*&mut\s+"
    r"(?:self|game|lists)\.(?:common\.)?(?:" + FIELDS + r")\s*\)"
    r"|std::mem::take\(&mut self\$\(\.\$list\)\+\)"
    r"|std::mem::take\(\s*projectiles\s*\)"
)


class EntityLifts(unittest.TestCase):
    def test_game_entity_lists_have_no_raw_takes(self):
        violations = []
        for crate in ("sunrise-gameplay-common", "sunrise-gameplay", "sunrise-client-gameplay"):
            source = ROOT / "crates" / crate / "src"
            paths = ([source / "game.rs"] if (source / "game.rs").is_file() else []) + sorted((source / "game").rglob("*.rs"))
            for path in paths:
                text = path.read_text()
                for match in RAW_TAKE.finditer(text):
                    line = text.count("\n", 0, match.start()) + 1
                    violations.append(f"{path.relative_to(ROOT)}:{line}")
        self.assertEqual(violations, [], "use the entity list's lift boundary")

    def test_guard_catches_direct_and_generated_entity_lists(self):
        for text in (
            "std::mem::take(&mut self.mobs)",
            "std::mem::take(&mut self.common.mobs)",
            "mem::take(\n &mut game.creatures.projectiles\n)",
            "std::mem::take(&mut self$(.$list)+)",
            "std::mem::take(projectiles)",
        ):
            with self.subTest(text=text):
                self.assertIsNotNone(RAW_TAKE.search(text))
        for text in ("std::mem::take(&mut self.mobs[i].outbox)",
                     "std::mem::take(&mut menu.dropped)"):
            with self.subTest(text=text):
                self.assertIsNone(RAW_TAKE.search(text))
