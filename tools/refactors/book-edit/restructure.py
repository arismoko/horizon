#!/usr/bin/env python3
"""Lay the modding book out as a manual: one page per topic, grouped by what a mod author does.

Reads `layout.tsv` beside this script (`old<TAB>new`, relative to docs/modding/src) and:

- removes the two aggregate pages (`concepts/trait-mods.md`, the old `how-to/native-mods.md`) and their
  `legacy-anchors.txt`; their fragments become ordinary pages;
- `git mv`s each page to its new path, so history follows it;
- gives each former fragment a `# Title` heading in place of its `<!-- chapter: Title -->` metadata,
  and shifts its headings so the page starts at level 1;
- creates the title-only group pages that `layout.tsv` implies and no page fills;
- rewrites every link to a moved page or to an anchor of an aggregate page, across the repository:
  relative Markdown links (catalog notes under crates/horizon-mod-vocab/src are relative to
  reference/hook-notes.md, where the book renders them), published `.html` URLs and plain
  `docs/modding/src/...` mentions;
- writes `SUMMARY.md` for the new layout.

Run from anywhere: `python3 tools/refactors/book-edit/restructure.py`.
"""
from pathlib import Path
import posixpath
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "docs/modding/src"
BOOK = "docs/modding/src"
SITE = "https://arismoko.github.io/horizon/"
NOTES = "crates/horizon-mod-vocab/src/"
TEXT = (".md", ".rs", ".toml", ".py", ".json", ".txt", ".yml", ".html")
AGGREGATES = {"concepts/trait-mods.md": "concepts/trait-mods", "how-to/native-mods.md": "how-to/native-mods"}
GROUPS = {
    "how-to/world.md": "Levels, dimensions and blocks",
    "how-to/items.md": "Items, recipes and loot",
    "how-to/entities.md": "Entities, players and commands",
    "how-to/client.md": "Client, rendering and sound",
}
CHAPTER = re.compile(r"\A<!-- chapter: (.+) -->\n(?:<!-- separator: \d+ -->\n)?\n*")
LINK = re.compile(r"(\]\()([^\s)]+)(\))|^( {0,3}\[[^\]\n]+\]:[ \t]*<?)([^\s>]+)", re.MULTILINE)

sys.path.insert(0, str(ROOT / "tools"))
from modding_concept_pages import headings  # noqa: E402

layout = {}
for line in (Path(__file__).parent / "layout.tsv").read_text().splitlines():
    if line.strip() and not line.startswith("#"):
        old, new = line.split("\t")
        layout[old] = new


SUMMARY = """# Summary

[Modding Horizon](introduction.md)

# Getting started

- [Installing and using mods](start/installing.md)
- [Your first native mod](start/first-mod.md)
- [Writing hooks](start/writing-hooks.md)

# Concepts

<!-- A page X.md with a folder X/ beside it gets the folder's pages as its sub-chapters, in file name order.
     A page directly in concepts/ or how-to/ that this file doesn't list joins the end of its part, by title. -->

- [Hooks and the Host](concepts/hooks-and-the-host.md)
- [Load phases and access](concepts/load-phases.md)
- [Registration and diagnostics](concepts/registration-and-diagnostics.md)
- [Parts](concepts/parts.md)
- [Capabilities](concepts/capabilities.md)
- [Attachments](concepts/attachments.md)
- [Transactions](concepts/transactions.md)
- [Registry keys](concepts/registry-keys.md)
- [Wire types](concepts/wire-types.md)
- [Dependencies between mods](concepts/dependencies.md)
- [Injections](concepts/injections.md)
- [The native route](concepts/native-route.md)
- [Testing a mod](concepts/testing.md)

# How-to

- [Content mods](how-to/content-mods.md)
- [Native mods: building and shipping](how-to/native-mods.md)
- [WebAssembly mods](how-to/wasm-mods.md)
- [Publishing your mod](how-to/publish.md)
- [Levels, dimensions and blocks](how-to/world.md)
- [Items, recipes and loot](how-to/items.md)
- [Entities, players and commands](how-to/entities.md)
- [Client, rendering and sound](how-to/client.md)

# Reference

- [The manifest and the package](reference/manifest.md)
- [The registry index and `.hpack`](reference/registry-index.md)
- [The mod API (rustdoc)](reference/api.md)
<!-- The hook-notes preprocessor supplies this chapter and its published URL from catalog notes. -->
- [Hook notes]()
<!-- The counterparts preprocessor renders the current declarations at the published URL. -->
- [NeoForge counterparts]()
- [Maths and network NBT](reference/maths-and-network-nbt.md)
- [Saved-world mod mismatches](reference/saved-world-mod-mismatch.md)
"""


def git(*args):
    subprocess.run(["git", *args], cwd=ROOT, check=True)


# (page, anchor) -> (new page, anchor); an aggregate's anchors belong to the fragment that held them.
anchors = {}
for aggregate, folder in AGGREGATES.items():
    for path in sorted((SRC / folder).glob("*.md")):
        old = path.relative_to(SRC).as_posix()
        if old not in layout:
            continue
        text = CHAPTER.sub("", path.read_text())
        found = list(headings(text))
        for i, (_, slug) in enumerate(found):
            target = (layout[old], "" if i == 0 else slug)
            anchors[aggregate, slug] = target
            anchors[old, slug] = target
    anchors[aggregate, ""] = ("concepts/parts.md", "") if aggregate.startswith("concepts") else (aggregate, "")


def relocate(old_page, anchor):
    """The new (page, anchor) for a link to `old_page#anchor`, both relative to docs/modding/src."""
    if (old_page, anchor) in anchors:
        return anchors[old_page, anchor]
    if old_page in AGGREGATES and anchor:
        return old_page, anchor  # an anchor no fragment holds: the build reports it
    page = layout.get(old_page, old_page)
    first = next((slug for _, slug in headings((SRC / page).read_text())), None) if (SRC / page).is_file() else None
    return page, "" if anchor == first else anchor


def normalise(text, title):
    """A fragment's text as a page: `# title`, then its own headings shifted below it."""
    text = CHAPTER.sub("", text)
    lines, fenced, levels = text.splitlines(), False, []
    for line in lines:
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        elif not fenced and re.match(r"^#{1,6} ", line):
            levels.append(len(line.split(" ", 1)[0]))
    first = next(iter(headings(text)), None)
    same = first is not None and first[0] == title and lines and lines[0].startswith("#")
    shift = (levels[0] - 1) if same else (min(levels) - 2 if levels else 0)
    out, fenced = [] if same else [f"# {title}", ""], False
    for i, line in enumerate(lines):
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        elif not fenced and re.match(r"^#{1,6} ", line):
            hashes, rest = line.split(" ", 1)
            level = max(len(hashes) - shift, 1 if (same and not out) else 2)
            line = "#" * level + " " + rest
        out.append(line)
    return "\n".join(out).rstrip("\n") + "\n"


def main():
    titles = {}
    for old in layout:
        match = CHAPTER.match((SRC / old).read_text())
        titles[old] = match[1] if match else None
    for aggregate, folder in AGGREGATES.items():
        git("rm", "-q", f"{BOOK}/{aggregate}", f"{BOOK}/{folder}/legacy-anchors.txt")
    for old, new in layout.items():
        (SRC / new).parent.mkdir(parents=True, exist_ok=True)
        git("mv", f"{BOOK}/{old}", f"{BOOK}/{new}")
        if titles[old]:
            (SRC / new).write_text(normalise((SRC / new).read_text(), titles[old]))
    for page, title in GROUPS.items():
        if not (SRC / page).exists():
            (SRC / page).write_text(f"# {title}\n")
            git("add", f"{BOOK}/{page}")
    (SRC / "SUMMARY.md").write_text(SUMMARY)
    back = {f"{BOOK}/{new}": f"{BOOK}/{old}" for old, new in layout.items()}

    def fix(url, source, old_source):
        if re.match(r"^[a-z]+:", url) or url.startswith(("#", "/")):
            return url
        path, _, anchor = url.partition("#")
        target = posixpath.normpath(posixpath.join(posixpath.dirname(old_source), path)) if path else old_source
        if not target.startswith(BOOK + "/"):
            if source == old_source:
                return url
            return posixpath.relpath(target, posixpath.dirname(source)) + ("#" + anchor if anchor else "")
        page, anchor = relocate(target.removeprefix(BOOK + "/"), anchor)
        dest = f"{BOOK}/{page}"
        if source.startswith(NOTES):
            source = f"{BOOK}/reference/hook-notes.md"
        relative = "" if dest == source and anchor else posixpath.relpath(dest, posixpath.dirname(source))
        return relative + ("#" + anchor if anchor else "")

    def published(match):
        page, anchor = relocate(match[1] + ".md", match[2] or "")
        return SITE + page.removesuffix(".md") + ".html" + ("#" + anchor if anchor else "")

    def mention(match):
        page, anchor = relocate(match[1], match[2] or "")
        return f"{BOOK}/{page}" + ("#" + anchor if anchor else "")

    files = subprocess.run(["git", "ls-files"], cwd=ROOT, check=True, capture_output=True,
                           text=True).stdout.splitlines()
    for name in files:
        if name == f"{BOOK}/SUMMARY.md":
            continue
        if not name.endswith(TEXT) or name.startswith("tools/refactors/") or not (ROOT / name).is_file():
            continue
        try:
            text = (ROOT / name).read_text()
        except UnicodeDecodeError:
            continue
        new = text
        if name.endswith((".md", ".txt")) or name.startswith(NOTES):
            old_source = back.get(name, name)
            if name.startswith(NOTES):
                old_source = f"{BOOK}/reference/hook-notes.md"
            new = LINK.sub(lambda m: m[1] + fix(m[2], name, old_source) + m[3] if m[1]
                           else m[4] + fix(m[5], name, old_source), new)
        new = re.sub(re.escape(SITE) + r"([\w/-]+)\.html(?:#([\w-]+))?", published, new)
        new = re.sub(re.escape(BOOK) + r"/([\w/.-]+\.md)(?:#([\w-]+))?", mention, new)
        if new != text:
            (ROOT / name).write_text(new)
            print("rewrote", name)


if __name__ == "__main__":
    main()
