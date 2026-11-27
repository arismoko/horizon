#!/usr/bin/env python3
"""Check gameplay crate dependency boundaries directly from Cargo.toml manifests.

    python3 tools/ci/check_gameplay_layers.py

Rules checked without running Cargo:
- `sunrise-gameplay-common` depends on neither side crate (`sunrise-gameplay`,
  `sunrise-client-gameplay`), on no desktop crate (name containing `client`,
  `render`, `audio`, `window`, `winit`, `wgpu`, `input`, or `theme`), and not on
  `mod_platform` (`sunrise-mod-api`) outside dev-dependencies.
- `sunrise-gameplay` depends neither on `sunrise-client-gameplay` nor on any
  desktop crate.
- `sunrise-client-gameplay` does not depend on `sunrise-gameplay`.
- None of the three gameplay crates depends on `sunrise-client`.
"""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path
from typing import Iterator

ROOT = Path(__file__).resolve().parents[2]

COMMON_CRATE = "sunrise-gameplay-common"
SERVER_CRATE = "sunrise-gameplay"
CLIENT_CRATE = "sunrise-client-gameplay"
ENTRY_CRATE = "sunrise-client"
GAMEPLAY_CRATES = (COMMON_CRATE, SERVER_CRATE, CLIENT_CRATE)
SIDE_CRATES = frozenset({SERVER_CRATE, CLIENT_CRATE})
DESKTOP_TOKENS = ("client", "render", "audio", "window", "winit", "wgpu", "input", "theme")
MOD_PLATFORM_NAMES = frozenset({"mod-platform", "sunrise-mod-api"})
DEP_SECTIONS = (
    ("dependencies", False),
    ("build-dependencies", False),
    ("dev-dependencies", True),
)


def normalize_crate_name(name: str) -> str:
    return name.replace("_", "-").lower()


def load_workspace_dependencies(root: Path) -> dict[str, object]:
    workspace_manifest = root / "Cargo.toml"
    if not workspace_manifest.is_file():
        return {}
    try:
        data = tomllib.loads(workspace_manifest.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError:
        return {}
    workspace = data.get("workspace")
    if not isinstance(workspace, dict):
        return {}
    deps = workspace.get("dependencies")
    return deps if isinstance(deps, dict) else {}


def resolve_package_name(dep_key: str, spec: object, workspace_deps: dict[str, object]) -> str:
    if isinstance(spec, dict):
        pkg = spec.get("package")
        if isinstance(pkg, str) and pkg:
            return pkg
        if spec.get("workspace") is True:
            ws_spec = workspace_deps.get(dep_key)
            if isinstance(ws_spec, dict):
                ws_pkg = ws_spec.get("package")
                if isinstance(ws_pkg, str) and ws_pkg:
                    return ws_pkg
    return dep_key


def iter_manifest_dependencies(
    manifest: dict[str, object],
    workspace_deps: dict[str, object],
) -> Iterator[tuple[str, bool, str, str]]:
    """Yield `(section_label, is_dev, dep_key, package_name)` for all Cargo dependency tables."""
    for section, is_dev in DEP_SECTIONS:
        table = manifest.get(section)
        if isinstance(table, dict):
            for dep_key, spec in sorted(table.items()):
                yield section, is_dev, dep_key, resolve_package_name(dep_key, spec, workspace_deps)
    targets = manifest.get("target")
    if isinstance(targets, dict):
        for cfg, target_table in sorted(targets.items()):
            if not isinstance(target_table, dict):
                continue
            for section, is_dev in DEP_SECTIONS:
                table = target_table.get(section)
                if isinstance(table, dict):
                    label = f"target.{cfg}.{section}"
                    for dep_key, spec in sorted(table.items()):
                        yield label, is_dev, dep_key, resolve_package_name(dep_key, spec, workspace_deps)


def desktop_token_match(names: tuple[str, ...]) -> str | None:
    for name in names:
        lower = name.lower()
        for token in DESKTOP_TOKENS:
            if token in lower:
                return token
    return None


def check_gameplay_layers(root: Path = ROOT) -> list[str]:
    root = root.resolve()
    workspace_deps = load_workspace_dependencies(root)
    problems: list[str] = []

    for crate in GAMEPLAY_CRATES:
        manifest_path = root / "crates" / crate / "Cargo.toml"
        rel = f"crates/{crate}/Cargo.toml"
        if not manifest_path.is_file():
            problems.append(f"{rel}: manifest is missing")
            continue
        try:
            manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as exc:
            problems.append(f"{rel}: invalid TOML ({exc})")
            continue

        pkg_table = manifest.get("package")
        pkg_name = pkg_table.get("name") if isinstance(pkg_table, dict) else None
        if pkg_name != crate:
            problems.append(f"{rel}: expected [package].name = {crate!r}, got {pkg_name!r}")

        for section, is_dev, dep_key, resolved_pkg in iter_manifest_dependencies(manifest, workspace_deps):
            names = (dep_key, resolved_pkg) if resolved_pkg != dep_key else (dep_key,)
            norm_names = {normalize_crate_name(n) for n in names}
            dep_label = dep_key if resolved_pkg == dep_key else f"{dep_key} (package {resolved_pkg})"

            if ENTRY_CRATE in norm_names:
                problems.append(f"{rel} [{section}]: {crate} may not depend on {ENTRY_CRATE} ({dep_label})")
                continue

            if crate == COMMON_CRATE:
                side_hits = sorted(norm_names & SIDE_CRATES)
                if side_hits:
                    problems.append(
                        f"{rel} [{section}]: {crate} may not depend on side crate {side_hits[0]} ({dep_label})"
                    )
                else:
                    token = desktop_token_match(names)
                    if token is not None:
                        problems.append(
                            f"{rel} [{section}]: {crate} may not depend on desktop crate {dep_label} "
                            f"(name contains {token!r})"
                        )
                if not is_dev and (norm_names & MOD_PLATFORM_NAMES):
                    problems.append(
                        f"{rel} [{section}]: {crate} may not depend on mod_platform outside dev-dependencies "
                        f"({dep_label})"
                    )
            elif crate == SERVER_CRATE:
                if CLIENT_CRATE in norm_names:
                    problems.append(
                        f"{rel} [{section}]: {crate} may not depend on {CLIENT_CRATE} ({dep_label})"
                    )
                else:
                    token = desktop_token_match(names)
                    if token is not None:
                        problems.append(
                            f"{rel} [{section}]: {crate} may not depend on desktop crate {dep_label} "
                            f"(name contains {token!r})"
                        )
            elif crate == CLIENT_CRATE:
                if SERVER_CRATE in norm_names:
                    problems.append(
                        f"{rel} [{section}]: {crate} may not depend on {SERVER_CRATE} ({dep_label})"
                    )

    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Check gameplay crate Cargo.toml dependency boundaries.")
    parser.add_argument("--root", type=Path, default=ROOT, help="repository root directory")
    args = parser.parse_args(argv)

    problems = check_gameplay_layers(args.root)
    if problems:
        print("Gameplay layers check failed:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    print(f"Gameplay layers check passed: {len(GAMEPLAY_CRATES)} manifests verified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
