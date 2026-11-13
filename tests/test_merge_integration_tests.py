"""The integration-test merge script refuses a crate whose own tests/main.rs it would overwrite."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("merge_integration_tests", ROOT / "tools/refactors/merge_integration_tests.py")
script = importlib.util.module_from_spec(spec)
spec.loader.exec_module(script)


class MergeIntegrationTests(unittest.TestCase):
    def test_a_crate_tests_main_is_refused_before_anything_moves(self):
        with tempfile.TemporaryDirectory() as directory:
            crate = Path(directory)
            (crate / "tests").mkdir()
            (crate / "tests/main.rs").write_text("#[test]\nfn kept() {}\n")
            (crate / "tests/other.rs").write_text("#[test]\nfn other() {}\n")
            with patch("sys.argv", ["merge", str(crate)]), patch.object(script.subprocess, "run") as run:
                with self.assertRaisesRegex(SystemExit, "keep it with --keep main"):
                    script.main()
            run.assert_not_called()
            self.assertFalse((crate / "tests/integration").exists())
            self.assertEqual((crate / "tests/main.rs").read_text(), "#[test]\nfn kept() {}\n")


if __name__ == "__main__":
    unittest.main()
