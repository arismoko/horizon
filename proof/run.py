import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

cache = Path.home() / ".cache/horizon-tag-reads"
home = Path(tempfile.mkdtemp(prefix="proof-", dir=cache))
binary = cache / "sample-target/fast/horizon-client"
ctl_bin = Path.home() / ".cache/horizon-content-state/target/fast/horizon-ctl"
resources = Path.home() / ".cache/horizon-upstream-build2-input/local/horizon-client/26.3"
env = os.environ.copy()
for key, value in {
    "HORIZON_HOME": "game", "HORIZON_LAUNCHER_HOME": "launcher",
    "XDG_DATA_HOME": "data", "XDG_CONFIG_HOME": "config",
    "HORIZON_LAUNCHER_TRASH": "trash", "HORIZON_LAUNCHER_CLIPBOARD": "clipboard",
    "HORIZON_LAUNCHER_DESKTOP": "desktop",
}.items():
    env[key] = str(home / value)
env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1")
endpoint = home / "ctl.sock"
expected = "TAG_PROOF block=Ok(true) block_nonmember=Ok(false) block_missing=Ok(false) item=Ok(true) item_nonmember=Ok(false) item_missing=Ok(false)"

def ctl(*args):
    out = subprocess.run([str(ctl_bin), "--socket", str(endpoint), *map(str, args)],
                         env=env, text=True, capture_output=True, check=True, timeout=120)
    return json.loads(out.stdout)

def wait_log(path, expected):
    for _ in range(90):
        text = path.read_text() if path.exists() else ""
        if expected in text:
            print(expected, flush=True)
            return
        time.sleep(1)
    raise AssertionError(f"missing tag result in {path}: {text[-5000:]}")

try:
    ctl("launch", "--game", binary, "--world", "TagProof", "--creative", "--offscreen",
        "--wait", "--resources", resources, "--game-dir", home / "integrated")
    wait_log(endpoint.with_suffix(".log"), "TAG_STARTUP block=Err(TagsUnavailable) item=Err(TagsUnavailable)")
    wait_log(endpoint.with_suffix(".log"), "TAG_LEGACY block=Ok(true) item=Ok(true)")
    ctl("command", "fill -3 119 -3 4 119 6 minecraft:stone")
    ctl("command", "time set day")
    ctl("command", "tp @s 0 120 0")
    print("integrated summon", ctl("command", 'summon minecraft:item 0 121 3 {Item:{id:"minecraft:oak_log",count:1},PickupDelay:32767s}'), flush=True)
    wait_log(endpoint.with_suffix(".log"), expected)
    ctl("look-at", "0.5", "120.5", "3.5")
    time.sleep(2)
    ctl("screenshot", str(Path.cwd() / "tag-integrated.png"))
    ctl("quit")

    server_dir = home / "server"
    server_dir.mkdir()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    (server_dir / "eula.txt").write_text("eula=true\n")
    (server_dir / "server.properties").write_text(
        f"server-port={port}\nonline-mode=false\nwhite-list=false\nspawn-protection=0\nview-distance=4\n")
    server_log = server_dir / "server.log"
    with server_log.open("w") as log:
        server = subprocess.Popen([str(binary), "--resources", str(resources), "--dedicated", "--nogui"],
                                  cwd=server_dir, env=env, stdin=subprocess.PIPE, stdout=log,
                                  stderr=subprocess.STDOUT, text=True)
        try:
            for _ in range(120):
                if server.poll() is not None:
                    raise RuntimeError("dedicated server exited")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=1):
                        break
                except OSError:
                    time.sleep(1)
            else:
                raise RuntimeError("dedicated server did not listen")
            wait_log(server_log, "TAG_STARTUP block=Err(TagsUnavailable) item=Err(TagsUnavailable)")
            wait_log(server_log, "TAG_LEGACY block=Ok(true) item=Ok(true)")
            ctl("launch", "--game", binary, "--offscreen", "--resources", resources,
                "--game-dir", home / "client")
            ctl("connect", f"127.0.0.1:{port}", "Probe")
            time.sleep(5)
            server.stdin.write('execute at Probe run summon minecraft:item ~ ~ ~3 {Item:{id:"minecraft:oak_log",count:1},PickupDelay:32767s}\n')
            server.stdin.flush()
            wait_log(server_log, expected)
            ctl("screenshot", str(Path.cwd() / "tag-dedicated.png"))
            ctl("quit")
            server.stdin.write("stop\n")
            server.stdin.flush()
            server.wait(timeout=60)
        finally:
            if server.poll() is None:
                server.terminate()
                server.wait(timeout=20)
finally:
    subprocess.run([str(ctl_bin), "--socket", str(endpoint), "quit"],
                   env=env, capture_output=True, timeout=120)
print("Proof data:", home, flush=True)
