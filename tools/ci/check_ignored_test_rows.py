#!/usr/bin/env python3
"""verify-static's guard for tools/ci/ignored-tests/: read every policy file as the ignored-test readers do.

Fails on a malformed row, a test listed in two files, a folder with no rows or with a file other than `*.txt`
and README.md, and a file at the retired single-list path. Builds nothing.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import ignored_tests

rows = ignored_tests.listed_entries()
print(f"{len(rows)} rows in {len({row.file for row in rows})} files of {ignored_tests.POLICY_PATH}/")
