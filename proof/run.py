import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import time

cache = Path.home() / ".cache/horizon-content-state"
target = cache / "target/fast"
home = Path(tempfile.mkdtemp(prefix="game-", dir=cache))
resources = Path.home() / ".cache/horizon-upstream-build2-input/local/horizon-client/26.3"
fixture = home / "fixture"
shutil.copytree(cache / "fixture", fixture)
(fixture / "assets/proof/blockstates/test.json").write_text(json.dumps({
    "variants": {
        "lit=true": {"model": "proof:block/test"},
        "lit=false": {"model": "proof:block/unlit"},
    },
}))
(fixture / "assets/proof/models/block/unlit.json").write_text(json.dumps({
    "parent": "minecraft:block/cube_all",
    "textures": {"all": "minecraft:block/gold_block"},
}))
env = os.environ.copy()
for key, value in {
    "HORIZON_HOME": "game", "HORIZON_LAUNCHER_HOME": "launcher",
    "XDG_DATA_HOME": "data", "XDG_CONFIG_HOME": "config",
    "HORIZON_LAUNCHER_TRASH": "trash", "HORIZON_LAUNCHER_CLIPBOARD": "clipboard",
    "HORIZON_LAUNCHER_DESKTOP": "desktop",
}.items():
    env[key] = str(home / value)
env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1")
socket = home / "ctl.sock"

def ctl(*args):
    result = subprocess.run([str(target / "horizon-ctl"), "--socket", str(socket), *map(str, args)],
                            env=env, text=True, capture_output=True, check=True, timeout=120)
    return json.loads(result.stdout)

def launch():
    result = ctl("launch", "--game", target / "horizon-client", "--world", "StateProof",
                 "--creative", "--offscreen", "--wait", "--resources", resources,
                 "--game-dir", home / "game", "--dev-mod", fixture)
    print("launch", result.get("launch", {}), flush=True)

def inspect(stage):
    for x, value in [(0, "true"), (1, "false")]:
        result = ctl("block", x, 120, 3)
        print(stage, value, json.dumps(result, sort_keys=True), flush=True)
        text = json.dumps(result)
        if f"proof:test[lit={value}]" not in text:
            raise AssertionError(f"{stage}: missing lit={value}: {text}")
    log = socket.with_suffix(".log").read_text()
    if "resource packs failed to load" in log:
        raise AssertionError("resource loading fell back to vanilla")

try:
    launch()
    print("platform", ctl("command", "fill -3 119 -3 4 119 6 minecraft:stone"), flush=True)
    print("day", ctl("command", "time set day"), flush=True)
    print("teleport", ctl("command", "tp @s 0 120 0"), flush=True)
    for x, value in [(0, "true"), (1, "false")]:
        print("setblock", value, ctl("command", f"setblock {x} 120 3 proof:test[lit={value}]"), flush=True)
    time.sleep(3)
    inspect("before reload")
    print("quit", ctl("quit"), flush=True)
    time.sleep(5)
    launch()
    time.sleep(3)
    inspect("after reload")
    print("look", ctl("look-at", "1", "120.5", "3.5"), flush=True)
    time.sleep(1)
    print("screenshot", ctl("screenshot", str(Path.cwd() / "content-state-reload.png")), flush=True)
finally:
    subprocess.run([str(target / "horizon-ctl"), "--socket", str(socket), "quit"],
                   env=env, capture_output=True, timeout=120)
print("saved game and logs:", home, flush=True)
