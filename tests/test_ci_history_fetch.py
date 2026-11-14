"""Execute both gate history-fetch steps with controlled Git/network results."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipIf(os.name == "nt", "executes Bash and executable test doubles")
class HistoryFetchTests(unittest.TestCase):
    def test_bounded_fetch_retries_and_fails_closed(self):
        if importlib.util.find_spec("yaml") is None:
            self.skipTest("requires PyYAML")
        import yaml
        workflow = yaml.load((ROOT / ".github/workflows/dyna-ci.yml").read_text(), Loader=yaml.BaseLoader)["jobs"]
        for job, name in [("changes", "Fetch this commit's history for selection"),
                          ("verify-static", "Fetch this commit's history")]:
            step = next(s for s in workflow[job]["steps"] if s.get("name") == name)
            for mode, expected, count in [("success", 0, 1), ("retry", 0, 2),
                                           ("failure", 1, 2), ("complete", 0, 0)]:
                with self.subTest(job=job, mode=mode), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    git = root / "git"
                    git.write_text(f"#!{sys.executable}\n"
                                   "import os, pathlib, sys\n"
                                   "mode = os.environ['MODE']\n"
                                   "if sys.argv[1] == 'rev-parse':\n"
                                   "    print(('false' if mode == 'complete' else 'true') if sys.argv[2] == '--is-shallow-repository' else 'abc123')\n"
                                   "    sys.exit(0)\n"
                                   "p = pathlib.Path(os.environ['CALLS'])\n"
                                   "n = int(p.read_text()) + 1 if p.exists() else 1\n"
                                   "p.write_text(str(n))\n"
                                   "sys.exit(1 if mode == 'failure' or (mode == 'retry' and n == 1) else 0)\n")
                    timeout = root / "timeout"
                    timeout.write_text(f"#!{sys.executable}\n"
                                       "import json, os, sys\n"
                                       "with open(os.environ['LIMITS'], 'a') as f: f.write(json.dumps(sys.argv[1:3]) + '\\n')\n"
                                       "os.execvp(sys.argv[3], sys.argv[3:])\n")
                    git.chmod(0o755)
                    timeout.chmod(0o755)
                    env = dict(os.environ, **step["env"], MODE=mode,
                               PATH=directory + os.pathsep + os.environ["PATH"],
                               CALLS=str(root / "calls"), LIMITS=str(root / "limits"))
                    result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", step["run"]],
                                            env=env, capture_output=True, text=True, timeout=5)
                    self.assertEqual(result.returncode, expected, result.stderr)
                    actual = int((root / "calls").read_text()) if (root / "calls").exists() else 0
                    self.assertEqual(actual, count)
                    limits = (root / "limits").read_text().splitlines() if count else []
                    self.assertEqual([json.loads(line) for line in limits], [["--kill-after=10s", "300s"]] * count)
                    if expected:
                        self.assertIn(f"::error::{name}:", result.stdout)
                        self.assertIn("GitHub failed or timed out twice", result.stdout)
                    else:
                        self.assertNotIn("::error::", result.stdout)


if __name__ == "__main__":
    unittest.main()
