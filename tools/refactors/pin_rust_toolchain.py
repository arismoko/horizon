"""Moves the active workflows from floating `stable` to the pinned Rust in tools/ci/rust-version.

Run once from the repository root. Hand corrections follow in their own commits.
- `bash tools/ci/rustup_install.sh stable …` and `rustup toolchain install stable …` become
  `bash tools/ci/rustup_install.sh …`, which installs the pin and makes it the job's toolchain.
- `cargo +stable`, `rustc +stable` and `rustup run stable rustfmt` lose the override.
- Step `env` entries `RUSTUP_TOOLCHAIN: stable` go (a step's env would override the job's), and an
  `env:` left empty goes with them.
- `dtolnay/rust-toolchain@stable` becomes `@master` with the pin, read by a step just before it.
"""
import pathlib
import re

DISABLED = {"ci.yml", "launcher.yml", "mod-registry.yml", "nightly.yml", "pack-bench.yml", "parity-main.yml"}
PIN_STEP = (  # read from the workspace root: a job may run steps in a subfolder
    "{i}- name: The pinned Rust (tools/ci/rust-version)\n"
    "{i}  id: rust\n"
    "{i}  shell: bash\n"
    '{i}  run: echo "version=$(cat \"$GITHUB_WORKSPACE/tools/ci/rust-version\")" >> "$GITHUB_OUTPUT"\n'
)


def rewrite(text: str) -> str:
    text = text.replace("tools/ci/rustup_install.sh stable ", "tools/ci/rustup_install.sh ")
    text = re.sub(r"rustup toolchain install stable ", "bash tools/ci/rustup_install.sh ", text)
    for tool in ("cargo", "rustc"):
        text = text.replace(f"{tool} +stable ", f"{tool} ")
    text = text.replace("rustup run stable rustfmt", "rustfmt")
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    converted = False
    for n, line in enumerate(lines):
        if line.strip() == "RUSTUP_TOOLCHAIN: stable":
            continue
        if line.strip() == "env:":
            indent = len(line) - len(line.lstrip())
            rest = [l for l in lines[n + 1 :] if l.strip() and l.strip() != "RUSTUP_TOOLCHAIN: stable"]
            if not rest or len(rest[0]) - len(rest[0].lstrip()) <= indent:
                continue
        match = re.match(r"(\s*)- uses: dtolnay/rust-toolchain@stable\s*$", line)
        if match:
            i = match[1]
            out.append(PIN_STEP.format(i=i))
            out.append(f"{i}- uses: dtolnay/rust-toolchain@master\n")
            following = lines[n + 1] if n + 1 < len(lines) else ""
            if following.strip() == "with:":
                # The `with:` that follows gains the toolchain below.
                converted = True
                continue
            out.append(f"{i}  with:\n{i}    toolchain: ${{{{ steps.rust.outputs.version }}}}\n")
            continue
        if converted and line.strip() == "with:":
            converted = False
            out.append(line)
            out.append(f"{line[: len(line) - len(line.lstrip())]}  toolchain: ${{{{ steps.rust.outputs.version }}}}\n")
            continue
        out.append(line)
    return "".join(out)


for path in sorted(pathlib.Path(".github/workflows").glob("*.yml")):
    if path.name in DISABLED:
        continue
    before = path.read_text()
    after = rewrite(before)
    if after != before:
        path.write_text(after)
        print(path)
