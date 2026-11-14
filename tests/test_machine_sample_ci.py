"""The sample leg builds the committed pin and removes its scratch folder."""
import os
import unittest
from unittest.mock import patch

from tools.ci import check_machine_sample as gate


class MachineSampleGateTests(unittest.TestCase):
    def test_builds_the_committed_pin_and_cleans_up(self):
        visited = []

        def check(destination, revision):
            self.assertTrue(destination.is_dir())
            self.assertEqual(revision, gate.PIN)
            visited.append(destination)

        with patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}), patch.object(gate, "check", check):
            gate.main()
        self.assertFalse(visited[0].exists())


if __name__ == "__main__":
    unittest.main()
