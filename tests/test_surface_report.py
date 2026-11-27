"""The surface report step: which base it compares, when it passes, when it fails."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("surface_report", ROOT / "tools/ci/surface_report.py")
surface_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(surface_report)

def python_stub(path, source, **values):
    """Writes `source` as a script under `path`, `values` bound as names above it, and returns the command that runs
    it with the Python running the tests: a stand-in `sunrise-stage` that every platform can run."""
    path.write_text("".join(f"{name} = {value!r}\n" for name, value in values.items()) + source)
    return [sys.executable, str(path)]


STAGE_STUB = r"""
import os
import sys
from pathlib import Path

args = sys.argv[1:]
with open(Path(WORK, "calls"), "a") as calls:
    calls.write(" ".join(args) + "\n")
if args[0] == "surface":
    if args[1].endswith("base") and BASE_EXPORT != 0:
        print("unclassified: x", file=sys.stderr)
        print("sunrise-stage surface: unknown key `access`", file=sys.stderr)
        sys.exit(1)
    if args[1].endswith("head") and HEAD_EXPORT != 0:
        print("sunrise-stage surface: a.rs: parse error", file=sys.stderr)
        sys.exit(1)
    Path(args[2]).write_text("{}\n")
    sys.exit(0)
if args[0] == "surface-catalogs":
    sys.exit(0)
print("Surface: report")
if not DIFF_ERR and DIFF_EXIT != 0:
    print("breaking change without a change note: fn:a", file=sys.stderr)
if DIFF_ERR:
    print(f"sunrise-stage: {args[1]}: {DIFF_ERR}", file=sys.stderr)
sys.exit(DIFF_EXIT)
"""

# The base's own tool: it exports with the exit status OWN_EXPORT.
OWN_STUB = r"""
import sys
from pathlib import Path

args = sys.argv[1:]
with open(Path(WORK, "calls"), "a") as calls:
    calls.write("own " + " ".join(args) + "\n")
if len(args) != 3:
    sys.exit(2)
if OWN_EXPORT != 0:
    print("sunrise-stage surface: base.rs: older form", file=sys.stderr)
    sys.exit(1)
Path(args[2]).write_text("{}\n")
"""

# Records its arguments, the two documents it compares and the note folders it is given.
LANDED_STUB = r"""
import os
import sys
from pathlib import Path

args = sys.argv[1:]
with open(Path(WORK, "calls"), "a") as calls:
    calls.write(" ".join(args) + "\n")
if args[0] == "surface":
    sys.exit(1)  # landed must not ask the current exporter to read a historical tree.
if args[0] == "surface-catalogs":
    sys.exit(0)
Path(WORK, "compared").write_text(Path(args[1]).read_text() + Path(args[2]).read_text())
for flag, value in zip(args, args[1:]):
    if flag in ("--notes", "--base-notes", "--covered-by", "--covered-base"):
        Path(WORK, "given" + flag[2:]).write_text("".join(name + "\n" for name in sorted(os.listdir(value))))
if DIFF_EXIT != 0:
    print("change note r61.toml names loop:a, which did not change", file=sys.stderr)
sys.exit(DIFF_EXIT)
"""

MERGE = "tree t\nparent aaaa\nparent bbbb\nauthor x\n\nMerge bbbb into aaaa\n"
SINGLE = "tree t\nparent aaaa\nauthor x\n\nA commit\n"


class BaseCommitTests(unittest.TestCase):
    def test_a_pull_request_compares_against_its_merge_commits_first_parent(self):
        sha, _ = surface_report.base_commit("pull_request", "", lambda *a: MERGE)
        self.assertEqual(sha, "aaaa")
        sha, _ = surface_report.base_commit("pull_request", "", lambda *a: SINGLE)
        self.assertIsNone(sha)

    def test_a_dispatch_compares_against_its_test_merges_first_parent(self):
        sha, _ = surface_report.base_commit("workflow_dispatch", "12", lambda *a: MERGE)
        self.assertEqual(sha, "aaaa")
        # A dispatch without `pr` checks main's own commit: there is no base to compare.
        sha, _ = surface_report.base_commit("workflow_dispatch", "", lambda *a: MERGE)
        self.assertIsNone(sha)

    def test_a_push_has_no_base(self):
        sha, why = surface_report.base_commit("push", "", None)
        self.assertIsNone(sha)
        self.assertEqual(why, "no base for a push run")


class GateTests(unittest.TestCase):
    def test_the_tracked_switch_is_read_and_must_be_a_boolean(self):
        self.assertIs(surface_report.gate(ROOT / "tools/ci/surface.toml"), True)
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory, "surface.toml")
            config.write_text("[surface]\ngate = true\n")
            self.assertIs(surface_report.gate(config), True)
            config.write_text("[surface]\ngate = \"yes\"\n")
            with self.assertRaises(SystemExit):
                surface_report.gate(config)


class FetchTests(unittest.TestCase):
    """fetch_commit(): shallow only where the checkout already is."""

    def calls(self, shallow, present):
        made = []

        def git(*args):
            made.append(args)
            if args[0] == "rev-parse":
                return "true\n" if shallow else "false\n"
            if args[0] == "cat-file" and not present:
                raise subprocess.CalledProcessError(128, ["git", *args])
            return ""

        surface_report.fetch_commit("abcd", 2, git)
        return [args for args in made if args[0] == "fetch"]

    def test_a_shallow_checkout_fetches_shallow(self):
        self.assertEqual(self.calls(shallow=True, present=False),
                         [("fetch", "-q", "--no-tags", "--depth", "2", "origin", "abcd")])

    def test_a_full_clone_is_never_fetched_shallow(self):
        self.assertEqual(self.calls(shallow=False, present=True), [])
        self.assertEqual(self.calls(shallow=False, present=False), [("fetch", "-q", "--no-tags", "origin", "abcd")])


class ArchiveTests(unittest.TestCase):
    def test_a_base_archive_keeps_sources_for_members_outside_crates(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, base = Path(directory, "repo"), Path(directory, "base")
            base.mkdir()
            member = "docs/examples"
            workspace = f'[workspace]\nmembers = ["{member}"]\n'
            manifest = '[package]\nname = "example-package"\n[lib]\nname = "example_library"\n'
            for name, content in {
                "Cargo.toml": workspace,
                f"{member}/Cargo.toml": manifest,
                f"{member}/src/lib.rs": "pub struct Example;\n",
                f"{member}/src/nested.rs": "pub struct Nested;\n",
                "crates/game/src/lib.rs": "",
                "tools/layering/config.toml": "",
            }.items():
                file = repo / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(content)
            with mock.patch.object(surface_report, "ROOT", repo):
                surface_report.git("init", "-q")
                surface_report.git("add", ".")
                surface_report.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                                   "-c", "commit.gpgsign=false", "commit", "-qm", "archive fixture")
                surface_report.unpack("HEAD", base)
            self.assertEqual((base / "Cargo.toml").read_text(), workspace)
            self.assertEqual((base / member / "Cargo.toml").read_text(), manifest)
            self.assertEqual((base / member / "src/lib.rs").read_text(), "pub struct Example;\n")
            self.assertEqual((base / member / "src/nested.rs").read_text(), "pub struct Nested;\n")


class ReportTests(unittest.TestCase):
    """report() against a stub sunrise-stage whose behaviour each test sets."""

    def run_report(self, base_export=0, diff_exit=0, gate_on=False, head_export=0, diff_err="",
                   with_internals=False, own_export=None):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            tool = python_stub(work / "stage.py", STAGE_STUB, WORK=str(work), BASE_EXPORT=base_export,
                               HEAD_EXPORT=head_export, DIFF_EXIT=diff_exit, DIFF_ERR=diff_err)
            own = None
            if own_export is not None:
                own_tool = python_stub(work / "own_stage.py", OWN_STUB, WORK=str(work), OWN_EXPORT=own_export)
                own = lambda: own_tool  # noqa: E731
            for side in ("base", "head"):
                marker = work / side / surface_report.SOURCE_LAYOUT
                marker.parent.mkdir(parents=True, exist_ok=True)
                marker.write_text(surface_report.source_layout(surface_report.ROOT) + "\n")
            code, text = surface_report.report(tool, work / "base", work / "head", work, gate_on,
                                               with_internals=with_internals, own_base_tool=own)
            return code, text, (work / "calls").read_text()

    def test_the_bases_own_export_is_augmented_with_the_heads_catalog_reader(self):
        code, text, calls = self.run_report(own_export=0, with_internals=True)
        self.assertEqual(code, 0)
        self.assertIn("The base was exported by its own sunrise-stage.\n", text)
        self.assertIn("Surface: report\n", text)
        own, catalogs = (next(call.split() for call in calls.splitlines() if call.startswith(prefix))
                         for prefix in ("own ", "surface-catalogs "))
        self.assertEqual(own[:2], ["own", "surface"])
        self.assertEqual(catalogs[1:], [*own[2:], "--with-internals"])
        self.assertLess(calls.index("surface-catalogs "), calls.index("surface-diff "))

    def test_cross_layout_endpoints_use_their_own_catalog_reader(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            base, head = work / "base", work / "head"
            marker = head / surface_report.SOURCE_LAYOUT
            marker.parent.mkdir(parents=True)
            marker.write_text("split\n")
            for owner in ("common", "server", "client"):
                game = head / "crates" / f"fixture-{owner}" / "src" / "game"
                game.mkdir(parents=True)
                (game / "mod.rs").write_text(f'include!("{owner}.rs");')
                (game / f"{owner}.rs").write_text(f"pub fn {owner}() {{}}")
            calls = []

            def run(command, **kwargs):
                tool, action, *args = command
                calls.append((tool, action, args[0]))
                if action == "surface":
                    Path(args[1]).write_text('{"schema": 3}')
                return subprocess.CompletedProcess(command, 0, "", "")

            with mock.patch.object(surface_report.subprocess, "run", side_effect=run), \
                    mock.patch.object(surface_report, "tree_tool", return_value=["split-stage"]), \
                    mock.patch.object(surface_report, "ROOT", base):
                code, _ = surface_report.report(["legacy-stage"], base, head, work, False)
                self.assertEqual(code, 0)
                self.assertIn(("split-stage", "surface-catalogs", str(head)), calls)
                self.assertEqual(calls[-1][:2], ("split-stage", "surface-diff"))

            calls.clear()
            with mock.patch.object(surface_report.subprocess, "run", side_effect=run), \
                    mock.patch.object(surface_report, "ROOT", head):
                code, _ = surface_report.report(
                    ["split-stage"], base, head, work, False, own_base_tool=lambda: ["legacy-stage"],
                    own_head_tool=lambda: ["split-stage"], landed="fixture")
                self.assertEqual(code, 0)
                self.assertIn(("legacy-stage", "surface-catalogs", str(base)), calls)
                self.assertIn(("split-stage", "surface-catalogs", str(head)), calls)
                self.assertEqual(calls[-1][:2], ("split-stage", "surface-diff"))

    def test_an_unreadable_base_fails_naming_the_selected_exporter(self):
        code, text, calls = self.run_report(own_export=1)
        self.assertEqual(code, 1)
        self.assertEqual(text, "No surface report: the base's own sunrise-stage could not export its tree "
                               "(sunrise-stage surface: base.rs: older form).\n")
        self.assertNotIn("surface-diff", calls)
        code, text, calls = self.run_report(base_export=1)
        self.assertEqual(code, 1)
        self.assertEqual(text, "No surface report: the head's sunrise-stage cannot read the base tree "
                               "(sunrise-stage surface: unknown key `access`).\n")
        self.assertNotIn("surface-diff", calls)

    def test_a_base_surface_of_another_schema_fails_with_one_line(self):
        code, text, _ = self.run_report(diff_exit=2, diff_err="surface schema 1, this tool reads 2")
        self.assertEqual(code, 1)
        self.assertTrue(text.startswith("No surface report: the head's sunrise-stage cannot read the base surface "), text)
        self.assertIn("surface schema 1, this tool reads 2", text)

    def test_the_schema_reset_checks_only_the_audited_base_and_still_gates_later_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            base, head = work / "old", work / "new"
            reset = head / surface_report.SCHEMA_RESET
            reset.parent.mkdir(parents=True)
            marker = head / surface_report.SOURCE_LAYOUT
            marker.parent.mkdir(parents=True)
            marker.write_text(surface_report.source_layout(surface_report.ROOT) + "\n")
            audit = {
                "checkpoint": "checked",
                "plain": {"old": "old-hash", "new": "checkpoint-hash"},
                "diff": {"old": "old-with-catalogs", "new": "checkpoint-with-catalogs",
                         "changes": [{"id": "fn:removed", "class": "breaking", "notes": ["native-only.toml"]}]},
            }
            reset.write_text(json.dumps(audit))

            def export(command, **_kwargs):
                tool, action, root, path, *flags = command
                if action == "surface-catalogs":
                    self.assertEqual(Path(root), work / "schema-reset")
                    self.assertEqual(json.loads(Path(path).read_text())["hash"], "checkpoint-hash")
                    self.assertEqual(flags, [])
                    Path(path).write_text(json.dumps({"schema": 3, "hash": "checkpoint-with-loops"}))
                    return subprocess.CompletedProcess(command, 0, "", "")
                self.assertEqual(flags, ["--with-loops"] if root == str(head) else [])
                schema, digest = {
                    ("old", "old-tool"): (2, "old-hash"),
                    ("new", "stage"): (3, "some-new-change"),
                    ("schema-reset", "checkpoint-tool"): (3, "checkpoint-hash"),
                }[(Path(root).name, tool)]
                Path(path).write_text(json.dumps({"schema": schema, "hash": digest}))
                return subprocess.CompletedProcess(command, 0, "", "")

            with (mock.patch.object(surface_report.subprocess, "run", side_effect=export),
                  mock.patch.object(surface_report, "base_tool", return_value=["checkpoint-tool"]) as base_tool,
                  mock.patch.object(surface_report, "compare", return_value=(1, "later change refused")) as compare):
                code, text = surface_report.report(["stage"], base, head, work, True, own_base_tool=lambda: ["old-tool"])
                self.assertEqual(code, 1)
                self.assertIn("Audited schema 2 → 3 reset", text)
                self.assertIn("later change refused", text)
                base_tool.assert_called_once_with("checked", work, archive_name="schema-reset")
                self.assertEqual(compare.call_args.args[1:5], (work / "schema-reset", head, work, True))
                self.assertEqual(json.loads((work / "base.json").read_text())["hash"], "checkpoint-with-loops")

                audit["plain"]["old"] = "unrelated-base"
                reset.write_text(json.dumps(audit))
                compare.reset_mock()
                code, text = surface_report.report(["stage"], base, head, work, True, own_base_tool=lambda: ["old-tool"])
                self.assertEqual(code, 1)
                self.assertIn("is not the audited schema-reset base", text)
                compare.assert_not_called()

    def test_this_tree_failing_to_export_fails_the_step(self):
        for base_export in (0, 1):
            code, text, calls = self.run_report(head_export=1, base_export=base_export)
            self.assertEqual(code, 1)
            self.assertEqual(text, "The surface export of this tree failed: sunrise-stage surface: a.rs: parse error.\n")
            self.assertNotIn("surface-diff", calls)

    def test_the_diff_decides_and_the_gate_is_passed_on(self):
        code, text, calls = self.run_report()
        self.assertEqual(code, 0)
        self.assertIn("CI does not compare listed internals", text)
        self.assertIn("--with-internals", text)
        self.assertIn("Surface: report\n", text)
        self.assertNotIn("--gate", calls)
        self.assertNotIn("--with-internals", calls)
        exports = [call.split() for call in calls.splitlines() if call.startswith("surface ")]
        self.assertEqual(len(exports), 2)
        for call in exports:
            self.assertIn("--with-loops", call)
        self.assertIn("--notes ", calls)
        code, text, calls = self.run_report(diff_exit=1, gate_on=True)
        self.assertEqual(code, 1)
        self.assertIn("fn:a", text)
        self.assertIn("--gate", calls)
        # An unreadable note (exit 2) fails too.
        code, _, _ = self.run_report(diff_exit=2)
        self.assertEqual(code, 1)

    def test_the_audit_exports_internals_on_both_sides(self):
        code, text, calls = self.run_report(with_internals=True)
        self.assertEqual((code, text), (0, "Surface: report\n"))
        exports = [call for call in calls.splitlines() if call.startswith("surface ")]
        self.assertEqual(len(exports), 2)
        for call in exports:
            self.assertIn("--with-internals", call.split())


@unittest.skipUnless(os.environ.get("SUNRISE_STAGE_TOOL"), "set SUNRISE_STAGE_TOOL to test the real gate")
class EngineCatalogGateTests(unittest.TestCase):
    ITEMS = [
        {"id": "loop:entities.tick", "kind": "loop", "hash": "loop-1", "entry": "ServerGame::tick_entities",
         "generic": "tick_entities_with", "slot": "TickSlot", "side": "server"},
        {"id": "internal:fn:sunrise_gameplay::game::ServerGame::tick [feature = \"fixture\"]",
         "kind": "internal", "hash": "internal-1", "signature": "fn tick(&self)",
         "signature_hash": "old-internal"},
    ]

    def compare(self, old, new, *, notes=()):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for name, records in (("base", old), ("head", new)):
                (work / f"{name}.json").write_text(json.dumps({"schema": 3, "hash": name, "engine_items": records}))
            if notes:
                changes = work / "head" / surface_report.NOTES
                changes.mkdir(parents=True)
                (changes / "engine-change.toml").write_text(f'ids = {json.dumps(notes)}\nwhy = "Use the new sites."\n')
            return surface_report.compare([os.environ["SUNRISE_STAGE_TOOL"]], work / "base", work / "head", work, True,
                                          with_internals=True)

    def test_undeclared_loop_removal_fails_and_declared_removal_passes(self):
        loop = self.ITEMS[0]
        record = loop["id"]
        code, text = self.compare([loop], [])
        self.assertEqual(code, 1, text)
        self.assertIn(f"- {record} (removed)", text)
        self.assertIn(f"breaking change without a change note: {record}", text)
        code, text = self.compare([loop], [], notes=[record])
        self.assertEqual(code, 0, text)
        self.assertNotIn("breaking change without a change note:", text)

    def test_undeclared_internal_removal_is_advisory_and_listed(self):
        internal = self.ITEMS[1]
        code, text = self.compare([internal], [])
        self.assertEqual(code, 0, text)
        self.assertIn("Surface: advisory (0 breaking, 0 conditional, 0 additive, 1 advisory;", text)
        self.assertIn("Listed internals (advisory): 0 added, 1 removed, 0 changed", text)
        self.assertIn(f"- {internal['id']} (removed)", text)
        self.assertNotIn("breaking change without a change note:", text)

    def test_additions_pass_and_each_kind_has_a_count(self):
        code, text = self.compare([], self.ITEMS)
        self.assertEqual(code, 0, text)
        self.assertIn("0 breaking, 0 conditional, 1 additive, 1 advisory", text)
        for kind in ("Published loops", "Listed internals (advisory)"):
            self.assertIn(f"{kind}: 1 added, 0 removed, 0 changed", text)

    def test_changed_contracts_keep_their_ids(self):
        loop, internal = self.ITEMS
        code, text = self.compare(self.ITEMS, [
            dict(loop, hash="loop-2", slot="NewTickSlot"),
            dict(internal, hash="internal-2", signature="fn tick(&mut self)", signature_hash="new-internal"),
        ])
        self.assertEqual(code, 1, text)
        self.assertIn("1 breaking, 0 conditional, 0 additive, 1 advisory", text)
        for item, detail in zip(self.ITEMS, ("`TickSlot` → `NewTickSlot`", "`fn tick(&self)` → `fn tick(&mut self)`")):
            self.assertIn(f"- {item['id']} (changed)", text)
            self.assertIn(detail, text)
        self.assertNotIn("(removed)", text)
        self.assertNotIn("(added)", text)


class SummaryTests(unittest.TestCase):
    def size(self, out: str) -> int:
        return len(out.encode())

    def test_a_long_report_is_cut_at_a_line_end(self):
        text = "".join(f"- fn:item{i} (added)\n" for i in range(20_000))
        out = surface_report.summary(text)
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertRegex(out, r"\(added\)\n… cut here; \d+ more lines in the step log\n```\n$")
        self.assertEqual(surface_report.summary("short\n"), "### Mod API surface\n\n```text\nshort\n```\n")

    def test_a_cut_inside_a_multi_byte_character_keeps_whole_characters(self):
        # "é" is two bytes, so the byte budget is half the characters and lands inside one.
        for pad in ("", "x"):
            out = surface_report.summary(pad + "é" * 200_000 + "\nnext\n")
            self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
            self.assertGreater(self.size(out), surface_report.SUMMARY_LIMIT - 200)
            self.assertNotIn("\ufffd", out)
            self.assertIn("é\n… cut here; 2 more lines in the step log\n", out)

    def test_one_line_longer_than_the_cap_is_cut(self):
        out = surface_report.summary("a" * 300_000 + "\n")
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertGreater(out.count("a"), 199_000)
        self.assertTrue(out.endswith("a\n… cut here; 1 more line in the step log\n```\n"))


def artifacts(normal=(), harness=()):
    """The compiler's JSON lines: one sunrise-stage artifact per normal path and per test-harness path, then the
    end of the build."""
    def artifact(path, test):
        return {"reason": "compiler-artifact", "target": {"name": "sunrise-stage", "kind": ["bin"]},
                "profile": {"test": test}, "executable": str(path)}
    lines = [artifact(path, False) for path in normal] + [artifact(path, True) for path in harness]
    return "".join(json.dumps(line) + "\n" for line in [*lines, {"reason": "build-finished", "success": True}])


class ScopedOs:
    """`os` for the module under test, with its own environment."""

    def __init__(self, environ: dict[str, str]):
        self.environ = environ

    def __getattr__(self, name):
        return getattr(os, name)


class MainTests(unittest.TestCase):
    """main(): the base it fetches, what it prints, what it writes to the job summary."""

    def run_main(self, event="pull_request", pr="", argv=(), tool="/no/tool", env=None, **patches):
        """main() with its own environment; `tool` None leaves SUNRISE_STAGE_TOOL unset, `env` adds variables."""
        with tempfile.TemporaryDirectory() as directory:
            summary_file = Path(directory) / "summary.md"
            own = {"EVENT": event, "PR": pr, "GITHUB_STEP_SUMMARY": str(summary_file), "RUNNER_TEMP": directory,
                   **(env or {})}
            environ = {key: value for key, value in os.environ.items() if key not in ("SUNRISE_STAGE_TOOL", "JOBS")}
            if tool is not None:
                own["SUNRISE_STAGE_TOOL"] = tool
            # The module reads its own environment: the fixture never writes the process's.
            with mock.patch.object(surface_report, "os", ScopedOs({**environ, **own})), \
                    contextlib.redirect_stdout(io.StringIO()) as out, contextlib.ExitStack() as stack:
                for name, value in patches.items():
                    stack.enter_context(mock.patch.object(surface_report, name, value))
                code = surface_report.main(argv)
            return code, out.getvalue(), summary_file.read_text() if summary_file.exists() else ""

    def test_an_unfetchable_base_reports_and_passes(self):
        def git(*args):
            raise subprocess.CalledProcessError(128, ["git", *args])

        code, out, summary = self.run_main(git=git)
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("No surface report: the base could not be fetched: "), out)
        self.assertIn("No surface report", summary)

    def test_a_dispatch_whose_checkout_is_not_a_merge_reports_and_passes(self):
        unpacked = []
        code, _, _ = self.run_main("workflow_dispatch", "7", git=lambda *a: SINGLE,
                                   unpack=lambda sha, into: unpacked.append(sha))
        # No base is selected, so nothing is archived, and the report passes.
        self.assertEqual((code, unpacked), (0, []))

    def test_a_report_is_printed_written_and_its_code_returned(self):
        fetched = []
        code, out, summary = self.run_main(
            base_commit=lambda *a: ("abcd", ""),
            unpack=lambda sha, into: fetched.append(sha),
            report=lambda *a, **kw: (1, "Surface: breaking\n"),
            # The checkout's own change notes may carry `landed`; LandedTests covers that pass.
            landed_notes=lambda head, base: {})
        self.assertEqual((code, fetched), (1, ["abcd"]))
        self.assertRegex(out, r"^Surface: breaking\nBase abcd; fetch \d+\.\ds, total \d+\.\ds\n$")
        self.assertTrue(summary.startswith("### Mod API surface\n\n```text\nSurface: breaking\nBase abcd"))

    def test_local_audit_uses_the_supplied_trees_without_fetching(self):
        with mock.patch.object(surface_report, "report", return_value=(0, "Surface: advisory\n")) as report, \
             mock.patch.object(surface_report, "base_commit") as find_base:
            code, out, _ = self.run_main(argv=["--with-internals", "--trees", "before", "after"])
        self.assertEqual((code, out), (0, "Surface: advisory\n"))
        self.assertEqual(report.call_args.args[1:3], (Path("before"), Path("after")))
        self.assertTrue(report.call_args.kwargs["with_internals"])
        find_base.assert_not_called()

    def test_a_given_base_is_resolved_to_its_commit_and_replaces_the_event_based_one(self):
        fetched, built = [], []
        sha = "cafe" * 10

        def git(*args):
            # Only the given ref is resolved; the event's merge commit is not read.
            self.assertEqual(args, ("rev-parse", "--verify", "origin/main"))
            return sha + "\n"

        def report(tool, base, head, work, gate_on, *, own_base_tool, **_):
            own_base_tool()
            return 0, "Surface: identical\n"

        code, out, _ = self.run_main(
            "push", argv=["--base", "origin/main"], git=git, report=report,
            unpack=lambda commit, into: fetched.append(commit),
            base_tool=lambda commit, work: built.append(commit) or "base-stage",
            landed_notes=lambda head, base: {})
        # The base's tool tree is named by the commit, so a ref's `/` never reaches a path.
        self.assertEqual((code, fetched, built), (0, [sha], [sha]))
        self.assertRegex(out, rf"^Surface: identical\nBase {sha}; fetch \d+\.\ds, total \d+\.\ds\n$")

    def test_a_given_base_that_cannot_be_fetched_fails(self):
        def unpack(sha, into):
            raise subprocess.CalledProcessError(128, ["git", "archive", sha])

        code, out, summary = self.run_main("push", argv=["--base", "cafe"], git=lambda *a: "cafe" * 10,
                                           unpack=unpack)
        self.assertEqual(code, 1)
        self.assertTrue(out.startswith("No surface report: the base could not be fetched"), out)
        self.assertIn("No surface report: the base could not be fetched", summary)

    def test_a_given_base_and_trees_are_refused(self):
        with self.assertRaises(SystemExit) as refused, contextlib.redirect_stderr(io.StringIO()) as error:
            self.run_main(argv=["--base", "cafe", "--trees", "before", "after"])
        self.assertEqual(refused.exception.code, 2)
        self.assertIn("--base and --trees", error.getvalue())

    def test_the_report_builds_the_tool_only_when_none_is_given(self):
        built = mock.Mock(return_value="/built/sunrise-stage")
        tools = []

        def report(tool, *rest, **_):
            tools.append(tool)
            return 0, "Surface: identical\n"

        shared = dict(base_commit=lambda *a: ("abcd", ""), unpack=lambda sha, into: None, report=report,
                      landed_notes=lambda head, base: {}, build_tool=built)
        self.run_main(tool="/given/stage", **shared)
        built.assert_not_called()
        self.run_main(tool=None, env={"JOBS": "-j3"}, **shared)
        built.assert_called_once_with(["-j3"])
        self.assertEqual(tools, [["/given/stage"], ["/built/sunrise-stage"]])

    def test_build_tool_mode_appends_the_tool_to_the_github_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            normal = Path(directory, "sunrise-stage")
            normal.touch()
            environment = Path(directory, "github-env")
            environment.write_text("JOBS=-j3\n")
            built = subprocess.CompletedProcess([], 0, artifacts([normal]))
            with mock.patch.object(surface_report.subprocess, "run", return_value=built) as cargo:
                code, out, summary = self.run_main(argv=["--build-tool"], tool=None,
                                                   env={"GITHUB_ENV": str(environment), "JOBS": "-j3"})
                self.assertEqual(code, 0)
                self.assertEqual(environment.read_text(), f"JOBS=-j3\nSUNRISE_STAGE_TOOL={normal.resolve()}\n")
                self.assertRegex(out, rf"^Stage generator build: \d+s\n{re.escape(str(normal.resolve()))}\n$")
                self.assertEqual(summary, "")
                self.assertEqual(cargo.call_args.args[0][:3], ["cargo", "build", "-j3"])
                # Without $GITHUB_ENV the path is only printed, and an empty $JOBS adds no argument.
                code, out, _ = self.run_main(argv=["--build-tool"], tool=None, env={"JOBS": ""})
                self.assertEqual(out.splitlines()[-1], str(normal.resolve()))
                self.assertEqual(cargo.call_args.args[0][:3], ["cargo", "build", "--locked"])

    def test_build_tool_mode_goes_with_neither_a_base_nor_trees(self):
        for argv in (["--build-tool", "--base", "cafe"], ["--build-tool", "--trees", "before", "after"]):
            with self.subTest(argv=argv), self.assertRaises(SystemExit) as refused, \
                    contextlib.redirect_stderr(io.StringIO()) as error:
                self.run_main(argv=argv, tool=None)
            self.assertEqual(refused.exception.code, 2)
            self.assertIn("--build-tool", error.getvalue())

    def local_audit(self, *flags, readable=True):
        compared = []

        def stage(command, **_):
            tool, action, first, second, *_ = command
            if action == "surface":
                if not readable and (tool, first) == ("head-stage", "before"):
                    return subprocess.CompletedProcess(command, 1, "", "unsupported older source")
                # Both tools can read the same source but generate different records for it.
                Path(second).write_text(json.dumps({"tree": first, "wire": tool == "base-stage"}))
            elif action == "surface-diff":
                compared.extend(json.loads(Path(path).read_text()) for path in (first, second))
            return subprocess.CompletedProcess(command, 0, "", "")

        tree_tool = mock.Mock(return_value=["base-stage"])
        layout = surface_report.source_layout(surface_report.ROOT)
        with mock.patch.object(surface_report.subprocess, "run", side_effect=stage), \
             mock.patch.object(surface_report, "source_layout", return_value=layout):
            code, out, _ = self.run_main(argv=["--trees", "before", "after", *flags], tool="head-stage",
                                         tree_tool=tree_tool)
        return code, out, tree_tool, compared

    def test_local_audit_does_not_build_without_opt_in(self):
        code, out, tree_tool, compared = self.local_audit()
        self.assertEqual(code, 0, out)
        tree_tool.assert_not_called()
        self.assertEqual(compared, [{"tree": "before", "wire": False}, {"tree": "after", "wire": False}])
        code, out, tree_tool, compared = self.local_audit(readable=False)
        self.assertEqual(code, 1, out)
        self.assertIn("unsupported older source", out)
        tree_tool.assert_not_called()
        self.assertEqual(compared, [])

    def test_requested_base_tool_preserves_the_record_even_if_the_head_can_export_the_base(self):
        code, out, tree_tool, compared = self.local_audit("--build-base-tool")
        self.assertEqual(code, 0, out)
        self.assertEqual(compared, [{"tree": "before", "wire": True}, {"tree": "after", "wire": False}])
        tree_tool.assert_called_once()
        self.assertEqual(tree_tool.call_args.args[0], Path("before"))

    def test_a_commit_both_comparisons_need_is_built_once(self):
        """A landed note may name the CI base itself: the base comparison and the landed one share its tool."""
        sha = "6" * 40
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            # The head's tool succeeds on the base, but produces the checkout's record instead of the base's.
            head_tool = python_stub(folder / "head_stage.py", r"""
import sys
from pathlib import Path

args = sys.argv[1:]
if args[0] == "surface":
    Path(args[2]).write_text('{"by": "checkout"}\n')
elif args[0] != "surface-catalogs":
    with open(Path(FOLDER, "compared"), "a") as compared:
        compared.write(Path(args[1]).read_text() + Path(args[2]).read_text())
""", FOLDER=str(folder))
            own_tool = python_stub(folder / "own_stage.py", r"""
import sys
from pathlib import Path

marker = Path(sys.argv[2], "marker").read_text()
Path(sys.argv[3]).write_text('{"schema": 3, "by": "' + marker + '"}\n')
""")
            built = []

            def base_tool(commit, work):
                built.append(commit)
                return own_tool

            def unpack(commit, into, paths=()):
                (into / "marker").write_text(commit)

            def git(*args):
                return MERGE if args[0] == "cat-file" else ""

            # main wraps the environment's one path as a command; this stand-in is a command of two words.
            report, landed_check = surface_report.report, surface_report.landed_check
            code, out, _ = self.run_main(
                base_commit=lambda *a: (sha, ""), unpack=unpack, git=git, base_tool=base_tool,
                landed_notes=lambda head, base: {sha: ["n.toml"]},
                report=lambda _tool, *a, **kw: report(head_tool, *a, **kw),
                landed_check=lambda _tool, *a, **kw: landed_check(head_tool, *a, **kw))
            self.assertEqual(code, 0, out)
            self.assertEqual(built, [sha, "aaaa"])
            exports = [json.loads(line)["by"] for line in (folder / "compared").read_text().splitlines()]
            # The base against this checkout, then the landed merge against its first parent.
            self.assertEqual(exports, [sha, "checkout", "aaaa", sha])

    def test_the_base_tool_is_copied_with_the_platforms_executable_suffix(self):
        for platform, name in (("win32", "sunrise-stage.exe"), ("linux", "sunrise-stage")):
            with self.subTest(platform), tempfile.TemporaryDirectory() as directory:
                folder = Path(directory)
                work = folder / "work"
                work.mkdir()

                def cargo(command, **_):
                    target = Path(command[command.index("--target-dir") + 1])
                    built = target / "debug" / name
                    built.parent.mkdir(parents=True, exist_ok=True)
                    built.write_text("stage")
                    return subprocess.CompletedProcess(command, 0, "", "")

                with mock.patch.object(surface_report.sys, "platform", platform), \
                     mock.patch.object(surface_report.subprocess, "run", cargo):
                    tool = surface_report.tree_tool(Path("one"), work, work / "target")
                self.assertEqual(Path(tool[0]).name, "base-stage-0" + name[len("sunrise-stage"):])
                self.assertEqual(Path(tool[0]).read_text(), "stage")


@unittest.skipUnless(shutil.which("cargo"), "needs Cargo for the exporter fixture")
class ExporterTargetTests(unittest.TestCase):
    SOURCES = {
        "Cargo.toml": '[package]\nname = "sunrise-stage"\nversion = "0.0.0"\nedition = "2024"\n'
                      '[build-dependencies]\ngenerator = { path = "generator" }\n'
                      '[workspace]\nmembers = ["generator"]\n',
        "Cargo.lock": 'version = 4\n[[package]]\nname = "generator"\nversion = "0.0.0"\n'
                      '[[package]]\nname = "sunrise-stage"\nversion = "0.0.0"\ndependencies = ["generator"]\n',
        "build.rs": 'fn main() {\n'
                    '    println!("cargo:rerun-if-changed=build.rs");\n'
                    '    println!("cargo:rustc-env=VALUE={}", generator::VALUE);\n}\n',
        "generator/Cargo.toml": '[package]\nname = "generator"\nversion = "0.0.0"\nedition = "2024"\n',
        "generator/src/lib.rs": 'pub const VALUE: &str = "A";\n',
        "src/main.rs": 'fn main() {\n'
                       '    let output = std::env::args_os().nth(3).unwrap();\n'
                       '    std::fs::write(output, format!(r#"{{"schema":3,"value":"{}"}}"#, env!("VALUE"))).unwrap();\n'
                       '}\n',
    }
    INPUT = "generator/src/lib.rs"

    def setUp(self):
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(prefix="surface-report-", dir=target)
        self.addCleanup(directory.cleanup)
        self.work = Path(directory.name)
        self.repo = self.work / "repo"
        for name, source in self.SOURCES.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source)
        self.env = {**os.environ, "CARGO_TARGET_DIR": str(self.work / "foreign-target"),
                    "CARGO_BUILD_BUILD_DIR": str(self.work / "foreign-build"), "CARGO_NET_OFFLINE": "true",
                    "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": ""}

    def backdate(self, tree):
        for path in self.SOURCES:
            os.utime(tree / path, ns=(1_000_000_000, 1_000_000_000))

    def test_backdated_local_audits_export_each_tree_without_modifying_it(self):
        a = self.repo
        b = self.work / "dirty"
        shutil.copytree(a, b)
        with mock.patch.object(surface_report, "ROOT", b):
            surface_report.git("init", "-q")
            self.commit()
        (b / self.INPUT).write_text('pub const VALUE: &str = "B";\n')
        for tree in (a, b):
            self.backdate(tree)
        head_tool = python_stub(self.work / "head_stage.py", r"""
import sys
from pathlib import Path

args = sys.argv[1:]
if args[0] == "surface":
    Path(args[2]).write_text('{}')
elif args[0] == "surface-diff":
    print(Path(args[1]).read_text())
""")
        driver = self.work / "driver"
        (driver / "tools/ci").mkdir(parents=True)
        shutil.copy2(ROOT / "tools/ci/surface.toml", driver / "tools/ci/surface.toml")
        report = surface_report.report
        env = {**self.env, "SUNRISE_STAGE_TOOL": head_tool[1], "RUNNER_TEMP": str(self.work),
               "GITHUB_STEP_SUMMARY": ""}
        with (mock.patch.object(surface_report, "ROOT", driver),
              mock.patch.object(surface_report, "os", ScopedOs(env)),
              mock.patch.object(surface_report, "report", side_effect=lambda _tool, *a, **kw: report(head_tool, *a, **kw))):
            for tree, value in ((a, "A"), (b, "B")):
                input_mtime = (tree / self.INPUT).stat().st_mtime_ns
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    code = surface_report.main(["--trees", str(tree), str(a), "--build-base-tool"])
                self.assertEqual(code, 0, output.getvalue())
                self.assertIn(f'{{"schema":3,"value":"{value}"}}', output.getvalue())
                self.assertEqual((tree / self.INPUT).stat().st_mtime_ns, input_mtime)
                self.assertFalse((tree / "target").exists())
        self.assertFalse((driver / "target").exists())
        self.assertFalse((self.work / "foreign-target").exists())
        self.assertFalse((self.work / "foreign-build").exists())

    def commit(self):
        surface_report.git("add", ".")
        surface_report.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                           "-c", "commit.gpgsign=false", "commit", "-qm", "exporter fixture")
        return surface_report.git("rev-parse", "HEAD").strip()

    @unittest.skipUnless(shutil.which("tar"), "needs tar to unpack the exporter fixture")
    def test_archived_builds_reconcile_a_b_a_and_keep_unchanged_units_fresh(self):
        run = subprocess.run
        unpack = surface_report.unpack
        artifacts_by_build = []

        def record_build(command, **kwargs):
            if command[0] != "cargo":
                return run(command, **kwargs)
            done = run([*command, "--message-format=json"], **kwargs)
            artifacts_by_build.append([item for line in done.stdout.splitlines()
                                       if (item := json.loads(line))["reason"] == "compiler-artifact"])
            return done

        def backdated_archive(sha, into, paths):
            identity = unpack(sha, into, paths)
            self.backdate(into)
            return identity

        with (mock.patch.object(surface_report, "ROOT", self.repo),
              mock.patch.object(surface_report, "os", ScopedOs(self.env)),
              mock.patch.object(surface_report.subprocess, "run", side_effect=record_build),
              mock.patch.object(surface_report, "unpack", side_effect=backdated_archive)):
            surface_report.git("init", "-q")
            a = self.commit()
            (self.repo / self.INPUT).write_text('pub const VALUE: &str = "B";\n')
            b = self.commit()
            # Reconcile A/B/A with fresh archives, then let an older writer build B before the warm A audit.
            builds = []
            for index, (sha, legacy) in enumerate(((a, False), (b, False), (a, False), (b, True), (a, False))):
                work = self.work / f"audit-{index}"
                work.mkdir()
                if legacy:
                    tree = work / "archive"
                    tree.mkdir()
                    unpack(sha, tree, ())
                    # The old tar -m kept extraction time; the only source difference needs a new mtime.
                    os.utime(tree / self.INPUT, None)
                    target = self.repo / "target" / "surface-base"
                    target.mkdir(parents=True, exist_ok=True)
                    with (target / "surface-report.lock").open("a") as lock:
                        surface_report.file_lock.lock(lock)
                        tool = surface_report.tree_tool(tree, work, target)
                else:
                    tool = surface_report.base_tool(sha, work)
                builds.append((tool, work / "surface.json"))
            for build in artifacts_by_build[:-1]:
                generator = next(item for item in build if item["target"]["name"] == "generator")
                self.assertFalse(generator["fresh"])
            # Each copied tool keeps its result even after later builds replace the cached binary.
            for (tool, output), value in zip(builds, ("A", "B", "A", "B", "A"), strict=True):
                done = surface_report.export_surface(tool, self.repo, output)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertEqual(json.loads(output.read_text())["value"], value)
        self.assertTrue(artifacts_by_build[-1])
        self.assertTrue(all(item["fresh"] for item in artifacts_by_build[-1]), artifacts_by_build[-1])
        self.assertFalse((self.work / "foreign-target").exists())
        self.assertFalse((self.work / "foreign-build").exists())


class BuildToolTests(unittest.TestCase):
    """build_tool(): the one place that picks the head's sunrise-stage among the compiler's artifacts."""

    def pick(self, normal=("sunrise-stage",), harness=("sunrise-stage-harness",), returncode=0, jobs=()):
        with tempfile.TemporaryDirectory() as directory:
            paths = {name: Path(directory, name) for name in (*normal, *harness)}
            for path in paths.values():
                path.touch()
            output = artifacts([paths[name] for name in normal], [paths[name] for name in harness])
            built = subprocess.CompletedProcess([], returncode, output)
            with mock.patch.object(surface_report.subprocess, "run", return_value=built) as cargo:
                tool = surface_report.build_tool(list(jobs))
            return tool, paths, cargo

    def test_the_normal_executable_is_picked_over_the_test_harness(self):
        tool, paths, cargo = self.pick(jobs=["-j3"])
        self.assertEqual(tool, str(paths["sunrise-stage"].resolve()))
        self.assertEqual(cargo.call_args.args[0], [
            "cargo", "build", "-j3", "--locked", "--profile", "test", "-p", "sunrise-stage", "--bin", "sunrise-stage",
            "--message-format=json-render-diagnostics"])
        self.assertEqual(cargo.call_args.kwargs["cwd"], surface_report.ROOT)

    def test_not_exactly_one_normal_executable_is_refused(self):
        for normal in (("first", "second"), ()):
            with self.subTest(normal=normal), self.assertRaises(SystemExit) as refused:
                self.pick(normal=normal)
            self.assertIn("expected one normal sunrise-stage executable", str(refused.exception))

    def test_a_failed_build_is_refused(self):
        with self.assertRaises(SystemExit) as refused:
            self.pick(returncode=101)
        self.assertEqual(str(refused.exception), "building the head's sunrise-stage failed")


@unittest.skipUnless(shutil.which("bash"), "needs bash for the stub tool")
class LandedTests(unittest.TestCase):
    SHA = "6" * 40

    def check(self, diff_exit, git_text=None, without_own_tool=False, own_tool=None, parent_schema=3):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for side in ("head", "base"):
                notes = work / side / surface_report.NOTES
                notes.mkdir(parents=True)
            (work / "head" / surface_report.NOTES / "r61.toml").write_text(
                f'ids = ["loop:a"]\nwhy = "w"\nlanded = "{self.SHA}"\n')
            (work / "head" / surface_report.NOTES / "own.toml").write_text('ids = ["hook:b"]\nwhy = "w"\n')
            tool = python_stub(work / "stage.py", LANDED_STUB, WORK=str(work), DIFF_EXIT=diff_exit)
            shas = surface_report.landed_notes(work / "head", work / "base")
            fetched = []

            def fetch(sha, into):
                # Each fetched tree names its commit, for the exporters to read; the merge brings a note of its own.
                fetched.append(sha)
                (into / "marker").write_text(sha)
                if sha == self.SHA:
                    (into / surface_report.NOTES).mkdir(parents=True)
                    (into / surface_report.NOTES / "then.toml").write_text('ids = ["loop:b"]\nwhy = "w"\n')

            git_text = git_text or (lambda *a: MERGE if a[0] == "cat-file" else "")
            if own_tool is None and not without_own_tool:
                exporter = python_stub(work / "historical_stage.py", r"""
import sys
from pathlib import Path

schema = PARENT_SCHEMA if Path(sys.argv[2], "marker").read_text() == "aaaa" else 3
Path(sys.argv[3]).write_text('{"schema": %s, "hash": "from-historical-stage"}\n' % schema)
""", PARENT_SCHEMA=parent_schema)
                own_tool = lambda _sha, _work: exporter
            code, text = surface_report.landed_check(
                tool, work / "head", work / "base", work, shas, git_text=git_text, fetch=fetch,
                own_tool=(lambda sha: own_tool(sha, work)) if own_tool else None)
            calls = (work / "calls").read_text() if (work / "calls").exists() else ""
            for given in ("notes", "base-notes", "covered-by", "covered-base"):
                if (work / f"given{given}").exists():
                    calls += f"{given}: " + (work / f"given{given}").read_text().replace("\n", " ") + "; "
            if (work / "compared").exists():
                calls += "compared " + (work / "compared").read_text().replace("\n", " ")
            return shas, code, text, calls, fetched

    def test_only_new_notes_that_carry_landed_are_found(self):
        shas, *_ = self.check(0)
        self.assertEqual(shas, {self.SHA: ["r61.toml"]})

    def test_a_note_whose_ids_changed_in_that_merge_passes(self):
        _, code, text, calls, fetched = self.check(0)
        self.assertEqual(code, 0, text)
        self.assertEqual(fetched, ["aaaa", self.SHA])
        self.assertIn(f"--landed {self.SHA}", calls)
        self.assertNotIn("--gate", calls)
        # The diff checks this pull request's notes (the late one for this merge among them), and reads the merge's
        # own notes against its first parent's apart from them, as coverage.
        self.assertIn("notes: own.toml r61.toml ;", calls)
        self.assertIn("covered-by: then.toml ;", calls)
        self.assertIn("covered-base: ;", calls)

    def test_a_note_whose_ids_did_not_change_there_fails_naming_it(self):
        _, code, text, _, _ = self.check(1)
        self.assertEqual(code, 1)
        self.assertIn(f"r61.toml (landed {self.SHA})", text)

    def test_a_landed_note_does_not_compare_a_schema_two_first_parent(self):
        _, code, text, calls, _ = self.check(0, parent_schema=2)
        self.assertEqual(code, 1)
        self.assertIn(f"r61.toml (landed {self.SHA})", text)
        self.assertIn("The first parent uses surface schema 2", text)
        self.assertNotIn("surface-catalogs", calls)
        self.assertNotIn("surface-diff", calls)

    def test_old_trees_are_exported_by_their_own_commits_tools(self):
        built = []

        def own_tool(sha, work):
            built.append(sha)
            # Says which commit's tool exported which tree.
            return python_stub(work / f"own-{sha}.py", r"""
import sys
from pathlib import Path

marker = Path(sys.argv[2], "marker").read_text()
Path(sys.argv[3]).write_text('{"schema": 3, "by": "' + SHA + ' tool, ' + marker + ' tree"}\n')
""", SHA=sha)

        _, code, text, calls, _ = self.check(0, own_tool=own_tool)
        self.assertEqual(code, 0, text)
        self.assertEqual(built, ["aaaa", self.SHA])
        # The diff compares the first parent, exported by its own tool, with the merge, exported by its own.
        self.assertIn('compared {"schema": 3, "by": "aaaa tool, aaaa tree"} '
                      '{"schema": 3, "by": "%s tool, %s tree"}' % (self.SHA, self.SHA), calls)
        self.assertIn(f"--landed {self.SHA}", calls)
        # Without the commit's exporter the check cannot verify the historical surface.
        _, code, text, calls, _ = self.check(0, without_own_tool=True)
        self.assertEqual(code, 1)
        self.assertIn("the landed base must be exported in its original schema", text)
        self.assertNotIn("surface-diff", calls)

    def test_a_landed_that_is_not_a_full_sha_is_not_returned(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for side in ("head", "base"):
                (work / side / surface_report.NOTES).mkdir(parents=True)
            for name, value in (("a", "--upload-pack=x"), ("b", "6597e08"), ("c", "G" * 40)):
                (work / "head" / surface_report.NOTES / f"{name}.toml").write_text(
                    f'ids = ["x"]\nwhy = "w"\nlanded = "{value}"\n')
            self.assertEqual(surface_report.landed_notes(work / "head", work / "base"), {})

    def test_a_commit_that_is_not_a_two_parent_merge_fails(self):
        _, code, text, calls, _ = self.check(
            0, git_text=lambda *a: SINGLE if a[0] == "cat-file" else "")
        self.assertEqual(code, 1)
        self.assertIn("not a merge commit with two parents", text)
        self.assertNotIn("surface-diff", calls)


if __name__ == "__main__":
    unittest.main()
