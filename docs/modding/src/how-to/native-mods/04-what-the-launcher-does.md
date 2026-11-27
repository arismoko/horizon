# What the launcher does

Native mods run with your rights. Read their source and dependencies before allowing native code.
Install them into the selected instance from the Mods page, then choose **Build**, or run:

```sh
sunrise-launcher native-build path/to/mod --instance INSTANCE --trust-native-code
```

The compile steps below are `cargo sunrise-mod native-build`'s too, the tool every SDK ships for each host
([`native-build`](../../reference/sdk/08-native-build.md)).
This is the player's side: a developer builds and runs with `cargo sunrise-mod build` and `run` ([Your first native mod](../../start/first-mod.md#5-build-against-the-prebuilt-game)).

Omit the folder to build the instance's enabled native mods. The launcher keeps the instance on
its pinned Sunrise commit; `--update` selects a newer installed base and rebuilds the mods against
that commit's SDK.

## Build steps

1. Copy the mod projects into an immutable incoming directory and validate their pins and dependencies.
2. Read the game build's id from its release's signed `sdk.json`, then the SDK's own `sdk.json`
   (`sdk-<build id>`), checked against its signed `SHA256SUMS`. A schema, package format or Rust
   newer than the launcher reads asks for a launcher update.
3. Check each mod made for another build against this one (see "Mods made for another build"
   below). A mod that doesn't fit is left out of the build and named, never removed.
4. Download only the SDK slices the instance's mods need: `runtime` and `api` when every mod uses
   the mod API alone, `engine` as well when any mod has a `[native.*]` table, internals or a
   registry dependency. Each slice is checked against `sdk.json`, unpacked into its own folder and
   used again on later builds after a quick check of its file names and sizes. Install the
   launcher's pinned Rust toolchain and required linker tools if needed; the player's Rust is not used.
5. Compile the mods against the SDK's game libraries. Registry dependencies use the mod's supplied
   lockfile and the same immutable SDK. The launcher asks for permission before executing the mods' own and dependency
   build scripts or procedural macros.
6. For `[[native.mixin]]` rows, download the pinned game source, copy the targeted functions into one patch
   library, apply the rows with horizon-mixins, and compile that library against the SDK. The game
   crates are not compiled. Each checked replacement is installed through its typed installer.
7. Install the SDK's unchanged game executable and its companion libraries (including the compiled
   mod libraries) as `native-<commit>-<key>`, and select that build for the instance.
   `native-mods.json` records the base, pins, mods and build tools. Client builds share their base's
   prepared resources; server exports include their resources.
   On Windows x64 the launcher also places the two GNU runtime libraries of its managed toolchain
   (`libgcc_s_seh-1.dll` and `libwinpthread-1.dll`) beside the executable, so it starts without a
   separate compiler installation.

A matching installed build is reused. A failed build stops with its diagnostic; it never compiles
the game as a fallback. The build log is `native/build.log`. The Mods page shows the running step
and lets you cancel it.

## Build refusals

| Error | What to do |
|---|---|
| `E_NATIVE_FEATURES` | Use the pinned SDK's feature set; remove mod feature declarations and custom SDK feature requests. |
| `E_NATIVE_HOST` | Build on a host supported by the prebuilt SDK. |
| `E_MIXIN_STATE_LAYOUT` | Store per-object mod data in attachments instead of declaring `[[native.state]]`. |

Unavailable downloads can be retried. Missing SDKs, incompatible compiler identities and unsupported
transformations are refusals, not reasons to build the game locally.

## Mods made for another build

A native mod is made for one game build (its `[native] sunrise` pin). On another build the
launcher compiles it when it fits: the mod uses only the mod API (no `[native.*]` table, no
internals), its package lists what it uses (`uses.json`, written by
`cargo sunrise-mod build`), and every item on that list is unchanged in the build's
`surface.json`. The compile stays the final check: a mod that fits and still doesn't compile is
left out of that build and the others build.

A mod that doesn't fit stays installed and is left out of the build, with the changed items named
(at most five; the build log lists them all). A mod with a `[native.*]` table or internals, or a
package without the list, works on the build it was made for only.

When a newer release or Nightly is installed, the Mods page checks every native mod against it
before anything large is downloaded and says what an update would do: how many mods fit, and
which will be switched off and why. **Update** builds on the newer game; staying changes nothing.

When worlds of the instance were saved with a mod the build would leave out, the build stops and
names those worlds: opening them without the mod removes its blocks and what they hold, for good.
Staying on the current build is the default. The other choice backs those worlds up into the game
folder's `backups/` first (`native-build --back-up-worlds` on the command line); a backup that
fails stops the build with nothing changed.

## Where SDKs come from

SDK releases are read from this repository's releases. `SUNRISE_SDK_BASE` replaces that base with
a mirror URL or a folder holding `sdk-<build id>/` folders (and the game releases' folders, for
their build ids), for example a copy for building offline. Every base is checked the same way,
against the release signature. Offline, a build whose SDK files are already kept works; one whose
files are missing says so, with the size to download. What a release holds and how it is checked:
[The mod SDK](../../reference/sdk/01-release.md); the fit rule in full:
[Fitting another build](../../reference/sdk/04-fit.md).

## Rebuilds, updates and toolchain isolation

A native build uses the instance's pinned Sunrise commit. Editing or disabling a mod rebuilds
the mods against that same prebuilt SDK; building never repins a mod.

`native-build --update` selects a newer installed release or Nightly and checks every mod against
it. A build without a published SDK says it has no mod tools yet; use a build that has them.

The launcher uses its pinned Rust toolchain, not the player's Rust installation. It isolates
compiler flags, Cargo configuration and target settings from the player's environment. Build
inputs and outputs stay in managed launcher storage. On Windows the build tools require paths
the linker can open; the launcher chooses a managed ASCII path when necessary.

## Build log

The full log of a build is `native/build.log`, beside the build's own folders under the launcher's data
folder. Each new attempt replaces it. Compiler output and the final result
stay in the log, including failures before compilation or while selecting or exporting the built game.

Choose **Build log** at the top right to open the latest build's log. The action is available
whenever a log exists, including after restarting the launcher. Each build replaces that log. The
panel shows a bounded preview of the end of the log and says when it truncated; **Copy log** copies the
whole file, **Open full log** opens it outside the launcher, **Show in folder** reveals it, and
**Refresh** reads it again (also after a build finishes). The panel is a snapshot read on open and
refresh, not a live stream: a running build's later output appears only on the next refresh. Nothing is
uploaded; the log stays on the player's computer.

`cargo sunrise-mod build` keeps its own log and prints that log's path when it ends, built or failed.

The launcher's `native-build` command writes build progress to stderr. A terminal keeps
detailed download updates; redirected stderr reports only the first and completed updates for each
Rust toolchain or mod-tools download. Build phases, compiler progress and failures are still
reported in order, and the full build log is unchanged.

### When a build fails on Linux

- **A linker error naming `-ludev` or `-lasound`.** The game links udev and ALSA, so their development files must be
  installed: `libudev-dev` and `libasound2-dev` on Debian and Ubuntu, `systemd-devel` and `alsa-lib-devel` on Fedora,
  `udev` and `alsa-lib` in the shell's packages on NixOS. The build's error leads with this when it happens.
- **A C or C++ dependency that cannot find its own headers under Nix.** Nix's compiler wrapper, when
  `NIX_ENFORCE_PURITY` is `1`, drops every include folder outside the Nix store, and a dependency's sources are in the
  build's own Cargo folder. The builds the tool starts set `NIX_ENFORCE_PURITY=0`, so this affects only a `cargo`
  you run yourself in such a shell.

## Cache and disk

The native build key includes the pinned game and horizon-mixins commits, Rust version, target,
compilation settings and mod files. A matching installed build is reused. Editing a mod creates
a new build; the launcher compiles mods and their patch library, not the game.

The launcher keeps these managed caches under its data folder:

- `native/prebuilt/`: the SDK slices of each build and target (`<build id>-<target>`, holding
  `runtime/`, `api/` and `engine/` as fetched), evicted as a unit with whatever an interrupted
  download left in it.
- `native/sdk/`: each build's small SDK files (`sdk.json`, `surface.json`, `build-data/`) and the
  build id of each release seen; kept, so a mod's fit is told offline. What the published releases keep is in
  [Retention](../../reference/sdk/01-release.md#retention).
- `native/source/`: pinned source used to copy functions for mixins.
- `native/tools/`: pinned horizon-mixins executables.
- `native/prebuilt-build/`: mod, registry dependency, patch-library and linking outputs.
- `native/internals-sync/`: separate metadata-only discovery source and check outputs.
- `rust/` and `cargo/`: the pinned compiler and Cargo downloads.

`native/build.log` records build diagnostics. `native-mods.json` in an installed build records its
inputs and embedded mod inventory. Client builds share their base version's prepared resources,
so retain that base while its native builds are installed. Dedicated server exports are standalone.

Run `sunrise-launcher storage` to inspect disk use and cleanup eligibility. Storage keeps
in-use versions and shared caches protected through runtime leases and publication locks. A busy
or unverifiable unit is retained with a reason; it is not counted as freed. Cargo lock files also
prevent unsafe cleanup. Do not delete managed cache directories while a build or game is running.
