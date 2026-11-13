#!/usr/bin/env python3
"""Move pages out of the modding book (docs/modding/src) to repository docs, and fix every link.

Reads `moves.tsv` beside this script: `old<TAB>new`, both relative to the repository root. An empty
`new` deletes the page. For each move it runs `git mv` (keeping history), then rewrites links in every
tracked text file:

- a relative Markdown link that resolves to a moved page points at its new path; from a page that
  stays in the book it becomes a GitHub URL, since mdBook publishes only `docs/modding/src`;
- a moved page's own relative links are re-based on its new folder; a link into the book stays a
  relative repository path, which GitHub renders;
- a moved concept fragment's `<!-- chapter: Title -->` becomes its `# Title` heading;
- `SUMMARY.md` drops the line of each moved page;
- a plain mention of the old path (`docs/modding/src/...` or the published `.html` URL) names the
  new path.

Run from anywhere: `python3 tools/refactors/book-edit/move_out.py`.
"""
from pathlib import Path
import posixpath
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
BOOK = "docs/modding/src"
SITE = "https://arismoko.github.io/horizon/"
BLOB = "https://github.com/arismoko/horizon/blob/main/"
TEXT = (".md", ".rs", ".toml", ".py", ".json", ".txt", ".yml", ".yaml", ".js", ".html")
LINK = re.compile(r"(\]\()([^\s)]+)(\))|^( {0,3}\[[^\]\n]+\]:[ \t]*<?)([^\s>]+)", re.MULTILINE)

moves = {}
for line in (Path(__file__).parent / "moves.tsv").read_text().splitlines():
    if line.strip() and not line.startswith("#"):
        old, _, new = line.partition("\t")
        moves[old.strip()] = new.strip()


def in_book(path):
    return path.startswith(BOOK + "/")


def target_of(source, url):
    """The repository path a relative link in `source` points at, and its anchor."""
    if re.match(r"^[a-z]+:", url) or url.startswith(("#", "/")):
        return None, None
    path, _, anchor = url.partition("#")
    return posixpath.normpath(posixpath.join(posixpath.dirname(source), path)), anchor


def link_to(source, dest, anchor):
    suffix = "#" + anchor if anchor else ""
    if in_book(source) and not in_book(dest):
        return BLOB + dest + suffix
    return posixpath.relpath(dest, posixpath.dirname(source)) + suffix


def rewrite_links(text, source, old_source):
    """Rewrite each relative link of `text`, written as if it lived at `old_source`."""
    def fix(url):
        target, anchor = target_of(old_source, url)
        if target is None:
            return url
        if target in moves:
            if not moves[target]:
                return url  # a deleted page: left for the hand edits, which the build reports
            return link_to(source, moves[target], anchor)
        if source != old_source:
            return link_to(source, target, anchor)
        return url

    def sub(m):
        if m[1]:
            return m[1] + fix(m[2]) + m[3]
        return m[4] + fix(m[5])
    return LINK.sub(sub, text)


def rewrite_mentions(text):
    for old, new in moves.items():
        if not new:
            continue
        text = text.replace(old, new)
        page = old.removeprefix(BOOK + "/").removesuffix(".md") + ".html"
        text = text.replace(SITE + page, BLOB + new)
    return text


def main():
    for old, new in moves.items():
        if new:
            (ROOT / new).parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "mv", old, new], cwd=ROOT, check=True)
        else:
            subprocess.run(["git", "rm", "-q", old], cwd=ROOT, check=True)
    back = {new: old for old, new in moves.items() if new}
    files = subprocess.run(["git", "ls-files"], cwd=ROOT, check=True, capture_output=True,
                           text=True).stdout.splitlines()
    for name in files:
        if not name.endswith(TEXT) or name.startswith("tools/refactors/"):
            continue
        path = ROOT / name
        if not path.is_file():
            continue
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        new = text
        if name == BOOK + "/SUMMARY.md":
            new = "".join(line for line in text.splitlines(True)
                          if not any(f"]({old.removeprefix(BOOK + '/')})" in line for old in moves))
        if name in back:
            # A concept fragment's title was mdBook metadata; outside the book it is the page's heading.
            new = re.sub(r"\A<!-- chapter: (.+) -->\n(?:<!-- separator: \d+ -->\n)?", r"# \1\n", new)
        if name.endswith((".md", ".txt")):
            new = rewrite_links(new, name, back.get(name, name))
        new = rewrite_mentions(new)
        if new != text:
            path.write_text(new)
            print("rewrote", name)


if __name__ == "__main__":
    main()
