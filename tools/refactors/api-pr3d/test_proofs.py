"""Focused negative tests for the move and record gates."""
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


if __name__ == "__main__":
    unittest.main()
