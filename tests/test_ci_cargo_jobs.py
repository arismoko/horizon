"""Build parallelism: a Dyna runner's CPU quota within its memory, fixed elsewhere."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml

spec = importlib.util.spec_from_file_location("cargo_jobs", ROOT / "tools/ci/cargo_jobs.py")
cargo_jobs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_jobs)
GIB = 1 << 30


def steps(workflow: str, job: str) -> dict:
    loaded = yaml.load((ROOT / ".github/workflows" / workflow).read_text(), Loader=yaml.BaseLoader)
    return {step.get("name"): step for step in loaded["jobs"][job]["steps"]}


class CargoJobsTests(unittest.TestCase):
    def test_cpu_quota_bounded_by_memory_with_a_floor_of_four(self):
        cases = [
            ((8, 16 * GIB, 3072), 5),  # dyna-large: memory, not its 8 CPUs, binds
            ((8, 16 * GIB, 4096), 4),
            ((16, 64 * GIB, 3072), 16),  # CPUs bind
            ((2, 4 * GIB, 3072), 4),  # never below the old fixed cap
        ]
        for args, expected in cases:
            with self.subTest(args=args):
                self.assertEqual(cargo_jobs.jobs(*args), expected)

    def test_limits_read_the_runner_cgroup(self):
        with tempfile.TemporaryDirectory() as directory:
            cgroup = Path(directory)
            (cgroup / "cpu.max").write_text("750000 100000\n")
            (cgroup / "memory.max").write_text(f"{16 * GIB}\n")
            self.assertEqual(cargo_jobs.limits(cgroup), (7, 16 * GIB))
            (cgroup / "cpu.max").write_text("max 100000\n")
            (cgroup / "memory.max").write_text("max\n")
            cpus, memory = cargo_jobs.limits(cgroup)
            self.assertEqual(cpus, len(os.sched_getaffinity(0)))
            self.assertEqual(memory, os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE"))
            result = subprocess.run([sys.executable, ROOT / "tools/ci/cargo_jobs.py", "--job-mib", "3072",
                                     "--cgroup", cgroup], capture_output=True, text=True, check=True)
            self.assertEqual(int(result.stdout), cargo_jobs.jobs(cpus, memory, 3072))

    @unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "requires POSIX Bash")
    def test_workflow_steps_route_parallelism_by_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            stubs = Path(directory)
            python = stubs / "python3"
            python.write_text('#!/bin/sh\necho "$*" > "$ARGS"\necho 6\n')
            python.chmod(0o700)
            receipt, args = stubs / "env", stubs / "args"

            def run(script, **env):
                receipt.write_text("")
                args.write_text("")
                subprocess.run(["/bin/bash", "-eu", "-c", script], check=True, capture_output=True,
                               env=dict(PATH=f"{stubs}:/usr/bin:/bin", GITHUB_ENV=str(receipt), ARGS=str(args), **env))
                return receipt.read_text(), args.read_text().strip()

            for job in ("lint", "verify"):
                script = steps("dyna-ci.yml", job)["Choose build parallelism"]["run"]
                for kind, expected in (("dyna", "JOBS=-j6\n"), ("boat", "JOBS=-j4\n"),
                                       ("github", "JOBS=\n"), ("other", "JOBS=\n")):
                    with self.subTest(job=job, kind=kind):
                        written, called = run(script, RUNNER_KIND=kind)
                        self.assertEqual(written, expected)
                        self.assertEqual(called, "tools/ci/cargo_jobs.py --job-mib 3072" if kind == "dyna" else "")

            script = steps("fn-replace-check.yml", "fn-replace-check")["Choose build parallelism"]["run"]
            for environment, expected in (("github-hosted", "CARGO_BUILD_JOBS=2\n"), ("self-hosted", "CARGO_BUILD_JOBS=6\n")):
                with self.subTest(environment=environment):
                    written, called = run(script, RUNNER_ENVIRONMENT=environment)
                    self.assertEqual(written, expected)
                    self.assertEqual(called, "tools/ci/cargo_jobs.py --job-mib 4096" if environment == "self-hosted" else "")


if __name__ == "__main__":
    unittest.main()
