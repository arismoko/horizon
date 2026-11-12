#!/usr/bin/env python3
"""Count marginal instructions for 700 warmed get/set pairs on the Game API."""
import json
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
env = dict(os.environ, CARGO_INCREMENTAL="0", HORIZON_TEST_RESOURCES=str(root / ".local/resources/26.3"), VALGRIND_LIB=str(root / ".local/valgrind/usr/libexec/valgrind"))
home = tempfile.TemporaryDirectory(prefix="attachment-bench-")
for key in ["HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"]:
    env[key] = home.name
# Cache only compilation, never the measurement. Include every Rust/Cargo input,
# including untracked new source files, and verify the executable's contents.
inputs = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"]).decode().split("\0")
h = hashlib.sha256()
for name in sorted(set(inputs)):
    if name.endswith((".rs", ".toml", ".lock")) and (root / name).is_file():
        h.update(name.encode())
        h.update((root / name).read_bytes())
cache = root / ".local/attachment-build.json"
old = json.loads(cache.read_text()) if cache.exists() else {}
binary = old.get("binary", "")
if old.get("source") != h.hexdigest() or not Path(binary).is_file() or old.get("executable") != hashlib.sha256(Path(binary).read_bytes()).hexdigest():
    p = subprocess.run(["cargo", "test", "--profile", "fast", "-p", "horizon-client", "--lib", "attachment_instruction_count", "--no-run", "--message-format=json"], env=env, text=True, stdout=subprocess.PIPE, check=True)
    artifacts = [json.loads(line) for line in p.stdout.splitlines() if line.startswith("{")]
    binary = next(a["executable"] for a in artifacts if a.get("executable") and a.get("profile", {}).get("test"))
    cache.write_text(json.dumps(dict(source=h.hexdigest(), binary=binary, executable=hashlib.sha256(Path(binary).read_bytes()).hexdigest())))
counts = []
for passes in [10, 20]:
    mode = "handle" if os.environ.get("ATTACHMENT_HANDLE") else "holder"
    output = root / f".local/{mode}-{passes}.callgrind"
    run = subprocess.run([str(root / ".local/valgrind/usr/bin/valgrind"), "--tool=callgrind", "--collect-atstart=no", f"--toggle-collect=*{'attachment_handle_measured_passes' if os.environ.get('ATTACHMENT_HANDLE') else 'attachment_measured_passes'}*", f"--callgrind-out-file={output}", binary, "game::attachment_instruction_bench::attachment_instruction_count", "--ignored", "--exact", "--nocapture"], env=dict(env, ATTACHMENT_PASSES=str(passes)), check=True, text=True, stdout=subprocess.PIPE)
    assert "1 passed" in run.stdout, run.stdout
    # Exact full test name prevents silently running no tests.
    counts.append(int(re.search(r"summary: (\d+)", output.read_text())[1]))
assert counts[1] > counts[0]
print((counts[1] - counts[0]) / (10 * 700 * 2))
