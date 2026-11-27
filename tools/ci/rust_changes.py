#!/usr/bin/env python3
"""Select CI checks from one fail-closed Git diff.

The default answers Rust selection.
--all emits the shared Rust and tooling decisions for dyna-checks (the machine sample is a daily leg,
machine-sample.yml, so dyna-checks selects nothing for it).
server_build: whether verify's clippy of the server build has anything new to check (needs_server_build).
"""

import argparse
import functools
import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(ROOT))
from tools.ci.pr_files import repo_path  # noqa: E402

# The root workspace's manifest and toolchain files.
ROOT_RUST = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}

# The Python under tools/ and tests/: verify's Python steps and the parity jobs run it, and no Rust build or test reads
# it (Rust does read tools/native-hook-fragments/*.json and tools/ci/*.sh, so those still count). The scripts a step
# gated on this selection runs are its inputs all the same (gated_by). This script and the module it imports decide
# whether Rust runs at all, so a change to them runs it.
RUST_DECIDES = {"tools/ci/rust_changes.py", "tools/ci/pr_files.py", "tools/release_train.py"}
# Cargo's own inputs, wherever they are: a manifest, lock file, build script, toolchain or lint setting, or a Rust
# source. Deleted or renamed ones count too, so a package under docs/ that loses its manifest still runs Rust.
CARGO_NAMES = {"Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain", "rust-toolchain.toml", "rustfmt.toml",
               ".rustfmt.toml", "clippy.toml", ".clippy.toml"}


def cargo_input(path):
    return posixpath.basename(path) in CARGO_NAMES or path.endswith(".rs") or path.startswith(".cargo/")


def rust_free_by_name(path):
    return not cargo_input(path) and (
        path.startswith("docs/") or path.endswith(".md")
        or path.startswith(("tools/", "tests/")) and path.endswith(".py") and path not in RUST_DECIDES)


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                          check=False)


@functools.lru_cache(maxsize=None)
def rust_literals():
    """(crate folders, {last segment: [(source, literal)]}) for every one-line string literal with no brace or
    backslash in this checkout's tracked Rust sources; None if they can't be read."""
    manifests = git("ls-files", "-z", "--", "Cargo.toml", "*/Cargo.toml")
    literals = git("grep", "-I", "-z", "-o", "-E", r'"[^"{}\\]+"', "--", "*.rs")
    if manifests.returncode != 0 or literals.returncode not in (0, 1):
        return None
    by_name = {}
    for line in literals.stdout.decode("utf-8", "replace").splitlines():
        source, _, literal = line.partition("\0")
        literal = literal[1:-1]
        by_name.setdefault(literal.rstrip("/").rsplit("/", 1)[-1], []).append((source, literal))
    return {posixpath.dirname(path) for path in manifests.stdout.decode("utf-8").split("\0") if path}, by_name


# Rust's tokens, coarsely: comments, character literals and whitespace are dropped, strings are unescaped.
TOKEN = re.compile(r"""
    (?P<space>\s+) | (?P<line>//[^\n]*) | (?P<block>/\*.*?\*/)
  | (?P<raw>(?:b|c)?r(?P<hashes>\#*)"(?P<rawbody>.*?)"(?P=hashes))
  | (?P<str>(?:b|c)?"(?P<body>(?:[^"\\]|\\.)*)")
  | (?P<char>'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'])')
  | (?P<ident>\w+) | (?P<punct>.)
""", re.S | re.X)
ESCAPE = re.compile(r"\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|\n\s*|.)")
INCLUDES = {"include", "include_str", "include_bytes"}


def unescape(body):
    def one(match):
        escape = match.group(0)
        if escape.startswith("\\u{"):
            return chr(int(escape[3:-1], 16))
        if escape.startswith("\\x"):
            return chr(int(escape[2:], 16))
        if escape.startswith("\\\n"):
            return ""
        return {"\\n": "\n", "\\t": "\t", "\\r": "\r", "\\0": "\0"}.get(escape, escape[1:])
    return ESCAPE.sub(one, body)


def tokens(text):
    for match in TOKEN.finditer(text):
        kind = match.lastgroup
        if kind == "raw":
            yield ("str", match.group("rawbody"))
        elif kind == "str":
            yield ("str", unescape(match.group("body")))
        elif kind in ("ident", "punct"):
            yield (kind, match.group(0))


def include_arguments(text):
    """The argument tokens of each include!, include_str!, include_bytes! and #[path = ...] in Rust source `text`;
    the same words inside strings and comments are not includes."""
    found = list(tokens(text))
    for at, token in enumerate(found):
        if token[0] == "ident" and token[1] in INCLUDES and found[at + 1:at + 2] == [("punct", "!")]:
            depth, group = 0, []
            for inner in found[at + 2:]:
                if inner[0] == "punct" and inner[1] in "([{":
                    depth += 1
                elif inner[0] == "punct" and inner[1] in ")]}":
                    depth -= 1
                group.append(inner)
                if depth <= 0:
                    break
            yield group[1:-1]
        elif token == ("ident", "path") and found[at - 2:at] == [("punct", "#"), ("punct", "[")] \
                and found[at + 1:at + 2] == [("punct", "=")]:
            argument = []
            for inner in found[at + 2:]:
                if inner == ("punct", "]"):
                    break
                argument.append(inner)
            yield argument


def resolve_include(source, crate, argument):
    """("exact", paths) for an include whose argument is a literal, or a concat! of literals after an optional
    env!("CARGO_MANIFEST_DIR") (env!("OUT_DIR") names a file the build makes: no repository path); else
    ("pattern", regex) matching any possible name based on the literal suffix."""
    folder = posixpath.dirname(source)
    if len(argument) == 1 and argument[0][0] == "str":
        literal = argument[0][1]
        if literal.startswith("/"):
            return "exact", set()
        # #[path] inside an inline module of a non-mod.rs file resolves below the file's own stem as well.
        stem = posixpath.splitext(source)[0]
        return "exact", {posixpath.normpath(posixpath.join(base, literal)) for base in (folder, stem)}
    concat = argument[:3] == [("ident", "concat"), ("punct", "!"), ("punct", "(")]
    pieces = argument[3:-1] if concat else argument
    parts, known, base = [], True, folder
    at = 0
    while at < len(pieces):
        if pieces[at:at + 3] == [("ident", "env"), ("punct", "!"), ("punct", "(")] and at + 4 < len(pieces) \
                and pieces[at + 3][0] == "str" and pieces[at + 4] == ("punct", ")"):
            name = pieces[at + 3][1]
            if name == "OUT_DIR" and at == 0:
                return "exact", set()
            if name == "CARGO_MANIFEST_DIR" and not parts and crate is not None:
                base = crate + "/"
            else:
                known = False
                parts.append(None)
            at += 5
        else:
            if pieces[at][0] == "str":
                parts.append(pieces[at][1])
            elif pieces[at] != ("punct", ","):
                known = False
                parts.append(None)
            at += 1
    # A literal piece after the crate's folder starts with "/": it continues that folder's path.
    def under(path):
        return path[1:] if base != folder and path.startswith("/") else path

    if known and parts:
        literal = "".join(parts)
        if literal.startswith("/") and base == folder:
            return "exact", set()
        return "exact", {posixpath.normpath(posixpath.join(base, under(literal)))}
    # A dynamic piece can contain `../` and escape the apparent prefix. Only a literal suffix
    # within the final path component can rule out a changed filename. Otherwise match any path.
    tail = []
    for part in reversed(parts):
        if part is None:
            break
        tail.append(part)
    suffix = "".join(reversed(tail))
    if not suffix or suffix.endswith(("/", "/.", "/..")) or suffix in {".", ".."}:
        return "pattern", re.compile("")
    return "pattern", re.compile(re.escape(suffix.rsplit("/", 1)[-1]) + r"\Z")


@functools.lru_cache(maxsize=None)
def rust_includes():
    """(the files this checkout's Rust includes by name, [(source, regex)] for the includes whose path is assembled
    from something other than literals); None if the sources can't be read."""
    scanned = rust_literals()
    # An attribute's `#[` and `path` can be on different lines: also list a line ending in `#[` or holding `path =`
    # and a quote or the end of the line. (An anchored `^ *path` pattern costs git grep half a second.)
    pattern = r'\binclude(_str|_bytes)?\b|#\[[[:space:]]*path|\bpath[[:space:]]*=[[:space:]]*("|$)|#\[[[:space:]]*$'
    listed = git("grep", "-l", "-z", "-E", pattern, "--", "*.rs")
    if scanned is None or listed.returncode not in (0, 1):
        return None
    manifests = scanned[0]
    exact, patterns = set(), []
    for source in filter(None, listed.stdout.decode("utf-8").split("\0")):
        try:
            text = (ROOT / source).read_text(encoding="utf-8", errors="replace")
        except OSError:
            return None
        crate = posixpath.dirname(source)
        while crate and crate not in manifests:
            crate = posixpath.dirname(crate)
        for argument in include_arguments(text):
            # `#[path = #path]` inside quote! is code a macro or build script writes from its own reads.
            if ("punct", "#") in argument:
                continue
            kind, found = resolve_include(source, crate if crate in manifests else None, argument)
            if kind == "exact":
                exact |= found
            else:
                patterns.append((source, found))
    return exact, patterns


# The selection outputs a workflow step or job can be gated on, by the decision behind them.
GATES = {"rust": ("rust_required", "steps.rust.outputs.", "outputs.server_build")}
# A repository file a step's commands name that the Rust-free rule could miss: Python under tools/ or tests/, or
# anything under docs/ (a folder when it ends in "/" or is one).
HELPER = re.compile(r"(?<![\w./-])((?:tools|tests)/[\w./-]+\.py|docs/[\w./-]+)")
# The same, in a string literal of a helper's code: a file or folder it reads or runs.
QUOTED = re.compile(r"""["']((?:tools|tests)/[\w./-]+\.py|docs/[\w./-]+)""")
IMPORT = re.compile(r"^[ \t]*(?:from[ \t]+([\w.]+)[ \t]+import[ \t]+(\([^)]*\)|[\w \t,]+)|import[ \t]+([\w. \t,]+))",
                    re.M)


def helper_path(found):
    """A path a helper names, without a sentence's full stop; None for the whole of docs/."""
    path = found.rstrip(".")
    return None if path.rstrip("/") == "docs" else path


def condition(lines):
    """The `if:` of the block `lines` (a job's header or one step), its folded continuation lines included."""
    for index, line in enumerate(lines):
        match = re.match(r"(\s*)(?:- )?if:\s*(.*)$", line)
        if match:
            # The key's own column: a list item's `- ` moves it two to the right.
            column = len(match.group(1)) + (2 if line.lstrip().startswith("- ") else 0)
            text = [match.group(2)]
            for more in lines[index + 1:]:
                if more.strip() and len(more) - len(more.lstrip()) <= column:
                    break
                text.append(more.strip())
            return " ".join(text)
    return ""


def gated_blocks(text):
    """(condition, commands) for each step of a workflow: the step's own `if:` and its job's."""
    lines = text.splitlines()
    try:
        start = next(index for index, line in enumerate(lines) if line.rstrip() == "jobs:")
    except StopIteration:
        return
    jobs = [index for index in range(start + 1, len(lines)) if re.match(r"  [\w-]+:\s*$", lines[index])]
    for at, job in enumerate(jobs):
        body = lines[job:jobs[at + 1] if at + 1 < len(jobs) else len(lines)]
        steps = next((index for index, line in enumerate(body) if re.match(r"\s+steps:\s*$", line)), len(body))
        header = condition(body[1:steps])
        items = [index for index in range(steps + 1, len(body)) if re.match(r"\s+- ", body[index])]
        indent = len(body[items[0]]) - len(body[items[0]].lstrip()) if items else 0
        items = [index for index in items if len(body[index]) - len(body[index].lstrip()) == indent]
        for item, begin in enumerate(items):
            step = body[begin:items[item + 1] if item + 1 < len(items) else len(body)]
            yield header + " " + condition(step), "\n".join(step)


def helper_closure(paths):
    """`paths` and what they bring in: for Python, the modules it imports and the files its string literals name,
    followed through the Python among them."""
    seen, queue = set(), list(paths)
    while queue:
        path = queue.pop()
        if path in seen:
            continue
        seen.add(path)
        if not path.endswith(".py"):
            continue
        try:
            text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        # The selector's own tables name paths it decides on, not files it reads; a change to it runs Rust anyway.
        if path not in RUST_DECIDES:
            queue += filter(None, map(helper_path, QUOTED.findall(text)))
        folder = posixpath.dirname(path)
        for package, names, modules in IMPORT.findall(text):
            def named(listed):
                return [name.split(" as ")[0].strip() for name in listed.strip("()").split(",")]

            candidates = named(modules) if modules else [package] + [f"{package}.{name}" for name in named(names)]
            for module in filter(None, candidates):
                relative = module.replace(".", "/")
                for candidate in (f"{relative}.py", f"{relative}/__init__.py", f"{folder}/{relative}.py"):
                    if (ROOT / candidate).is_file():
                        queue.append(posixpath.normpath(candidate))
    return seen


@functools.lru_cache(maxsize=None)
def gated_helpers():
    """{decision: the files and folders a step gated on it runs or reads}, from this checkout's workflows and what
    those files import or name; None if the workflows can't be read."""
    folder = ROOT / ".github" / "workflows"
    try:
        texts = [path.read_text(encoding="utf-8") for path in sorted(folder.iterdir())
                 if path.suffix in (".yml", ".yaml")]
    except OSError:
        return None
    named = {decision: set() for decision in GATES}
    for text in texts:
        for gate, body in gated_blocks(text):
            for decision, outputs in GATES.items():
                if any(output in gate for output in outputs):
                    named[decision] |= set(filter(None, map(helper_path, HELPER.findall(body))))
    return {decision: helper_closure(paths) for decision, paths in named.items()}


def gated_by(decision, paths):
    """Whether a step gated on `decision` runs a file among `paths`; with no readable workflows, any Python tool."""
    helpers = gated_helpers()
    if helpers is None:
        return any(path.startswith(("tools/", "tests/")) and path.endswith(".py") for path in paths)
    return any(path == helper or path.startswith(helper.rstrip("/") + "/") and (
        helper.endswith("/") or (ROOT / helper).is_dir()) for helper in helpers[decision] for path in paths)


def rust_reads(paths):
    """The paths among `paths` (each Rust-free by name) that Rust builds or tests read anyway, found in this checkout's
    Rust sources without a build:
    - a string literal names the file, as include_str!/include_bytes!/include!/#[path] and the tests' reads do: a bare
      name from the source's folder, a path with a folder from the source's folder or any folder above it (so
      concat!(env!("CARGO_MANIFEST_DIR"), "/..."), Path::new(env!("CARGO_MANIFEST_DIR")).join("...") and paths from
      the repository root resolve);
    - or a literal with a folder names a folder holding the file, below the top level and not a crate's own folder
      (that crate's sources count already);
    - or an include!, include_str!, include_bytes! or #[path] names it, also in a literal with spaces or escapes or
      a concat! of literals; one whose path is assembled from anything else (a macro's argument, a constant) counts
      for every file its literal pieces could name, in order, and for every file when it has none;
    - or the file belongs to a Cargo package under docs/ (the book's examples), other than its *.md.
    A deleted file still counts when a literal names it. If the sources can't be read, every path counts."""
    paths = set(paths)
    scanned = rust_literals()
    included = rust_includes()
    if scanned is None or included is None:
        return paths
    manifests, by_name = scanned
    exact, patterns = included
    folders = {}
    for path in paths:
        parts = path.split("/")
        for end in range(2, len(parts)):
            folders.setdefault("/".join(parts[:end]), set()).add(path)
    read = set()
    # Only a literal ending in a changed file's name, or in the name of a folder holding one, can name it.
    for name in {part for path in paths for part in path.split("/")} - {".", ".."}:
        for source, literal in by_name.get(name, ()):
            bases = [posixpath.dirname(source)]
            while "/" in literal and bases[-1]:
                bases.append(posixpath.dirname(bases[-1]))
            for base in bases:
                target = posixpath.normpath(posixpath.join(base, literal.lstrip("/")))
                if target in paths:
                    read.add(target)
                elif "/" in literal and target in folders and target not in manifests:
                    read |= folders[target]
    read |= paths & exact
    read |= {path for path in paths for _, pattern in patterns if pattern.search(path)}
    packages = tuple(folder + "/" for folder in manifests if folder.startswith("docs/"))
    return read | {path for path in paths if path.startswith(packages) and not path.endswith(".md")}


def needs_rust(paths):
    # Docs, *.md and the Python tools are Rust-free by name, unless Rust embeds or tests them (rust_reads) or a step
    # gated on this decision runs them (gated_by).
    paths = list(paths)
    if any(not rust_free_by_name(path) for path in paths):
        return True
    return bool(paths) and (gated_by("rust", paths) or bool(rust_reads(paths)))


# verify's second clippy, the server build (sunrise-client without default features): what it checks differently from
# the first is the feature-gated code of the client crates and the features the manifests choose. Pull requests that
# change neither skip it; main's pushes and the merge queue always run it.
SERVER_BUILD_CHECK = {".github/workflows/ci.yml", "tools/ci/rust_changes.py"}


def needs_server_build(paths):
    return any(path.startswith(("crates/sunrise-client", "crates/sunrise-gameplay",
                                "crates/sunrise-game/", "crates/sunrise-game-data/"))
               or path.endswith("Cargo.toml") or path in ROOT_RUST
               or path.startswith(".cargo/") or path in SERVER_BUILD_CHECK for path in paths)


# Stands for "everything changed" when no diff can be computed: a root manifest every check and shard selector
# treats as touching all of them.
UNKNOWN_CHANGE = "Cargo.lock"


def needs_sample(paths):
    return any(path in ROOT_RUST or path == "tools/ci/check_machine_sample.py"
               or path.startswith(("crates/sunrise-mod-", "crates/sunrise-core/",
                                   "crates/sunrise-game-discovery/")) for path in paths)


# Python whose path literals are examples of what selection decides, not files it reads: the selectors and their tests.
PATH_EXAMPLES = RUST_DECIDES | {"tests/test_ci_rust_changes.py", "tests/test_ci_pr_files.py",
                                "tests/test_ci_test_scope.py"}
# A Python string literal on one line, or a chain of them joined with pathlib's `/`: `ROOT / "docs" / "status.md"`.
PY_PATH = r"""("[^"\\]+"|'[^'\\]+')([[:space:]]*/[[:space:]]*("[^"\\]+"|'[^'\\]+'))*"""


@functools.lru_cache(maxsize=None)
def python_literals():
    """[(source, path)] for each one-line string literal, or `/`-joined chain of them, in this checkout's tracked
    Python; None if it can't be read."""
    found = git("grep", "-I", "-z", "-o", "-E", PY_PATH, "--", "*.py")
    if found.returncode not in (0, 1):
        return None
    out = []
    for line in found.stdout.decode("utf-8", "replace").splitlines():
        source, _, chain = line.partition("\0")
        if source in PATH_EXAMPLES:
            continue
        pieces = re.findall(r"""["']([^"']+)["']""", chain)
        out += [(source, piece) for piece in pieces]
        if len(pieces) > 1:
            out.append((source, "/".join(pieces)))
    return out


def glob_pattern(glob):
    """A compiled glob with pathlib's rules: `*` and `?` stay within a folder, `**` crosses folders."""
    regex = ""
    for token in re.findall(r"\*\*/|\*\*|\*|\?|[^*?]+", glob):
        regex += {"**/": "(?:.*/)?", "**": ".*", "*": "[^/]*", "?": "[^/]"}.get(token) or re.escape(token)
    return re.compile(regex + r"\Z")


def python_reads(paths):
    """The paths among `paths` that the repository's Python (the tools, their tests, the Python suite) names: a
    literal or a `/`-joined chain of literals naming the file, a folder holding it below the top level, or a glob
    matching it. A literal with a folder resolves from the source's folder or any folder above it, a bare name only
    from the source's folder. If the sources can't be read, every path counts."""
    paths = set(paths)
    literals = python_literals()
    if literals is None:
        return paths
    names = {part for path in paths for part in path.split("/")}
    read = set()
    for source, literal in literals:
        if literal.rstrip("/").rsplit("/", 1)[-1] not in names and "*" not in literal:
            continue
        bases = [posixpath.dirname(source)]
        while "/" in literal.strip("/") and bases[-1]:
            bases.append(posixpath.dirname(bases[-1]))
        for base in bases:
            target = posixpath.normpath(posixpath.join(base, literal.lstrip("/")))
            if "*" in target or "?" in target:
                pattern = glob_pattern(target)
                read |= {path for path in paths if pattern.match(path)}
            elif target in paths:
                read.add(target)
            elif "/" in target:
                read |= {path for path in paths if path.startswith(target + "/")}
    return read


def needs_tooling(changes):
    paths = [path for _, path in changes]
    return needs_rust(paths) or any(
        path.startswith(("tools/", "tests/", ".github/", "docs/modding/", "roadmap/"))
        or path in {"README.md", "ROADMAP.md", "docs/README.md"}
        # Tests check linked target existence, even when they do not read its contents.
        or status != "M"
        for status, path in changes
    ) or bool(python_reads(paths))


def selections(changes):
    paths = [path for _, path in changes]
    return {"rust_required": needs_rust(paths),
            "tooling_required": needs_tooling(changes)}


def framed_entries(data):
    """Complete nonempty `git diff --name-status --no-renames -z` records, or None."""
    fields = data.split(b"\0")
    if len(fields) < 3 or fields[-1] or len(fields) % 2 != 1:
        return None
    try:
        rows = [(status.decode("ascii"), path.decode("utf-8")) for status, path in zip(fields[:-1:2], fields[1::2])]
    except UnicodeError:
        return None
    if not all(status in {"A", "D", "M", "T"} and repo_path(path) for status, path in rows):
        return None
    return rows if len({path for _, path in rows}) == len(rows) else None


def changed_entries(base, head):
    # Git reads an empty endpoint as the checkout's HEAD, which could make the diff empty or another one.
    if not base or not head:
        print("rust_changes: missing diff endpoint; treating everything as changed", file=sys.stderr)
        return [("?", UNKNOWN_CHANGE)]
    # Disable rename folding: retain both deleted and added paths, including source moved into docs.
    command = ["git", "diff", "--name-status", "--no-renames", "-z", f"{base}...{head}", "--"]
    for fetch_base in (False, True):
        # The event's base can be newer than the checked-out history (main moved on): fetch it once.
        if fetch_base:
            subprocess.run(["git", "fetch", "-q", "--no-tags", "origin", base], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        diff = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
        if diff.returncode == 0:
            rows = framed_entries(diff.stdout)
            if rows is not None:
                return rows
            break
    # Never an error and never "nothing changed": assume everything did.
    print(f"rust_changes: no diff for {base}...{head}; treating everything as changed", file=sys.stderr)
    return [("?", UNKNOWN_CHANGE)]


def listed_entries(path, count):
    """The (status, path) pairs of a `git diff --name-status -z` listing saved in a file (tools/ci/pr_files.py),
    complete only with the `count` of records it printed, so a listing that lost a whole record is refused."""
    try:
        rows = framed_entries(Path(path).read_bytes())
    except OSError:
        return None
    return rows if rows is not None and len(rows) == count else None


def pr_entries(base, head, listing=None, count=None):
    if not all(re.fullmatch(r"[0-9a-f]{40}", sha) for sha in (base, head)):
        print("rust_changes: invalid event commits; treating everything as changed", file=sys.stderr)
        return [("?", UNKNOWN_CHANGE)]
    if listing:
        # A saved listing is used whole or not at all: any doubt about it runs every check, as no history was fetched.
        cached = listed_entries(listing, count)
        return cached if cached is not None else [("?", UNKNOWN_CHANGE)]
    return changed_entries(base, head)


def changed_paths(base, head):
    return [path for _, path in changed_entries(base, head)]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    parser.add_argument("--all", action="store_true", help="emit centralized dyna-checks selections")
    parser.add_argument("--files", metavar="LIST", help="read base...head's changes from this listing (pr_files.py) "
                        "instead of git")
    parser.add_argument("--entries", type=int, metavar="N", help="the number of entries pr_files.py wrote to LIST")
    args = parser.parse_args()
    if args.all:
        changes = pr_entries(args.base, args.head, args.files, args.entries)
        try:
            required_checks = selections(changes)
        except Exception as error:  # Selection is an optimization, never a reason not to run checks.
            print(f"rust_changes: cannot classify changes: {error}; running all checks", file=sys.stderr)
            required_checks = {"rust_required": True, "tooling_required": True}
        for name, required in required_checks.items():
            print(f"{name}={str(required).lower()}")
    else:
        paths = changed_paths(args.base, args.head)
        print(f"required={str(needs_rust(paths)).lower()}")
        print(f"sample={str(needs_sample(paths)).lower()}")
        print(f"server_build={str(needs_server_build(paths)).lower()}")
