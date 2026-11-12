#!/usr/bin/env python3
"""Point the modding book's links to repository files outside docs/modding/src at GitHub.

mdBook publishes only docs/modding/src, so a relative link that leaves it is broken on the published
site. Each such Markdown link or reference destination becomes a github.com URL on main (`blob/` for a
file, `tree/` for a folder). Run from anywhere: `python3 tools/refactors/book-edit/outside_links.py`.
"""
from pathlib import Path
import posixpath
import re

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "docs/modding/src"
GITHUB = "https://github.com/arismoko/horizon/"
LINK = re.compile(r"(\]\()([^\s)]+)(\))|^( {0,3}\[[^\]\n]+\]:[ \t]*<?)([^\s>]+)", re.MULTILINE)


def fix(url, page):
    if re.match(r"^[a-z]+:", url) or url.startswith(("#", "/")):
        return url
    path, _, anchor = url.partition("#")
    target = posixpath.normpath(posixpath.join(posixpath.dirname(page), path))
    if target.startswith("docs/modding/src/"):
        return url
    kind = "tree" if (ROOT / target).is_dir() else "blob"
    return f"{GITHUB}{kind}/main/{target}" + ("#" + anchor if anchor else "")


for path in sorted(SRC.rglob("*.md")):
    page = path.relative_to(ROOT).as_posix()
    text = path.read_text()
    new = LINK.sub(lambda m: m[1] + fix(m[2], page) + m[3] if m[1] else m[4] + fix(m[5], page), text)
    if new != text:
        path.write_text(new)
        print("rewrote", page)
