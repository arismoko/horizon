import contextlib
import io
import struct
import tempfile
import unittest
from pathlib import Path

from tools import prebuilt_game


class AllocatorSelectionTests(unittest.TestCase):
    def test_windows_archives_use_the_system_allocator(self):
        self.assertEqual(prebuilt_game.cargo_features(False, "win32"), ["--features", "system-allocator"])
        self.assertEqual(prebuilt_game.cargo_features(True, "win32"),
                         ["--no-default-features", "--features", "system-allocator"])

    def test_other_systems_keep_mimalloc(self):
        for platform in ("linux", "darwin"):
            self.assertEqual(prebuilt_game.cargo_features(False, platform), [])
            self.assertEqual(prebuilt_game.cargo_features(True, platform), ["--no-default-features"])


class PrebuiltContentsTests(unittest.TestCase):
    def folder(self, *names: str) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for name in names:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_bytes(b"")
        return root

    def test_compiler_outputs_pass(self):
        prebuilt_game.check(self.folder(
            "horizon-prebuilt.json", "Cargo.lock", "native_main.rs.template", "deps/libhorizon_client.so", "deps/libmod_api-1.rmeta",
            "native/libwindows.0.52.0.a", "points.json", "internals.json",
            "deps/libserde_derive-2.so", "deps/libhorizon_client.dll.a", "deps/libhorizon_mod_wasm.so",
            "deps/horizon_mod_wasm.dll", "deps/libhorizon_mod_wasm.dylib",
        ))

    def test_anything_else_is_refused(self):
        for stray in ("assets/minecraft/lang/en_us.json", "deps/Cargo.lock", "deps/client.jar", "deps/libmod_api-3.rlib", "native/libfoo.so", "deps/internals.json", "notes.txt"):
            with self.subTest(stray), self.assertRaises(SystemExit):
                prebuilt_game.check(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so", stray))

    def test_a_folder_without_either_runtime_is_refused(self):
        for present, missing in (("horizon_client", "horizon-mod-wasm"), ("horizon_mod_wasm", "horizon-client")):
            with self.subTest(present), self.assertRaisesRegex(SystemExit, missing):
                prebuilt_game.check(self.folder(f"deps/lib{present}.so"))

    def dll(self, root: Path, name: str, count: int, pe64: bool = True) -> None:
        # One PE section: RVA 0x1000 maps to raw offset 0x200, holding the export directory.
        data = bytearray(0x400)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<H", data, 0x86, 1)  # NumberOfSections
        optional_size = 240 if pe64 else 224
        struct.pack_into("<H", data, 0x94, optional_size)
        struct.pack_into("<H", data, 0x98, 0x20B if pe64 else 0x10B)
        struct.pack_into("<II", data, 0x98 + (112 if pe64 else 96), 0x1000, 40)
        struct.pack_into("<III", data, 0x98 + optional_size + 12, 0x1000, 0x200, 0x200)
        struct.pack_into("<I", data, 0x200 + 24, count)  # NumberOfNames
        (root / "deps" / name).write_bytes(data)

    def test_exports_reports_every_dll(self):
        root = self.folder("deps/libhorizon_client.so")
        self.dll(root, "horizon_client.dll", 45000)
        self.dll(root, "horizon_mod_wasm.dll", 12000)
        self.dll(root, "serde_derive.dll", 1, pe64=False)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(root)
        for name, count in (("horizon_client", 45000), ("horizon_mod_wasm", 12000), ("serde_derive", 1)):
            self.assertIn(f"{name}.dll: {count} exported names", output.getvalue())

    def test_exports_warns_about_any_dll_near_the_limit_without_failing(self):
        for large in ("horizon_client", "horizon_mod_wasm"):
            root = self.folder("deps/libhorizon_client.so")
            for name in ("horizon_client", "horizon_mod_wasm"):
                self.dll(root, f"{name}.dll", 49000 if name == large else 48999)
            output = io.StringIO()
            with self.subTest(large), contextlib.redirect_stdout(output):
                prebuilt_game.exports(root)
            warnings = [line for line in output.getvalue().splitlines() if line.startswith("::warning::")]
            self.assertEqual(len(warnings), 1)
            self.assertIn(f"{large}.dll exports 49,000 names", warnings[0])
            self.assertEqual(output.getvalue().count("exported names (Windows' limit"), 2)

    def test_exports_is_silent_without_dlls(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so"))
        self.assertEqual(output.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
