"""The ignored-tests matrix names every leg of a split package, and only those."""
import json
from pathlib import Path
import subprocess
import sys
import unittest

from tests.test_ci_concurrency import yaml
from tools.ci import ignored_shards

ROOT = Path(__file__).resolve().parents[1]
GAMEPLAY_SHARDS = ("sunrise-gameplay-common", "sunrise-gameplay", "sunrise-client-gameplay")


class ShardMatrix(unittest.TestCase):
    def test_a_split_package_becomes_one_entry_per_leg(self):
        self.assertEqual(
            ignored_shards.expand(["sunrise-client", "sunrise-world"]),
            ["sunrise-client#1of2", "sunrise-client#2of2", "sunrise-world"],
        )

    def test_every_shard_is_covered_by_the_full_matrix(self):
        entries = ignored_shards.expand(ignored_shards.SHARDS)
        for package in ignored_shards.SHARDS:
            legs = ignored_shards.LEGS.get(package, 1)
            owned = [entry for entry in entries if entry.split("#")[0] == package]
            self.assertEqual(len(owned), legs)

    def test_a_package_without_legs_keeps_its_plain_name(self):
        self.assertEqual(ignored_shards.expand(["sunrise-items"]), ["sunrise-items"])

    def test_gameplay_crates_are_single_leg_shards_in_the_full_matrix(self):
        printed = subprocess.run(
            [sys.executable, str(ROOT / "tools/ci/ignored_shards.py"), "--matrix", "--all"],
            capture_output=True, text=True, check=True,
        ).stdout
        shards = json.loads(printed.removeprefix("matrix="))["shard"]
        for package in GAMEPLAY_SHARDS:
            self.assertIn(package, ignored_shards.SHARDS)
            self.assertNotIn(package, ignored_shards.LEGS)
            self.assertIn(package, shards)
        self.assertIn("sunrise-client#1of2", shards)
        self.assertIn("sunrise-client#2of2", shards)

    def test_gameplay_crate_edits_select_their_shard_and_dependents(self):
        deps = ignored_shards.workspace_dependencies()
        for package in GAMEPLAY_SHARDS:
            selected = ignored_shards.select([f"crates/{package}/src/lib.rs"], deps)
            self.assertIn(package, selected)
            self.assertIn("sunrise-client", selected)
        self.assertEqual(
            set(GAMEPLAY_SHARDS) <= set(ignored_shards.select(["crates/sunrise-gameplay-common/src/lib.rs"], deps)),
            True,
        )

    def test_ignored_tests_workflow_prepares_witnesses_for_gameplay_shards(self):
        steps = {
            step.get("name"): step
            for step in yaml.safe_load((ROOT / ".github/workflows/ignored-tests.yml").read_text())["jobs"][
                "ignored-tests-shards"
            ]["steps"]
        }
        self.assertIn("sunrise-gameplay-common", steps["Generate the network parity tests' vanilla witnesses"]["if"])
        self.assertIn("sunrise-client-gameplay", steps["Generate the item glint tests' vanilla witness"]["if"])
        self.assertIn("sunrise-gameplay", steps["Generate the fishing test's vanilla reference inputs"]["if"])


if __name__ == "__main__":
    unittest.main()
