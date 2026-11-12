"""One real positive stage/export proof, run before documentation checks in verify.

Kept outside tests/ discovery so the later offline suite cannot stage twice.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("nightly_catalog", ROOT / "tools/ci/nightly_catalog.py")
CATALOG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CATALOG)


class StageCatalog(unittest.TestCase):
    def test_actual_pinned_export_from_clean_checkout(self):
        self.assertTrue(shutil.which("cargo") and shutil.which("rustc"), "real stage requires Rust")
        subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=ROOT, check=True)
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        (ROOT / "target").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as temporary:
            output = Path(temporary) / "mod-catalog.json"
            stage_tool = os.environ.get("HORIZON_STAGE_TOOL")
            CATALOG.export(ROOT, commit, output, stage_tool=Path(stage_tool) if stage_tool else None)
            document = json.loads(output.read_text())
            self.assertEqual(document["schema_version"], 1)
            hook = next(hook for hook in document["hooks"] if hook["id"] == "use_item")
            self.assertIn("hand", [argument["name"] for argument in hook["arguments"]])
            self.assertTrue(document["points"])
            self.assertFalse((ROOT / ".horizon-source-staged").exists())


if __name__ == "__main__":
    unittest.main()
