#!/usr/bin/env python3
"""Writes crates/sunrise-parity-fixtures/Cargo.toml from crates/sunrise-client/Cargo.toml (#4896).

The `sunrise-parity-fixtures` program compiles the parity fixtures through its thin `src/main.rs` wrapper
over the gameplay crates and `sunrise-client`, with every dependency optional behind `fixtures` / `client-defaults`
so a workspace build that doesn't ask for `fixtures` builds and unifies nothing from it. `client-defaults` forwards
`sunrise-client`'s default features.

Usage: parity_fixtures_manifest.py [--check]   (--check: fail when the manifest isn't what this writes)
"""
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLIENT = ROOT / "crates/sunrise-client/Cargo.toml"
MANIFEST = ROOT / "crates/sunrise-parity-fixtures/Cargo.toml"


def value(item):
    if isinstance(item, bool):
        return "true" if item else "false"
    if isinstance(item, str):
        return '"' + item.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(item, list):
        return "[" + ", ".join(value(entry) for entry in item) + "]"
    if isinstance(item, dict):
        return "{ " + ", ".join(f"{key} = {value(entry)}" for key, entry in item.items()) + " }"
    raise TypeError(f"no TOML form for {item!r}")


def table(rows):
    return "".join(f"{name if name.replace('-', '').replace('_', '').isalnum() else value(name)} = {value(entry)}\n"
                   for name, entry in rows.items())


def render():
    client = tomllib.loads(CLIENT.read_text(encoding="utf-8"))
    package = client["package"]
    client_defaults = [
        "fixtures",
        "dep:sunrise-client-gameplay",
        "dep:sunrise-client",
        "sunrise-client-gameplay/parity-fixtures",
        "sunrise-client/parity-fixtures",
        *(f"sunrise-client/{name}" for name in client.get("features", {}).get("default", [])),
    ]
    dependencies = {
        "sunrise-client": {"path": "../sunrise-client", "optional": True, "default-features": False},
        "sunrise-client-gameplay": {"path": "../sunrise-client-gameplay", "optional": True, "default-features": False},
        "sunrise-gameplay": {"path": "../sunrise-gameplay", "optional": True, "default-features": False},
        "sunrise-gameplay-common": {"path": "../sunrise-gameplay-common", "optional": True, "default-features": False},
    }
    features = {
        "client-defaults": client_defaults,
        "fixtures": [
            "dep:sunrise-gameplay-common",
            "dep:sunrise-gameplay",
            "sunrise-gameplay-common/parity-fixtures",
            "sunrise-gameplay/parity-fixtures",
        ],
    }
    return (
        "[package]\n"
        'name = "sunrise-parity-fixtures"\n'
        f"version = {value(package['version'])}\n"
        "edition.workspace = true\n"
        "license.workspace = true\n"
        "rust-version.workspace = true\n"
        'description = "The parity harnesses\' sunrise-client fixtures, without the lib test (CI only)"\n'
        "publish = false\n"
        "autolib = false\n"
        "autobins = false\n"
        "autoexamples = false\n"
        "autotests = false\n"
        "autobenches = false\n"
        "\n"
        "[[bin]]\n"
        'name = "sunrise-parity-fixtures"\n'
        'path = "src/main.rs"\n'
        'required-features = ["fixtures"]\n'
        "test = false\n"
        "bench = false\n"
        "doc = false\n"
        "\n"
        "[lints]\n"
        "workspace = true\n"
        "\n"
        "[dependencies]\n" + table(dependencies)
        + "\n[features]\n" + table(features)
    )


def main(argv):
    text = render()
    if argv == ["--check"]:
        if MANIFEST.read_text(encoding="utf-8") != text:
            print(f"{MANIFEST.relative_to(ROOT)} is out of date: run python3 tools/ci/parity_fixtures_manifest.py")
            return 1
        return 0
    if argv:
        print(__doc__)
        return 2
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    MANIFEST.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
