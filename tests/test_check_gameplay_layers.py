"""Unit tests for `tools/ci/check_gameplay_layers.py`."""

from __future__ import annotations

import argparse
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import pr_ready
from tools.ci import check_gameplay_layers as cgl

ROOT = Path(__file__).resolve().parents[1]


def write_workspace(root: Path, manifests: dict[str, str], workspace_toml: str | None = None) -> None:
    if workspace_toml is not None:
        (root / "Cargo.toml").write_text(workspace_toml, encoding="utf-8")
    for crate, content in manifests.items():
        path = root / "crates" / crate / "Cargo.toml"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def clean_manifests() -> dict[str, str]:
    return {
        "sunrise-gameplay-common": (
            '[package]\nname = "sunrise-gameplay-common"\nversion = "0.1.0"\n\n'
            "[dependencies]\n"
            'sunrise-core = { path = "../sunrise-core" }\n'
            'mod_api = { package = "sunrise-mod-vocab", path = "../sunrise-mod-vocab" }\n\n'
            "[dev-dependencies]\n"
            'mod_platform = { package = "sunrise-mod-api", path = "../sunrise-mod-api" }\n\n'
            "[build-dependencies]\n"
            'sunrise-mod-api-gen = { path = "../sunrise-mod-api-gen" }\n'
        ),
        "sunrise-gameplay": (
            '[package]\nname = "sunrise-gameplay"\nversion = "0.1.0"\n\n'
            "[dependencies]\n"
            'sunrise-gameplay-common = { path = "../sunrise-gameplay-common" }\n'
            'mod_platform = { package = "sunrise-mod-api", path = "../sunrise-mod-api", optional = true }\n'
        ),
        "sunrise-client-gameplay": (
            '[package]\nname = "sunrise-client-gameplay"\nversion = "0.1.0"\n\n'
            "[dependencies]\n"
            'sunrise-gameplay-common = { path = "../sunrise-gameplay-common" }\n'
            'sunrise-client-render = { path = "../sunrise-client-render" }\n'
            'wgpu = "30"\n'
            'winit = "0.30"\n'
        ),
    }


class CheckGameplayLayersTests(unittest.TestCase):
    def test_real_workspace_manifests_pass(self) -> None:
        self.assertEqual(cgl.check_gameplay_layers(ROOT), [])

    def test_clean_synthetic_workspace_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_workspace(root, clean_manifests())
            self.assertEqual(cgl.check_gameplay_layers(root), [])

    def test_common_rejects_side_crates_desktop_crates_and_non_dev_mod_platform(self) -> None:
        cases = [
            ('[dependencies]\nsunrise-gameplay = { path = "../sunrise-gameplay" }\n', "side crate sunrise-gameplay"),
            (
                '[dev-dependencies]\nsunrise-client-gameplay = { path = "../sunrise-client-gameplay" }\n',
                "side crate sunrise-client-gameplay",
            ),
            ('[dependencies]\nsunrise-client = { path = "../sunrise-client" }\n', "may not depend on sunrise-client"),
            (
                '[dependencies]\nmod_platform = { package = "sunrise-mod-api", path = "../sunrise-mod-api" }\n',
                "mod_platform outside dev-dependencies",
            ),
            (
                '[build-dependencies]\nsunrise-mod-api = { path = "../sunrise-mod-api" }\n',
                "mod_platform outside dev-dependencies",
            ),
            (
                '[target."cfg(unix)".dependencies]\nmod_platform = { package = "sunrise-mod-api", path = "../sunrise-mod-api" }\n',
                "mod_platform outside dev-dependencies",
            ),
        ]
        for token in cgl.DESKTOP_TOKENS:
            cases.append((f'[dependencies]\nsome-{token}-dep = "1"\n', f"name contains {token!r}"))

        for extra, expected in cases:
            with self.subTest(expected=expected, extra=extra.strip()):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    manifests = clean_manifests()
                    manifests["sunrise-gameplay-common"] = (
                        '[package]\nname = "sunrise-gameplay-common"\nversion = "0.1.0"\n\n' + extra
                    )
                    write_workspace(root, manifests)
                    problems = cgl.check_gameplay_layers(root)
                    self.assertTrue(any(expected in p for p in problems), problems)

    def test_common_allows_target_dev_mod_platform(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifests = clean_manifests()
            manifests["sunrise-gameplay-common"] += (
                '\n[target."cfg(unix)".dev-dependencies]\n'
                'mod_platform = { package = "sunrise-mod-api", path = "../sunrise-mod-api" }\n'
            )
            write_workspace(root, manifests)
            self.assertEqual(cgl.check_gameplay_layers(root), [])

    def test_server_rejects_client_gameplay_entry_and_desktop_crates(self) -> None:
        cases = [
            ('[dependencies]\nsunrise-client-gameplay = { path = "../sunrise-client-gameplay" }\n',
             "may not depend on sunrise-client-gameplay"),
            ('[dependencies]\naliased = { package = "sunrise-client-gameplay", path = "../sunrise-client-gameplay" }\n',
             "may not depend on sunrise-client-gameplay"),
            ('[dev-dependencies]\nsunrise-client = { path = "../sunrise-client" }\n',
             "may not depend on sunrise-client"),
            ('[target."cfg(windows)".dependencies]\nwgpu = "30"\n', "name contains 'wgpu'"),
            ('[dependencies]\nsunrise-theme = { path = "../sunrise-theme" }\n', "name contains 'theme'"),
            ('[dependencies]\nsunrise-audio-input = { path = "../sunrise-audio-input" }\n', "name contains 'audio'"),
        ]
        for extra, expected in cases:
            with self.subTest(expected=expected):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    manifests = clean_manifests()
                    manifests["sunrise-gameplay"] = (
                        '[package]\nname = "sunrise-gameplay"\nversion = "0.1.0"\n\n' + extra
                    )
                    write_workspace(root, manifests)
                    problems = cgl.check_gameplay_layers(root)
                    self.assertTrue(any(expected in p for p in problems), problems)

    def test_client_gameplay_rejects_server_and_entry_but_allows_desktop(self) -> None:
        cases = [
            ('[dependencies]\nsunrise-gameplay = { path = "../sunrise-gameplay" }\n',
             "may not depend on sunrise-gameplay"),
            ('[dev-dependencies]\nsunrise_gameplay = { path = "../sunrise-gameplay" }\n',
             "may not depend on sunrise-gameplay"),
            ('[dependencies]\nsunrise-client = { path = "../sunrise-client" }\n',
             "may not depend on sunrise-client"),
        ]
        for extra, expected in cases:
            with self.subTest(expected=expected):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    manifests = clean_manifests()
                    manifests["sunrise-client-gameplay"] = (
                        '[package]\nname = "sunrise-client-gameplay"\nversion = "0.1.0"\n\n' + extra
                    )
                    write_workspace(root, manifests)
                    problems = cgl.check_gameplay_layers(root)
                    self.assertTrue(any(expected in p for p in problems), problems)

    def test_workspace_dependency_package_rename_is_checked(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifests = clean_manifests()
            manifests["sunrise-gameplay"] += "graphics = { workspace = true }\n"
            ws = '[workspace.dependencies]\ngraphics = { package = "wgpu", version = "30" }\n'
            write_workspace(root, manifests, workspace_toml=ws)
            problems = cgl.check_gameplay_layers(root)
            self.assertTrue(any("graphics (package wgpu)" in p for p in problems), problems)

    def test_missing_manifest_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifests = clean_manifests()
            del manifests["sunrise-gameplay"]
            write_workspace(root, manifests)
            problems = cgl.check_gameplay_layers(root)
            self.assertEqual(problems, ["crates/sunrise-gameplay/Cargo.toml: manifest is missing"])

    def test_wired_into_pr_ready_and_dyna_ci(self) -> None:
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(
                argparse.Namespace(host="local", quick=True, harnesses=False),
                "origin/main",
                {"README.md"},
                {"sunrise-core": "crates/sunrise-core"},
                {"sunrise-core"},
                {},
                merge_base="deadbeef",
            )
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["Gameplay layers"].command, "python3 tools/ci/check_gameplay_layers.py")

        wf = (ROOT / ".github/workflows/dyna-ci.yml").read_text(encoding="utf-8")
        self.assertIn("python3 tools/ci/check_gameplay_layers.py", wf)


if __name__ == "__main__":
    unittest.main()
