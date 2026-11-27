# Sunrise: a map for agents

Minecraft: Java Edition 26.3 rewritten in Rust (wgpu). Vanilla is the specification wherever it has the
feature: Sunrise behaves and looks bit-exactly like it on the same machine, while its code is free to differ
([CONTRIBUTING.md](CONTRIBUTING.md#vanilla-first)). Read [CONTRIBUTING.md](CONTRIBUTING.md) once;
this page tells you where things are.
Feature → files → harness for the most-changed areas: [docs/architecture.md](docs/architecture.md).

## Rules that bite

- **Never commit Mojang's files**: game jars, assets, decompiled source, prepared resources (`.local/`).
  `tools/check_public_tree.py` rejects them; `git config core.hooksPath .githooks` runs it on commit.
- **Keep your runs away from real player data.** Any game or launcher you start gets fresh `mktemp -d`
  folders in `SUNRISE_HOME` (logs, crash reports, telemetry consent, recordings; not saves), `SUNRISE_LAUNCHER_HOME`, `XDG_DATA_HOME` and
  `XDG_CONFIG_HOME`, plus `SUNRISE_LAUNCHER_NO_SELF_UPDATE=1` and `SUNRISE_NO_AUDIO=1`. Saves, `options.txt` and
  caches go in the game folder, by default the working directory: start the game from a scratch folder with
  `--game-dir` pointing at it, or they land in the repo root, hidden by `.gitignore`. For the
  launcher, `SUNRISE_LAUNCHER_TRASH`, `SUNRISE_LAUNCHER_CLIPBOARD` and `SUNRISE_LAUNCHER_DESKTOP` pointing into the scratch home. Never write to
  `~/.local/share/{applications,sunrise,sunrise-launcher}` or `~/.local/bin`.
- **Mark what an agent posts.** A PR body starts with `> 🤖 Opened by <login>'s AI agent` and a blank line, where
  `<login>` is the GitHub account the PR is opened from (`gh api user --jq .login`). Issues say "Filed by", reviews
  "Review by". Every comment starts with `🤖 Agent:`. That line is the only mark: commits carry no
  `Co-Authored-By:` and PR bodies no "Generated with" line.
- Everything players run is Rust. Python/Java only for dev tools and harness drivers in `tools/`.
- Sunrise-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- One concern per PR, at whatever size it needs; no line limit. Labels: `feature` (players get something new), `skip-release` (they see no
  change), none for a fix. Agents never add `release-now`: the maintainer decides it. Merges reach players through the
  nightly, published by the daily run when every leg is green ([docs/release-flow.md](docs/release-flow.md#daily-run)).
- **CI tiers** ([CONTRIBUTING.md](CONTRIBUTING.md#ci-tiers)): every PR push runs `format` and `verify-static`; rollups,
  syncs and main pushes add `lint`, `verify` (unit tests only) and `launcher`; the ignored-test shards,
  `fn-replace-check`, the SDK build and its linkage probe, the machine-sample build and the modding book's
  quickstart run once a day (`daily.yml`). A new test that needs prepared resources, boots the game, starts a
  child process, uses the network or takes 2 s or more is `#[ignore]`d with a row in `tools/ci/ignored-tests/`;
  say in the PR when you could not run it. A daily red becomes an issue listing the suspect PRs, fixed by an
  ordinary PR. `tools/ci/move_to_daily.py` moves a `verify` test that needs resources, a child process or the
  network. Builds: Dev (a commit of main, gate green; published as
  `dev-<date>-<short>`, which no launcher offers), nightly (published by a fully green daily run), release (cut from a nightly).
- **No hardcoded user-facing strings.** Keep UI translation-ready from the start. In the launcher ([sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher)), wrap visible text with `tr("...")` or `trf("... {arg} ...", &[("arg", &val)])` from `crate::i18n` instead of raw string literals or `format!`. In the game, use translation keys (`Text::tr`, `lang::get`), never hardcoded English strings.
- **Claim before you start**: comment `/claim` on the issue (no issue yet: open one, then claim it). An assigned
  issue is taken, and so is one labelled `in progress` (claimed on Discord): pick another. `/unclaim` when you stop;
  a claim with no open PR linking it is released after 5 days. The issues free to claim, without reading threads:
  `gh issue list --search 'is:open no:assignee -label:"in progress" -label:tracker -label:proposal -label:"not claimable" -linked:pr'`
- **Machine translations stay unreviewed.** `tools/lang_review/` records only text a person who speaks the language
  wrote or checked; never `lang_coverage.py --mark-reviewed` machine text ([docs/translations.md](docs/translations.md)).
- **Controllers reach what registers.** Draw controls with a registering helper (`ui::button`, `settings_shell::foot_button`); `graphics_style::button` and `screen_draw::button` only paint, and registering one part of a screen ends its crosshair, so register all of it.
- **Render passes keep HDR's light.** A pass that reads, filters or redraws the world's picture (upscaling,
  anti-aliasing, sharpening, frame generation, post effects, captures) follows [docs/hdr-passes.md](docs/hdr-passes.md).
- **Docs say where and how, not how many.** Point to where a thing lives and the command that lists or measures it.
  Don't write counts, growing lists, timings or current status into a doc: they go stale, and the next agent
  spends a turn fixing them.
- **Contract details go in rustdoc, not concept pages.** A PR that adds or changes a hook, method or edge case updates its `///` doc comment or `#[notes]`, not a concept page in `docs/modding/src/concepts/`; runnable Rust snippets belong in `docs/modding/examples/` via `{{#include}}`.

## Mod API: the clean break

Sunrise (sunrise-org/sunrise) is where the native mod API is built: hooks, the `Host`, handles, Parts,
attachments and mod loading. The mod API is taking **one clean
break**, and every change to it follows these rules. Reviews block a PR that breaks any of them.

- **No compatibility paths.** No aliases, shims, deprecated names, migration or fallback code, version
  gates, or "kept for old mods" branches. Old mods rebuild; the compiler says what moved. Removing an old
  path updates every caller, the catalog, docs, tests and sunrise-machine-sample in the same change.
- **One hook per moment.** A hook with a NeoForge 26.3 counterpart takes its name, side and full contract.
  Two hooks at the same vanilla call site (including `_v1`/`_v2` pairs) merge into that one, and the old
  one is deleted, not kept beside it. A veto before an action and a notice after it are separate moments
  when vanilla runs code between them. A hook whose vanilla call site is gone is removed from the catalog.
- **Every hook gets a `Host`.** Hooks that fire inside client-lib split into decide, fire, apply: client-lib
  decides, the game fires the hook with a `Host`, then applies. The listener's writes and the apply run in
  one transaction, and the apply re-checks the decision's preconditions and refuses (never recomputes) if
  they changed.
- **`Host` shape.** A small shared `Host` (handle constructors, registries, settings, `transaction`), with
  `ServerHost: Host` and `ClientHost: Host` generated from each catalog entry's `side`, so a wrong-side
  call doesn't compile; a hook that fires on both sides gets a typed `HostSide`. Per-object methods live on
  handles (reads take `&dyn Host`, writes `&mut dyn ServerHost`), and the ID-based helper a handle method
  covers is removed when it lands. Drawing goes through a `DrawContext` given only to render and screen
  hooks. Items, fluids and energy go through Parts under transactions. Commands, players, messages and
  scheduling are small service traits reached from `Host`; attachments live on handles. Don't add a
  method to `Host` that has a natural receiver.
- **One dependency model.** Mods declare required and optional dependencies in `[dependencies]` in
  `sunrise-mod.toml`, using semver constraints on manifest versions. The game dispatches in dependency
  order. Native mods link through Cargo; a library mod can publish functions and statics to its
  dependents, which share one copy of the library.
- **Native binaries target the exact game build.** The loader checks the build ID a native mod was built
  against. `.smod` is the one package format for both native and content-only mods.
- **DRY.** One implementation per behaviour: reuse the existing helper, write path or generator rule, and
  let the catalog and generator own repetition instead of hand-written match arms.
- **SOLID.** Each module, trait and handle has one job; traits stay small and split by side and concern;
  new behaviour comes in through the catalog, handles and parts; mods depend on `mod_api`, never on game
  internals, and lower crates never reach up into `Game`.
- **Neat.** Names, layout and docs match the code around them. No dead code, leftover TODO paths, or
  anything a later PR must tidy up.
- **Fast.** No allocation, string building or scan over every mod on hot paths (tick, per-entity and
  per-block hooks). Dispatch only to the mods and parts that subscribe, resolve keys once at load, and a
  PR that touches a hot path states its tick-time effect.

## Don't lose time

Where agent time goes: reading code, compiling `sunrise-client`, waiting in loops, and work someone else had
already done.

- **Check nobody else is on it.** Before you write code, look at the issue's linked pull requests and run
  `gh pr list --state open --search "<issue number, function or file name>"`; then `/claim`.
- **Find code with the map, not by reading.** Start from the tables below and
  `python3 tools/port_ledger.py where Class.method`. Read the function you change and its callers, not whole
  files.
- **Compile cheaply while you work.** `cargo check -p <crate>` after an edit. Clippy, fmt and tests run once, at
  the end, through `pr_ready.py`. `cargo clippy -p sunrise-client` and `cargo build -p sunrise-client` are the
  slowest commands here: never after every edit, and build the game only when you need to run it.
- **Don't wait in loops.** Start a long command in the background and carry on, or wait for it once. No
  `sleep` or `until grep` polling of builds, harnesses or CI.
- **One test per changed behaviour.** None for a refactor or a file move, none that reads source text, and no
  harness where vanilla has nothing to compare (Sunrise's own features). Performance work is proven by the area's
  bit-exact harnesses (add one first if none covers the code) and a before/after benchmark.
- **A red check isn't always yours.** When `verify` fails in a test your change can't reach, say so in the pull
  request and move on; don't hunt it.
- **Unsure about a rule? Finish the work.** Take the reading that lets you finish, and say in the pull request
  which one you took. Don't stop, and don't change direction halfway.

## Crates

Game workspace (root `Cargo.toml`); `crates/sunrise-telemetry-server` is a separate workspace, and the launcher is its own repository, [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher).

| Crate | Owns |
|---|---|
| `sunrise-client` | Gameplay library (`src/lib.rs`: `game.rs` + `game/`, `entry.rs`, `autobins = false`, no `src/main.rs`). Also the dedicated server (`--dedicated`). Re-exports `sunrise-client-lib` for existing mod paths. |
| `sunrise-client-lib` | Reusable client systems, `sunrise-ctl`, probes and their tests. Its manifest points to the existing sources under `crates/sunrise-client/`; the library root is `src/client_lib.rs`. It never depends on gameplay. |
| `sunrise-gameplay` | Vanilla's logical side, what the server jar holds (common code included), moved out of `sunrise-client` module by module ([#2210](https://github.com/sunrise-org/sunrise/issues/2210)). `game_tests.rs` and helpers here and in `game/administration/` (such as `test_entities.rs`, `commands/test_locate.rs`, and `commands/test_runs.rs`) implement vanilla GameTest; they can also contain Rust unit tests, while files such as `commands/test_summary_context_tests.rs` are test-only. Engine layer, not yet portable: `portable.pending` lets it take `sunrise-client-lib` until the types it needs move down. Never depends on `sunrise-client-gameplay`. |
| `sunrise-client-gameplay` | Client-only gameplay moved out of `sunrise-client` (occlusion/entity/particle culling, `section_scene.rs`, `transient_blocks.rs`...). Depends on `sunrise-client-lib` and portable crates, never on `sunrise-gameplay` and intentionally not on `mod_api` (`sunrise-mod-vocab`). Engine layer, desktop. |
| `sunrise-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs`. Re-exported as `sunrise_client::items`... |
| `sunrise-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form. Re-exported as `sunrise_client::command` |
| `sunrise-game-data` | Portable game data below commands and gameplay, used by client and server: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables. Re-exported under the old paths (`sunrise_client::text`...) |
| `sunrise-client-input` | Key bindings and key names, toggle keys, mouse options, controller settings and glyphs, the clipboard, window options (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `window.rs`...). Re-exported under the old paths |
| `sunrise-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings): its modules (`ls crates/sunrise-client-render/src`), which sunrise-client re-exports under the same paths. Nothing in it reaches gameplay |
| `sunrise-stage` | Prebuilt game staging, SDK surface export and comparison, and standalone mod source scans (`surface`, `surface-diff`, `uses`, `fits`) |
| `sunrise-mod-vocab` | Source of truth for the public mod API (engine, portable; imported by engine crates as `mod_api`): host area traits in `src/host/` (named `<Area><Tier>`), hooks in `src/hooks/`, domain types, and `pub const` limits. Re-exported via `OUT_DIR/mod_api_exports_root.rs`. |
| `sunrise-mod-api` | Platform facade re-exporting `sunrise-mod-vocab` (`mod_vocab::*`), plus registration, parts and namespace authority (`src/load_state.rs`, `src/parts/owned_namespaces.rs`). |
| `sunrise-mod-api-gen`, `sunrise-mod-surface`, `sunrise-game-discovery` | Build-time generator for `Host`/`ServerHost`/`ClientHost` forwarders, `fire_<hook>`, `GoalCx`, mixins/loops and `surface.json`; `surface.json` schema/hashing; `game/` module and hook discovery. |
| `sunrise-mod-host` | Platform mod loader (registers the Mods title button via `sunrise-hooks`). |
| `sunrise-mod-runtime` | Engine-independent mod attachment storage, query index and generic registry slot tables; depends only on `sunrise-core` |
| `sunrise-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, items, effects, food, damage. Sits below `sunrise-game-data`, `sunrise-items`, `sunrise-protocol` and `sunrise-command` (so `World`/`Chunk` cannot move into it without cycling), and depends on `sunrise-mod-vocab`. |
| `sunrise-border` | Engine crate for world-border math, with only `sunrise-core` as a dependency (not listed in `portable.crates`). |
| `sunrise-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `sunrise-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `sunrise-assets` | Resource decoding: block-state and item models, baking, atlases |
| `sunrise-prepare` | Builds `.local/sunrise-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `sunrise-server` | `sunrise-server`: `server.toml`, CLI, supervisor of the headless game ([docs/hosting.md](docs/hosting.md)) |
| `sunrise-server-files` | `server.properties`, eula, ops/whitelist/bans, usercache |
| `sunrise-java-properties` | `java.util.Properties` load/store and the JDK date text, standard library only; used by `sunrise-server-files` |
| `sunrise-friends`, `sunrise-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `sunrise-mod-package`, `cargo-sunrise-mod` | Content mods (`.smod`: manifest, content fence, no code), the registry index and `.spack`, and `cargo sunrise-mod` (`new`, `build`, `run`, `check`, `publish`, `registry`, `native-build`) ([docs/modding/](docs/modding/src/introduction.md)). Code mods are native, built by the launcher ([docs/modding/src/how-to/native-mods.md](docs/modding/src/how-to/native-mods.md)) |
| `sunrise-native-build` | Native mods compiled against a signed prebuilt SDK: the launcher's build, and `cargo sunrise-mod native-build`'s JSON contract that every `sdk-<build id>` ships ([docs/modding/src/reference/sdk/08-native-build.md](docs/modding/src/reference/sdk/08-native-build.md)) |
| `sunrise-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `sunrise-proxy` | Shared proxy selection and connection pools for CLI downloads |
| `sunrise-connected` | Connected textures whatever file declares them: the neighbour geometry, the format-neutral `Rule` (OptiFine's and mods' both fill it) and its `table` method. Read by `sunrise-client-render`'s `connected.rs` and the mesher, and by `sunrise-optifine`; a mod's files are `assets/<mod>/sunrise/connected/` ([docs/modding/src/how-to/client/connected-textures.md](docs/modding/src/how-to/client/connected-textures.md)) |
| `sunrise-optifine` | OptiFine-format pack files below every feature that reads them: `.properties`, the name/range/block matchers, paths and `optifine/` folders, and custom entity models (`cem/`: `.jem`/`.jpm` parsing and baking) |
| `sunrise-polytone` | Polytone-format pack files (`assets/*/polytone/`), read clean-room from the public wiki: targets, biome and dimension modifiers and their environment attribute layers |
| `sunrise-shaders` | Shader packs (the `shaders/` folder format), rebuilt clean-room from a written spec of the format; no wgpu |
| `sunrise-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `sunrise-perf-format` | The `.hzperf` capture contract ([docs/perf-platform.md](docs/perf-platform.md) §3): `proto/sunrise/perf/v1/capture.proto`, its generated types, the gzip writer and its thread. No Sunrise dependencies; nothing in the game reads a capture |
| `sunrise-telemetry`, `sunrise-telemetry-server`, `sunrise-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `sunrise-addresses` | Every built-in address of an outside service vanilla doesn't contact (Sunrise layer; [docs/layers.md](docs/layers.md#built-in-addresses)) |
| `sunrise-system` | The machine the game runs on (OS, CPU, memory, drive, power, GPU maker and memory), for the crash report, the renderer's GPU info and telemetry |
| `sunrise-platform` | The platform's public API: what the Sunrise layer may use of the engine ([docs/layers.md](docs/layers.md)); re-exports `sunrise-system` today |
| `sunrise-hooks`, `sunrise-game`, `sunrise-official`, `sunrise-heartbeat` | The layer seam: `sunrise-hooks` is the engine→platform/Sunrise static extension seam (title buttons, settings, HUD, lang, shutdown — **not** the mod API); `sunrise-game` is the thin playable executable (`src/main.rs`, `[[bin]] test = false`); `sunrise-official` is the shipped `sunrise-client` executable registering Sunrise's in-game parts; `sunrise-heartbeat` is the first of them ([docs/layers.md](docs/layers.md#how-a-sunrise-crate-plugs-in)) |
| `sunrise-reconnect` | Auto-reconnect: its switch, its policy and backoff, on the engine's connection hooks; registered by `sunrise-official` ([docs/extras.md](docs/extras.md)) |
| `sunrise-dualsense`, `sunrise-threads`, `sunrise-release-sign`, `sunrise-bots`, `xtask` | DualSense output, thread priorities, signed release checksums, load-test bots, repo tasks |
| [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) | GPUI launcher, in its own repository (moved there with its history, 2026-11-18): make launcher changes there. `launcher-bridge.yml` signs that repository's releases with this repository's release key and publishes them there and here as `launcher@X.Y.Z`, for launchers installed from here |

Other top-level folders: `tools/` (harnesses and dev scripts), `tests/` (Python unit tests: `python3 -m
unittest discover -s tests`), `mods/sunrise/src/test/java/dev/sunriselab/parity/` (Java oracles),
`docs/`, `.github/workflows/`. The website (playsunrise.net) is `frontend/site` in the private sunrise-org/sunny: its build
reads `README.md`, `docs/api/`, `roadmap/roadmap.json` and `crates/sunrise-theme/tokens.css` from main.

## Write tests with signal

- Test observable behavior, not words in docs, comments or source. Keep checks for machine-consumed contracts.
- Use independent expected values or a vanilla oracle; do not derive the answer from the code under test.
- Test outcomes, not only calls to stubs. Use mocks to isolate behavior, not as the behavior itself.
- Skip tests of trivial constants, derives and getters, or behavior already covered by another test.
- One test per changed behaviour is enough. A refactor or a move that changes no behaviour gets no new test.
- Avoid pinning implementation details; a harmless refactor should not break a behavior test.
- Make slow tests faster without removing cases or weakening their coverage.
- Never delete parity harnesses or tests that pin vanilla behaviour: they are the spec.
- No workarounds: when a harness differs, fix the cause until it matches bit for bit; never add a tolerance, rounding,
  a special case or a compensating patch to pass it. Removing one, from the code or a harness, is welcome while the
  harnesses stay bit-exact: smaller code is a goal.

## Build and test fast

```sh
python3 tools/pr_ready.py                  # the self-check: fmt, clippy -D warnings, touched crates' tests, harnesses; paste its block in the PR
python3 tools/pr_ready.py --quick          # same without harnesses
cargo test -p sunrise-client -p sunrise-game -p sunrise-client-lib --lib --bins --tests   # skips the examples (pr_ready does this); `--doc` separately
cargo test -p sunrise-client-lib --lib mod_content::tests:: -- --ignored   # the slow content-root tests, #[ignore]d; pr_ready runs them when mod content, assets, blocks or items change
cargo build --profile fast -p sunrise-game        # optimized, quick to link: use it to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN (server-thread host)
python3 tools/agent_control_e2e.py         # sunrise-ctl / MCP end to end
python3 tools/lang_coverage.py --summary   # one line per language: how many keys it carries and what it misses
python3 tools/lang_coverage.py --summary --show-keys   # the same, then the keys behind the counts
```

- GPU tests share `gfx::test_gpu::lock()`: bind the guard before creating GPU objects so it outlives
  every resource, queue, device and instance, including GPU-owning workers (join them before unlocking).
  Helpers returning GPU objects leave the guard with their caller; helpers returning only CPU data may
  own it themselves, without another outer guard. CPU-only tests need no guard. Run renderer coverage
  with `cargo test -p sunrise-client-render --lib -- gfx::`; no global serial-test flag.

- For a player-facing change, drive the game once with `sunrise-ctl` (launch, the action, a `state` or `screenshot`)
  and report the result in the PR's "Tried in game" line; never claim it when it was not run.
- Harnesses and the game need the vanilla data in `.local/` (`minecraft/`, `sunrise-client/26.3`, `datagen/`,
  `tools/`): see [docs/parity-workflow.md](docs/parity-workflow.md). Without it, pr_ready lists the
  harnesses for CI. Fresh git worktrees start without `.local/`; symlink `.local/{minecraft,sunrise-client,datagen}` read-only from an existing prepared checkout instead of re-running `sunrise-prepare`.
- Checking PRs, keep one warm worktree (`.local/` linked as above) and move it between heads with `git checkout --detach <sha>`: Cargo rebuilds only what changed, where a fresh worktree pays a cold build each time (most noticeable on Windows). Never share one `CARGO_TARGET_DIR` between worktrees: Cargo and build-script outputs (such as `sunrise-mod-api-gen`'s) built for one worktree's paths can be reused by another and give wrong results. Don't `git clean` the warm `target/` away (no `git clean -x` there).
- `state-digest` lives on `sunrise-client` (`crates/sunrise-client/Cargo.toml`) and is not forwarded on `sunrise-game`: enable it as `--features sunrise-client/state-digest` (with `-p sunrise-game`, or `-p sunrise-client --features state-digest`) for `tools/state_digest_scene.py` / `tools/state_digest_compare.py`.
- There is no `rustfmt.toml` at the repo root (defaults + `--edition 2024`); `python3 tools/ci/format_scan.py` is the complete formatting check (it runs `cargo fmt --all -- --check` plus per-file `rustfmt` over `include!()` fragments, build-included `sunrise-mod-{api,vocab}` files, `game/`, and `sunrise-mod-api-gen` dispatch fixtures).
- `Cargo.toml` profile invariants: keep `[profile.test.package.sunrise-client]` (`opt-level = 0`, `debug = 0`) and `sunrise-client-lib` (`opt-level = 0`) to prevent single-`rustc` OOM, keep `[profile.fast]` at `lto = "off"` (not `false`) for incremental caching, and keep `sunrise-client-lib/default` in `crates/sunrise-game/Cargo.toml`'s `default` features so Cargo does not compile `sunrise-client` twice.
- A harness = `tools/test_<name>_parity.py` (`python3 tools/parity.py --all --list` names them) + a Java oracle (`mods/sunrise/src/test/java/dev/sunriselab/parity/Vanilla*Oracle.java`)
  + a Rust probe: a module `crates/<crate>/examples/probe/<name>.rs` of that crate's probe binary (`ls crates/*/examples/probe`),
  or the game binary with a fixture env var (`game/interface/dispense_fixture.rs`, `game/blocks/sculk_fixture.rs`,
  `game/administration/command_harness.rs`, `game/network/net_harness.rs`). Copy a nearby harness for a new one.
  A sunrise-client fixture (a `#[test]` under `game` whose body is one call to `run_<test>()`) runs from the
  `sunrise-parity-fixtures` program (`crates/sunrise-parity-fixtures`, `lab.parity_fixtures()`, [docs/parity-workflow.md](docs/parity-workflow.md#sunrise-client-fixtures));
  a harness never builds the `sunrise-client` lib test.
- Native mods compile against a pinned prebuilt SDK; `[[native.mixin]]` rows compile into a function-replacement patch library.
  Use `native-build --trust-native-code`; the launcher never rebuilds the game for a mod.
- Probes build into `.local/parity-target/`; harnesses find binaries via `lab.cargo_target()`, never `target/`.
- Harness selection uses Cargo dep-info and reachable-code narrowing (`tools/parity_reach.py`).
  `sunrise-client-lib` builds the control tool and client probes without gameplay; their dep-info
  excludes `game.rs` and `game/`, so gameplay edits do not rebuild or select those probes.
  Source paths remain in `crates/sunrise-client/`. Measure actual harness selection on refreshed
  records with `tools/parity.py --changed-files paths.txt --list`; historical counts are not a guarantee.
- Rust integration tests: `crates/sunrise-client/tests/` (`ls crates/sunrise-client/tests`).
- Linux needs `libasound2-dev libudev-dev`; `--no-default-features` drops audio, controllers, `discord`, `friends` and `heartbeat` (the server build).
- Run and drive the game (starts the `sunrise-client` beside `sunrise-ctl`; other arguments go to the game):
  `target/fast/sunrise-ctl --socket $PWD/ctl.sock launch --world W --creative --offscreen --wait --resources $PWD/.local/sunrise-client/26.3`,
  then `sunrise-ctl --socket … state|keys|turn|command|screenshot|quit`
  ([docs/sunrise-ctl-commands.md](docs/sunrise-ctl-commands.md)).

## Checks CI runs that you can run locally

`.github/workflows/dyna-ci.yml` splits static checks across `format`, `verify-static` and `verify`:

```sh
# Pure Python / source-only (fast, no Cargo)
python3 tools/ci/check_repository_hygiene.py                 # format / lint
python3 tools/ci/check_api_boundary.py --source-only         # format / lint (ENGINE set is hardcoded, separate from layers.json)
python3 tools/check_public_tree.py                           # verify-static (CI adds --range BASE..HEAD or --history)
python3 tools/ci/surface_extension_tags.py --check           # verify-static
python3 tools/sync_upstream.py --check                       # verify-static
python3 tools/check_addresses.py --base origin/main          # verify-static local; CI: ${BASE_SHA:+--base "$BASE_SHA" --pull-request-merge}
python3 tools/hotspots/check_guards.py --base origin/main    # verify-static local; CI: ${BASE_SHA:+--base "$BASE_SHA" --pull-request-merge}
python3 tools/hotspots/check_lists.py && python3 tools/hotspots/check_stand_ins.py
python3 tools/check_mod_vocabulary_state.py                  # verify-static
python3 tools/ci/check_port_map_paths.py --base origin/main  # verify-static (--refresh prunes resolved stale rows)
python3 tools/ci/check_game_boot_tests.py && python3 tools/ci/reexec_include_ignored.py --check
python3 -m unittest discover -s tests -v --durations=25      # verify-static
python3 -m unittest discover -s tools/ci -p 'test_prepared_*.py' -v   # verify-static
python3 -m unittest tests/test_ci_test_scope.py              # targeted example before the full suite

# Invoke Cargo, rustfmt, or discovery binaries
python3 tools/ci/format_scan.py                              # format / lint
python3 tools/check_layers.py --base origin/main             # verify-static local (tools/layers.json); CI: ${BASE_SHA:+--base "$BASE_SHA" --pull-request-merge}
bash tools/ci/check_hook_wiring.sh                           # verify-static (all target cfgs without building sunrise-client)
python3 tools/ci/check_ignored_test_rows.py                  # verify-static
python3 tools/ci/check_unreached_sources.py                  # verify-static (sunrise-game-discovery)
python3 tools/ci/check_process_tests.py && python3 tools/ci/check_nested_locks.py && python3 tools/ci/check_extras_features.py && python3 tools/ci/check_parity_harness_builds.py
```

- **PR merge flag (`--pull-request-merge`):** CI passes `--base "$BASE_SHA" --pull-request-merge` to `check_addresses.py`, `check_guards.py` and `check_layers.py` when `BASE_SHA` is set from the PR event (otherwise `--base` is omitted). This flag requires a two-parent `HEAD` whose first parent is at or descended from `--base`. For a local topic branch, use `--base origin/main`.
- **Mod API surface check (`verify`):** when heavy `verify` requires Rust checks for a PR or a dispatch with a PR input, it builds `sunrise-stage` and runs `python3 tools/ci/surface_report.py` after the build succeeds (without `--trees`, building the base commit's own `sunrise-stage` and comparing the public mod API and published loops, excluding advisory listed internals); `surface.gate` in `tools/ci/surface.toml` controls whether breaking changes without new change notes fail the job (not whether the report runs). Local form: build the head tool (`cargo build -p sunrise-stage --bin sunrise-stage`), archive the base into a fresh directory (`base_dir=$(mktemp -d) && git archive origin/main | tar -x -C "$base_dir"`), then run `SUNRISE_STAGE_TOOL=$PWD/target/debug/sunrise-stage python3 tools/ci/surface_report.py --trees "$base_dir" "$PWD"` (add `--with-internals` for an internals audit, or `--build-base-tool` when the generator changed). Dump exact `internal:*` IDs with `target/debug/sunrise-stage surface "$PWD" /tmp/out.json --with-internals`.
- **Change notes in `crates/sunrise-mod-vocab/changes/`:** `.toml` files are validated against CI's default non-internals diff and fail as stale if they name `internal:*` IDs; use `.md` for internals-only or narrative notes.
- **Layer enforcement:** `python3 tools/check_layers.py --base origin/main`. The rules it enforces are listed in the docstring at the top of `tools/check_layers.py`; the configuration is `tools/layers.json`.

## Vanilla reference

- Start parity work from the port ledger, not grep: `python3 tools/port_ledger.py issue N` (or `where
  Class.method`, `what file:line`), and `check --base origin/main` before the PR. How:
  [.claude/skills/vanilla-parity/SKILL.md](.claude/skills/vanilla-parity/SKILL.md), [docs/port-ledger.md](docs/port-ledger.md).
- Decompiled sources: `.local/minecraft/26.3/sources/{client,server}/` (`python3 tools/lab.py decompile`).
- Bytecode queries (callers through lambdas, every overload):
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
- A harness runs the oracle against the real 26.3 jars and the Rust probe on the same inputs, then compares
  their JSON. Oracle runs are cached by `tools/parity_shim.py`; `--fresh` reruns them.
- Match vanilla's `Math` results with `sunrise_core::jmath`, and vanilla's RNG with `sunrise_core::random`:
  random draws must happen in vanilla's order, in performance work too ([CONTRIBUTING.md](CONTRIBUTING.md#vanilla-first)).

## sunrise-client: where things live

`L:` = lib (`crates/sunrise-client/src/`, or `crates/sunrise-client-render/src/` for the modules that crate holds, under
the same `sunrise_client::<module>` paths), `G:` = gameplay (`crates/sunrise-client/src/game/`, organised into area
folders under `game/<area>/` declared flat via `#[path]` in `game.rs`), also in the lib in normal builds.
Gameplay keeps its `sunrise_client::<module>` imports through a self-crate alias.
Gameplay code that needs no `Game` lives in the side crate of its side: `crates/sunrise-gameplay/src/` (SG) for the
server's logic, `crates/sunrise-client-gameplay/src/` (SCG) for client-only code. `G:` keeps the `impl Game` glue
under the old names, and the two side crates never depend on each other (the `sides` in `tools/layers.json`).
`sunrise-client` is a library crate (`autobins = false`, no `src/main.rs`) whose `lib` target owns the allocator,
`entry`, `game`, and its unit tests; the thin executable is `crates/sunrise-game/src/main.rs` (`[[bin]] test = false`,
keeping the `GAME_IDENTITY` game identity marker for launcher imports and calling `sunrise_game::main`).

| Subsystem | Files |
|---|---|
| Game loop, tick, frame | `game.rs`: `fn run_with`, `fn tick`, `fn render` (thin wrapper around `render_frame`/`render_with`, `entity_geometry`, `hud_with_reload`); G: `runtime/window_handler.rs` (`impl ApplicationHandler for Game`: `window_event`, `about_to_wait`), `world/level_tick.rs` (`fn tick_world`, `fn tick_mobs`); L: `clock.rs`, `spans.rs`; `sunrise-client-input/src/window.rs` |
| Rendering | L: `gfx.rs` (`Renderer::render`), `gfx/renderer_construction.rs` (`Renderer::new`), `gfx/` (cull, occlusion, translucency, upload, timing, lod, weather, stars, end_sky), `*.wgsl`, `particles.rs`, `block_entity_render.rs`, `item_render.rs`, `fog.rs`, `clouds.rs`, `camera.rs`, `first_person.rs`; G: `interface/render_tick.rs`, `interface/item_draw.rs`, `interface/screen_backgrounds.rs` (panorama, menu blur, border wall) |
| Chunks, streaming, light | L: `streaming.rs` (worker pool: generate, light, mesh), `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `mesher.rs` + `mesher/`, `vanillagen.rs` (binds sunrise-world's generator), `lod.rs` + `lod/`, `server_terrain.rs` (a server's kept chunks), `bluemap.rs` (far terrain from its web map), `ticket_levels.rs`; G: `world/world_loading.rs`, `world/world_entry.rs`, `world/postprocess.rs`, `world/world_update.rs` (`fn update_world`) |
| World generation | `crates/sunrise-world/src/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `carver/`) |
| Block behaviour | `crates/sunrise-world/src/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `redstone.rs`, `place.rs`, `interact.rs`); G: `blocks/block_ticks.rs` (`fn tick_blocks`); `game.rs` `fn process_level_events` |
| Mobs | L: `mobs.rs` (types), `mobs/` (`ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `species.rs`, `spawning.rs`, `render.rs`, `anim.rs`); G: `mobs/creatures/mod.rs` (ticking, spawning, mob events), one file per mob family (`mobs/dragon.rs`, `mobs/wither.rs`, `mobs/villagers.rs`, `mobs/piglins.rs`, `mobs/golems.rs`...), `mobs/natural_spawning.rs` |
| Other entities | L: `entities.rs`, `projectiles.rs`, `entities/arrows.rs`; G: `entities/boats/mod.rs`, `entities/minecarts.rs`, `mobs/mounts.rs`, `entities/armor_stands.rs`, `entities/paintings.rs`, `entities/frames.rs`, `entities/fireworks.rs`, `entities/tnt.rs`, `entities/explosion.rs` |
| Player actions | `game.rs` (`fn tick_mining`); G: `items/use_hand.rs` (`fn use_hand`), `items/item_actions.rs` (`fn use_item`), `entities/melee.rs` (`fn attack`), `blocks/block_breaking.rs` (`fn break_block`), `items/hand.rs`, `items/combat.rs`, `items/consume.rs`, `items/using_item.rs` (an item in use, tick by tick), `items/potions.rs`, `items/shield.rs`, `items/crossbow.rs`, `items/trident.rs`, `items/fishing.rs`; L: `motion.rs` + `motion/` (movement), `collisions.rs`; crate `sunrise-items`: `items.rs`, `item_data.rs`, `enchant.rs`, `blocks.rs` |
| Inventory and containers | L: `menu.rs` (`AbstractContainerMenu.clicked`), `container_screen.rs`, `containers.rs`, `creative.rs`, `crafting.rs`, `recipe_book*.rs`, `stations.rs`, `merchant.rs`; G: `interface/menu_clicks.rs` (`fn menu_ops`, `fn click_inventory`), `blocks/stations.rs`, `items/trading.rs`, `items/recipe_book.rs`, `blocks/machines/mod.rs` |
| Commands | parse: crate `sunrise-command` (`args.rs`, `selector.rs`, `tree_*.rs`, `vanilla.rs`); run: G: `administration/commands/mod.rs` (dispatch) + `administration/commands/*.rs` (`admin`, `execute`, `items`, `players`, `world_cmds`...); L: `scoreboard_cmd.rs`, `data_cmd.rs`; `crates/sunrise-world/src/nbt_path.rs` (re-exported by `sunrise-game-data`), `sunrise-game-data/src/scoreboard.rs`; G: `administration/functions.rs`, `administration/command_blocks.rs` |
| UI screens | `Screen` enum in `game.rs`; L: `title.rs` + `title/`, `options_ui.rs`, `multiplayer.rs`, `pack_screen.rs`, `settings_shell.rs` (Sunrise's settings screens on it: `sunrise_ui.rs`, Sunrise Settings, hosting `graphics_ui.rs` and `extras_ui.rs`; `controller_ui.rs`), `screen_draw.rs`, `ui.rs` (widgets), `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `hud_kit/` (Sunrise overlays' `Room` and `Plate`, the collapsed page strip `tabs` that Shaders' screens use, [docs/ui-kit.md](docs/ui-kit.md)), `pad_glyph.rs` (controller glyphs, by what the button does), `chat.rs`, `tab_list.rs`; G: `interface/startup_menu.rs`, `runtime/loading_screen.rs` (the loading screen, the pre-game `App`), `interface/screens.rs` (pause, death), `interface/options.rs`, `network/lan_screen.rs`, `interface/chat.rs`, `interface/sign_edit.rs`, `interface/books.rs`, `interface/creative_screen.rs` |
| Multiplayer client | L: `net.rs` (network thread); G: `network/remote/mod.rs` (`Game::apply_packet`), `network/remote/packets_*.rs` (packet handlers), `network/remote/tick.rs` (`Game::tick_remote`), `network/remote_*.rs` (mob data, objects, projectiles, vehicles, stations) |
| LAN host / server side | L: `lan/` (`conn.rs` handshake + login threads, `hub.rs` + `hub/` play state, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`); G: `network/lan/mod.rs` (`LanHost`), `network/lan_actions/` (`turns.rs`: `as_guest`/`with_guest_swap`/`guest_turn`, `tick.rs`: `fn tick_guests`), `network/lan_*.rs` per feature, `network/lan_entities.rs` (tracked entities) |
| Integrated server | G: `world/integrated.rs` (the server thread), `world/levels.rs` (one level per dimension, `fn tick_server`), `world/server_travel.rs` |
| Dedicated server | G: `interface/dedicated/mod.rs`, `interface/dedicated/status.rs`; crates `sunrise-server-files`, `sunrise-server` |
| Save and load | L: `save.rs` (level.dat, regions, playerdata), `entity_nbt.rs`, `options_file.rs`; `sunrise-game-data/src/block_entity_nbt.rs`; G: `world/saving.rs`, `world/persist.rs` (entities), `world/world_session.rs` (opening and closing a world); `crates/sunrise-world/src/{region.rs,datafix/}`, `crates/sunrise-core/src/nbt.rs` |
| Input | G: `runtime/window_handler.rs` (`fn window_event`), `interface/play_keys.rs` (`fn key_pressed`), `interface/play_input.rs` (`fn play_mouse`, toggled keys, the hotbar wheel); crate `sunrise-client-input` (`keybinds.rs`, `keymapping.rs`, `key_names.rs`, `mouse.rs`, `input.rs`, `interaction_input.rs`, `gamepad.rs`, `clipboard.rs`); L: `osk.rs`; G: `interface/pad/mod.rs`, `interface/pad_menu.rs`, `interface/pad_craft.rs`, `interface/pointer_capture.rs` (mouse look, pointer grab) |
| Audio | L: `audio.rs` (mixer), `sound_engine.rs`, `sound_events.rs`, `music.rs`, `ambience.rs`; G: `interface/music.rs`, `interface/subtitle_hud.rs` |
| Resource packs | L: `packs.rs` + `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `sprites.rs`, `mipmap.rs`, `model_library.rs`, `font.rs`, `mobs/cem.rs` + `mobs/cem_anim.rs` (OptiFine/EMF entity models and their animations); `sunrise-game-data/src/lang.rs`; crates `sunrise-assets`, `sunrise-optifine`; G: `world/packs/mod.rs` |
| Agent control | L: `control.rs` (socket), `bin/sunrise-ctl/`; G: `runtime/control.rs`, `runtime/agent/mod.rs`, `runtime/events.rs`, `runtime/ui_control.rs` ([docs/agent-control.md](docs/agent-control.md)) |
| Mods | crates `sunrise-mod-vocab`, `sunrise-mod-api`, `sunrise-mod-package`, `sunrise-mod-host`; G: `host/mod.rs` (`OUT_DIR/game_host_generated.rs`), `native_hooks/fire.rs` (`OUT_DIR/mod_api_fire_generated.rs`, other `native_hooks/*.rs` are hand-written and kept free of `#[cfg]`/macros for `horizon-mixins`), `modding/mods/mod.rs` (content and mod load), `modding/mods/standalone.rs`, `modding/native_goal_ticks/mod.rs` (`OUT_DIR/mod_api_goal_view_generated.rs`); L: `mod_content.rs`, `mod_list.rs` |
| Sunrise extras, settings | L: `extras.rs`, `graphics.rs`, `settings.rs`; G: `interface/extras.rs` + `interface/extras/` (`mouse_tweaks.rs`, `sorting.rs`), `interface/options.rs` |

## How `Game` is organised

- A module that needs no `Game` moves to `sunrise-gameplay` or `sunrise-client-gameplay` (#2210), and its `game/` file
  keeps the `impl Game` glue under the old name. A service trait (`MobLevel`) carries a family's
  level calls, statically dispatched because it runs per mob per tick. `tools/layers.json` forbids any dependency
  between the two side crates.
- `struct Game` (`game.rs`, private; `collision`/`motion` borrow `'a` from `res`) keeps the core loop and fields. Each
  `game/<area>/<feature>.rs` (or `<feature>/mod.rs`) adds an `impl Game<'_>` block and state types. Direct area modules
  are declared in `game.rs` with an explicit `#[path]` (so an area folder is not a module). Parent source files declare
  nested submodules with `mod` and splice fragments with `include!()`.
- Find a feature's code: `grep -rn "fn tick_<thing>\|fn <verb>_<thing>" crates/sunrise-client/src/game*`;
  the file is under `game/<area>/`. Find its callers in `game.rs`'s `fn tick` and `game/world/level_tick.rs`'s `fn tick_world`.
- Mode checks: prefer `options.execution_role` (`ExecutionRole::{Client, IntegratedServer, DedicatedServer}` via
  `is_logical_server()`, `is_dedicated_server()`, and `host_side()` in `game/runtime/execution_role.rs`) and `lan_acting`
  (active guest turn). Beware: `options.dedicated` is also set to `Some(Vec::new())` on the integrated server thread
  (`game/world/integrated.rs`), so `options.dedicated.is_some()` is not dedicated-only. Other mode fields: `remote`
  (client of a server), `lan` (`LanHost`), `integrated` (window's server thread), `serving` (is that server thread).
  `fn tick` returns `tick_remote()` when `remote` is set; a server ticks its levels (`fn tick_world`).
- A server's players run through the shared gameplay code: `game/network/lan_actions/turns.rs` (`as_guest`/`with_guest_swap`/`guest_turn`)
  swaps the guest's state into `Game` for one call (plus saves/clears client UI/input fields — `keys`, `vanilla_mappings`,
  `screen`, `sign_edit`, `book`, `command_block_screen`, `mount_menu`, `message` — to catch SP side effects), and
  `game/world/levels.rs` (`Parked`, `swap_level`, `replace_level`) swaps whole dimensions while carrying server-wide
  `World` fields across swaps (`game_time`, `day_time`, `end_clock`, `weather`, `health_rules`, `watched`,
  `carrier_data_existed_at_open`). Changes reach clients through the block log, level events and the entity tracker
  (`lan/tracking.rs`, `game/network/lan_entities.rs`).
- Shared read-only data: `Resources` (`res`: blocks, items, recipes, mob types, biomes) and `PackResources`
  (`pack`), both in `game.rs`, their methods in `game/runtime/resource_tables.rs`. Integrated servers construct `Game`
  with `Audio::silent()` and dedicated servers with `Audio::for_server()`; both use `UiResources::server()` (font atlas
  and placeholder panorama), while `WorldRest` and `PackResources` mix server gameplay tables with client render/audio
  assets.

## Single player runs on a server thread

`game/world/integrated.rs` runs a second `Game` (`serving`) on a "Server thread" with one level per dimension;
the window joins it as a client through memory (`remote`), exactly as a LAN guest or a dedicated server's
player does. A window's `Game` never ticks a world itself: the old in-frame path is gone
(`--in-frame` and `--no-server-thread` only print a notice; `SUNRISE_SERVER_THREAD` is ignored).
Open `[server-thread]` issues list the gaps left.

Single player, LAN guests and dedicated servers all run the server code, so a fix there serves all three.
When you change how the server and its clients talk (packets, tracking, guests, the server thread itself), check
it there. Other gameplay fixes are covered by their own harness and CI's parity job:

```sh
cargo run --profile fast -p sunrise-game -- --resources .local/sunrise-client/26.3 --world T               # server thread
python3 tools/lan_e2e.py --check <name>                  # host on its server thread, guest is a server client
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

## Where a type or symbol really lives

| Looking for | Real home & lookup |
|---|---|
| Mod API host traits, hooks, types & hard limits | `crates/sunrise-mod-vocab/src/{host,hooks,...}/` (re-exported by `sunrise-mod-api` via `OUT_DIR/mod_api_exports_root.rs`; handle methods on `PlayerRef`/`EntityRef`/`LevelRef`/`BlockEntityRef` are generated by `#[host_area]` from `#[doc(hidden)]` `*Backend`/`*BackendMut` methods in `host/`; hooks are not 1:1 with filenames — grep `fn ` in `hooks/`; limits: `rg -n "pub const " crates/sunrise-mod-vocab/src/`) |
| Mod namespace authority & bare-ID rules | `crates/sunrise-mod-api/src/{load_state.rs,parts/owned_namespaces.rs}` and `crates/sunrise-mod-vocab/src/content/config_files.rs` (`ConfigPath::owned_by`); bare declaration names and config paths bind strictly to the mod's primary `id`, not extra manifest `namespaces` |
| Generated `impl Game` host forwarders & `fire_<hook>` | `OUT_DIR/{game_host_generated,mod_api_fire_generated,mod_api_goal_view_generated}.rs` (generated by `crates/sunrise-client/build.rs`, `Target::Client`; `#[table_backend]` in `sunrise-mod-vocab/src/host/block_states.rs` delegates off `Game` to `BlockTables`; `GoalCx` lives in `game/modding/native_goal_ticks/mod.rs`; `resident_owners!` in `game/entities/entity_residency.rs` macro-generates inherent `Game` methods) |
| `enum Dim` & `EXTRA_DIMS` | `crates/sunrise-client/src/game.rs` (private `struct Game<'a>` is here too), with extra inherent `impl Dim` blocks in `game/world/runtime_levels.rs`, `game/world/respawn_position.rs`, and `game/network/lan/mod.rs` |
| Two `struct GameLevel` types | `game.rs::GameLevel<'w>` (`LevelAccess`, built by `Game::with_level` on both server and remote client) vs `game/mobs/natural_spawning.rs::GameLevel<'g, 'a>` (natural & trial spawning) |
| `Game::ui` (`UiState`) | `game/interface/chat.rs` — not client-only: also owns server `registries`, `rules`, `dispatcher`, world clocks, `difficulty`, and `death_cause` |
| `BlockEntityAnims` & `sunrise_client::*` render types | `crates/sunrise-client-render/src/` (re-exported under `sunrise_client::*`; `BlockEntityAnims` in `block_entity_anim.rs` mixes client animations with server `gateway_servers`/`conduit_servers`; grep `crates/`, not `crates/sunrise-client/src/`) |
| `HostSide` selection & `ExecutionRole` | `crates/sunrise-gameplay/src/execution_role.rs` and `game/runtime/execution_role.rs` |
| Protocol packet enums & macro-defined `EntityLists` | `packet_enum!` in `crates/sunrise-protocol/src/packets/*.rs`; `EntityLists` inside `macro_rules!` in `game/entities/entity.rs` |
| Engine-internal UI & extension hooks | `docs/ui-kit.md` (`hud_kit`, `settings_shell`) and `sunrise_hooks::*` are engine-internal, not mod-facing (mod UI lives under `crates/sunrise-mod-vocab/src/client/` and `host/`) |

## Moving or renaming code: what else names it

- **`game.rs` declarations & `include!()`:** direct `game/<area>/*` modules need an explicit `#[path]` in `game.rs`. Parent source files declare nested submodules with `mod` and splice fragments with `include!()`; `check_unreached_sources.py` accepts both. `network/lan_actions/shared.rs` and `native_hooks/shared.rs` are unconditionally spliced via `include!()`, and `network/remote/mod.rs` `include!()`s test fragments (convert `include!()` fragments to `mod` when moving crates). `network/friends.rs` and `network/friends_off.rs` share `mod friends` behind `#[cfg(feature = "friends")]`.
- **Port map (`tools/port_map/*.tsv`):** `check_port_map_paths.py` requires an actual `fn <name> { ... }` body in the cited file (not a `pub use` re-export); update rows and run `python3 tools/ci/check_port_map_paths.py --refresh` to drop superseded lines from `tools/ci/port_map_known_stale.txt`.
- **Ignored tests & `SHARDS`:** daily execution, `tools/ci/ignored_inventory.py`, and `tools/ci/ignored-test-exceptions.json` match complete test identities (`package`, `target`, full `game::<mod>::...` path), while `rowless_problems` and `source_problems` in `check_ignored_test_rows.py` match by package and final function name. `sunrise-gameplay` and `sunrise-client-gameplay` are not in `tools/ci/ignored_shards.py` `SHARDS`, so an `#[ignore]` moved there fails `rowless_problems` until covered by a valid row mark (e.g. `manual`, `needs`, known failure, or a `daily` row paired with `#[ignore = "daily"]`).
- **Test scope & areas (`verify-static`):** `tests/test_ci_test_scope.py` checks crates for tracked external path literals recognized by `tools/ci/test_scope.py::reads()` and accepts coverage in `READERS` or `SCANNED` and exclusions in `NOT_READS` (`sunrise-prepare` also `include_str!`s `game/runtime/launch_api.rs`); `tools/test_areas.toml` (`tests/test_test_areas.py`) fails if a `paths` glob matches nothing or a `game-boot` daily test moves without an area entry.
- **Mod API forwarders, loops, mixins & proofs:** `OUT_DIR/game_host_generated.rs` forwards to inherent `Game::{name}` under `#[deny(unconditional_recursion)]` (keep a thin inherent forwarder when moving a covered method out of `game/`); `#[sunrise_loop(name = "block_entities.tick")]` (`game/blocks/block_entity_ticks.rs`) is CI-gated in `surface.json` (`loops.rs` scans only `game*`); function-replacement staging (`replace.rs`) auto-instruments `game/` (`marked_only: false`) and discovers eligible lower-crate functions from `#[replaceable]` markers (`replace.rs::marked`), while `crates/sunrise-mod-api-gen/internals.txt` separately lists declared lower-crate SDK internals for widening, discovery and export (not a function-replacement staging prerequisite); check `rg -n 'target\s*=|file\s*=' tools/ci/fn-replace-check/` (daily CI), `crates/sunrise-mod-vocab/behaviours/*.toml` (checked by `syn` in `published_behaviour_proofs_exist`), and `crates/sunrise-mod-vocab/extension-tags/villager_secondary.json` (+ `.github/workflows/modding-book.yml`).
- **Parity fixtures & live-source Python tests:** keep `#[test] fn <name>() { run_<name>(); }` parity fixtures under `game::` (`crates/sunrise-parity-fixtures/build.rs` rejects other paths) and run `python3 tools/ci/parity_fixtures_manifest.py` after any change to `crates/sunrise-client/Cargo.toml`; also grep `crates/sunrise-client/src/game` in `tests/test_*.py` (several extract live `Game` methods and compile them with `rustc` against inline mock `struct Game`), `tools/test_*_parity.py`, `tools/test_entity_shadow_ownership.py`, `tools/test_*_mutations.py`, and `docs/modding/src/concepts/native-route/`.

## Traps

- **Server sound capture & forwarding:** `Audio::silent()` (integrated server) and `Audio::for_server()` (dedicated server) leave `audio.capture` off, and `Audio::play` never sends packets itself. `start_lan_hub` (`game/network/lan/mod.rs`) enables `self.audio.capture = Some(Vec::new())` so `Audio::play` records positional sounds (and `game/interface/level_sounds.rs` records `bound_sounds`), which `lan_broadcast_effects` (`game/network/lan_actions/effects.rs`) drains and forwards to guests — never treat server `self.audio.play(...)` calls as dead client-only code.
- **Single-side filenames & hidden side crossings:** in `game/network/`, `cauldron_prediction.rs`, `lan_particles.rs`, `lan_screen.rs`, `lan_crafting.rs`, `lan/sync.rs`, and `villager_client.rs` mix server and client code; `game/modding/` files like `native_server.rs` and `native_level_refs.rs` implement both `ServerHost` writes and shared `Host`/`Backend` reads; client code in `game/` can still import `sunrise_gameplay::*` through shims while `check_layers.py` passes (`rg sunrise_gameplay:: crates/sunrise-client/src/game/`); gameplay files still share a `winit::keyboard::KeyCode` dependency (`mobs/creatures/mod.rs`, `mobs/mounts.rs` and `mobs/nautili.rs` read `self.keys`, `network/lan_actions/turns.rs` swaps `self.keys` and inserts a shift key, and `items/hand.rs` checks `self.movement_key_held(...)`).
- **Drop-order & teardown invariants:** `Game::player_autosaves` must precede `session_lock` in `game.rs`; `Streaming::drop` (`crates/sunrise-client-render/src/streaming.rs`) aborts generation jobs and drains `saving > 0` for up to 30 s, then dropping `jobs: Registration` (`crates/sunrise-threads/src/jobs.rs`) waits for running pool jobs to finish (`Streaming.shared` is declared before `jobs`, while the registered pool source holds another `Arc<Shared>`).
- **Grep & static-scan traps in `game/`:** callers of `Game` methods hide behind `&mut *enclosing.game` / `scope.game.method()`, file-local wrapper structs (`RemoteInputWorld(self)` in `game/network/remote/mod.rs`), and `Fn(&mut Game)` / `&Self` closures; field accesses include `game.field` and multiline `self` followed by `.field` on the next line (`game/blocks/spill.rs`), and several field names collide between `Game` and `GameLevel` (`world`, `border`, `events`, `spill`, `tickets`...); `#[cfg(test)]` appears mid-file on struct fields, enum variants, and imports (not only trailing `mod tests`).
- **Stale docs & path regexes:** `crates/sunrise-mod-vocab/src/` (and its `///` doc comments) is the source of truth for mod API capabilities and timing/isolation caveats — `docs/modding/src/` lags behind; `tools/port_ledger.py` still classifies remote/LAN seams with flat `/game/remote*.rs` and `/game/lan*.rs` regexes.
- **Hidden client→server static bridges & `World.dirty_sections`:** `F3+I` (`game/administration/commands/data.rs::entity_tag_in_level`), `game/network/remote/tick.rs` (calling `handle_mob_events_reusable`, defined in `game/mobs/creatures/mod.rs`), and `--control` (`game/runtime/station_control.rs::station_command`) statically reach server-heavy dispatchers; `crates/sunrise-client-render/src/streaming.rs` explicitly clears `World.dirty_sections` when meshing is disabled (headless servers), while simulation/mod test suites still assert `dirty_sections` (`rg dirty_sections crates/sunrise-client/src/game/`).

## Docs worth knowing

[docs/parity-workflow.md](docs/parity-workflow.md) (runner, gate, shards, scopes) ·
[docs/native-client.md](docs/native-client.md) (options, controls, sunrise-ctl) ·
[docs/server-levels.md](docs/server-levels.md) (levels per dimension) ·
[docs/status.md](docs/status.md) (what is ported) · [docs/known-issues.md](docs/known-issues.md) ·
[docs/release-flow.md](docs/release-flow.md) · [docs/README.md](docs/README.md) (index).
