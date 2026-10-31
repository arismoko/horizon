import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

mode = sys.argv[1]
cache = Path.home() / ".cache/horizon-native-stop"
binary = cache / "target/fast/horizon-client"
ctl = cache / "target/fast/horizon-ctl"
resources = Path.home() / ".cache/horizon-upstream-build2-input/local/horizon-client/26.3"

def setup(name):
    home = Path(tempfile.mkdtemp(prefix=f"{mode}-{name}-", dir=cache))
    env = os.environ.copy()
    for key, value in {
        "HORIZON_HOME": "game", "HORIZON_LAUNCHER_HOME": "launcher",
        "XDG_DATA_HOME": "data", "XDG_CONFIG_HOME": "config",
        "HORIZON_LAUNCHER_TRASH": "trash", "HORIZON_LAUNCHER_CLIPBOARD": "clipboard",
        "HORIZON_LAUNCHER_DESKTOP": "desktop",
    }.items():
        env[key] = str(home / value)
    env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1",
               STOP_SENT=str(home / "sent"), STOP_RECEIVED=str(home / "received"))
    return home, env

def call(home, env, *args, check=True):
    result = subprocess.run([str(ctl), "--socket", str(home / "ctl.sock"), *map(str, args)],
                            env=env, text=True, capture_output=True, timeout=120, check=check)
    if check:
        return json.loads(result.stdout)
    return result

def launch(home, env, world=False):
    args = ["launch", "--game", str(binary), "--offscreen", "--resources", str(resources),
            "--game-dir", str(home / "game")]
    if world:
        args += ["--world", "StopProbe", "--creative", "--wait"]
    return call(home, env, *args)

def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]

def check_delivery(host, guest, label):
    sent = host / "sent"
    for _ in range(20):
        if sent.exists():
            break
        time.sleep(0.5)
    if not sent.exists() or "Probe:Ok(())" not in sent.read_text():
        raise AssertionError(f"{label}: no successful send to Probe: {sent.read_text() if sent.exists() else 'missing'}")
    for _ in range(20):
        if (guest / "received").exists():
            break
        time.sleep(0.5)
    received = (guest / "received").exists()
    print(f"{label} {mode}: sent={sent.read_text()!r} received={received}", flush=True)
    if received != (mode == "after"):
        raise AssertionError(f"{label}: unexpected delivery result")

host, host_env = setup("lan-host")
guest, guest_env = setup("lan-guest")
try:
    print("LAN host:", launch(host, host_env, True), flush=True)
    port = call(host, host_env, "lan", "open", "0", "commands")["port"]
    print("LAN guest:", launch(guest, guest_env), flush=True)
    print(call(guest, guest_env, "connect", f"127.0.0.1:{port}", "Probe"), flush=True)
    print(call(guest, guest_env, "wait-for", "loaded", "--timeout", "120"), flush=True)
    time.sleep(2)
    print(call(guest, guest_env, "screenshot", str(Path.cwd() / f"stop-lan-{mode}.png")), flush=True)
    print("LAN stopping:", call(host, host_env, "quit"), flush=True)
    check_delivery(host, guest, "LAN")
finally:
    call(guest, guest_env, "quit", check=False)
    call(host, host_env, "quit", check=False)

host, host_env = setup("dedicated")
guest, guest_env = setup("dedicated-guest")
server_dir = host / "server"; server_dir.mkdir()
port = free_port()
(server_dir / "eula.txt").write_text("eula=true\n")
(server_dir / "server.properties").write_text(
    f"server-port={port}\nonline-mode=false\nwhite-list=false\nspawn-protection=0\nview-distance=4\n"
)
log_path = host / "server.log"
with log_path.open("w") as log:
    server = subprocess.Popen([str(binary), "--resources", str(resources), "--dedicated", "--nogui"],
                              cwd=server_dir, env=host_env, stdin=subprocess.PIPE,
                              stdout=log, stderr=subprocess.STDOUT, text=True)
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
        print("Dedicated guest:", launch(guest, guest_env), flush=True)
        print(call(guest, guest_env, "connect", f"127.0.0.1:{port}", "Probe"), flush=True)
        print(call(guest, guest_env, "wait-for", "loaded", "--timeout", "120"), flush=True)
        time.sleep(2)
        print(call(guest, guest_env, "screenshot", str(Path.cwd() / f"stop-dedicated-{mode}.png")), flush=True)
        server.stdin.write("stop\n"); server.stdin.flush()
        check_delivery(host, guest, "dedicated")
        server.wait(timeout=30)
    finally:
        call(guest, guest_env, "quit", check=False)
        if server.poll() is None:
            server.terminate(); server.wait(timeout=20)
        print("Dedicated log:", log_path.read_text()[-6000:], flush=True)
