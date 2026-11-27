# Commands

The `cargo sunrise-mod` commands for mod projects. One tool serves every build: it reads each build's data from its
SDK, fetched as [In a mod project](03-in-a-project.md) says, and refuses a schema it doesn't know. Run them in a mod's
folder.

| Command | What it does |
|---|---|
| `new <path> --template native [--sunrise <build>]` | Makes a code mod pinned to the current Nightly's build, or the one named, and fetches its SDK. Without `--template` it makes a content mod, which has no SDK. Both templates scaffold [`page.toml` and `page/`](../../how-to/publish.md#your-mods-page) and the [release workflow caller](#release). |
| `build [--with <path>]…` | Compiles a native mod and its [declared libraries](../../concepts/dependencies.md) against the pinned SDK, then packs only the project into a `.smod` with its [`uses.json`](04-fit.md#usesjson). For content-only mods it just packs the project. If the project has `page.toml`, writes the page and its named images separately to `target/sunrise-mod/<id>.page.tar`. See [`build` for a native mod](#build-for-a-native-mod). |
| [`release [<version>\|patch\|minor\|major] [--check] [--pull-request]`](#release) | Checks a mod in a registered repository, commits the version bump, and pushes the branch and tag. `--check` previews without changing the checkout; `--pull-request` commits on a new `release/<tag>` branch instead. |
| [`release setup`](#release) | Updates only the shared workflow caller and removes old release files. No registration or commit required. |
| [`collection [--check]`](../../how-to/collections.md) | Checks the registry collection kept in this folder with the registry's rules, pushes it to your fork of sunrise-org/sunrise-mods and prints the link that opens its pull request. Without `collection.toml` it writes one listing this repository's published projects from the signed registry index. `--check` stops after the checks. |
| `run [--resources <folder>] [--jars <dir>] [--with <path>]… [-- <game args>]` | For a native mod, builds, stages the game with the mod and its libraries in `target/sunrise-mod/run/` and starts it. |
| `check [--uses] [--with <path>]…` | Checks the package rules and each [render pipeline shader](../../how-to/client/render-pipelines.md). For a native mod it then type-checks through the same SDK route and with the same libraries as `build`, without packaging or linking mod binaries, and reports what the project uses of its pinned build. `--uses` lists each item with the line that brought it in, and how the mod builds each struct it builds with a literal. [Page problems](../../how-to/publish.md#your-mods-page) are reported without failing the check. Only pages that select categories fetch the registry's list; if the list is unavailable, category IDs and their kinds cannot be checked. |
| `check [<file.smod>] --build <name> [--build <name>]…` | Reports fit for each named build instead of running the ordinary check. Takes no other options. Name the build `nightly` to check the current Nightly. Exits 0 when all fit, 1 when one does not fit, and 2 when fit cannot be judged. |
| `check --release <tag>` | Checks the project, compiles a native mod as `build` does, and fails on missing or invalid page data. If the page selects categories, the registry's list must be available and every selected ID must apply to the package kind. The tag must be `v<version>` at the Git repository root or `<id>/v<version>` for a mod in a subfolder. |
| [`check <file.smod>`](#check-a-released-package) `[--attestation <bundle>] [--repository <id>] [--json]` | Checks the package and verifies its GitHub artifact attestation offline. |
| `hooks <text> [--all]` | Searches the pinned build's hooks offline, by id, doc and their NeoForge, Fabric and Paper names; every word must match. `--all` searches every record. Matches in the id come first, then in a counterpart, then in the doc. |
| `sdk diff <build> <build>` | What changed in the surface from the first build to the second, [classified](02-surface.md#what-a-change-is), with the second build's change notes (why, and what a mod does now) under the changes they name: the notes of the changes since the SDK published before it. It reads only the builds' small files, so it works for builds whose compiled files are pruned. |
| `pin [<build>]` | Moves the pin to the current Nightly (or the build named). Reports API uses and declared internals that would break before moving the pin. |
| `use-jar <path \| modrinth-url \| <project> <version>>` | Lists an exact original Java jar as a passive asset and data source in `sunrise-mod.toml` (recording only `id`, `version` and `sha512`) and caches it in the local jar store. Accepts a local jar path, a Modrinth version page URL, or a Modrinth project and version. |
| [`internals add <path>…` / `internals sync`](#native-dependency-tools) | Declares or refreshes `internals` entries in `sunrise-mod.toml` from SDK build data or pinned game source. |
| [`self-update`](../../start/install-tool.md#updating-the-tool) | Replaces a standalone copy of the tool with the newest signed `mod-tool` build. Never changes a mod's pin. |

## Check a released package

```sh
cargo sunrise-mod check my_mod-0.1.0.smod
```

Keep `my_mod-0.1.0.smod.sigstore.json` beside the downloaded package, or pass `--attestation <path>`.
`check` validates the package without running its code. It verifies the signed SHA-256 and requires the
signing workflow to be `sunrise-org/sunrise-mods/.github/workflows/release.yml` at a
`refs/tags/release-v*` ref. An unsigned `.smod` fails this check; to check a local project before release,
run `cargo sunrise-mod check` without a file from that project's folder.

The report shows the GitHub repository name and numeric ID, the signing workflow ref, and the source
commit. Use `--repository <id>` to require a known numeric repository ID, even if that repository was
renamed. With `--json`, the verified claims are in `report.attestation`.

## `build` for a native mod

Before compiling, `build` fetches the build's `runtime` and `api` slices for this computer, plus `engine` for mods
that need it. It verifies the signed sums and unpacks the slices into the tool's cache, shared by every project:
`$XDG_CACHE_HOME/sunrise-mod` (else `~/.cache/sunrise-mod`) on Linux, `~/Library/Caches/sunrise-mod` on macOS,
`%LOCALAPPDATA%\sunrise-mod` on Windows; `SUNRISE_MOD_CACHE` names another folder. Deleting the folder clears it.
The Rust is the one `sdk.json` names: `rustup`'s, installed with `--profile minimal` when missing, or the `rustc` on
`PATH` when it is exactly that version; otherwise `build` stops and says the version and both ways to get it.
A mod with crates.io dependencies and no `Cargo.lock` gets one; `build --update-lock` resolves again, and is refused for
a content mod. `build` uses the same compiler as [`native-build`](08-native-build.md), with registry crates approved
(the mod's own `Cargo.toml`), for the `client` side. It prints `Built <library>` and the build log's path, and exits
non-zero with the error on failure.

In `build`, `run` and project `check`, repeat `--with <path>` for local mod projects or `.smod` files.
Optional libraries must be supplied with `--with`. For required dependencies not supplied locally, the tool
reads the Sunrise mod registry, verifies the selected
files against the index and caches their unpacked source in `registry/<id>/<version>/<sha256>/` in the tool's
cache. An archive taken down from the registry is refused even if already cached. A different Sunrise pin must
pass the native fit check before the library can be used.

`run` additionally needs `--resources <folder>`, prepared resources made from your own Minecraft (`sunrise-prepare
--prepare --output <folder>`). It copies the built game program, its libraries, the `native-mods.json` record naming
the mod libraries and their package files into `target/sunrise-mod/run/app/`, keeps the game folder and `SUNRISE_HOME`
in `run/game/` and `run/home/`, and starts the program with `--game-dir`, `--resources` and the arguments after `--`.
`run -- --dedicated` uses the server SDK and server-side dependency declarations. `--jars` supplies exact
originals for both the project and its staged dependencies.
When the pinned game supports `SUNRISE_LOG_STDERR`, `run` shows the native mod's own log records (INFO and above)
live in the terminal. The full client log is normally in `target/sunrise-mod/run/home/logs/latest.log`.
For a native mod, `run` verifies and stages the required originals after building, before replacing the development
app or starting the game. See [Java jars as external assets](../../how-to/content-mods.md#java-jars-as-external-assets)
for the default store, `--jars <dir>` and the development home requirements.

## `release`

Run from a mod's package folder in a registered repository with a clean Git working tree and `origin` pointing at GitHub.
The default version bump is `patch`. The version must increase and its tag must not already exist, either
locally or on `origin`. A root package gets `v<version>` and `.github/workflows/sunrise-release.yml`; a
subfolder package gets `<id>/v<version>` and `.github/workflows/sunrise-release-<id>.yml`. A native release
updates the mod manifest, Cargo manifest, and the mod's own entry in its committed `Cargo.lock` without
changing its pinned dependencies. Before it tags, the release check refuses a `Cargo.lock` that Cargo would change
for the mod's manifests and its pinned SDK (`cargo fetch --locked`, the registry scan's own check); the shared
release workflow runs the same check, and `pin <build>` with the pinned build regenerates the lock.

`release --check` checks the proposed version and tag in a separate checkout. `release --pull-request` is for a
branch that takes only pull requests: it commits on a new branch `release/<tag>` (which must not exist locally or
on `origin`), pushes that branch and then the tag, and prints GitHub's page for a pull request back into the
branch you released from. `release setup` reads the
registry's current shared workflow reference and refreshes the caller without requiring registration. It
removes old copied Sunrise release files, but leaves the manifest, commit history and tags alone; it needs a
clean tree, so each file it replaces or removes can be restored from Git. Without `setup`, a release makes the
same workflow changes in its own commit. Follow
the [releasing guide](../../how-to/publish.md#releasing) to register and publish.

## `check --build`

`check --build` exits 0 when every build fits, 1 when one does not, and 2 when it cannot tell: an input it
cannot read or fetch, or a question that doesn't apply (a content mod, a mod made for exactly one build, or a
package without a uses list). A mod's scheduled CI can run it to learn of a break before players do:

```text
Greeter:
nightly-2026-11-13 (afe471c)  made for
nightly-2026-11-14 (0c1d2e3)  fits
nightly-2026-11-20 (9a8b7c6)  does not fit: 2 items changed
    the hook `player_logged_in` (changed)
    the method `ServerHost::give_item` (removed)
```

## `pin`

Before it repins a native mod, `pin` lists the items the mod uses that changed, their source lines, and any
`migrate` lines from the change notes of every SDK between the old pin and the new one, each naming the build it
came with. For a removed item it suggests the closest one in the new build (one
with the same counterpart name first, then the nearest id and doc text). It then lists the game
code the mod declares (mixin targets, loops, internals) that changed. A mod made for exactly one
build is only named; its declared internals are checked as the pin moves.

When `check` fails to compile a native mod, it keeps the compiler's error and adds the change notes (why and
`migrate`) of the API items the mod's source names that the pinned build no longer has, from every SDK since the
build the mod last compiled against (which `check` records in `.sunrise/built`; with none recorded, the pinned
build's own notes). Each SDK's release page
lists the notes since the SDK before it.

The same checks without the tool, from a build's `surface.json` and a mod folder, are `sunrise-stage uses` and
`sunrise-stage fits` ([Checking a mod against a build](../../how-to/native-mods/14-checking-a-mod-against-a-build.md)).

## Native dependency tools

Run these commands in your native mod’s project folder.

| Command | Behaviour |
| --- | --- |
| `cargo sunrise-mod internals sync` | Checks a trusted native mod's library against an isolated copy of its pinned game source with the Rust version named by its signed SDK. Replaces top-level `internals` with compiler-discovered paths, kinds and hashes, including uses inside globs and macros. Removes unused declarations, preserves retained entries and prints additions/removals. Skips discovery when the mod neither depends on `sunrise-internals` nor declares `internals`. Never runs automatically on build. |
| `cargo sunrise-mod internals add <path>…` | Adds each declared internal and its tool-written hash to top-level `internals` in your mod's `sunrise-mod.toml`. Reads `internals.json` from the pinned SDK, or discovers a catalog with the mod's `[[native.module]]` tables applied. Unknown paths, shape-only entries and paths available only under a `cfg` are refused before the manifest is changed. Existing declarations are refreshed, not duplicated. |
| `cargo sunrise-mod pin [<build>]` | Moves the mod's pin to the current Nightly's build (or the build given: a release tag such as `nightly-<date>`, `26.3-0.1.0` or `dev-<date>-<short sha>`, or a sha); the next command fetches its SDK into `.sunrise/sdk/`. `pin` fetches the destination SDK's `sdk.json` (into `.sunrise/cache/<build>/`) before changing `[native] sunrise`. It leaves any explicit `[native] mixins` revision and `Cargo.toml` alone. A mod with a `Cargo.lock` gets the new SDK in `.sunrise/sdk/` at once, and `cargo update --workspace`, run with the SDK's Rust, brings the lock to it, keeping every registry pin still in use. For a native mod's every declared internal, prints `missing`, `changed` (old signature → new signature), or `unchanged`, then refreshes the available hashes. Missing or gated declarations keep their old hash and must be removed or replaced before building. Build names: [build names](03-in-a-project.md#build-names). |

The native-mod commands run in the mod's project folder and preserve the manifest's comments and
layout. `sync` performs a metadata-only `cargo check` on the current host, using default features;
Rust build scripts and procedural macros run with your rights, but the game does not run. It keeps
a separate source and check cache under `discovery/` in the tool's per-user cache (`SUNRISE_MOD_CACHE` can
set its root), never in the normal game or prebuilt target directories. The pin's source must be cached or
downloadable; an unavailable source is `E_INTERNAL_SOURCE_UNAVAILABLE`, not a fallback to another
pin. Compiler or catalog errors leave the manifest untouched. Discovery may prepare source and
metadata-check game dependencies; it does not compile or link a game executable. This separate
check cache is not a native mod build route. `[[native.state]]` is refused with
`E_MIXIN_STATE_LAYOUT`; use attachments for per-object mod data.

Without support modules, `internals add` reads `internals.json` from the pinned SDK's build data
(`build-data.tar.xz`). The tool verifies the SDK release, `sdk-<build id>`, against its signed `SHA256SUMS`,
using `SUNRISE_SDK_BASE` when set. No game or mod code is run.
With `[[native.module]]` tables, `internals add` uses the Rust named by the signed SDK to stage the pinned source
with the mod's modules applied and save the catalog under `target/sunrise-mod/catalog-snapshots/`. Only use
source discovery on mods you trust. It does not run the game.
`pin` compares the SDKs' build data for the old and new pins. Each build's data is kept
in `.sunrise/cache/<build id>/build-data/`, so `pin` can show old signatures after that release expires.
If a needed catalog is neither cached nor downloadable, the command fails without changing the pin
or declarations. A mod with no declared internals needs no catalogs to update. Quote paths containing
Rust lifetimes or other shell metacharacters, for example:

```sh
cargo sunrise-mod internals add 'sunrise_gameplay::game::ServerGame' "sunrise_gameplay::game::server_side::maps_server::ServerGame<'_>::dimension_id"
cargo sunrise-mod internals sync
cargo sunrise-mod pin
```
