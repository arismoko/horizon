import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

mode, variant = sys.argv[1:]
cache = Path.home() / ".cache/horizon-pickup-invalidation"
home = Path(tempfile.mkdtemp(prefix=f"dedicated-{mode}-{variant}-", dir=cache))
server_dir = home / "server"
server_dir.mkdir()
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
(server_dir / "eula.txt").write_text("eula=true\n")
(server_dir / "server.properties").write_text(
    f"server-port={port}\nonline-mode=false\nspawn-protection=0\nview-distance=4\n"
)
binary = cache / "target/fast/horizon-client"
ctl = cache / "target/fast/horizon-ctl"
resources = Path.home() / ".cache/horizon-upstream-build2-input/local/horizon-client/26.3"
env = os.environ.copy()
for key, value in {
    "HORIZON_HOME": "game", "HORIZON_LAUNCHER_HOME": "launcher",
    "XDG_DATA_HOME": "data", "XDG_CONFIG_HOME": "config",
    "HORIZON_LAUNCHER_TRASH": "trash", "HORIZON_LAUNCHER_CLIPBOARD": "clipboard",
    "HORIZON_LAUNCHER_DESKTOP": "desktop",
}.items():
    env[key] = str(home / value)
env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1", PICKUP_PROOF=str(home / "marker"))
if variant == "cancel":
    env["PICKUP_CANCEL"] = "1"
log_path = home / "server.log"
log = log_path.open("w")
server = subprocess.Popen(
    [str(binary), "--resources", str(resources), "--dedicated", "--nogui"],
    cwd=server_dir, env=env, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True,
)
endpoint = home / "ctl.sock"
def control(*args):
    return subprocess.run([str(ctl), "--socket", str(endpoint), *args], env=env, text=True, capture_output=True, check=True, timeout=45).stdout

try:
    for _ in range(120):
        if server.poll() is not None:
            raise RuntimeError("dedicated server exited before listening")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                break
        except OSError:
            time.sleep(1)
    else:
        raise RuntimeError("dedicated server did not listen")
    print(control("launch", "--game", str(binary), "--offscreen", "--resources", str(resources), "--game-dir", str(home / "client")))
    print(control("connect", f"127.0.0.1:{port}", "Probe"))
    time.sleep(5)
    server.stdin.write('execute at Probe run summon minecraft:item ~ ~ ~ {Item:{id:"minecraft:diamond",count:1},PickupDelay:0s}\n')
    server.stdin.flush()
    for _ in range(60):
        if (home / "marker").exists():
            break
        if server.poll() is not None:
            raise RuntimeError("dedicated server exited before hook marker")
        time.sleep(1)
    else:
        raise RuntimeError("dedicated hook did not remove an item")
    print("Dedicated marker:", (home / "marker").read_text(), mode, variant)
    time.sleep(3)
    output = log_path.read_text()
    if mode == "before":
        if "index out of bounds" not in output:
            raise RuntimeError("dedicated baseline did not panic")
        print("Expected dedicated index out of bounds:", variant)
    else:
        if server.poll() is not None or "panicked at" in output:
            raise RuntimeError("dedicated server failed after the hook")
        state = json.loads(control("state"))
        print("Dedicated client state:", state)
        print(control("screenshot", str(Path.cwd() / f"pickup-dedicated-{variant}.png")))
        print("Dedicated server remained responsive:", variant)
    control("quit")
    if server.poll() is None:
        server.stdin.write("stop\n")
        server.stdin.flush()
        server.wait(timeout=30)
finally:
    if server.poll() is None:
        server.terminate()
        server.wait(timeout=20)
    log.close()
    print("Dedicated log:", log_path.read_text()[-8000:])
