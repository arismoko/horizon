"""A rollup or sync pull request's Discord post: its Summary section, cut to Discord's limits."""
import unittest

from tools.ci import merge_updates as mu

BODY = """> 🤖 Opened by Aria's agent swarm, Dyna, not by Aria.

## Summary
> 🤖 Opened by Aria's agent swarm, Dyna, not by Aria.
Rollup 62: five pull requests.

- #1 one
- #2 two

## Members
not this
"""


class Summary(unittest.TestCase):
    def test_the_section_stops_at_the_next_heading_without_the_disclosure(self):
        self.assertEqual(mu.summary(BODY), "Rollup 62: five pull requests.\n\n- #1 one\n- #2 two")

    def test_no_summary_gives_title_and_link_only(self):
        self.assertEqual(mu.summary("## What and why\ntext"), "")
        self.assertEqual(mu.summary(None), "")
        message = mu.payload("sync", "merged", "Sync 34", "https://example/pr/9", "no heading")
        self.assertEqual(message["embeds"][0], {
            "title": "Merged: Sync 34", "url": "https://example/pr/9", "color": 0x2ECC71,
            "footer": {"text": "Sync merged"},
        })

    def test_a_summary_running_to_the_end_is_kept_whole(self):
        self.assertEqual(mu.summary("## Summary\r\nlast\r\n"), "last")


class Cut(unittest.TestCase):
    def test_short_text_is_unchanged(self):
        self.assertEqual(mu.cut("short", 10), "short")

    def test_long_text_ends_at_a_paragraph_break_within_the_limit(self):
        text = "a" * 3000 + "\n\n" + "b" * 1000
        out = mu.cut(text, 3500)
        self.assertEqual(out, "a" * 3000 + "…")
        self.assertLessEqual(len(out), 3500)

    def test_without_a_break_it_still_fits(self):
        out = mu.cut("x" * 300, 250)
        self.assertEqual(len(out), 250)
        self.assertTrue(out.endswith("…"))


class Payload(unittest.TestCase):
    def test_opened_rollup_is_blue_pings_nobody_and_keeps_limits(self):
        message = mu.payload("rollup", "opened", "t" * 400, "https://example/pr/1",
                             "## Summary\n" + ("p" * 2000 + "\n\n") * 3)
        embed = message["embeds"][0]
        self.assertEqual(message["allowed_mentions"], {"parse": []})
        self.assertEqual(message["flags"], 4096)
        self.assertEqual(embed["color"], 0x3498DB)
        self.assertEqual(embed["footer"], {"text": "Rollup opened"})
        self.assertTrue(embed["title"].startswith("Opened: ttt"), embed["title"])
        self.assertLessEqual(len(embed["title"]), 250)
        self.assertLessEqual(len(embed["description"]), 3500)
        self.assertEqual(embed["description"], "p" * 2000 + "…")



class TablesBecomeLists(unittest.TestCase):
    def test_a_table_is_posted_as_list_lines_without_its_header(self):
        body = "## Summary\nMembers:\n\n| PR | Author |\n| --- | :-: |\n| #1 | ann |\n| #2 | bo |\n\nAfter.\n"
        self.assertEqual(mu.summary(body), "Members:\n\n- #1 · ann\n- #2 · bo\n\nAfter.")

    def test_a_line_that_only_contains_a_bar_is_kept(self):
        self.assertEqual(mu.summary("## Summary\na | b\n"), "a | b")


if __name__ == "__main__":
    unittest.main()
