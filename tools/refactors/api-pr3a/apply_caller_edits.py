#!/usr/bin/env python3
"""Replay exact compiler-guided caller edits from a reviewed JSON log.

Each entry names a file, the complete old/new text and an expected occurrence
count. A changed baseline fails rather than silently rewriting unrelated code.
Usage: apply_caller_edits.py LOG.json
"""
import json
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[3]
for entry in json.loads(Path(sys.argv[1]).read_text()):
    path = root / entry['path']
    text = path.read_text()
    count = text.count(entry['old'])
    if count != entry['count']:
        raise ValueError(f'{path}: expected {entry["count"]} matches, found {count}')
    path.write_text(text.replace(entry['old'], entry['new']))
