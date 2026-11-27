"""Crate renames must not rename player data paths or the separate registry repo."""
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import shutil
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

from tests.git_fixture import git_in

spec = importlib.util.spec_from_file_location("rename_crate", Path(__file__).resolve().parents[1] / "tools/rename_crate.py")
rename = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rename)


class RenameCrateTests(unittest.TestCase):
    def test_manifest_and_crate_paths_change_without_touching_player_paths(self):
        source = '''[package]
name = "sunrise-mods"
[dependencies]
sunrise-mods = { path = "../sunrise-mods" }
[dev-dependencies.sunrise-mods]
path = "../crates/sunrise-mods"
# Put files in sunrise-mods/ and enable them in sunrise-mods.txt.
'''
        expected = '''[package]
name = "sunrise-mod-package"
[dependencies]
sunrise-mod-package = { path = "../sunrise-mod-package" }
[dev-dependencies.sunrise-mod-package]
path = "../crates/sunrise-mod-package"
# Put files in sunrise-mods/ and enable them in sunrise-mods.txt.
'''
        self.assertEqual(rename.rewrite("crates/sunrise-mods/Cargo.toml", source), expected)
        self.assertEqual(rename.destination("crates/sunrise-mods/tests/file.smod"),
                         "crates/sunrise-mod-package/tests/file.smod")
        self.assertEqual(rename.destination("crates/sunrise-mods-other/src/lib.rs"),
                         "crates/sunrise-mods-other/src/lib.rs")
        self.assertEqual(rename.rewrite("crates/sunrise-mod-registry/tests/submit.rs", '../sunrise-mods/tests/index'),
                         '../sunrise-mod-package/tests/index')

    def test_rust_library_identifiers_and_link_arguments_change(self):
        source = '''use sunrise_mods::{Content, Side};
mod sunrise_mods { fn fixture() {} }
let link = "sunrise_mods={}";
let module = sunrise_mods::Package;
fn sunrise_mods_test_name() {}
let folder = "sunrise-mods";
let registry = "https://github.com/horizon-rust/horizon-mods";
'''
        expected = '''use sunrise_mod_package::{Content, Side};
mod sunrise_mod_package { fn fixture() {} }
let link = "sunrise_mod_package={}";
let module = sunrise_mod_package::Package;
fn sunrise_mods_test_name() {}
let folder = "sunrise-mods";
let registry = "https://github.com/horizon-rust/horizon-mods";
'''
        self.assertEqual(rename.rewrite("crates/sunrise-client/src/native_mod_host.rs", source), expected)

    @unittest.skipUnless(shutil.which("rustfmt"), "requires rustfmt")
    def test_verify_rejects_an_unchanged_file_that_still_needs_the_mapping(self):
        with TemporaryDirectory() as directory, patch.object(rename, "ROOT", Path(directory)):
            crate = rename.ROOT / rename.OLD_PATH
            crate.mkdir(parents=True)
            (crate / "Cargo.toml").write_text('[package]\nname = "sunrise-mods"\n')
            consumer = rename.ROOT / "consumer.rs"
            consumer.write_text("use sunrise_mods::Manifest;\n")
            git = git_in(rename.ROOT)
            git("init", "-q")
            git("add", "--", rename.OLD_PATH, "consumer.rs")
            git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "Baseline")
            git("mv", rename.OLD_PATH, rename.NEW_PATH)
            (rename.ROOT / rename.NEW_PATH / "Cargo.toml").write_text(
                '[package]\nname = "sunrise-mod-package"\n', newline="\n")
            with self.assertRaisesRegex(AssertionError, "unexpected content change: consumer.rs"):
                rename.verify("HEAD")
            consumer.write_text("use sunrise_mod_package::Manifest;\n", newline="\n")
            with redirect_stdout(io.StringIO()):
                rename.verify("HEAD")

    def test_book_filters_and_translations_obey_the_boundary(self):
        source = '{{#include ../../crates/sunrise-mods/src/lib.rs}}\n(crates/sunrise-mods)\ncrates/sunrise-mods/**\n'
        expected = '{{#include ../../crates/sunrise-mod-package/src/lib.rs}}\n(crates/sunrise-mod-package)\ncrates/sunrise-mod-package/**\n'
        self.assertEqual(rename.rewrite("docs/modding/src/reference.md", source), expected)
        translation = '{"native.mods": "In sunrise-mods; internal type sunrise_mods::Package"}'
        self.assertEqual(rename.rewrite("crates/sunrise-client/assets/sunrise/lang/en_us.json", translation), translation)
        self.assertEqual(rename.rewrite("roadmap/roadmap.toml", 'dev = "sunrise-mods/native/"'),
                         'dev = "sunrise-mods/native/"')
