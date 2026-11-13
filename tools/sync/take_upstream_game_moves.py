#!/usr/bin/env python3
"""Take upstream's game.rs extractions: both halves of each move, one destination at a time.

Upstream split `game.rs` into `game/<feature>.rs` files (horizon-rust/horizon#11857). Where the fork had
edited a moved function, earlier syncs kept the fork's copy in `game.rs` and dropped upstream's new file.
This script restores upstream's structure mechanically from `game-moves.tsv`:

    upstream_path   fork_path   functions   items   fork_copies

`fork_copies` names upstream functions of the destination that the fork had already moved into files of
its own (`path#fn;path#fn`): they are the same move's other half and are deleted there too (a fork file
left with no items is deleted).

For one row (`--apply FORK_PATH`):
  * a new fork_path gets upstream's file verbatim (at --upstream); an existing fork file gets upstream's
    copies of `functions` appended in one `impl Game<'_>` block;
  * `functions` and `items` (structs, enums, consts, statics, type aliases, `impl Type` blocks) are
    deleted from the fork's game.rs with their doc comments and attributes. A name defined twice in
    game.rs is matched to upstream's body by similarity; impl blocks left empty are deleted.
The fork's edits to those functions are NOT carried by `--apply`. `--reapply FORK_PATH --fork-ref REF`
writes them in a separate commit: each moved function and item in the destination is replaced by the
fork's own text at REF (the commit before the move), keeping the fork's visibility where it had one and
upstream's otherwise (fields and methods of a moved struct or impl too), so that commit's diff is
exactly the fork's edits against upstream's copy. What is
left (helpers the fork renamed or deleted, imports) is corrected by hand in further commits. `--list` prints each row's
coverage and the upstream functions of a new destination that the fork already defines elsewhere
(duplicates the correction commit must settle). Upstream code is never moved ahead of upstream.

    python3 tools/sync/take_upstream_game_moves.py --upstream <sha> --list
    python3 tools/sync/take_upstream_game_moves.py --upstream <sha> --apply game/items/held_items.rs
    python3 tools/sync/take_upstream_game_moves.py --upstream <sha> --reapply game/items/held_items.rs --fork-ref HEAD~1
"""
import argparse
import csv
import difflib
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = "crates/horizon-client/src/"
GAME_RS = SRC + "game.rs"
MANIFEST = Path(__file__).with_name("game-moves.tsv")


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], check=True, capture_output=True, text=True).stdout


def code_mask(text):
    """`text` with string, char and comment contents blanked, so braces count only in code."""
    out, i, n = list(text), 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out[i:j] = " " * (j - i)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out[i:j] = [ch if ch == "\n" else " " for ch in text[i:j]]
            i = j
        elif c == "r" and re.match(r'r#*"', text[i:i + 20]) and not (i and (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = re.match(r'r(#*)"', text[i:]).group(1)
            end = text.find('"' + hashes, i + 2 + len(hashes))
            j = n if end < 0 else end + 1 + len(hashes)
            out[i:j] = [ch if ch == "\n" else " " for ch in text[i:j]]
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j += 1
            out[i:j] = [ch if ch == "\n" else " " for ch in text[i:j]]
            i = j
        elif c == "'":
            m = re.match(r"'(\\(x..|u\{[0-9a-fA-F]+\}|.)|[^\\'])'", text[i:i + 12])
            if m:
                out[i:i + m.end()] = " " * m.end()
                i += m.end()
            else:
                i += 1  # a lifetime or a label
        else:
            i += 1
    return "".join(out)


ITEM = re.compile(
    r"^(?P<indent>[ \t]*)(?:pub(?:\([^)]*\))?\s+)?(?:(?:const|async|unsafe|extern\s+\"C\")\s+)*"
    r"(?:(?P<kind>fn|struct|enum|const|static|type|trait|union|mod)\s+(?P<name>\w+)|(?P<impl>impl)\b(?P<head>[^{;]*))",
    re.M,
)


def items(text, depth_limit=1):
    """Items at brace depth 0 and 1: (kind, name, start, end, depth, parent_impl). `start` takes in the
    doc comments and attributes right above; `end` is past the closing brace or semicolon line."""
    mask = code_mask(text)
    depth_at, depth = [0] * (len(text) + 1), 0
    for i, ch in enumerate(mask):
        depth_at[i] = depth
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
    found = []
    for m in ITEM.finditer(mask):
        pos = m.start("kind") if m.group("kind") else m.start("impl")
        d = depth_at[pos]
        if d > depth_limit:
            continue
        # The item's end: matching brace of its body, or the `;` ending a body-less item.
        j, open_at, nest = pos, None, 0
        while j < len(mask):
            if mask[j] in "([":
                nest += 1
            elif mask[j] in ")]":
                nest -= 1
            elif mask[j] == "{" and nest == 0:
                open_at = j
                break
            elif mask[j] == ";" and nest == 0:
                break
            j += 1
        if open_at is None:
            end = j + 1
        else:
            k, level = open_at, 0
            while k < len(mask):
                if mask[k] == "{":
                    level += 1
                elif mask[k] == "}":
                    level -= 1
                    if level == 0:
                        break
                k += 1
            end = k + 1
            if text[end:end + 1] == ";":
                end += 1
        if text[end:end + 1] == "\n":
            end += 1
        start = text.rfind("\n", 0, m.start()) + 1
        # Doc comments, attributes (one or several lines) and plain comments directly above belong
        # to the item; a blank line or a module doc ends them.
        while start > 0:
            prev = text.rfind("\n", 0, start - 1) + 1
            line = text[prev:start].strip()
            if line.startswith("///") or line.startswith("#[") or (line.startswith("//") and not line.startswith("//!")):
                start = prev
                continue
            if line.endswith("]") and line:
                # The last line of a multi-line attribute: take it from its `#[`.
                top, ok = prev, False
                for _ in range(8):
                    if top == 0:
                        break
                    top = text.rfind("\n", 0, top - 1) + 1
                    above = text[top:start].split("\n")[0].strip()
                    if not above:
                        break
                    if above.startswith("#["):
                        ok = True
                        break
                if ok:
                    start = top
                    continue
            break
        if m.group("kind"):
            kind, name = m.group("kind"), m.group("name")
        else:
            head = re.sub(r"<[^<>]*(<[^<>]*>[^<>]*)*>", "", m.group("head")).strip()
            target = head.split(" for ")[-1].strip()
            kind, name = ("impl", re.sub(r"\W.*", "", target)) if " for " not in head else ("impl_for", head)
        found.append(dict(kind=kind, name=name, start=start, end=end, depth=d, body=text[start:end]))
    for it in found:
        it["parent"] = next(
            (p for p in found if p["kind"] in ("impl", "impl_for") and p["start"] <= it["start"] and it["end"] <= p["end"] and p is not it),
            None,
        )
    return found


def norm(body):
    return [l.strip() for l in body.split("\n") if l.strip() and not l.strip().startswith("//")]


def similarity(a, b):
    return difflib.SequenceMatcher(None, norm(a), norm(b), autojunk=False).ratio()


def rows():
    with open(MANIFEST, newline="") as f:
        return [r for r in csv.DictReader((l for l in f if not l.startswith("#")), delimiter="\t")]


def split(field):
    return [x for x in (field or "").split(",") if x]


def upstream_fn(up_items, name):
    return [it for it in up_items if it["kind"] == "fn" and it["name"] == name]


def plan_removals(game_text, up_text, functions, extra):
    """The game.rs spans to delete for one row, and the upstream fn texts by name."""
    game_items, up_items = items(game_text), items(up_text)
    spans, chosen = [], {}
    for name in functions:
        theirs = upstream_fn(up_items, name)
        if not theirs:
            raise SystemExit(f"{name}: not in upstream's file")
        ours = [it for it in game_items if it["kind"] == "fn" and it["name"] == name]
        if not ours:
            raise SystemExit(f"{name}: not in the fork's game.rs")
        best = max(ours, key=lambda it: max(similarity(it["body"], t["body"]) for t in theirs))
        spans.append((best["start"], best["end"]))
        chosen[name] = max(theirs, key=lambda t: similarity(best["body"], t["body"]))["body"]
    for name in extra:
        ours = [it for it in game_items if it["name"] == name and it["depth"] == 0 and it["kind"] not in ("fn", "impl_for")]
        if not ours:
            raise SystemExit(f"{name}: no such item at game.rs's top level")
        spans.extend((it["start"], it["end"]) for it in ours)
    return spans, chosen


def delete_spans(text, spans):
    for start, end in sorted(set(spans), reverse=True):
        text = text[:start] + text[end:]
        # No blank line pair left where the item was.
        while text[max(start - 2, 0):start] == "\n\n" and text[start:start + 1] == "\n":
            text = text[:start] + text[start + 1:]
    # impl blocks the deletions left empty.
    text = re.sub(r"\n[ \t]*impl(?:<[^>]*>)? [^{\n]+\{\s*\}[ \t]*\n", "\n", text)
    # Test modules the deletions left with only their imports.
    text = re.sub(r"\n(?:[ \t]*#\[cfg\(test\)\]\n)?[ \t]*mod \w+ \{\s*(?:use [^;{]*(?:\{[^}]*\})?[^;]*;\s*)*\}[ \t]*\n", "\n", text)
    text = text.rstrip("\n") + "\n"
    text = re.sub(r"\n\n+([ \t]*\}\n)", r"\n\1", text)
    text = re.sub(r"(\{\n)\n+", r"\1", text)
    return re.sub(r"\n\n\n+(?=[ \t]*(?:#|//|(?:impl|fn|pub|struct|enum|const|static|type|mod|use)\b))", "\n\n", text) if "\n\n\n" in text else text


def fork_copies(row):
    for entry in split((row.get("fork_copies") or "").replace(";", ",")):
        path, name = entry.split("#")
        yield path, name


def is_empty_module(text):
    return not [it for it in items(text) if it["depth"] == 0]


def apply(row, upstream):
    fork_path, up_path = row["fork_path"], row["upstream_path"]
    up_text = git("show", f"{upstream}:{SRC}{up_path}")
    game_file = ROOT / GAME_RS
    game_text = game_file.read_text()
    spans, chosen = plan_removals(game_text, up_text, split(row["functions"]), split(row["items"]))
    dest = ROOT / SRC / fork_path
    if dest.exists():
        block = "\nimpl Game<'_> {\n" + "".join(
            chosen[name] if chosen[name].endswith("\n") else chosen[name] + "\n" for name in split(row["functions"])
        ) + "}\n"
        dest.write_text(dest.read_text().rstrip("\n") + "\n" + block)
    else:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(up_text)
    game_file.write_text(delete_spans(game_text, spans))
    by_file = {}
    for path, name in fork_copies(row):
        by_file.setdefault(path, []).append(name)
    up_items = items(up_text, depth_limit=2)
    for path, names in by_file.items():
        file = ROOT / SRC / path
        text = file.read_text()
        own = items(text, depth_limit=2)
        cut = []
        for name in names:
            theirs = [t["body"] for t in upstream_fn(up_items, name)]
            ours = [it for it in own if it["kind"] == "fn" and it["name"] == name]
            if not ours or not theirs:
                raise SystemExit(f"{path}#{name}: not found")
            best = max(ours, key=lambda it: max(similarity(it["body"], t) for t in theirs))
            cut.append((best["start"], best["end"]))
        text = delete_spans(text, cut)
        if is_empty_module(text):
            subprocess.run(["git", "-C", str(ROOT), "rm", "-q", str(file)], check=True)
        else:
            file.write_text(text)


def leading_comments(text):
    lines = text.split("\n")
    n = 0
    while n < len(lines) and lines[n].strip().startswith("//"):
        n += 1
    return lines[:n], "\n".join(lines[n:])


def with_visibility(fork_text, upstream_text):
    """`fork_text` with upstream's visibility when the fork's item had none, and upstream's doc
    comment when the fork's had none (the fork dropped none on purpose: they were lost in a sync)."""
    fork_docs, rest = leading_comments(fork_text)
    up_docs, _ = leading_comments(upstream_text)
    if not fork_docs and up_docs:
        fork_text = "\n".join(up_docs + [rest])
    signature = re.compile(r"^([ \t]*)(pub(?:\([^)]*\))?\s+)?((?:(?:const|async|unsafe)\s+)*(?:fn|struct|enum|const|static|type|trait)\s)", re.M)
    mine, theirs = signature.search(fork_text), signature.search(upstream_text)
    if not mine or mine.group(2) or not theirs or not theirs.group(2):
        return fork_text
    return fork_text[: mine.start(3)] + theirs.group(2) + fork_text[mine.start(3):]


MEMBER = re.compile(r"^([ \t]+)(pub(?:\([^)]*\))?\s+)?((?:(?:const|async|unsafe)\s+)*fn\s+(\w+)|(\w+)\s*:(?!:))", re.M)


def members(text, kind):
    """Matches of `MEMBER` directly inside the item's braces: a struct's fields, an impl's
    methods (an enum's variant fields and anything nested are not members)."""
    mask = code_mask(text)
    depth, depth_at = 0, []
    for ch in mask:
        depth_at.append(depth)
        depth += (ch == "{") - (ch == "}")
    for m in MEMBER.finditer(mask):
        if depth_at[m.start(3)] != 1 or not m.group(1):
            continue
        if (kind == "struct") != bool(m.group(5)):
            continue
        yield m


def with_member_visibility(fork_text, upstream_text, kind):
    """Fields of a moved struct and methods of a moved impl keep upstream's visibility where the
    fork's had none, and the fork's own get `pub(super)`: a private member of an item in game.rs
    was visible to all of `game`, one in a child module is not."""
    if kind not in ("struct", "impl"):
        return fork_text
    wanted = {(m.group(4) or m.group(5)): m.group(2) for m in members(upstream_text, kind) if m.group(2)}
    for m in sorted(members(fork_text, kind), key=lambda m: -m.start()):
        if not m.group(2):
            name = m.group(4) or m.group(5)
            fork_text = fork_text[: m.start(3)] + wanted.get(name, "pub(super) ") + fork_text[m.start(3):]
    return fork_text


def reapply(row, upstream, fork_ref):
    dest = ROOT / SRC / row["fork_path"]
    text = dest.read_text()
    fork_game = git("show", f"{fork_ref}:{GAME_RS}")
    sources = [(fork_game, name) for name in split(row["functions"])]
    sources += [(git("show", f"{fork_ref}:{SRC}{path}"), name) for path, name in fork_copies(row)]
    replace = []
    taken = set()
    dest_items = items(text, depth_limit=2)
    for source, name in sources:
        theirs = [it for it in dest_items if it["kind"] == "fn" and it["name"] == name and it["start"] not in taken]
        ours = [it for it in items(source, depth_limit=2) if it["kind"] == "fn" and it["name"] == name]
        best_pair = max(((similarity(o["body"], t["body"]), o, t) for o in ours for t in theirs), key=lambda x: x[0])
        _, o, t = best_pair
        taken.add(t["start"])
        replace.append((t["start"], t["end"], with_visibility(o["body"], t["body"])))
    fork_top = [it for it in items(fork_game) if it["depth"] == 0]
    for name in split(row["items"]):
        theirs = [it for it in dest_items if it["name"] == name and it["depth"] == 0 and it["kind"] not in ("fn", "impl_for")]
        ours = [it for it in fork_top if it["name"] == name and it["kind"] not in ("fn", "impl_for")]
        for kind in ("struct", "enum", "const", "static", "type", "trait", "impl"):
            t_k = [it for it in theirs if it["kind"] == kind]
            o_k = [it for it in ours if it["kind"] == kind]
            for t, o in zip(t_k, o_k):
                body = with_member_visibility(with_visibility(o["body"], t["body"]), t["body"], kind)
                replace.append((t["start"], t["end"], body))
            if len(t_k) != len(o_k):
                print(f"{name}: {len(o_k)} fork {kind} item(s) against upstream's {len(t_k)}: settle by hand")
    for start, end, body in sorted(replace, reverse=True):
        text = text[:start] + (body if body.endswith("\n") else body + "\n") + text[end:]
    dest.write_text(text)


def list_rows(upstream):
    game_text = (ROOT / GAME_RS).read_text()
    game_names = {it["name"] for it in items(game_text) if it["kind"] == "fn"}
    tree = git("grep", "-n", "-E", r"^\s*(pub(\([^)]*\))? )?(const |async |unsafe )*fn \w+", "--", SRC + "game")
    elsewhere = {}
    for line in tree.splitlines():
        path, _, rest = line.split(":", 2)
        m = re.search(r"fn (\w+)", rest)
        elsewhere.setdefault(m.group(1), set()).add(path[len(SRC):])
    for row in rows():
        fns = split(row["functions"])
        left = [f for f in fns if f in game_names]
        print(f"{row['fork_path']} <- {row['upstream_path']}: {len(left)}/{len(fns)} still in game.rs")
        if (ROOT / SRC / row["fork_path"]).exists():
            continue
        up_items = items(git("show", f"{upstream}:{SRC}{row['upstream_path']}"), depth_limit=2)
        for it in up_items:
            if it["kind"] == "fn" and it["name"] not in fns and it["name"] in elsewhere:
                print(f"    duplicate: {it['name']} also in {', '.join(sorted(elsewhere[it['name']]))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--upstream", required=True)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--list", action="store_true")
    group.add_argument("--apply", metavar="FORK_PATH")
    group.add_argument("--reapply", metavar="FORK_PATH")
    parser.add_argument("--fork-ref", help="with --reapply: the commit before the move")
    args = parser.parse_args()
    if args.list:
        list_rows(args.upstream)
        return
    path = args.apply or args.reapply
    matching = [r for r in rows() if r["fork_path"] == path]
    if len(matching) != 1:
        raise SystemExit(f"{path}: not one manifest row")
    if args.apply:
        apply(matching[0], args.upstream)
    else:
        reapply(matching[0], args.upstream, args.fork_ref or "HEAD~1")


if __name__ == "__main__":
    main()
