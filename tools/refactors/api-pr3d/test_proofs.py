"""Focused negative tests for the move and record gates."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from gate import EVENTS, records
from move import rewrite
from moved_lines_check import apply_seam, hand_lines, lines


class ProofTests(unittest.TestCase):
    def test_rewrites_only_named_render_paths(self):
        source = "mod_api::render::Vertex mod_api::render as native_render mod_api::WorldDepth"
        self.assertEqual(
            rewrite(source, "mod_render"),
            "mod_render::render::Vertex mod_render::render as native_render mod_api::WorldDepth",
        )

    def test_multiset_preserves_duplicates_and_comments(self):
        self.assertEqual(lines("use x;\nvalue\nvalue\n// retained"),
                         lines("// retained\nvalue\nvalue\npub use y;"))
        self.assertNotEqual(lines("value\nvalue"), lines("value"))
        self.assertNotEqual(lines("// before"), lines("// after"))

    def test_declared_seam_checks_occurrences(self):
        edit = [{"old": "old", "new": "new", "count": 1}]
        self.assertEqual(apply_seam("old", edit), "new")
        with self.assertRaises(ValueError):
            apply_seam("old old", edit)
        self.assertEqual(hand_lines("a\nb", "a\nc"), 2)

    def test_records_refuse_missing_duplicates_and_field_changes(self):
        source = "\n".join(f"pub struct {name}<'a> {{ x: &'a u32 }}" for name in EVENTS)
        self.assertEqual(set(records(source)), set(EVENTS))
        with self.assertRaises(ValueError):
            records("")
        with self.assertRaises(ValueError):
            records(source + source)
        self.assertNotEqual(records(source), records(source.replace("u32", "u64")))
        attributed = source.replace("pub struct", "#[non_exhaustive]\n#[derive(Debug)]\npub struct")
        self.assertNotEqual(records(source), records(attributed))
        self.assertNotEqual(records(attributed), records(attributed.replace("Debug", "Clone")))

    def test_records_refuse_multiline_and_comment_separated_attributes(self):
        source = "\n".join(f"#[allow(missing_docs)]\npub struct {name} {{ x: u32 }}"
                           for name in EVENTS)
        records(source)
        spaced = source.replace("#[allow(missing_docs)]", "#[cfg(any())]\n \n  #[allow(missing_docs)]", 1)
        self.assertNotEqual(records(source), records(spaced))
        for prefix in ("#[cfg(\n    any()\n)]\n", "#[cfg(any())]\n// separated\n",
                       "#[cfg(any())]\n/* separated */\n"):
            mutated = source.replace("#[allow(missing_docs)]", prefix + "#[allow(missing_docs)]", 1)
            with self.subTest(prefix=prefix), self.assertRaises(ValueError):
                records(mutated)

    def test_cli_refuses_inner_attribute_mutations(self):
        source = "\n".join(f"pub struct {name} {{ x: u32 }}" for name in EVENTS)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            before, after, unchanged = (root / name for name in ("before.rs", "after.rs", "same"))
            before.write_text(source)
            unchanged.write_text("")
            command = [sys.executable, str(Path(__file__).with_name("gate.py")),
                       "--before-render", str(before), "--after-render", str(after)]
            for name in ("wit", "host"):
                command += [f"--before-{name}", str(unchanged), f"--after-{name}", str(unchanged)]
            after.write_text(source)
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
            for prefix in ("#!", "# !", "#\n!", "#! /* gap */ ",
                           "# /* gap */ ! // gap\n", "# // gap\n !\n"):
                after.write_text(prefix + "[cfg(any())]\n" + source)
                with self.subTest(prefix=prefix):
                    result = subprocess.run(command, capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("inner attributes are unsupported", result.stderr)
                    self.assertNotIn("PASS:", result.stdout)


if __name__ == "__main__":
    unittest.main()
