#!/usr/bin/env python3
"""Remove the `access` key from `[[inject]]`: only API access is left, so the key says nothing.

Mechanical part: drops `access = "api"` lines from manifests, fixtures and manifest text embedded
in Rust tests, and the `access: Access::Api` field initialisers. The parser's refusal and the docs
are edited by hand in the commits that follow. Run from the repository root.
"""
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
FILES = subprocess.run(
    ["git", "ls-files", "crates", "docs/modding", "launcher/src", "tools/ci"],
    cwd=ROOT, check=True, capture_output=True, text=True,
).stdout.split()
LINE = re.compile(r"^[ \t]*access = \"api\"\n", re.M)
ESCAPED = re.compile(r"access = \\\"api\\\"\\n")
FIELD = re.compile(r"^[ \t]*access: Access::Api,\n", re.M)

for name in FILES:
    path = ROOT / name
    if path.suffix not in {".toml", ".rs", ".md"} or not path.is_file():
        continue
    text = path.read_text()
    new = FIELD.sub("", ESCAPED.sub("", LINE.sub("", text)))
    if new != text:
        path.write_text(new)
        print(name)
