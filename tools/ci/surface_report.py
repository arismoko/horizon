"""The mod API surface report: what a pull request changes in what mods can name.

In CI, each tree's `sunrise-stage` exports its own SDK surface. Catalog readers must match the source layout.
The head's tool compares the surfaces and writes the classified report to the job summary. CI compares published loops, not listed
internals. On `pull_request`, the checkout is GitHub's merge commit, whose first parent is the base, and a dispatch
with `pr` checks out that pull request's test merge the same way (tools/ci/pr_gate.py). CI builds
the base's tool from a full archive of that commit, including `crates/`, `tools/layering`, Cargo manifests, and
workspace members outside `crates/` (such as `docs/modding/examples/`). A Dyna slot keeps `target/surface-base-v3`:
its Cargo target, build directory, mtime snapshot and lock share one protocol version. Under that lock, the
archive's Git objects reconcile the snapshot before Cargo builds. Older writers keep their own directory, so
an older build cannot leave artifacts that the current snapshot doesn't describe.
Only that one commit is fetched (`--depth 1`, in CI's shallow checkout; a full clone is never fetched shallow).

Locally, `--base SHA` compares this checkout against that commit exactly as CI compares against its base (the base's
own sunrise-stage, the landed-note check, the "Base" line); tools/pr_ready.py runs it with the branch's merge base.
Without SUNRISE_STAGE_TOOL the head's tool is built by `build_tool`: `cargo build --locked --profile test -p
sunrise-stage --bin sunrise-stage` in this checkout, with the extra cargo arguments in $JOBS. CI's "Build the source
stage generator" step runs `--build-tool`, which does the same, appends SUNRISE_STAGE_TOOL to $GITHUB_ENV for the
report step, and prints the path. `--base` and `--trees` are two ways to pick the base, so they do not go together,
and neither goes with `--build-tool`.

Use --with-internals to audit listed internals too. With --trees BASE HEAD, use --build-base-tool when the generator
changed: it always builds and uses BASE's own `sunrise-stage`, even if HEAD's could read the old sources. Only pass
it for a whole checkout you trust: it compiles and runs BASE's code, build scripts included. The build uses the audit's
scratch target, not the shared archive cache. Without the flag, --trees is a source-only comparison for two trees that
use the same generator.

It passes when the CI base cannot be found or fetched, saying so in one line. It fails if either export cannot be
compared, for a new change note that names an id that did not change or cannot be read, and, with `gate = true` in
tools/ci/surface.toml (on), for a breaking change no new note names.

A new note that carries `landed = "<merge sha>"` is retroactive: it is left out of the comparison above and checked
against that merge instead. Both the merge and its first parent must already use the native-only surface schema.
Each is exported by its own tool to verify its schema; `surface-diff --landed` then fails if an id did not change there.
The notes that merge brought itself (new against its first parent) count as coverage there, read apart from the late
ones (`--covered-by`), so the comparison lists as unnoted only what no note covers, then or now. That costs two exports,
only on a pull request that adds such a note. A merge that does not have two parents, or a tree that cannot be
exported, fails naming the note and the sha.
"""

from argparse import ArgumentParser
from collections.abc import Callable, Sequence
from concurrent.futures import ThreadPoolExecutor
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import file_lock
from tools.ci.source_mtimes import restore_tree_mtimes

ROOT = Path(__file__).resolve().parents[2]
NOTES = "crates/sunrise-mod-vocab/changes"
SOURCE_LAYOUT = "crates/sunrise-mod-api-gen/source-layout"
# Remove this audit and its gate branch after the first complete schema-3 SDK publishes.
SCHEMA_RESET = "tools/ci/surface-schema-3-reset.json"
# GitHub refuses a step summary over 1 MiB; the full report stays in the step log.
# Counted in bytes of UTF-8, frame included.
SUMMARY_LIMIT = 200_000


def gate(config: Path) -> bool:
    value = tomllib.loads(config.read_text())["surface"]["gate"]
    if not isinstance(value, bool):
        raise SystemExit(f"{config}: surface.gate must be true or false")
    return value


def merge_parent(commit_text: str) -> str | None:
    """The first parent of a merge commit (`git cat-file -p` text), or None for any other commit."""
    parents = [line.split()[1] for line in commit_text.splitlines() if line.startswith("parent ")]
    return parents[0] if len(parents) == 2 else None


def base_commit(event: str, pr: str, git) -> tuple[str | None, str]:
    """The commit to compare against, or None with the reason.

    A pull_request run and a dispatch with `pr` both check out the pull request's test merge: its first parent is the
    base the pull request would merge into."""
    if event == "pull_request" or (event == "workflow_dispatch" and pr):
        parent = merge_parent(git("cat-file", "-p", "HEAD"))
        return (parent, "") if parent else (None, "the checkout is not a pull request's merge commit")
    return None, f"no base for a {event or 'local'} run"


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def fetch_commit(sha: str, depth: int, git_text=git) -> None:
    """Fetches `sha` with `depth` commits of history. Only a checkout that is already shallow (CI's) is fetched
    shallow: in a full clone a shallow fetch grafts the commit and its pushes are refused, so there the commit is
    fetched in full, and not at all when it is already there."""
    if git_text("rev-parse", "--is-shallow-repository").strip() == "true":
        git_text("fetch", "-q", "--no-tags", "--depth", str(depth), "origin", sha)
        return
    try:
        git_text("cat-file", "-e", f"{sha}^{{commit}}")
    except subprocess.CalledProcessError:
        git_text("fetch", "-q", "--no-tags", "origin", sha)


def archive_paths(tree: str) -> tuple[str, ...]:
    """The source needed to export a surface at `tree`, including libraries outside crates/."""
    manifest = tomllib.loads(git("show", f"{tree}:Cargo.toml"))
    members = manifest.get("workspace", {}).get("members", [])
    extras = sorted({member for member in members if not member.startswith("crates/")})
    return ("crates", "tools/layering", ":(glob)**/Cargo.toml", *extras)


def unpack(sha: str, into: Path, paths: tuple[str, ...] | None = None) -> str:
    """Extract the selected paths (empty tuple for the full tree) and return the exact tree ID."""
    fetch_commit(sha, 1)
    tree = git("rev-parse", "--verify", f"{sha}^{{tree}}").strip()
    if paths is None:
        paths = archive_paths(tree)
    with subprocess.Popen(["git", "archive", tree, *paths], cwd=ROOT, stdout=subprocess.PIPE) as archive:
        subprocess.run(["tar", "-x", "-C", str(into)], stdin=archive.stdout, check=True)
    if archive.returncode != 0:
        raise subprocess.CalledProcessError(archive.returncode, "git archive")
    return tree


def base_tool(sha: str, work: Path, *, archive_name: str | None = None) -> list[str]:
    """Build a commit's own exporter from a fresh full archive, the only source allowed in the kept target."""
    # Version the whole cache together: a writer that doesn't update this snapshot must not share its artifacts.
    target = ROOT / "target" / "surface-base-v3"
    target.mkdir(parents=True, exist_ok=True)
    with open(target / "surface-report.lock", "a") as lock:
        file_lock.lock(lock)
        tree = work / (archive_name or f"base-tool-{sha[:12]}")
        tree.mkdir()
        identity = unpack(sha, tree, ())
        restore_tree_mtimes(ROOT, identity, tree, target / "ci-source-mtimes-v3.json")
        return tree_tool(tree, work, target)


def tree_tool(tree: Path, work: Path, target: Path) -> list[str]:
    """Build a tree's sunrise-stage and copy it into the audit's scratch directory."""
    target = target.resolve()
    suffix = ".exe" if sys.platform == "win32" else ""  # Cargo names the program so on Windows, and Windows runs only that
    tool = work / f"base-stage-{len(list(work.glob('base-stage-*')))}{suffix}"
    built = subprocess.run(["cargo", "build", "--locked", "--profile", "test",
                            "--bin", "sunrise-stage", "--target-dir", str(target)],
                           cwd=tree, capture_output=True, text=True,
                           env={**os.environ, "CARGO_BUILD_BUILD_DIR": str(target)})
    if built.returncode != 0:
        raise RuntimeError(f"building that tree's sunrise-stage failed: {last_line(built.stderr)}")
    shutil.copy2(target / "debug" / f"sunrise-stage{suffix}", tool)
    return [str(tool)]


def build_tool(jobs: list[str]) -> str:
    """The head's sunrise-stage, built in the test profile in this checkout (`jobs`: extra cargo arguments, CI's
    $JOBS), as the path of the one normal executable the compiler reports; this is the only place that picks it."""
    built = subprocess.run(["cargo", "build", *jobs, "--locked", "--profile", "test", "-p", "sunrise-stage",
                            "--bin", "sunrise-stage", "--message-format=json-render-diagnostics"],
                           cwd=ROOT, stdout=subprocess.PIPE, text=True)
    if built.returncode != 0:
        raise SystemExit("building the head's sunrise-stage failed")
    artifacts = [json.loads(line) for line in built.stdout.splitlines()]
    executables = {item["executable"] for item in artifacts
                   if item.get("reason") == "compiler-artifact"
                   and item["target"]["name"] == "sunrise-stage"
                   and "bin" in item["target"]["kind"]
                   and not item["profile"]["test"] and item.get("executable")}
    if len(executables) != 1:
        raise SystemExit(f"expected one normal sunrise-stage executable, got {executables}")
    executable = str(Path(executables.pop()).resolve(strict=True))
    if "\r" in executable or "\n" in executable:
        raise SystemExit("sunrise-stage executable path must not contain CR or LF")
    return executable


def source_layout(tree: Path) -> str:
    try:
        layout = (tree / SOURCE_LAYOUT).read_text(encoding="utf-8").strip()
    except FileNotFoundError:
        return "legacy"
    if layout not in ("legacy", "split"):
        raise ValueError(f"{tree / SOURCE_LAYOUT}: unknown source layout {layout!r}")
    return layout


def last_line(text: str) -> str:
    lines = [line for line in text.splitlines() if line.strip() and not line.startswith("unclassified:")]
    return lines[-1] if lines else "no message"


def export_surface(tool: Sequence[str], tree: Path, output: Path, *flags: str) -> subprocess.CompletedProcess:
    return subprocess.run([*tool, "surface", str(tree), str(output), *flags], capture_output=True, text=True)


def audited_checkpoint(reset_path: Path, work: Path, old_hash: str) -> tuple[Path | None, str, str | None]:
    """Verify the one-time transition using both plain SDK exports, before adding any engine catalogs."""
    reset = json.loads(reset_path.read_text())
    plain = reset["plain"]
    if old_hash != plain["old"]:
        return None, f"The schema-2 base {old_hash} is not the audited schema-reset base.\n", None
    missing = [change["id"] for change in reset["diff"]["changes"]
               if change["class"] == "breaking" and not change.get("notes")]
    if missing:
        return None, f"The schema-reset audit lacks change notes for {', '.join(missing)}.\n", None
    checkpoint = work / "schema-reset"
    output = work / "base.json"
    commit = reset["checkpoint"]
    try:
        tool = base_tool(commit, work, archive_name=checkpoint.name)
        done = export_surface(tool, checkpoint, output)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        return None, f"Could not export the schema-reset checkpoint: {error}.\n", None
    if done.returncode != 0:
        return None, f"Could not export the schema-reset checkpoint: {last_line(done.stderr)}.\n", None
    actual = json.loads(output.read_text())
    if (actual["schema"], actual["hash"]) != (3, plain["new"]):
        return None, "The schema-reset checkpoint does not reproduce the audited schema-3 surface.\n", None
    return checkpoint, (f"Audited schema 2 → 3 reset: {plain['old']} → {plain['new']} ({SCHEMA_RESET}).\n"
                        "Checking all subsequent changes against that checkpoint.\n"), tool


def report(tool: Sequence[str], base: Path, head: Path, work: Path, gate_on: bool, *,
           with_internals: bool = False, own_base_tool: Callable[[], Sequence[str]] | None = None,
           own_head_tool: Callable[[], Sequence[str]] | None = None,
           notes: tuple[Path, Path] | None = None, landed: str | None = None,
           covered: tuple[Path, Path] | None = None) -> tuple[int, str]:
    """Export and compare the trees; return (exit code, report text).

    When given, `own_base_tool` supplies the base's exporter. Landed comparisons require their own on both sides.
    `notes` overrides the trees' (new, base) note folders; `landed` selects the notes of one merge, and `covered`
    is the (new, base) pair whose new notes count only as coverage (the merge's own).
    """
    catalogs = "--with-internals" if with_internals else "--with-loops"
    reader_layout = source_layout(ROOT)
    if own_base_tool is None and landed is None and source_layout(base) != reader_layout:
        def own_base_tool():
            return tree_tool(base, work, work / "base-target")

    own_exports = []
    if own_base_tool is not None or landed is not None:
        own_exports.append(("base", base, own_base_tool))
    if landed is not None:
        own_exports.append(("head", head, own_head_tool))
    elif source_layout(head) != reader_layout:
        def head_tool():
            return tree_tool(head, work, work / "head-target")

        own_exports.append(("head", head, head_tool))
    if landed is None:
        with ThreadPoolExecutor(max_workers=1) as pool:
            # Only source-only audits read both sides with the head's generator. Reading different sources
            # successfully does not prove it generated the same types and signatures as the base's generator.
            base_export = (pool.submit(export_surface, tool, base, work / "base.json", catalogs)
                           if own_base_tool is None else None)
            head_export = (export_surface(tool, head, work / "head.json", catalogs)
                           if source_layout(head) == reader_layout else None)
        if head_export is not None and head_export.returncode != 0:
            return 1, f"The surface export of this tree failed: {last_line(head_export.stderr)}.\n"
        if base_export is not None and (done := base_export.result()).returncode != 0:
            return 1, f"No surface report: the head's sunrise-stage cannot read the base tree ({last_line(done.stderr)}).\n"
    note = ""
    diff_tool = tool
    for name, tree, own_tool in own_exports:
        if own_tool is None:
            return 1, f"No surface report: the landed {name} must be exported in its original schema.\n"
        try:
            output = work / f"{name}.json"
            endpoint_tool = own_tool()
            own = export_surface(endpoint_tool, tree, output)
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            return 1, f"No surface report: the {name}'s own sunrise-stage could not be used: {error}.\n"
        if own.returncode != 0:
            return 1, (f"No surface report: the {name}'s own sunrise-stage could not export its tree "
                       f"({last_line(own.stderr)}).\n")
        note += f"The {name} was exported by its own sunrise-stage.\n"
        reset_path = head / SCHEMA_RESET
        if landed is not None:
            schema = json.loads(output.read_text())["schema"]
            if schema < 3:
                side = "first parent" if name == "base" else "merge"
                return 1, (f"The {side} uses surface schema {schema}; a landed note requires both trees to use "
                           "the native-only surface schema (3 or later). Put notes for older merges in changes/history/.\n")
        elif name == "base" and not with_internals and reset_path.is_file():
            old = json.loads(output.read_text())
            if old.get("schema") == 2:
                checkpoint, text, endpoint_tool = audited_checkpoint(reset_path, work, old["hash"])
                note += text
                if checkpoint is None:
                    return 1, note
                base = tree = checkpoint
        catalog_tool = tool if source_layout(tree) == reader_layout else endpoint_tool
        if name == "head" and landed is None:
            diff_tool = catalog_tool
        catalogs_command = [*catalog_tool, "surface-catalogs", str(tree), str(output)]
        if with_internals:
            catalogs_command.append("--with-internals")
        audited = subprocess.run(catalogs_command, capture_output=True, text=True)
        if audited.returncode != 0:
            return 1, f"No surface report: could not audit the {name} catalogs ({last_line(audited.stderr)}).\n"
    code, text = compare(diff_tool, base, head, work, gate_on, with_internals=with_internals, notes=notes, landed=landed,
                         covered=covered)
    return code, note + text


def compare(tool: Sequence[str], base: Path, head: Path, work: Path, gate_on: bool, *,
            with_internals: bool, notes: tuple[Path, Path] | None = None,
            landed: str | None = None, covered: tuple[Path, Path] | None = None) -> tuple[int, str]:
    """Run the gate on the exported documents and the change notes (the trees' own unless `notes` is given);
    `covered` is a (new, base) pair of note folders whose new notes count as coverage only."""
    new_notes, base_notes = notes or (head / NOTES, base / NOTES)
    command = [*tool, "surface-diff", str(work / "base.json"), str(work / "head.json"),
               "--notes", str(new_notes), "--base-notes", str(base_notes), "--json", str(work / "diff.json")]
    if landed:
        command += ["--landed", landed]
    if covered:
        command += ["--covered-by", str(covered[0]), "--covered-base", str(covered[1])]
    if gate_on:
        command.append("--gate")
    diff = subprocess.run(command, capture_output=True, text=True)
    # A base surface of another schema (one the base's own tool wrote) cannot be compared: say so, and fail.
    base_schema = f"{work / 'base.json'}: surface schema "
    if diff.returncode == 2 and base_schema in diff.stderr:
        return 1, f"No surface report: the head's sunrise-stage cannot read the base surface ({last_line(diff.stderr)}).\n"
    scope = "" if with_internals else (
        "CI does not compare listed internals; use surface_report.py --with-internals for a local audit.\n")
    return (0 if diff.returncode == 0 else 1), scope + diff.stdout + diff.stderr


def landed_notes(head: Path, base: Path) -> dict[str, list[str]]:
    """The new notes of `head` that carry `landed`, by merge sha. New is the reader's rule: a file name or content
    the base does not have. An unreadable note is the PR's own comparison's to report, so it is skipped here."""
    old = list((base / NOTES).glob("*.toml"))
    names, contents = {path.name for path in old}, {path.read_bytes() for path in old}
    found: dict[str, list[str]] = {}
    for path in sorted((head / NOTES).glob("*.toml")):
        if path.name in names or path.read_bytes() in contents:
            continue
        try:
            sha = tomllib.loads(path.read_text()).get("landed")
        except (tomllib.TOMLDecodeError, UnicodeDecodeError):
            continue
        # Exactly what the reader accepts, lowercased: the sha reaches git and a folder name. Anything else is
        # the reader's to refuse in the pull request's own comparison.
        if isinstance(sha, str) and re.fullmatch(r"[0-9a-f]{40}", sha := sha.lower()):
            found.setdefault(sha, []).append(path.name)
    return found


def landed_check(tool: Sequence[str], head: Path, base: Path, work: Path, shas: dict[str, list[str]], *,
                 git_text=None, fetch=None, own_tool: Callable[[str], Sequence[str]] | None = None) -> tuple[int, str]:
    """Check each merge's notes against that merge's first parent and the merge; (exit code, text)."""
    git_text, fetch = git_text or git, fetch or unpack
    code, text = 0, ""
    for sha, files in sorted(shas.items()):
        who = f"{', '.join(files)} (landed {sha})"
        folder = work / f"landed-{sha}"
        try:
            # Two commits deep, so the merge's parents are known.
            fetch_commit(sha, 2, git_text)
            parent = merge_parent(git_text("cat-file", "-p", sha))
        except (OSError, subprocess.CalledProcessError) as error:
            return 1, text + f"Landed check failed for {who}: the merge could not be fetched: {error}.\n"
        if parent is None:
            return 1, text + f"Landed check failed for {who}: it is not a merge commit with two parents.\n"
        trees = {"base": parent, "head": sha}
        try:
            for name, commit in trees.items():
                (folder / name).mkdir(parents=True)
                fetch(commit, folder / name)
        except (OSError, subprocess.CalledProcessError) as error:
            return 1, text + f"Landed check failed for {who}: a tree could not be fetched: {error}.\n"
        # The late notes are this pull request's, checked against the merge; the merge's own notes (new against its
        # first parent) are read apart from them, as coverage.
        for tree in ("base", "head"):
            (folder / tree / NOTES).mkdir(parents=True, exist_ok=True)
        sub_code, sub_text = report(tool, folder / "base", folder / "head", folder, False,
                                    own_base_tool=(lambda: own_tool(parent)) if own_tool else None,
                                    own_head_tool=(lambda: own_tool(sha)) if own_tool else None,
                                    notes=(head / NOTES, base / NOTES), landed=sha,
                                    covered=(folder / "head" / NOTES, folder / "base" / NOTES))
        if sub_code != 0:
            code = 1
        text += f"Landed {who}, first parent {parent}:\n{sub_text}"
    return code, text


def summary(text: str) -> str:
    """The report for the job summary, at most SUMMARY_LIMIT bytes of UTF-8 in all."""
    head, foot = "### Mod API surface\n\n```text\n", "```\n"
    # Room for the frame and for the line saying what was cut.
    budget = SUMMARY_LIMIT - len((head + foot).encode()) - 64
    data = text.encode()
    if len(data) > budget:
        kept = data[:budget]
        line_end = kept.rfind(b"\n")
        # A line longer than the budget is cut inside itself, on a character boundary.
        kept = kept[: line_end + 1] if line_end >= 0 else kept
        shown = kept.decode(errors="ignore")
        rest = text[len(shown) :].count("\n")
        lines = "line" if rest == 1 else "lines"
        text = shown.rstrip("\n") + f"\n… cut here; {rest} more {lines} in the step log\n"
    return head + text + foot


def main(argv=None) -> int:
    parser = ArgumentParser(description="Compare mod API surfaces and published engine catalogs.")
    parser.add_argument("--with-internals", action="store_true", help="include advisory listed-internal changes")
    parser.add_argument("--trees", nargs=2, metavar=("BASE", "HEAD"), type=Path,
                        help="compare two local trees with the same generator instead of fetching the CI base; "
                             "use --build-base-tool if the generator changed")
    parser.add_argument("--base", metavar="COMMIT",
                        help="compare against this commit (a sha or a ref such as origin/main) as CI compares "
                             "against its base, including the base's own sunrise-stage (a local run)")
    parser.add_argument("--build-tool", action="store_true",
                        help="only build the head's sunrise-stage (cargo arguments from $JOBS), append "
                             "SUNRISE_STAGE_TOOL=<path> to $GITHUB_ENV when that is set, and print the path")
    parser.add_argument("--build-base-tool", action="store_true",
                        help="with --trees: always build and use BASE's own sunrise-stage "
                             "(runs BASE's code; a whole, trusted checkout only)")
    args = parser.parse_args(argv)
    if args.build_base_tool and not args.trees:
        parser.error("--build-base-tool goes with --trees; CI builds the base commit's own tool by itself")
    if args.base and args.trees:
        parser.error("--base and --trees both pick the base; use one")
    if args.build_tool and (args.base or args.trees):
        parser.error("--build-tool only builds the tool; it goes with neither --base nor --trees")
    jobs = os.environ.get("JOBS", "").split()
    if args.build_tool:
        built_at = time.monotonic()
        tool = build_tool(jobs)
        print(f"Stage generator build: {int(time.monotonic() - built_at)}s")
        if environment_file := os.environ.get("GITHUB_ENV"):
            with open(environment_file, "a") as environment:
                environment.write(f"SUNRISE_STAGE_TOOL={tool}\n")
        print(tool)
        return 0
    tool = [os.environ.get("SUNRISE_STAGE_TOOL") or build_tool(jobs)]
    started = time.monotonic()
    gate_on = gate(ROOT / "tools/ci/surface.toml")
    with tempfile.TemporaryDirectory(dir=os.environ.get("RUNNER_TEMP")) as directory:
        work = Path(directory)
        if args.trees:
            base, head = args.trees
            own = (lambda: tree_tool(base, work, work / "base-target")) if args.build_base_tool else None
            code, text = report(tool, base, head, work, gate_on, with_internals=args.with_internals,
                                own_base_tool=own)
        else:
            base = work / "base"
            try:
                # The base's tool tree and the report's last line take the commit's sha, so a ref such as
                # origin/main is resolved first; a full sha resolves to itself, fetched or not.
                sha, why = (git("rev-parse", "--verify", args.base).strip(), "") if args.base else base_commit(
                    os.environ.get("EVENT", ""), os.environ.get("PR", ""), git)
                if sha:
                    base.mkdir()
                    unpack(sha, base)
            except (OSError, subprocess.CalledProcessError, KeyError, ValueError) as error:
                sha, why = None, f"the base could not be fetched: {error}"
            if sha:
                fetched = time.monotonic() - started
                built: dict[str, Sequence[str]] = {}

                def own(commit: str) -> Sequence[str]:
                    """A commit's own sunrise-stage, built once per run: a landed note may name the base itself."""
                    if commit not in built:
                        built[commit] = base_tool(commit, work)
                    return built[commit]

                code, text = report(tool, base, ROOT, work, gate_on, with_internals=args.with_internals,
                                    own_base_tool=lambda: own(sha))
                if shas := landed_notes(ROOT, base):
                    landed_code, landed_text = landed_check(tool, ROOT, base, work, shas, own_tool=own)
                    code, text = code or landed_code, text + landed_text
                text += f"Base {sha}; fetch {fetched:.1f}s, total {time.monotonic() - started:.1f}s\n"
            else:
                # CI passes without a base it can find; a base named with --base that cannot be read is an error.
                code, text = (1 if args.base else 0), f"No surface report: {why}.\n"
    print(text, end="")
    if summary_path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary_path, "a") as out:
            out.write(summary(text))
    return code


if __name__ == "__main__":
    sys.exit(main())
