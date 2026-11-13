"""The parallel test-binary runner lists Cargo's test executables and reports every failure."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("run_test_binaries", ROOT / "tools/ci/run_test_binaries.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def artifact(executable, test=True, name="horizon-world", kind="lib"):
    return json.dumps({
        "reason": "compiler-artifact", "package_id": f"path+file:///repo/crates/{name}#0.1.0",
        "manifest_path": f"/repo/crates/{name}/Cargo.toml", "profile": {"test": test},
        "target": {"name": name, "kind": [kind], "src_path": f"/repo/crates/{name}/src/lib.rs"},
        "executable": executable,
    })


class RunTestBinariesTests(unittest.TestCase):
    def test_package_identity_reads_both_id_forms(self):
        self.assertEqual(runner.package_identity("horizon-world 0.1.0 (path+file:///repo/crates/horizon-world)"),
                         ("horizon-world", "0.1.0"))
        self.assertEqual(runner.package_identity("path+file:///repo/crates/horizon-world#0.1.0"),
                         ("horizon-world", "0.1.0"))
        self.assertEqual(runner.package_identity("path+file:///repo/docs/modding/examples#horizon-modding-examples@0.0.0"),
                         ("horizon-modding-examples", "0.0.0"))

    def test_only_test_executables_each_once_with_their_package_environment(self):
        messages = "\n".join([
            artifact("/t/a"), artifact("/t/a"), artifact("/t/b", test=False), artifact(None),
            json.dumps({"reason": "build-finished", "success": True}), "not json",
        ])
        binaries = runner.test_binaries(messages)
        self.assertEqual([b["executable"] for b in binaries], ["/t/a"])
        self.assertEqual(binaries[0]["cwd"], "/repo/crates/horizon-world")
        self.assertEqual(binaries[0]["env"]["CARGO_PKG_NAME"], "horizon-world")
        self.assertEqual(binaries[0]["env"]["CARGO_CRATE_NAME"], "horizon_world")

    def fake_toolchain(self, root: Path, binaries: list[Path]) -> Path:
        """A cargo that lists `binaries` (and records its arguments) and a rustc beside it."""
        listing = [artifact(str(binary)).replace("/repo", str(root)) for binary in binaries]
        cargo = root / "cargo"
        cargo.write_text(f'#!/bin/sh\necho "$@" > {root}/cargo-args\ncat <<\'EOF\'\n' + "\n".join(listing) + "\nEOF\n")
        rustc = root / "rustc"
        rustc.write_text(f'#!/bin/sh\ncase "$1" in --print) echo {root}/sysroot ;; *) echo "host: x86_64-unknown-linux-gnu" ;; esac\n')
        for tool in (cargo, rustc):
            tool.chmod(0o755)
        return cargo

    def run_main(self, *argv: str) -> tuple[int, str]:
        with contextlib.redirect_stdout(io.StringIO()) as log:
            code = runner.main(list(argv))
        return code, log.getvalue()

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_every_binary_runs_and_each_failure_is_listed_at_the_end(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "crates/horizon-world"
            package.mkdir(parents=True)
            passing, failing, unstartable = root / "passing", root / "failing", root / "unstartable"
            passing.write_text('#!/bin/sh\necho "cwd=$PWD pkg=$CARGO_PKG_NAME args=$*"\n')
            failing.write_text("#!/bin/sh\necho 'test a::b ... ok'\necho 'test a::c ... FAILED'\nexit 101\n")
            unstartable.write_text("#!/bin/sh\nexit 0\n")  # Not executable: the binary cannot start.
            for binary in (passing, failing):
                binary.chmod(0o755)
            cargo = self.fake_toolchain(root, [unstartable, passing, failing])
            code, output = self.run_main("--jobs", "2", "--", str(cargo), "test", "--workspace", "--", "--ignored")
            self.assertEqual(code, 1)
            # Arguments after cargo's own -- go to every binary, not to the listing.
            self.assertEqual((root / "cargo-args").read_text().split(),
                             ["test", "--workspace", "--no-run", "--message-format=json-render-diagnostics"])
            self.assertIn(f"cwd={package} pkg=horizon-world args=--ignored", output)
            self.assertIn("ran 3 test binaries", output)
            # std's directory beside the toolchain: a proc-macro crate's test binary links it dynamically.
            self.assertIn(f"{root}/sysroot/lib/rustlib/x86_64-unknown-linux-gnu/lib", output)
            summary = output[output.index("FAILED TESTS"):].splitlines()
            self.assertIn("::error::horizon-world lib.rs (failing): exit 101", summary)
            self.assertIn("    a::c", summary)
            self.assertNotIn("    a::b", summary)
            self.assertTrue(any(line.startswith("::error::horizon-world lib.rs (unstartable): could not start")
                                for line in summary), summary)
            self.assertFalse(any("(passing)" in line for line in summary))

    def test_report_reads_each_result_and_the_time_report_time_adds(self):
        output = tempfile.TemporaryFile("w+")
        output.write("running 4 tests\ntest a::fast ... ok <0.012s>\ntest a::slow ... ok <6.250s>\n"
                     "test a::bad ... FAILED <0.100s>\ntest a::panics - should panic ... ok <2.500s>\n"
                     "test a::plain ... FAILED\nsome output test a::x ... ok <9.000s> in a payload\n")
        result = {"binary": {"label": "pkg lib.rs (pkg-1)"}, "output": output, "seconds": 1.0, "max_rss_kib": 0}
        with contextlib.redirect_stdout(io.StringIO()):
            failed, times = runner.report(result)
        self.assertEqual(failed, ["a::bad", "a::plain"])
        self.assertEqual(times, [(0.012, "a::fast"), (6.25, "a::slow"), (0.1, "a::bad"), (2.5, "a::panics")])

    def test_a_test_over_the_limit_fails_by_name_and_one_over_the_target_warns(self):
        errors, warnings = runner.timing_verdict(
            [(0.5, "a::fast", "pkg"), (5.0, "a::at_the_line", "pkg"), (5.01, "a::over", "pkg"), (2.2, "a::slowish", "pkg")],
            2.0, 5.0)
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith("pkg: a::over took 5.01s"))
        self.assertIn('#[ignore = "daily"]', errors[0])
        self.assertEqual([warning.split(" took")[0] for warning in warnings], ["pkg: a::at_the_line", "pkg: a::slowish"])

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_time_limits_add_report_time_and_fail_the_run_on_a_slow_test(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates/horizon-world").mkdir(parents=True)
            binary = root / "timed"
            binary.write_text('#!/bin/sh\necho "args=$* bootstrap=$RUSTC_BOOTSTRAP"\n'
                              "echo 'test a::slow ... ok <7.500s>'\necho 'test a::fast ... ok <0.001s>'\n")
            binary.chmod(0o755)
            cargo = self.fake_toolchain(root, [binary])
            code, output = self.run_main("--jobs", "1", "--time-limits", "2,5", "--", str(cargo), "test")
            self.assertEqual(code, 1)
            self.assertIn("args=-Z unstable-options --report-time bootstrap=1", output)
            self.assertIn("::error::horizon-world lib.rs (timed): a::slow took 7.50s", output)
            self.assertIn("2 tests timed, 7.5 test-seconds in total; slowest: a::slow 7.50s, a::fast 0.00s", output)
            # Without the option nothing is added and the times are not judged.
            code, output = self.run_main("--jobs", "1", "--", str(cargo), "test")
            self.assertEqual(code, 0)
            self.assertIn("args= bootstrap=\n", output)

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_a_listing_without_test_binaries_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            cargo = self.fake_toolchain(Path(directory), [])
            code, output = self.run_main("--jobs", "2", "--", str(cargo), "test")
            self.assertEqual(code, 1)
            self.assertIn("cargo listed no test binaries", output)


if __name__ == "__main__":
    unittest.main()
