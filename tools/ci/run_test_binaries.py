"""Run a workspace's built test binaries several at a time instead of one by one.

    python3 tools/ci/run_test_binaries.py --jobs 3 -- cargo test $JOBS --profile test --workspace --lib --bins --tests

The cargo command (without anything after its own `--`) is run once more with `--no-run
--message-format=json-render-diagnostics` to list the test executables, already built, so nothing compiles. Each
binary runs from its package's directory with the arguments after `--`, the package's main CARGO_* variables
(manifest dir and path, package name and version, crate name), CARGO, and a library path with the target's and the
toolchain's libraries; this is close to `cargo test`'s environment, not all of it. Output goes to a temporary file per
binary and is printed whole when the binary ends. Binaries start largest first, since the largest are the slow ones.
Every binary runs; one that fails, crashes, is killed or cannot start fails the run, and the run ends with a list of
the failing binaries and their failed tests' names. A listing with no test binaries fails too.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


def package_identity(package_id: str) -> tuple[str, str]:
    """(name, version) from a Cargo package id, old (`name 1.0 (path+...)`) or new (`path+...#name@1.0`)."""
    if found := re.fullmatch(r"(\S+) (\S+) \(.*\)", package_id):
        return found[1], found[2]
    fragment = package_id.rsplit("#", 1)[-1]
    name, _, version = fragment.rpartition("@")
    if not name:  # `path+file:///x/horizon-world#0.1.0`: the name is the last path segment.
        name = package_id.split("#", 1)[0].rstrip("/").rsplit("/", 1)[-1]
    return name, version


def test_binaries(messages: str) -> list[dict]:
    """The test executables Cargo reported, each once, with what running it needs."""
    binaries, seen = [], set()
    for line in messages.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message["profile"]["test"]:
            continue
        executable = message.get("executable")
        if not executable or executable in seen:
            continue
        seen.add(executable)
        name, version = package_identity(message["package_id"])
        manifest = message["manifest_path"]
        binaries.append({
            "executable": executable,
            "label": f"{name} {Path(message['target']['src_path']).name} ({Path(executable).name})",
            "cwd": str(Path(manifest).parent),
            "env": {
                "CARGO_MANIFEST_DIR": str(Path(manifest).parent),
                "CARGO_MANIFEST_PATH": manifest,
                "CARGO_PKG_NAME": name,
                "CARGO_PKG_VERSION": version,
                "CARGO_CRATE_NAME": message["target"]["name"].replace("-", "_"),
            },
        })
    return binaries


def run_one(binary: dict, extra_env: dict, test_args: list[str]) -> dict:
    """Run one test binary with its output in a temporary file, never raising: a binary that cannot start fails."""
    result = {"binary": binary, "output": tempfile.TemporaryFile("w+", errors="replace"), "max_rss_kib": 0}
    started = time.monotonic()
    try:
        process = subprocess.Popen([binary["executable"], *test_args], cwd=binary["cwd"],
                                   env={**os.environ, **extra_env, **binary["env"]},
                                   stdout=result["output"], stderr=subprocess.STDOUT)
    except OSError as error:
        result.update(status=f"could not start ({error.strerror or error})", seconds=0.0)
        return result
    _, wait_status, usage = os.wait4(process.pid, 0)
    process.returncode = code = os.waitstatus_to_exitcode(wait_status)
    status = "ok" if code == 0 else f"killed by signal {-code}" if code < 0 else f"exit {code}"
    result.update(status=status, seconds=time.monotonic() - started, max_rss_kib=usage.ru_maxrss)
    return result


def report(result: dict) -> list[str]:
    """Print one finished binary's output, streamed from its file, and return its failed test names."""
    print(f"     Running {result['binary']['label']} [{result['seconds']:.1f}s, "
          f"max RSS {result['max_rss_kib'] // 1024} MiB]")
    output, failed_tests = result["output"], []
    output.seek(0)
    for line in output:
        print(line, end="")
        if found := re.match(r"test (\S+) \.\.\. FAILED$", line.rstrip("\n")):
            failed_tests.append(found[1])
    output.close()
    print(flush=True)
    return failed_tests


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--jobs", type=int, required=True, help="test binaries running at once")
    parser.add_argument("cargo", nargs=argparse.REMAINDER,
                        help="the cargo test command, after --; arguments after its own -- go to every test binary")
    args = parser.parse_args(argv)
    command = args.cargo[1:] if args.cargo[:1] == ["--"] else args.cargo
    command, test_args = (command[:command.index("--")], command[command.index("--") + 1:]) if "--" in command \
        else (command, [])
    listing = subprocess.run([*command, "--no-run", "--message-format=json-render-diagnostics"],
                             stdout=subprocess.PIPE, text=True)
    if listing.returncode:
        print(f"::error::listing the test binaries failed (cargo exit {listing.returncode})")
        return listing.returncode
    binaries = sorted(test_binaries(listing.stdout), key=lambda b: os.path.getsize(b["executable"]) if os.path.exists(b["executable"]) else 0,
                      reverse=True)
    if not binaries:
        print("::error::cargo listed no test binaries: nothing would be tested")
        return 1
    # The toolchain's std and the target's dynamic libraries on the library path (a proc-macro crate's test binary
    # links std dynamically), and CARGO set to the toolchain's cargo, as `cargo test` sets them.
    toolchain = [command[1]] if len(command) > 1 and command[1].startswith("+") else []
    rustc = Path(shutil.which(command[0]) or "").with_name("rustc")
    rustc = str(rustc) if rustc.is_file() else shutil.which("rustc")
    if rustc is None:
        print("::error::no rustc beside cargo or on PATH: the toolchain's std cannot be put on the library path")
        return 1
    sysroot = subprocess.run([rustc, *toolchain, "--print", "sysroot"], capture_output=True, text=True,
                             check=True).stdout.strip()
    host = re.search(r"^host: (\S+)$", subprocess.run([rustc, *toolchain, "-vV"], capture_output=True, text=True,
                                                     check=True).stdout, re.M)[1]
    deps = {str(Path(b["executable"]).parent) for b in binaries}
    library_path = os.pathsep.join([*sorted(deps), f"{sysroot}/lib/rustlib/{host}/lib", f"{sysroot}/lib",
                                    *filter(None, [os.environ.get("LD_LIBRARY_PATH")])])
    toolchain_cargo = Path(sysroot, "bin", "cargo")
    print(f"library path: {library_path}", flush=True)
    extra_env = {"LD_LIBRARY_PATH": library_path,
                 "CARGO": str(toolchain_cargo) if toolchain_cargo.is_file() else os.environ.get("CARGO", "cargo")}
    print(f"running {len(binaries)} test binaries, {args.jobs} at a time", flush=True)
    failures, peaks, started = [], [], time.monotonic()
    with ThreadPoolExecutor(args.jobs) as pool:
        for result in pool.map(lambda b: run_one(b, extra_env, test_args), binaries):
            failed_tests = report(result)
            peaks.append((result["max_rss_kib"], result["binary"]["label"]))
            if result["status"] != "ok":
                failures.append((result["binary"]["label"], result["status"], failed_tests))
    print(f"ran {len(binaries)} test binaries in {time.monotonic() - started:.1f}s; {len(failures)} failed")
    print("largest resident sets: " + ", ".join(f"{label} {kib // 1024} MiB" for kib, label in sorted(peaks)[-5:][::-1]))
    if failures:
        # At the end, where a log tail shows it: every failing binary and its failed tests' names.
        print("FAILED TESTS")
        for label, status, failed_tests in failures:
            print(f"::error::{label}: {status}")
            for name in failed_tests:
                print(f"    {name}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
