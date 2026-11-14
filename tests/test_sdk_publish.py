"""The mod SDK is published in one file, sdk-publish.yml, which every publisher calls (#4022, #4108)."""
import unittest

from tests.test_ci_concurrency import ROOT, yaml

WORKFLOWS = ROOT / ".github/workflows"


def load(name):
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


class SdkPublishTests(unittest.TestCase):
    def test_only_sdk_publish_releases_an_sdk_and_every_caller_hands_it_the_key(self):
        publishers = sorted(path.name for path in WORKFLOWS.glob("*.yml") if "Release sdk-<sha>" in path.read_text())
        self.assertEqual(publishers, ["sdk-publish.yml"])
        callers = {}
        for path in WORKFLOWS.glob("*.yml"):
            for name, job in load(path.name)["jobs"].items():
                if job.get("uses") == "./.github/workflows/sdk-publish.yml":
                    callers[path.name] = job
        self.assertEqual(sorted(callers), ["daily.yml", "prebuilt-game.yml", "release.yml"])
        for name, job in callers.items():
            with self.subTest(caller=name):
                self.assertEqual(job["secrets"], "inherit")
                self.assertEqual(job["permissions"]["contents"], "write")
                self.assertEqual(set(job["with"]), {"commit", "publish"})

    def test_a_snapshot_publishes_its_sdk_only_when_it_is_no_dry_run(self):
        job = load("release.yml")["jobs"]["snapshot-sdk-publish"]
        self.assertEqual(job["with"]["publish"], "${{ needs.decide.outputs.dry_run != 'true' }}")
        self.assertIn("needs.snapshot-sdk.outputs.needed == 'true'", job["if"])


if __name__ == "__main__":
    unittest.main()
