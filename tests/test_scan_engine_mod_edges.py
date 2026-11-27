import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools.layering import rust_text, scan_engine_mod_edges as scan

LIB = """\
use mod_api::{Flow, HookId, Host};
use sunrise_mod_runtime::attachments::Holder;
// mod_api::InComment is not code
const S: &str = "mod_api::InString";
struct Game<'a> { level: u8, items: &'a [u8] }
impl Host for Game<'_> {}
pub mod game;
fn fire(game: &mut Game<'_>) -> Flow {
    if mod_api::__private::listened(HookId::BlockDrops) {
        game.native_drops();
    }
    Flow::Continue
}
#[cfg(test)]
mod tests {
    #[test]
    fn mods() { mod_api::testing::with_mods(Default::default(), || ()); }
}
#[cfg(test)]
mod more_tests;
"""

MODDING = """\
impl super::Game<'_> {
    pub(in crate::game) fn native_drops(&mut self) {
        let _ = self.level;
        self.private_helper();
        mod_api::__private::install(Default::default());
    }
}
"""


def package(name, path, deps=(), lib=None):
    return {
        "name": name,
        "manifest_path": f"{path}/Cargo.toml",
        "targets": [{"name": lib or name.replace("-", "_"), "kind": ["lib"], "src_path": f"{path}/src/lib.rs"}],
        "dependencies": [{"name": d, "kind": k, "path": f"/x/{d}"} for d, k in deps],
    }


class ScanTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name).resolve()
        files = {
            "crates/sunrise-client/Cargo.toml": "[package]\nname = \"sunrise-client\"\n\n[dependencies]\n"
                                                "sunrise-mod-api = { path = \"x\" }\nsunrise-mod-runtime = 1\n",
            "crates/sunrise-client/src/lib.rs": LIB,
            "crates/sunrise-client/src/more_tests.rs": "fn t() { mod_api::Flow::Continue; }\n",
            "crates/sunrise-client/src/game.rs": "impl Game<'_> { fn private_helper(&self) {} }\n",
            "crates/sunrise-client/src/game/modding/drops.rs": MODDING,
            "crates/sunrise-mod-api/Cargo.toml": "[package]\nname = \"sunrise-mod-api\"\n",
            "crates/sunrise-mod-api/build.rs": "fn main() {}\n",
            "crates/sunrise-mod-api/src/lib.rs": "pub struct Flow;\nstatic TABLE: Vec<u8> = Vec::new();\n"
                                                 "include!(concat!(env!(\"OUT_DIR\"), \"/mod_api_generated.rs\"));\n",
            "crates/sunrise-mod-api/src/types.rs": "pub struct BlockPos(pub i32);\n",
            "tools/layers.json": json.dumps({"crates": {"sunrise-client": "engine", "sunrise-mod-api": "platform",
                                                        "sunrise-mod-runtime": "engine"},
                                             "allowed": ["sunrise-client -> sunrise-mod-api"]}),
        }
        for path, text in files.items():
            (self.root / path).parent.mkdir(parents=True, exist_ok=True)
            (self.root / path).write_text(text)
        packages = [
            package("sunrise-client", self.root / "crates/sunrise-client",
                    [("sunrise-mod-api", None), ("sunrise-mod-runtime", None)]),
            package("sunrise-mod-api", self.root / "crates/sunrise-mod-api", [("sunrise-core", None)], "mod_api"),
            package("sunrise-mod-runtime", self.root / "crates/sunrise-mod-runtime"),
        ]
        patches = [mock.patch.object(scan, "ROOT", self.root), mock.patch.object(scan, "metadata", lambda: packages),
                   mock.patch.object(scan, "git_head", lambda: "0" * 40)]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)
        self.result = scan.scan()

    def tearDown(self):
        self.dir.cleanup()

    def edges(self, file):
        return [(e["line"], e["kind"], e["path"], e["test"]) for e in self.result["edges"]
                if e["file"] == f"crates/sunrise-client/src/{file}"]

    def test_finds_uses_impls_calls_and_skips_comments_and_strings(self):
        found = self.edges("lib.rs")
        self.assertIn((1, "use", "mod_api::Flow", False), found)
        self.assertIn((2, "use", "sunrise_mod_runtime::attachments::Holder", False), found)
        self.assertIn((6, "impl", "mod_api::Host", False), found)
        self.assertIn((9, "call", "mod_api::__private::listened", False), found)
        self.assertFalse([e for e in found if "InComment" in e[2] or "InString" in e[2]])

    def test_marks_test_items_and_test_modules(self):
        self.assertIn((17, "call", "mod_api::testing::with_mods", True), self.edges("lib.rs"))
        self.assertEqual([(1, "type", "mod_api::Flow::Continue", True)], self.edges("more_tests.rs"))

    def test_strands_come_from_the_rules(self):
        by_path = {e["path"]: e["strand"] for e in self.result["edges"]}
        self.assertEqual("hooks", by_path["mod_api::__private::listened"])
        self.assertEqual("host", by_path["mod_api::Host"])
        self.assertEqual("attachments", by_path["sunrise_mod_runtime::attachments::Holder"])
        self.assertEqual("loading", by_path["mod_api::__private::install"])

    def test_manifest_edges_carry_their_line_and_layer(self):
        rows = [(e["dep"], e["line"], e["layer_exception"]) for e in self.result["manifest_edges"]]
        self.assertEqual([("sunrise-mod-api", 5, True), ("sunrise-mod-runtime", 6, False)], rows)

    def test_move_set_reports_impls_inbound_and_private_items(self):
        drops = self.result["move_set"]["crates/sunrise-client/src/game/modding/drops.rs"]
        self.assertEqual((1, "loading"), (drops["inherent_impls"], drops["strand"]))
        self.assertIn("fn:private_helper", drops["outbound"])
        inbound = [(e["file"], e["name"]) for e in self.result["inbound"]]
        self.assertEqual([], inbound)  # lib.rs is outside game/, so its call isn't a game-tree reference

    def test_vocabulary_separates_plain_files_from_blockers(self):
        files = {Path(f["file"]).name: f for f in self.result["vocabulary"]["files"]}
        self.assertTrue(files["types.rs"]["pure"])
        self.assertFalse(files["lib.rs"]["pure"])
        self.assertEqual([3], files["lib.rs"]["generated_include"])
        self.assertEqual([2], files["lib.rs"]["state"])

    def test_the_scan_is_deterministic(self):
        self.assertEqual(json.dumps(self.result, sort_keys=True), json.dumps(scan.scan(), sort_keys=True))

    def test_map_writes_the_tables_to_the_given_file_and_no_committed_page(self):
        page = self.root / "docs/layering/mod-host-map.md"
        page.parent.mkdir(parents=True)
        page.write_text("hand-written\n")
        out = self.root / "target/layering/tables.md"
        self.assertEqual(0, scan.main(["--map", str(out)]))
        tables = out.read_text()
        self.assertTrue(tables.startswith(f"# Mod-host map tables at {'0' * 40}\n"))
        self.assertIn("sunrise-mod-api", tables)
        self.assertEqual("hand-written\n", page.read_text())

    def test_split_gameplay_crates_are_inventoried_in_move_set(self):
        split_dir = tempfile.TemporaryDirectory()
        self.addCleanup(split_dir.cleanup)
        root = Path(split_dir.name).resolve()
        files = {
            "crates/sunrise-gameplay-common/Cargo.toml": "[package]\nname = \"sunrise-gameplay-common\"\n\n[dependencies]\nsunrise-mod-api = { path = \"x\" }\n",
            "crates/sunrise-gameplay-common/src/lib.rs": "pub mod game;\n",
            "crates/sunrise-gameplay-common/src/game.rs": "pub struct CommonGame<'a> { level: u8, items: &'a [u8] }\nimpl CommonGame<'_> { fn private_helper(&self) {} }\n",
            "crates/sunrise-gameplay/Cargo.toml": "[package]\nname = \"sunrise-gameplay\"\n\n[dependencies]\nsunrise-mod-api = { path = \"x\" }\n",
            "crates/sunrise-gameplay/src/lib.rs": "pub mod game;\n",
            "crates/sunrise-gameplay/src/game/modding/drops.rs": (
                "impl crate::game::ServerGame<'_> {\n"
                "    pub(in crate::game) fn native_drops(&mut self) {\n"
                "        self.private_helper();\n"
                "        mod_api::__private::install(Default::default());\n"
                "    }\n"
                "}\n"
            ),
            "crates/sunrise-mod-api/Cargo.toml": "[package]\nname = \"sunrise-mod-api\"\n",
            "crates/sunrise-mod-api/src/lib.rs": "pub struct Flow;\n",
            "tools/layers.json": json.dumps({"crates": {
                "sunrise-gameplay-common": "engine",
                "sunrise-gameplay": "engine",
                "sunrise-mod-api": "platform",
            }, "allowed": []}),
        }
        for path, text in files.items():
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text)
        packages = [
            package("sunrise-gameplay-common", root / "crates/sunrise-gameplay-common", [("sunrise-mod-api", None)]),
            package("sunrise-gameplay", root / "crates/sunrise-gameplay", [("sunrise-mod-api", None)]),
            package("sunrise-mod-api", root / "crates/sunrise-mod-api", [], "mod_api"),
        ]
        with mock.patch.object(scan, "ROOT", root), mock.patch.object(scan, "metadata", lambda: packages), \
                mock.patch.object(scan, "git_head", lambda: "0" * 40):
            res = scan.scan()
        drops = res["move_set"]["crates/sunrise-gameplay/src/game/modding/drops.rs"]
        self.assertEqual((1, "loading"), (drops["inherent_impls"], drops["strand"]))
        self.assertIn("fn:private_helper", drops["outbound"])


class RustTextTest(unittest.TestCase):
    def test_use_trees_flatten_with_aliases_and_globs(self):
        self.assertEqual([(("a", "B"), "C"), (("a", "d"), None), (("a",), "a")],
                         rust_text.use_leaves("a::{B as C, d::*, self}"))

    def test_blanking_keeps_offsets_and_lifetimes(self):
        text = "fn f<'a>(x: &'a str) { let c = '}'; } /* } */ \"}\""
        code = rust_text.blank(text)
        self.assertEqual(len(text), len(code))
        self.assertIn("<'a>", code)
        self.assertEqual(1, code.count("}"))


if __name__ == "__main__":
    unittest.main()
