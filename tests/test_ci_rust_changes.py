import contextlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from tools.ci import rust_changes
from tools.ci.rust_changes import (
    UNKNOWN_CHANGE, changed_entries, changed_paths, needs_rust, needs_server_build, needs_tooling,
    selections,
)


@contextlib.contextmanager
def fixture_repository(files):
    """The selector reading a small committed repository of `files` instead of this checkout."""
    caches = [getattr(rust_changes, name) for name in ("rust_literals", "rust_includes", "gated_helpers",
                                                         "python_literals")
              if hasattr(rust_changes, name)]
    with tempfile.TemporaryDirectory() as directory:
        for path, text in files.items():
            Path(directory, path).parent.mkdir(parents=True, exist_ok=True)
            Path(directory, path).write_text(text)
        subprocess.check_call(["git", "-C", directory, "init", "--quiet"])
        subprocess.check_call(["git", "-C", directory, "add", "."])
        root = rust_changes.ROOT
        rust_changes.ROOT = Path(directory)
        for cache in caches:
            cache.cache_clear()
        try:
            yield
        finally:
            rust_changes.ROOT = root
            for cache in caches:
                cache.cache_clear()


class RustChangesTests(unittest.TestCase):
    def test_only_content_changes_skip_rust(self):
        self.assertFalse(needs_rust([]))
        self.assertFalse(needs_rust([
            "docs/setup.md", "docs/media/banner.webp", "README.md",
        ]))
        # The Python tools: verify's Python steps run their tests, and no Rust build or test reads them.
        self.assertFalse(needs_rust([
            "tools/parity.py", "tools/test_tick_parity.py", "tests/test_ci_runners.py", "docs/setup.md",
        ]))
        for path in [
            "crates/sunrise-gameplay-common/src/game.rs", "xtask/src/main.rs", "Cargo.lock",
            ".cargo/config.toml", ".github/workflows/ci.yml", "tools/ci/rust_changes.py",
            "tools/release_train.py", "tools/ci/sign_release.sh", "tools/native-hook-fragments/a.json",
            "tools/parity_hosts.json", "mods/sunrise/src/x.py",
            "research-lock.json", "crates/sunrise-theme/tokens.css", "unknown-file",
        ]:
            with self.subTest(path=path):
                self.assertTrue(needs_rust(["README.md", path]))

    def test_documentation_that_rust_embeds_or_tests_requires_rust(self):
        for path in [
            # Generated references that Rust tests compare with their generator's output.
            "docs/api/server-reference.md", "docs/sunrise-ctl-commands.md",
            # Embedded with include_str!.
            "docs/modpacks/atm10-mods.csv",
            # A page a Rust test requires to be absent: adding it fails that test.
            "docs/modding/src/reference/hook-notes.md",
            # A workspace crate's sources under docs/.
            "docs/modding/examples/src/greeter.rs",
        ]:
            with self.subTest(path=path):
                self.assertTrue(needs_rust(["docs/setup.md", path]))
        self.assertFalse(needs_rust(["docs/hosting.md", "docs/modding/src/introduction.md"]))

    def test_the_server_build_clippy_follows_the_client_crates_and_manifests(self):
        for path in ["crates/sunrise-world/src/lib.rs", "crates/sunrise-perf/src/lib.rs", "tools/parity.py",
                     "docs/setup.md"]:
            with self.subTest(path=path):
                self.assertFalse(needs_server_build([path]))
        for path in ["crates/sunrise-client/src/entry.rs", "crates/sunrise-client-render/src/hud.rs",
                     "crates/sunrise-gameplay-common/src/game.rs", "crates/sunrise-gameplay/src/game.rs",
                     "crates/sunrise-client-gameplay/src/game.rs", "crates/sunrise-game/src/main.rs",
                     "crates/sunrise-game-data/src/lang.rs", "crates/sunrise-world/Cargo.toml", "Cargo.lock",
                     "Cargo.toml", ".cargo/config.toml", ".github/workflows/ci.yml", "tools/ci/rust_changes.py"]:
            with self.subTest(path=path):
                self.assertTrue(needs_server_build(["docs/setup.md", path]))

    def test_moving_source_into_docs_still_requires_rust(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args])

            git("init", "--quiet")
            source = Path(directory, "source.rs")
            source.write_text("fn main() {}\n")
            git("add", ".")
            git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "base")
            base = git("rev-parse", "HEAD").decode().strip()
            Path(directory, "docs").mkdir()
            source.rename(Path(directory, "docs/example.rs"))
            git("add", "-A")
            git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "move")
            # Exercise the same diff command as CI in the temporary repository.
            previous = Path.cwd()
            try:
                os.chdir(directory)
                paths = changed_paths(base, "HEAD")
            finally:
                os.chdir(previous)
            self.assertIn("source.rs", paths)
            self.assertTrue(needs_rust(paths))

    def test_tooling_keeps_watched_markdown_and_non_documentation(self):
        for path in [
            "tools/README.md", "tests/fixtures/example.md", ".github/pull_request_template.md",
            "docs/modding/src/introduction.md", "roadmap/journal/example.md",
            "README.md", "ROADMAP.md", "docs/README.md",
            "crates/sunrise-gameplay-common/src/game.rs", "packaging/server/start.sh", "research-lock.json",
            "unknown-file",
        ]:
            with self.subTest(path=path):
                self.assertTrue(needs_tooling([("M", path)]))
                self.assertTrue(needs_tooling([("M", "docs/setup.md"), ("M", path)]))
        self.assertFalse(needs_tooling([("M", "docs/setup.md")]))
        self.assertFalse(needs_tooling([]))

    def test_documentation_the_python_suite_reads_keeps_it(self):
        # The status page's gap count and the book's build entry are read by the Python tests.
        for path in ["docs/status.md", "docs/modding-book.md"]:
            with self.subTest(path=path):
                self.assertTrue(selections([("M", path)])["tooling_required"])
        self.assertFalse(selections([("M", "docs/setup.md")])["tooling_required"])

    def test_python_reads_follow_joins_folders_and_globs(self):
        with fixture_repository({
            "tests/test_pages.py": (
                'PAGE = ROOT / "docs" / "status.md"\n'
                'IMAGES = "docs/media/"\n'
                'BOOK = sorted(ROOT.glob("docs/book/*.md"))\n'
                'NAME = "notes.md"\n'
            ),
            "docs/status.md": "x\n", "docs/media/a.webp": "x\n", "docs/book/a.md": "x\n",
            "docs/book/deep/b.md": "x\n", "docs/notes.md": "x\n", "docs/other.md": "x\n",
        }):
            # A glob's `*` stays within its folder; a bare name is the source folder's own file.
            self.assertEqual(rust_changes.python_reads([
                "docs/status.md", "docs/media/a.webp", "docs/book/a.md", "docs/book/deep/b.md", "docs/notes.md",
                "docs/other.md"]), {"docs/status.md", "docs/media/a.webp", "docs/book/a.md"})

    def test_documentation_membership_changes_keep_link_checks(self):
        for status in ["A", "D", "T", "R100", "?"]:
            with self.subTest(status=status):
                self.assertTrue(needs_tooling([(status, "docs/setup.md")]))
        self.assertTrue(needs_tooling([("D", "docs/old.md"), ("A", "docs/new.md")]))

    def test_boundary_uses_fail_closed_rust_selection(self):
        # The lint boundary step consumes this same central decision.
        for path in ["Cargo.toml", "crates/example/Cargo.toml",
                     "crates/example/src/lib.rs", UNKNOWN_CHANGE]:
            with self.subTest(path=path):
                self.assertTrue(selections([("M", path)])["rust_required"])
        self.assertFalse(selections([("M", "docs/setup.md")])["rust_required"])

    def test_shared_selections(self):
        self.assertEqual(selections([("M", "docs/setup.md")]), {
            "rust_required": False, "tooling_required": False,
        })
        for path in ["Cargo.lock", "tools/ci/rust_changes.py", ".github/workflows/dyna-ci.yml"]:
            selected = selections([("M", path)])
            self.assertTrue(selected["rust_required"])
            self.assertTrue(selected["tooling_required"])
        # The machine sample is a daily leg: a mod API change selects nothing extra in dyna-checks.
        self.assertFalse(selections([("M", "crates/sunrise-mod-api/README.md")])["rust_required"])

    def test_git_status_and_cli_selections_for_actual_documentation_changes(self):
        selector = Path(__file__).resolve().parents[1] / "tools/ci/rust_changes.py"
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args])

            git("init", "--quiet")
            docs = Path(directory, "docs")
            docs.mkdir()
            for name in ["modified.md", "deleted.md", "old.md", "type.md"]:
                (docs / name).write_text("old\n")
            git("add", ".")
            commit = ["-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                      "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture"]
            git(*commit)
            base = git("rev-parse", "HEAD").decode().strip()
            (docs / "modified.md").write_text("new\n")
            (docs / "deleted.md").unlink()
            (docs / "old.md").rename(docs / "renamed.md")
            (docs / "added.md").write_text("added\n")
            (docs / "type.md").unlink()
            git("add", "-A")
            # type.md becomes the entry git stores for a symbolic link (mode 120000, its target as content), made
            # without one on disk.
            link = subprocess.run(["git", "-C", directory, "hash-object", "-w", "--stdin"], input=b"modified.md",
                                  capture_output=True, check=True).stdout.decode().strip()
            git("update-index", "--add", "--cacheinfo", f"120000,{link},docs/type.md")
            git(*commit)
            previous = Path.cwd()
            try:
                os.chdir(directory)
                entries = changed_entries(base, "HEAD")
            finally:
                os.chdir(previous)
            self.assertEqual(set(entries), {
                ("M", "docs/modified.md"), ("D", "docs/deleted.md"), ("D", "docs/old.md"),
                ("A", "docs/renamed.md"), ("A", "docs/added.md"), ("T", "docs/type.md"),
            })
            import sys
            head = subprocess.check_output(["git", "-C", directory, "rev-parse", "HEAD"], text=True).strip()
            command = [sys.executable, str(selector), "--all"]
            output = subprocess.check_output([*command, base, head], cwd=directory, text=True)
            self.assertEqual(dict(line.split("=") for line in output.splitlines()), {
                "rust_required": "false", "tooling_required": "true",
            })

    def test_a_deleted_or_moved_cargo_input_requires_rust(self):
        # The checkout no longer has the package's manifest, but the workspace still lists the package: Cargo
        # can no longer load it.
        with fixture_repository({"Cargo.toml": "[workspace]\nmembers = [\"docs/examples\"]\n",
                                 "docs/examples/src/lib.rs": "pub fn f() {}\n", "docs/setup.md": "x\n"}):
            for status in ["D", "M", "A"]:
                with self.subTest(status=status):
                    self.assertTrue(selections([(status, "docs/examples/Cargo.toml")])["rust_required"])
            for path in ["docs/examples/build.rs", "docs/examples/src/old.rs", "docs/examples/Cargo.lock",
                         "docs/examples/rust-toolchain.toml"]:
                with self.subTest(path=path):
                    self.assertTrue(needs_rust(["docs/setup.md", path]))

    def test_includes_the_literal_scan_cannot_read_still_select_rust(self):
        with fixture_repository({
            "crates/demo/Cargo.toml": "[package]\nname = \"demo\"\n",
            "crates/demo/src/lib.rs": (
                '#![doc = include_str!("../guide/my page.md")]\n'
                'pub const PARTS: &str =\n'
                '    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/notes/", "pa", "rts.md"));\n'
                'macro_rules! icon { ($name:literal) => { include_bytes!(concat!("../icons/", $name, ".svg")) }; }\n'
                '// include_str!("../guide/commented.md")\n'
                'pub const TEXT: &str = "include_str!(\\"../guide/quoted.md\\")";\n'
            ),
            "crates/demo/guide/my page.md": "x\n", "crates/demo/notes/parts.md": "x\n",
            "crates/demo/icons/a.svg": "x\n", "crates/demo/guide/commented.md": "x\n",
            "crates/demo/guide/quoted.md": "x\n", "crates/demo/README.md": "x\n",
        }):
            for path in ["crates/demo/guide/my page.md", "crates/demo/notes/parts.md", "crates/demo/icons/a.svg",
                         "crates/demo/icons/new.svg"]:
                with self.subTest(path=path):
                    self.assertTrue(needs_rust([path]))
            self.assertFalse(needs_rust(["crates/demo/README.md"]))
            # Words in a comment or a string are not an include: the source holds three.
            source = Path(rust_changes.ROOT, "crates/demo/src/lib.rs").read_text()
            self.assertEqual(len(list(rust_changes.include_arguments(source))), 3)

    def test_dynamic_include_can_escape_its_prefix(self):
        with fixture_repository({
            "crates/demo/Cargo.toml": '[package]\nname = "demo"\n',
            "crates/demo/src/lib.rs": (
                'macro_rules! doc { ($name:literal) => { include_str ! (concat!("../notes/", $name, ".md")) }; }\n'
                'const TEXT: &str = doc!("../outside");\n'
            ),
            "crates/demo/notes/keep.txt": "x\n",
            "crates/demo/outside.md": "x\n",
        }):
            self.assertTrue(needs_rust(["crates/demo/outside.md"]))

    def test_helpers_of_gated_steps_count_as_their_inputs(self):
        workflow = (
            "on: pull_request\njobs:\n"
            "  lint:\n    runs-on: ubuntu-latest\n    steps:\n"
            "      - name: Boundary\n        if: needs.changes.outputs.rust_required != 'false'\n"
            "        run: python3 tools/ci/check_boundary.py --source-only\n"
            "      - name: Prose\n        run: python3 tools/ci/prose.py docs/guide.md\n"
            "  test:\n    runs-on: ubuntu-latest\n    steps:\n"
            "      - id: rust\n        run: python3 tools/ci/rust_changes.py a b\n"
            "      - if: steps.rust.outputs.required != 'false'\n"
            "        run: |\n          python3 tools/ci/shards.py --prepare\n"
        )
        with fixture_repository({
            ".github/workflows/ci.yml": workflow,
            "tools/ci/check_boundary.py": "from tools.ci import boundary_rules\n",
            "tools/ci/boundary_rules.py": "LAYERS = 'docs/layers.md'\n",
            "tools/ci/shards.py": "import json\n", "tools/ci/prose.py": "",
            "tools/ci/unused.py": "", "docs/layers.md": "x\n", "docs/guide.md": "x\n",
        }):
            # The step's script, what it imports, and the file that module reads.
            for path in ["tools/ci/check_boundary.py", "tools/ci/boundary_rules.py", "docs/layers.md",
                         "tools/ci/shards.py"]:
                with self.subTest(path=path):
                    self.assertTrue(needs_rust([path]))
            for path in ["tools/ci/prose.py", "tools/ci/unused.py", "docs/guide.md"]:
                with self.subTest(path=path):
                    self.assertFalse(needs_rust([path]))

    def test_this_repositorys_gated_helpers_select_their_steps(self):
        # The boundary check, the test binaries' runner and the formatting driver run only under rust_required, so a
        # change to one alone runs it.
        for path in ["tools/ci/check_api_boundary.py", "tools/ci/run_test_binaries.py", "tools/ci/format_scan.py"]:
            with self.subTest(path=path):
                self.assertTrue(selections([("M", path)])["rust_required"])
        self.assertFalse(selections([("M", "tools/parity.py")])["rust_required"])

    def test_pr_selection_uses_the_full_gate_on_invalid_or_empty_changes(self):
        unknown = [("?", UNKNOWN_CHANGE)]
        with tempfile.TemporaryDirectory() as directory:
            listing = Path(directory, "list")
            sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
            for data in [b"", b"M\0docs/setup.md", b"M\0", b"X\0docs/setup.md\0"]:
                listing.write_bytes(data)
                with self.subTest(data=data):
                    self.assertEqual(rust_changes.pr_entries(sha, sha, listing, 1), unknown)
            listing.write_bytes(b"M\0docs/setup.md\0")
            self.assertEqual(rust_changes.pr_entries("", sha, listing, 1), unknown)
            self.assertEqual(rust_changes.pr_entries(sha, sha, listing, 1), [("M", "docs/setup.md")])
            self.assertEqual(selections(unknown), {"rust_required": True, "tooling_required": True})

    def test_a_listing_that_lost_a_record_or_repeats_or_misnames_one_selects_everything(self):
        unknown = [("?", UNKNOWN_CHANGE)]
        sha = "a" * 40
        with tempfile.TemporaryDirectory() as directory:
            listing = Path(directory, "list")
            listing.write_bytes(b"M\0docs/a.md\0")
            # pr_files.py printed two entries: the listing lost its last record.
            self.assertEqual(rust_changes.pr_entries(sha, sha, listing, 2), unknown)
            for data in [b"M\0docs/a.md\0D\0docs/a.md\0", b"M\0docs/../src/lib.rs\0", b"M\0./src/lib.rs\0"]:
                listing.write_bytes(data)
                with self.subTest(data=data):
                    self.assertEqual(rust_changes.pr_entries(sha, sha, listing, data.count(b"\0") // 2), unknown)

    def test_an_empty_diff_endpoint_means_everything_changed(self):
        self.assertEqual(changed_entries("", "a" * 40), [("?", UNKNOWN_CHANGE)])
        self.assertEqual(changed_entries("a" * 40, ""), [("?", UNKNOWN_CHANGE)])

    def test_a_path_attribute_split_over_lines_is_an_include(self):
        # `#[` and `path` on different lines, and the value continued with an escaped newline.
        source = '#[\n    path = "../docs/gen\\\n        erated.txt"\n]\nmod generated;\n'
        with fixture_repository({"src/lib.rs": source, "docs/generated.txt": "", "docs/other.md": ""}):
            self.assertTrue(selections([("M", "docs/generated.txt")])["rust_required"])
            self.assertFalse(selections([("M", "docs/other.md")])["rust_required"])

    def test_out_dir_after_another_piece_is_not_a_build_output_path(self):
        with fixture_repository({"src/lib.rs": 'const C: &str = include_str!(concat!(env!("A"), env!("OUT_DIR"), "/../docs/x.md"));',
                                 "docs/x.md": ""}):
            self.assertTrue(selections([("M", "docs/x.md")])["rust_required"])

    def test_pr_file_producer_changes_require_formatting(self):
        self.assertTrue(selections([("M", "tools/ci/pr_files.py")])["rust_required"])

    def test_a_base_missing_from_the_checkout_means_everything_changed(self):
        with tempfile.TemporaryDirectory() as directory:
            subprocess.check_call(["git", "-C", directory, "init", "--quiet"])
            subprocess.check_call([
                "git", "-C", directory, "-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "head",
            ])
            previous = Path.cwd()
            try:
                os.chdir(directory)
                # A base newer than the checked-out history, as when main moved after the PR event.
                paths = changed_paths("a" * 40, "HEAD")
            finally:
                os.chdir(previous)
            self.assertEqual(paths, [UNKNOWN_CHANGE])
            self.assertTrue(needs_rust(paths))


if __name__ == "__main__":
    unittest.main()
