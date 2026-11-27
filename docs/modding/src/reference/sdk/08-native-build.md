# `native-build`

`cargo sunrise-mod native-build` compiles native mods against one build's SDK. It is the JSON form of what
`cargo sunrise-mod build` does for one mod, for a caller that builds for someone else: a build server, or a player's
launcher. Both commands use the same compiler (`crates/sunrise-native-build`). Every SDK release carries the tool
for each host it is built for, as `tool-<target>.tar.xz` holding `cargo-sunrise-mod` (`.exe` on Windows), so a caller
that already checked the SDK's signed `sdk.json` gets a tool made from that build's own source.
Use that SDK's tool for the JSON commands: its horizon-mixins revision and patch match the SDK by construction.
For standalone `cargo sunrise-mod build` and `run`, the mod tool selects the verified SDK tool when a
lower-crate mixin is pinned to a build that does not record lower-crate compiler cfgs.

Native mod authors keep the same code and `sunrise-mod.toml`. On Windows the SDK sets the main-thread stack
reservation when it links the fixed game executable, not when it links native mod libraries. If your tooling
inspects the SDK's `sunrise-prebuilt.json` identity, stop reading `link_args`: that field has no replacement.

The SDK tool uses the same library search path for mod, shared registry dependency and patch libraries.
On macOS, `@loader_path` resolves dependencies relative to the library that needs them, not the game executable.
On Linux, `$ORIGIN` in `DT_RPATH` also covers transitive dependencies. Install the returned libraries together
beside the executable as listed in the `built` event; mods do not supply these linker flags.

The format-6 identity uses `package_format`, not `format_version`. A compiled package must carry its build and
compiler identity, `slices`, `facade_crates`, `game_features`, `registry_lock`, `registry_patches` and `native_links`.
`native_log` was added within format 6; when absent, the SDK does not export the native log sink.
The `api` slice's `internals.json` must describe the same build as the `runtime` slice's identity.

The caller does what needs a player or their game: it checks that native code is allowed, fetches and verifies the
SDK's slices, installs the pinned Rust, chooses the mods and which of them a build leaves out, and installs what the
tool built. The tool compiles, and nothing else.

It reads a plan from the first line of standard input, writes one JSON event per line to standard output, and appends
its log to `build.log` in the plan's `work` folder: emptying it before a build is the caller's, since a caller's own
lines about the build may go there first. The caller runs the tool as a subprocess and owns cancellation of the
process tree. On Unix, start the tool in its own process group to be able to stop the whole build: its build
commands stay in that group.

```sh
cargo sunrise-mod native-build < plan.json
```

| Exit code | Meaning |
|---|---|
| 0 | It built: the last line is a `built` event. |
| 1 | It did not: the last line is a `failed` event. |
| 2 | No plan: the first line is not a plan of this contract (not one JSON object, a field missing, wrong or unknown). Standard error says why. |
| 3 | Another contract: the plan's `contract` isn't `2`. It is read before any other field, so a newer plan is refused for its number, never for a field this tool doesn't know. Standard error names both contracts. |

## The plan

One JSON object on one line. Its `contract` is checked first: anything but `2` exits 3. Then a field the tool
doesn't know is refused (exit 2).

| Field | What it is |
|---|---|
| `contract` | `2`. A change to any field or event below takes a new number. |
| `work` | The build's own folder: `build.log`, the horizon-mixins tool, the game source a mixin patch copies from, and kept compiles under `prebuilt-build/`. The launcher passes its `native/` folder, so the kept compiles are the ones its in-process build made. |
| `toolchain` | `dir` (holding `bin/cargo` and `bin/rustc`), `target` (its host triple), `cargo_home`, `cargo_config` (`--config` values every cargo command gets; the launcher's Windows ARM64 flags) and `c_bin` (the folder of its C and link programs, when it brings them). The Rust release must be the SDK's (`sdk.json`'s `toolchain.rust`). |
| `package` | `runtime`, `api` and `engine`: the folders the SDK's three slices for this host and side are unpacked into. The API route reads only `runtime` and `api`. |
| `mods` | A native mod's folder, or a folder of mod folders, read in path order. Directories without `sunrise-mod.toml` are not packages. The [package reader](../manifest.md) validates any manifest it finds; only native packages are compiled. A caller leaves a native mod out by not putting it there. |
| `target` | `client` (the game) or `server` (the headless engine). |
| `registry` | Optional. `true` or `false` answers in advance whether registry crates may be added and the mods' build scripts run (the build scripts, the mods' own among them, and procedural macros run while compiling). Absent, the tool asks with a `consent` event. |
| `mixins_from_source` | Optional, `false` by default. Builds horizon-mixins from its pinned source instead of downloading its release. |

## The events

Each line is an object whose `event` names it.

| Event | Fields | When |
|---|---|---|
| `mod` | `id`, `done`, `total` | A mod's own library starts compiling, `done` of `total`. Only the API and direct rustc routes emit this event. |
| `consent` | `added_crates`, `build_scripts`, `proc_macros` | Registry crates or build scripts, the mods' own included, need a yes and the plan gave no `registry`. The caller answers with one line on standard input: `true` agrees, anything else refuses. |
| `built` | `route`, `binary`, `libraries`, `records`, `mods`, `internals`, `mixins`, `used` | The executable and the libraries to install beside it, in load order; `records` is each library's identity, the build record's `libraries`: one row per library, the patch library included, whose `identity` is `<build id> <mod id> <version> <rustc release> <source> <dependencies> unwind`: `source` is the digest of the mod's files (`none` for the patch library), followed, when the build compiles crates.io dependencies, by `+` and the digest of the crates.io packages the library is compiled with as the build resolved them (name, version, source and the features of each compilation unit, target and host apart), since one build unifies them across all its mods; `dependencies` names each native mod the library links as `<id>=<SHA-256 of its identity>`, sorted and joined by `,` (`none` when it links none), and `unwind` is the panic strategy the library's entry reports (the loader refuses `abort`). The loader refuses, before loading any library, one whose file does not carry the identity its row names, or whose dependencies field doesn't match the identities installed beside it; `route` is `prebuilt-api` or `prebuilt`, as the record names it; `mods` is the compiled mods in dependency order. `internals` has each compiled mod's `[[internals]]` by mod id, every absent `hash` taken from the SDK's catalog, as the build record keeps them; `mixins` is the horizon-mixins revision the build ran; `used` lists the kept folders and files under `work` it used, for a caller's cache to count as used. |
| `failed` | `error`, `blame`, `used` | Why it did not build, as the build log ends with it. `blame` is `null`, or the mod the error is that mod's own and how: `{"mod": id, "kind": "compile" \| "features"}`, the one a caller leaves out next time. `used` as for `built`. |

`mod` is optional per-mod progress, not per-crate Cargo progress. For registry builds, read `build.log` for
compiler diagnostics and use `built` or `failed` together with the subprocess exit status to determine the outcome.

For SDK downloads and native-build downloads, the mod tool uses the first
nonblank setting in this order: `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`, `all_proxy` for HTTPS, or
`HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy` for HTTP. A caller with a proxy setting of its own
passes it in these variables. Both choose the route again at each redirect.

A proxy address without a scheme means HTTP. Supported schemes are `http://`, `socks4://`, `socks4a://`,
`socks5://`, `socks5h://` and `socks://` (an alias for SOCKS5). An `https://` proxy address is not supported,
even for HTTPS downloads. HTTP proxy credentials are sent only on HTTPS CONNECT, not for plain HTTP requests
such as downloads from an HTTP SDK mirror.

Local addresses bypass the proxy. `NO_PROXY` (or `no_proxy` if `NO_PROXY` is unset) is a comma-separated
list: `*` bypasses all hosts; a host or domain (`example.org`, `.example.org`, `*.example.org`) matches
that name and its subdomains. The list accepts IP literals, including bare or bracketed IPv6, but not CIDR.
An entry with a port (`example.org:8443` or `[2001:db8::1]:8443`) matches only that destination port,
including the URL scheme's default port when no port is written.

The tool never edits an instance, never selects a version and never installs anything outside `work`.

## `native-check`

`cargo sunrise-mod native-check` says how a set of native mods stands against one build before anything compiles:
each mod's refusal, or its fit (SDK design §10), and the route a build of the usable ones takes. It reads one plan
line on standard input and writes one reply line on standard output. Exit codes are those of `native-build`, except
that 1 comes with an error reply, `{"contract": 2, "error": {"kind": "unread", "message": …}}`: a file the plan names
(the mods, the surface, the build data) didn't read. A tool from before `native-check` (contract 1) knows no such
command: it prints its usage on standard error and nothing on standard output, so a caller that finds no reply line
knows the tool is too old.

```sh
cargo sunrise-mod native-check < plan.json
```

| Plan field | What it is |
|---|---|
| `contract` | `2`, checked first as for `native-build`. |
| `mods` | A mod's folder, or a folder of mod folders. |
| `build` | The build id (the game's full commit) the mods are checked against. |
| `surface` | Optional. The build's `surface.json`, read only for a mod made for another build. |
| `build_data` | Optional. The build's complete unpacked build data (`functions.json`, `internals.json`, `loops.json`). The check reads it when comparing a mod made for another build that declares game code (a `[[native.mixin]]` or `[[native.loop]]` row, an `internals` list), or for same-build mixin or loop rows. An omitted folder is reported as `needs` when the verdict depends on it. When read, a supplied folder with a missing file or data for the wrong build is an error even if a misfit is already known. |

The reply is `{"contract": 2, "route": …, "mods": […]}`. `route` is `prebuilt-api` or `prebuilt` for the usable
mods, `null` when none is. Each of `mods` has:

| Field | What it is |
|---|---|
| `id` | The mod's id. |
| `refusal` | Why no build takes it, as the build log says it; then `verdict` is `null`. |
| `verdict` | `{"kind": …, "made_for": build}`: `made-for` (no `made_for`), `fits`, `exact` (`why`: the `[native]` table whose rows no list can judge, or `internals` for a declared function whose body this build's data does not hold), `no-list` (its package's list of what it uses cannot show a fit; `why` is `missing`, `unreadable`, `newer-schema` (a tool from a newer build reads it), `other-build` (made for another build than its pin), `hook-versions` (nothing it can judge changed, but it holds no version of the hooks in `hooks`; packing the mod again writes them) or `game-code` (nothing it can judge changed, but it holds no record of game code the mod declares; packing the mod again writes it)), `does-not-fit` (`misfits`: each `{id, change}`: a surface id, or a `game:` id for game code the mod declares (`game:fn:<path>`, `game:internal:<kind>:<path>`, `game:loop:<name>`, `game:tool:horizon-mixins`), or for a mixin target that build's function index refuses, display text naming the target, error code and reason; and how it changed). `made-for` and `fits` are usable. |
| `needs` | `surface` or `build-data` when the plan didn't give the file its verdict needs; then `verdict` is `null`. The caller fetches it and asks again. |

A `change` is `"removed"`, `"changed"`, `{"lost-impl": trait}`, `{"fields": {gained, at}}`, `{"version": {named, now}}`,
`{"not-buildable": {at}}`, `{"path": path}` or `{"route": {from, owner}}`. On a mod's own build, an indexed
mixin refusal or a loop name absent from the catalog (`E_LOOP_UNKNOWN`) appears in `refusal`.
When checking a refused mixin's fit on another build, the result remains
`does-not-fit`: the misfit's `id` is display text with the full target, error code and reason, and its
`change` is `"changed"`. A known change to a declared game function is also `does-not-fit` with `change: "changed"`;
its `id` names the full function path and says `body changed` or `declaration changed`.

## `native-discover` and `native-lock`

Two more tools a caller with the SDK's Rust runs on one native mod. Each reads one plan line and writes one reply
line, with the exit codes of `native-check`: 0 a reply, 1 an error reply `{"contract": 2, "error": {"kind", "message"}}`,
2 no plan, 3 another contract. Both plans have `contract` (`2`), `work` (the tool's own folder: the pinned game's
source and caches), `toolchain` (as for `native-build`) and `mods` (the mod's folder).

| Command | Its own plan field | Reply | Error `kind` |
|---|---|---|---|
| `native-discover` | `catalog`, optional: a folder that doesn't exist yet | without `catalog`, `{"internals": […]}`: the game internals the mod names, each as the pinned game's catalog lists it (`cargo sunrise-mod internals sync` writes them into `sunrise-mod.toml`). With it, `{"catalog": folder}`: the pinned game's `internals.json` with only this mod's `[[native.module]]` rows applied, copied there | `source` (the pinned game's source isn't cached and couldn't be downloaded), `discover` |
| `native-lock` | `update`, optional: resolve again when the mod has a lock | `{"lock": path}`: the mod's registry dependencies pinned in its `Cargo.lock`, starting from the versions the pinned game locks | `lock` |

`native-lock` reads the pinned game's `Cargo.lock`, kept as `game-lock/<commit>.lock` in `work` and downloaded from
that commit of the game's repository the first time; with no network, put the original file there. It creates `work`
when it is missing. The same 16 MiB limit applies to the kept and downloaded copies. Fully pinned Git packages in
that game lock are treated as game-local packages, not as sources for Cargo to fetch. This does not allow Git
sources in a mod's lock. A shared crates.io package keeps the game's version on the same semver line. If a direct or
transitive requirement excludes that version or requests a feature it lacks, `native-lock` refuses without replacing
the mod's `Cargo.lock`.

`native-discover` checks the mod against the pinned game's source with the deprecation lint forced on the mod alone;
it runs the tool itself as Cargo's rustc wrapper for that.

## `native-layout`

`cargo sunrise-mod native-layout` lays out a local prebuilt game, the folder `tools/prebuilt_game.py build` makes from a
checkout, as an SDK release's slices, for a build no SDK is published for. The plan is `{"contract": 2, "prebuilt",
"build", "host", "target", "out"}`. It checks that `prebuilt` is that build id's game for that host triple and side
(`client` or `server`), built with this SDK's Rust. Then it lays its files out by links under `out`, which it replaces;
`prebuilt` itself is never changed. The reply is `{"package": {"runtime", "api", "engine"}}`, the three folders a
`native-build` plan names. A folder that is something else gives exit 1 with an error reply of kind `layout`.

## Changelog

- contract 2: `native-check`, `native-discover`, `native-lock`, `native-layout`; `failed` names the blamed mod (`blame`) instead of a caller matching its text; `built` adds `internals` and `mixins` for the build record; both list the kept files they `used`.
- contract 1: the plan, `mod`, `consent`, `built` and `failed`.
