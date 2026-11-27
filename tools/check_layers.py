#!/usr/bin/env python3
"""Check that each crate depends only on the layers its own layer may use (docs/layers.md).

tools/layers.json's `may_depend` says which: engine on engine; platform on engine and platform; sunrise on
platform and sunrise only, so the Sunrise layer builds against the platform's public API and can live in its own
repository. `tool` crates (dev tools) may use anything and nothing shipped uses them. `crates` gives every crate its
layer and `allowed` lists today's exceptions. `optional` lists exceptions that hold only while the dependency is
optional (behind a feature), for the layer and the portable check alike. The check fails on:
- a workspace crate with no layer;
- a normal, build or optional dependency on a layer its crate's layer may not use that `allowed` doesn't list,
  and that isn't an optional dependency listed in `optional`;
- an `optional` entry whose dependency is no longer optional;
- with --base, an `allowed` or `optional` entry the branch added from a crate that already had a layer on the base,
  except for an approved transfer of an existing exception;
- a shipped crate depending on a tool;
- a portable crate depending on a crate of the repository that isn't portable, or on a `forbidden` library;
- a `portable.pending` crate doing the same beyond the crates its entry lists, listed as portable too, or (with
  --base) gaining an entry its base list lacks.
- a `sides` crate reaching another one, or a crate below them (or sunrise-client-lib) reaching one (#2210).
An `allowed` or `optional` entry with no dependency left is reported, not failed. Dev-dependencies are exempt: tests may reach the platform, but shipped code may not.

    python3 tools/check_layers.py                     # the check
    python3 tools/check_layers.py --base origin/main  # and no new exceptions since the base
    python3 tools/check_layers.py --list              # each crate's layer and the exceptions left
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

CONFIG = ROOT / "tools" / "layers.json"
CONFIG_PATH = "tools/layers.json"
TOOL = "tool"
DOC = "docs/layers.md"
# The section of docs/layers.md that says what a layer may depend on.
SECTION = {"engine": "#engine", "platform": "#platform", "sunrise": "#sunrise", TOOL: "#tools"}
WHERE = "#where-a-new-feature-goes"
CLIENT_GAMEPLAY_TRANSFERS = {
    "sunrise-dualsense": ("allowed", True),
    "sunrise-friends": ("allowed", True),
    "sunrise-shaders": ("allowed", False),
    "sunrise-telemetry": ("allowed", False),
    "sunrise-optifine": ("optional", True),
    "sunrise-theme": ("optional", True),
}
PORTABLE_GAMEPLAY = {"sunrise-gameplay-common", "sunrise-gameplay"}


def load_config(path=CONFIG):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def base_config(base):
    """tools/layers.json where the branch left `base` (their merge base), or None when there is none. On `base` itself,
    the exceptions main dropped since then would look like ones this branch added."""
    merge_base = subprocess.run(["git", "merge-base", base, "HEAD"], capture_output=True, text=True, cwd=ROOT)
    if merge_base.returncode != 0:
        return None
    from tools import merge_base as shared
    # A baseline that is there but can't be read stops the check (merge_base.text_at).
    text = shared.text_at(merge_base.stdout.strip(), CONFIG_PATH, ROOT)
    return json.loads(text) if text is not None else None


def metadata(manifest):
    """`cargo metadata` for one workspace, without resolving dependencies (no network, no lockfile)."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path", str(manifest)],
        check=True, capture_output=True, text=True, cwd=ROOT,
    ).stdout
    return json.loads(out)["packages"]


def edges(packages):
    """{crate: {dependency, ...}} for normal and build dependencies on other local crates (path dependencies)."""
    graph = {}
    for package in packages:
        deps = graph.setdefault(package["name"], set())
        for dep in package["dependencies"]:
            if dep.get("path") and dep.get("kind") != "dev":
                deps.add(dep["name"])
    return graph


def optional_edges(packages):
    """The path dependencies, as 'crate -> dependency', that are optional everywhere they are declared (normal and
    build, every target): one mandatory declaration makes the edge mandatory."""
    declared, mandatory = set(), set()
    for package in packages:
        for dep in package["dependencies"]:
            if dep.get("path") and dep.get("kind") != "dev":
                edge = f"{package['name']} -> {dep['name']}"
                declared.add(edge)
                if not dep.get("optional"):
                    mandatory.add(edge)
    return declared - mandatory


def optional_rows(config):
    """The edges of `optional`: exceptions that hold only while the dependency stays optional."""
    return {row["edge"] for row in config.get("optional", [])}


def crossings(config, graph):
    """The dependencies on a layer the crate's layer may not use, as 'crate -> dependency'."""
    may = config["may_depend"]
    layers = config["crates"]
    out = []
    for crate in sorted(graph):
        for dep in sorted(graph[crate]):
            if layers.get(crate) in may and layers.get(dep) in may and layers[dep] not in may[layers[crate]]:
                out.append(f"{crate} -> {dep}")
    return out


def reachable(graph, start):
    """Every crate `start` depends on, directly or through others."""
    seen, todo = set(), [start]
    while todo:
        for dep in graph.get(todo.pop(), ()):
            if dep not in seen:
                seen.add(dep)
                todo.append(dep)
    return seen


def side_problems(config, graph):
    """Gameplay's side crates (#2210) reaching each other, or a crate below them reaching a side crate."""
    sides = config.get("sides", {}).get("crates", [])
    problems = []
    for crate in sides:
        reach = reachable(graph, crate)
        problems += [f"{crate} -> {other}: gameplay's sides may not depend on each other ({CONFIG_PATH} sides)"
                     for other in sides if other != crate and other in reach]
    if sides:
        below = {"sunrise-client-lib"}.union(*(reachable(graph, crate) for crate in sides)) - set(sides)
        for crate in sorted(below):
            problems += [f"{crate} -> {side}: a crate below gameplay's sides may not depend on them "
                         f"({CONFIG_PATH} sides)" for side in sides if side in reachable(graph, crate)]
    return problems


def transferred_exception(edge, key, graph, optional, config, base):
    crate, dependency = edge.split(" -> ")
    approved = CLIENT_GAMEPLAY_TRANSFERS.get(dependency)
    if crate != "sunrise-client-gameplay" or approved != (key, edge in optional):
        return False
    if dependency not in graph.get(crate, ()):
        return False
    previous = f"sunrise-client -> {dependency}"
    baseline = base.get("allowed", ())
    if previous not in baseline and (key != "optional" or previous not in optional_rows(base)):
        return False
    if dependency == "sunrise-telemetry":
        return True
    current = set(config.get("allowed", ())) | optional_rows(config)
    return previous not in current and dependency not in graph.get("sunrise-client", ())


def check(config, graph, base=None, optional=frozenset()):
    """(problems, notes): problems fail the check, notes are only printed. `optional` is optional_edges()."""
    may = config["may_depend"]
    layers = config["crates"]
    allowed = set(config.get("allowed", []))
    recorded_optional = optional_rows(config)
    problems = []
    for crate in sorted(graph):
        layer = layers.get(crate)
        if layer is None:
            problems.append(f"{crate}: no layer. Give it one in {CONFIG_PATH} ({DOC}{WHERE} says which)")
        elif layer != TOOL and layer not in may:
            problems.append(f"{crate}: unknown layer {layer!r} in {CONFIG_PATH}")
    for crate in sorted(graph):
        layer = layers.get(crate)
        if layer is None or layer == TOOL or layer not in may:
            continue
        for dep in sorted(graph[crate]):
            # A path dependency with no layer is a vendored crate (third_party/) or a fixture.
            if layers.get(dep) == TOOL:
                problems.append(f"{crate} ({layer}) -> {dep} (tool): shipped code may not depend on a dev tool "
                                f"({DOC}{SECTION[TOOL]})")
    found = set(crossings(config, graph))
    for edge in sorted(found - allowed):
        crate, dep = edge.split(" -> ")
        if edge in recorded_optional:
            if edge not in optional:
                problems.append(f"{edge}: listed in 'optional' in {CONFIG_PATH}, but the dependency is no longer "
                                f"optional. Keep it behind its feature, or move the code to the right layer "
                                f"({DOC}{SECTION[layers[crate]]})")
            continue
        problems.append(f"{edge}: {layers[crate]} may not depend on {layers[dep]}. "
                        f"See {DOC}{SECTION[layers[crate]]}: move the code to the right layer, or offer an "
                        f"extension point the other side plugs into")
    problems += side_problems(config, graph)
    if base is not None:
        base_layers = base.get("crates", {})
        # An exception may move between the lists (an `allowed` edge put behind a feature), but not appear.
        base_exceptions = set(base.get("allowed", [])) | optional_rows(base)
        for key, entries in (("allowed", allowed), ("optional", recorded_optional)):
            for edge in sorted(entries - base_exceptions):
                crate, dep = edge.split(" -> ")
                # A crate new since the base (split out of another, or renamed) may carry its old crate's
                # exceptions; a new dependency target excuses nothing.
                transferred = transferred_exception(edge, key, graph, optional, config, base)
                if not transferred and crate in base_layers:
                    problems.append(f"{edge}: added to '{key}' in {CONFIG_PATH}. The exceptions only shrink; "
                                    f"see {DOC}{SECTION.get(layers.get(crate), '')}")
    notes = [f"{edge}: no longer a dependency; you can remove it from 'allowed' in {CONFIG_PATH}"
             for edge in sorted(allowed - found)]
    notes += [f"{edge}: no longer a dependency; you can remove it from 'optional' in {CONFIG_PATH}"
              for edge in sorted(recorded_optional - found - allowed)]
    return problems, notes


def portable(config, packages, base=None):
    """Problems of the portable crates (docs/layers.md#engine): a path dependency on a crate that isn't portable,
    or a normal or build dependency on a library they never take (window, GPU, sound, gamepad, clipboard, HTTP)."""
    spec = config.get("portable", {})
    crates, forbidden = set(spec.get("crates", [])), set(spec.get("forbidden", []))
    pending = {crate: set(deps) for crate, deps in spec.get("pending", {}).items() if not crate.startswith("_")}
    optional_baselines = optional_rows(config)
    problems = {f"{crate}: listed as portable in {CONFIG_PATH}, but not an engine crate there"
                for crate in crates | set(pending) if config["crates"].get(crate) != "engine"}
    problems |= {f"{crate}: in both portable.crates and portable.pending of {CONFIG_PATH}"
                 for crate in crates & set(pending)}
    base_pending = (base or {}).get("portable", {}).get("pending", {})
    for crate, deps in pending.items():
        if crate in base_pending:
            for dep in sorted(deps - set(base_pending[crate])):
                problems.add(f"{crate} -> {dep}: added to portable.pending in {CONFIG_PATH}. "
                             f"The exceptions only shrink")
    for package in packages:
        if package["name"] not in crates and package["name"] not in pending:
            continue
        for dep in package["dependencies"]:
            if dep.get("kind") == "dev":
                continue
            edge = f"{package['name']} -> {dep['name']}"
            if (package["name"] in PORTABLE_GAMEPLAY and dep["name"] == "sunrise-mod-api-gen"
                    and dep.get("kind") != "build"):
                problems.add(f"{edge}: portable gameplay may use the generator only as a build dependency")
            if dep.get("optional") and edge in optional_baselines:
                continue
            if dep.get("path") and dep["name"] not in crates | pending.get(package["name"], set()):
                problems.add(f"{edge}: a portable crate depends only on the other portable crates "
                             f"({DOC}{SECTION['engine']})")
            elif not dep.get("path") and dep["name"] in forbidden:
                problems.add(f"{edge}: a portable crate takes no window, GPU, sound, gamepad, clipboard or HTTP "
                             f"library ({DOC}{SECTION['engine']})")
    return sorted(problems)


def workspace_packages(config):
    return [package for manifest in config["workspaces"] for package in metadata(ROOT / manifest)]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="refuse 'allowed' entries added since this git ref")
    parser.add_argument("--list", action="store_true", help="print each crate's layer and the exceptions")
    from tools import merge_base
    parser.add_argument("--pull-request-merge", action="store_true", help=merge_base.PULL_REQUEST_MERGE_HELP)
    args = parser.parse_args(argv)
    if args.base:
        args.base, error = merge_base.pull_request_base(args.base, args.pull_request_merge)
        if error:
            print(f"Layer check failed: {error}")
            return 1
    config = load_config()
    packages = workspace_packages(config)
    graph = edges(packages)
    if args.list:
        for crate in sorted(graph):
            print(f"{config['crates'].get(crate, '?'):<9} {crate}")
        ups = crossings(config, graph)
        print(f"\n{len(ups)} dependencies on a layer the crate's layer may not use:")
        for edge in ups:
            print(f"  {edge}")
        return 0
    base = base_config(args.base) if args.base else None
    optional = optional_edges(packages)
    problems, notes = check(config, graph, base, optional)
    problems += portable(config, packages, base)
    for note in notes:
        print(f"note: {note}")
    if problems:
        print(f"Layer check failed ({DOC}):")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"Layer check passed: {len(graph)} crates, {len(config.get('allowed', []))} exceptions left "
          f"to move ({DOC}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
