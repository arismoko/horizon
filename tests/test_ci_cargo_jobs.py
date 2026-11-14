"""Build parallelism: a Dyna runner's CPU quota within its memory, fixed elsewhere."""
import importlib.util
import os
import re
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
        machine_cpus = len(os.sched_getaffinity(0))
        machine_memory = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
        small = min(machine_memory, 16 * GIB)
        cases = [
            # (files under the cgroup root, expected (cpus, memory))
            ({"cpu.max": "750000 100000\n", "memory.max": f"{small}\n"}, (7, small)),  # v2 with a quota
            ({"cpu.max": "max 100000\n", "memory.max": "max\n"}, (machine_cpus, machine_memory)),  # v2, no limit
            ({"cpu/cpu.cfs_quota_us": "200000\n", "cpu/cpu.cfs_period_us": "100000\n",
              "memory/memory.limit_in_bytes": f"{small}\n"}, (2, small)),  # v1 with a quota
            ({"cpu/cpu.cfs_quota_us": "-1\n", "cpu/cpu.cfs_period_us": "100000\n",
              "memory/memory.limit_in_bytes": "9223372036854771712\n"}, (machine_cpus, machine_memory)),  # v1, none
            ({}, (machine_cpus, machine_memory)),  # no cgroup files: a GitHub-hosted job's root cgroup
        ]
        for files, expected in cases:
            with self.subTest(files=sorted(files)), tempfile.TemporaryDirectory() as directory:
                cgroup = Path(directory)
                for name, text in files.items():
                    (cgroup / name).parent.mkdir(parents=True, exist_ok=True)
                    (cgroup / name).write_text(text)
                self.assertEqual(cargo_jobs.limits(cgroup), expected)
                result = subprocess.run([sys.executable, ROOT / "tools/ci/cargo_jobs.py", "--job-mib", "3072",
                                         "--cgroup", cgroup], capture_output=True, text=True, check=True)
                self.assertEqual(int(result.stdout), cargo_jobs.jobs(*expected, 3072))

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

            for job, mib in (("lint", 3072), ("verify", 4096)):
                script = steps("dyna-ci.yml", job)["Choose build parallelism"]["run"]
                for kind, expected in (("dyna", "JOBS=-j6\n"), ("boat", "JOBS=-j4\n"),
                                       ("github", "JOBS=\n"), ("other", "JOBS=\n")):
                    with self.subTest(job=job, kind=kind):
                        written, called = run(script, RUNNER_KIND=kind)
                        self.assertEqual(written, expected)
                        self.assertEqual(called, f"tools/ci/cargo_jobs.py --job-mib {mib}" if kind == "dyna" else "")

            script = steps("fn-replace-check.yml", "fn-replace-check")["Choose build parallelism"]["run"]
            for environment, expected in (("github-hosted", "CARGO_BUILD_JOBS=2\n"), ("self-hosted", "CARGO_BUILD_JOBS=6\n")):
                with self.subTest(environment=environment):
                    written, called = run(script, RUNNER_ENVIRONMENT=environment)
                    self.assertEqual(written, expected)
                    self.assertEqual(called, "tools/ci/cargo_jobs.py --job-mib 4096" if environment == "self-hosted" else "")


    def test_cap_bounds_every_cargo_run_and_test_harness_of_the_job(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory) / "work/repo"
            workspace.mkdir(parents=True)
            env = Path(directory) / "env"
            env.write_text("")
            cargo_jobs.cap(5, 8, workspace, env)
            config = (workspace.parent / ".cargo/config.toml").read_text()
            self.assertIn("[build]\njobs = 5\n", config)
            self.assertEqual(env.read_text(), "RUST_TEST_THREADS=8\nCI_JOBS=5\nCI_CPUS=8\n")

    def test_every_dyna_job_that_builds_caps_its_parallelism_first(self):
        # A runner's nproc reports the whole host: an uncapped build there starts one compiler per
        # host core inside the runner's CPU quota.
        builds = re.compile(r"\bcargo (build|test|check|clippy|rustc|run|doc)\b|vanilla-data\.sh|"
                            r"prebuilt_game\.py|ignored_tests\.py|run_test_binaries\.py|test_\w+_parity\.py")
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            for name, job in (yaml.safe_load(path.read_text()).get("jobs") or {}).items():
                if "dyna" not in str(job.get("runs-on", "")):
                    continue
                runs = [step.get("run") or "" for step in job.get("steps", [])]
                first = next((i for i, run in enumerate(runs) if builds.search(run)), None)
                if first is None:
                    continue
                with self.subTest(workflow=path.name, job=name):
                    capped = [i for i, run in enumerate(runs) if "tools/ci/cargo_jobs.py" in run and "--cap" in run]
                    self.assertTrue(capped and capped[0] < first, "cap parallelism before the first build step")

if __name__ == "__main__":
    unittest.main()
