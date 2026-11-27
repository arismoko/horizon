"""check_mod_gpu_access.py finds `self.renderer` reads outside comments and the test module."""
import unittest

from tools.ci import check_mod_gpu_access as check


class ModGpuAccessTests(unittest.TestCase):
    def test_reads_are_found_and_comments_and_tests_are_not(self):
        source = (
            "fn a(&self) { self.renderer.as_ref(); }\n"
            "// self.renderer in a comment\n"
            "fn b(&self) { self.game\n    .renderer.as_ref(); self.mod_gpu(); }\n"
            "fn c(&self) { self.client.renderer.as_ref(); }\n"
            "fn d(&self) { self.game.client\n    .renderer.as_ref(); }\n"
            "/* self.client.renderer\n in a block */\n"
            "#[cfg(test)]\nmod tests { fn e(&self) { self.client.renderer.take(); } }\n"
        )
        self.assertEqual([n for n, _ in check.offenders(source)], [1, 3, 5, 6])

    def test_test_import_does_not_hide_production_before_test_support_module(self):
        source = (
            "#[cfg(test)]\nuse mod_api::prelude::*;\n"
            "fn draw(&self) { self.client.renderer.take(); }\n"
            '#[cfg(any(test, feature = "test-support"))]\n'
            "pub mod tests { fn setup(&self) { self.client.renderer.take(); } }\n"
        )
        self.assertEqual([n for n, _ in check.offenders(source)], [3])

    def test_the_guarded_files_are_clean(self):
        for name in check.GUARDED:
            self.assertEqual(check.offenders((check.ROOT / name).read_text()), [], name)


if __name__ == "__main__":
    unittest.main()
