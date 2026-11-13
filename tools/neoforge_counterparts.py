#!/usr/bin/env python3
"""mdBook preprocessor: render declared NeoForge counterparts without a source snapshot."""
import json
from pathlib import Path
import re
import sys

from modding_hook_notes import expand

PAGE = "reference/neoforge-counterparts.md"


def attribute_end(text, start):
    """The index just past the `]` closing the attribute at `start` (`#[...]`)."""
    i = start + 2
    depth = 1
    while i < len(text):
        c = text[i]
        if c == '"':
            i += 1
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                    continue
                if text[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if c == "r":
            j = i + 1
            while j < len(text) and text[j] == "#":
                j += 1
            if j < len(text) and text[j] == '"':
                close = '"' + "#" * (j - i - 1)
                k = text.find(close, j + 1)
                assert k != -1, "unterminated raw string"
                i = k + len(close)
                continue
        if c == "[":
            depth += 1
        elif c == "]":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise AssertionError(f"unterminated attribute at {start}")


def hooks(root):
    rows = []
    for path in sorted((root / "crates/horizon-mod-vocab/src/hooks").glob("*.rs")):
        if path.name == "mod.rs":
            continue
        text = path.read_text(encoding="utf8")
        i = 0
        while True:
            start = text.find("#[hook(", i)
            if start < 0:
                break
            end = attribute_end(text, start)
            attr = text[start + len("#[hook("):end - 2]
            i = end
            # A hook's #[notes] or /// line may stand between #[hook] and its fn.
            j = end
            while True:
                while j < len(text) and text[j].isspace():
                    j += 1
                if text.startswith("#[", j):
                    j = attribute_end(text, j)
                    continue
                if text.startswith("//", j):
                    k = text.find("\n", j)
                    j = len(text) if k < 0 else k + 1
                    continue
                if text.startswith("/*", j):
                    k = text.find("*/", j + 2)
                    assert k != -1, "unterminated block comment"
                    j = k + 2
                    continue
                break
            name = re.match(r"fn\s+(\w+)", text[j:])

            def key(k):
                v = re.search(k + r'\s*=\s*"([^"]*)"', attr)
                return v.group(1) if v else None

            neo = key("neoforge")
            if neo is None:
                lst = re.search(r"neoforge\s*=\s*\[(.*?)\]", attr, re.S)
                if lst:
                    neo = ", ".join(re.findall(r'"([^"]*)"', lst.group(1)))
            if name:
                rows.append((name.group(1), key("side") or "", neo))
    return sorted(rows)


def table(rows, head):
    out = [f"| Hook | Side | {head} |", "| --- | --- | --- |"]
    out += [f"| `{n}` | {s} | {c} |" for n, s, c in rows]
    return "\n".join(out)


def page(root):
    rows = hooks(root)
    mapped = [(n, s, f"`{c}`") for n, s, c in rows if c and not c.lower().startswith("none")]
    none = [
        (n, s, re.sub(r"^none\s*\(?(.*?)\)?$", r"\1", c, flags=re.S).strip() or "none")
        for n, s, c in rows
        if c and c.lower().startswith("none")
    ]
    return f"""# NeoForge counterparts

Each hook's `neoforge = ...` key in `crates/horizon-mod-vocab/src/hooks/` names the NeoForge 26.3 event or method it
matches, since a counterpart's name and contract are the rule for a hook. This page lists the keys as declared.
The book build renders it directly from those declarations; there is no generated Markdown snapshot to update.
A key names the event class (`PlayerEvent.Clone`) or runtime callback (`IPayloadHandler.handle`);
notes about cancelling or the event bus belong in the hook's `///` doc, not in the key.

{len(rows)} hooks: {len(mapped)} name a counterpart, {len(none)} say they have none.
This is a declaration reference, checked against NeoForge 26.3-26.3.0.102's public Javadoc. Registration events and the
callbacks they register are different moments: packet receipt names `IPayloadHandler.handle`, and per-draw item
decoration names `IItemDecorator.render`, not their registration events. A `none` explains why no event or method
matches that hook's runtime contract.

## Hooks with a counterpart

{table(mapped, "NeoForge")}

## Hooks that declare no counterpart

A hook with no NeoForge counterpart is judged on its merits like any other. The note is the key's own reason.

{table(none, "Reason")}
"""


def preprocess(context, book):
    root = Path(context["root"]).resolve().parents[1]
    return expand(book, page(root), "NeoForge counterparts", PAGE)


if __name__ == "__main__" and sys.argv[1:2] != ["supports"]:
    print(json.dumps(preprocess(*json.load(sys.stdin))))
