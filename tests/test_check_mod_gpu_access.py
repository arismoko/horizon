"""check_mod_gpu_access.py finds `self.renderer` reads outside comments and the test module."""
import unittest

from tools.ci import check_mod_gpu_access as check


class ModGpuAccessTests(unittest.TestCase):
    def test_reads_are_found_and_comments_and_tests_are_not(self):
        source = (
            "fn a(&self) { self.renderer.as_ref(); }\n"
            "// self.renderer in a comment\n"
            "fn b(&self) { self.game\n    .renderer.as_ref(); self.mod_gpu(); }\n"
            "/* self.renderer\n in a block */\n"
            "#[cfg(test)]\nmod tests { fn c(&self) { self.renderer.take(); } }\n"
        )
        self.assertEqual([n for n, _ in check.offenders(source)], [1, 3])

    def test_the_guarded_files_are_clean(self):
        for name in check.GUARDED:
            self.assertEqual(check.offenders((check.ROOT / name).read_text()), [], name)


if __name__ == "__main__":
    unittest.main()
