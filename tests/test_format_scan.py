import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.ci import format_scan  # noqa: E402


class FormatScanTest(unittest.TestCase):
    def test_cargo_fmt_then_every_file_no_mod_line_reaches_by_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ("crates/sunrise-client/src/game/world/levels.rs",
                         "crates/sunrise-client/src/game/world/levels/tests.rs",
                         "crates/sunrise-mod-api/src/lib.rs",
                         "crates/sunrise-mod-vocab/src/hooks/player.rs",
                         "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/a.rs",
                         "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/b/c.rs",
                         # Reached by cargo fmt through its `mod` lines, so not listed by path.
                         "crates/sunrise-client/src/lib.rs",
                         "crates/sunrise-client/src/game/world/notes.md"):
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text("")
            self.assertEqual(format_scan.commands(root), [
                ["cargo", "fmt", "--all", "--", "--check"],
                ["rustfmt", "--edition", "2024", "--check", "--config", "skip_children=true",
                 "crates/sunrise-client/src/game/world/levels.rs",
                 "crates/sunrise-client/src/game/world/levels/tests.rs",
                 "crates/sunrise-mod-api/src/lib.rs",
                 "crates/sunrise-mod-vocab/src/hooks/player.rs",
                 "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/a.rs",
                 "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/b/c.rs"],
            ])

    def test_split_library_roots_follow_literal_fragments_outside_the_owner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "crates/sunrise-mod-api-gen/source-layout"
            marker.parent.mkdir(parents=True)
            marker.write_text("split")
            for owner in format_scan.GAMEPLAY_OWNERS:
                package = root / "crates" / owner
                source = package / "engine"
                source.mkdir(parents=True)
                (package / "Cargo.toml").write_text('[lib]\npath = "engine/root.rs"\n')
                (source / "root.rs").write_text(
                    'include!{r#"../../../fragments/shared.rs"#};\n'
                    '// include!("../../../fragments/comment.rs");\n'
                    'include!(concat!(env!("OUT_DIR"), "/generated.rs"));\n')
            fragments = root / "fragments"
            fragments.mkdir()
            (fragments / "shared.rs").write_text('include!["nested.rs"];')
            (fragments / "nested.rs").write_text("pub struct Nested;")
            (fragments / "comment.rs").write_text("pub struct Comment;")
            commands = format_scan.commands(root)
            files = [path for command in commands[1:] for path in command[len(format_scan.RUSTFMT):]]
            self.assertEqual(files, sorted(f"crates/{owner}/engine/root.rs" for owner in format_scan.GAMEPLAY_OWNERS)
                             + ["fragments/nested.rs", "fragments/shared.rs"])

    def test_long_file_lists_are_split_and_every_check_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            game = root / "crates/sunrise-client/src/game"
            game.mkdir(parents=True)
            for index in range(format_scan.CHUNK + 1):
                (game / f"m{index:03}.rs").write_text("")
            commands = format_scan.commands(root)
            self.assertEqual([len(command) - len(format_scan.RUSTFMT) for command in commands[1:]],
                             [format_scan.CHUNK, 1])
            ran = []
            # The first rustfmt chunk fails; the second still runs, and the scan fails.
            with mock.patch.object(format_scan, "ROOT", root), \
                    mock.patch.object(format_scan.subprocess, "run",
                                      side_effect=lambda command, cwd, capture_output=True, text=True: ran.append(command) or mock.Mock(
                                          returncode=1 if command == commands[1] else 0, stdout="", stderr="")):
                self.assertEqual(format_scan.main(), 1)
            self.assertEqual(sorted(ran), sorted(commands))


if __name__ == "__main__":
    unittest.main()
