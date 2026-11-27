# Parity workflow

How parity work is run, checked and merged. What is ported and where it is checked: [Sunrise status](status.md) and [port ledger](port-ledger.md).

## Datafix downloads and author self-checks

Datafix generation owns a private writable root, defaulting to checkout `.local/datafix`.
`SUNRISE_DATAFIX_ROOT` overrides it (relative values resolve from the invoking working directory).
Minecraft input symlinks never select this output root. The tool refuses shared vanilla
roots and their ancestors, escaping default symlinks, escaping descendant symlinks, file roots and unwritable
ancestors before downloads, world generation or `generate.lock` creation. Choose a new private
directory on refusal; never change permissions on shared fixtures to bypass the guard.

Author `pr_ready` excludes datafix by default and prints one opt-in reminder. To explicitly
allow it among the affected harnesses, use a private directory. Local opt-in preflights the
root conservatively even when affected dependencies are unknown, and passes its validated
absolute path into subprocesses so a working-directory change cannot reinterpret it:

```sh
SUNRISE_DATAFIX_ROOT="$PWD/.local/datafix-private" python3 tools/pr_ready.py --datafix
```

Direct `tools/parity.py datafix` is also an explicit harness run and may download jars and
runtimes, launch servers and generate worlds. Metadata for cold Linux 1.8.9 alone lists
8,320,755 bytes for the server jar plus 235,129,362 bytes for the pinned legacy runtime:
243,450,117 bytes before JSON metadata and generated worlds. Other versions/runtimes add
more; cached payload can be zero. No runtime measurement is available for this repair.
The fork's active Dyna gate does not currently run datafix; upstream-style source selection
is not proof of active fork CI coverage. Fork issue #3027 tracks the separate coverage decision.

## Running the vanilla harnesses

Same-machine fixtures created with `lab.game_env` use `SUNRISE_TEST_LOOPBACK=1`: Sunrise
listens on loopback and does not scan or announce LAN worlds. This avoids Windows Firewall
permission prompts for newly compiled fixture executables. Set `SUNRISE_TEST_LOOPBACK=0`
explicitly when testing real network interfaces or multicast discovery. Local vanilla server
fixtures also set `server-ip=127.0.0.1`; use one installed JDK via `JAVA_HOME` and its `bin`
on `PATH`, rather than copying Java to each fixture directory. Keep the firewall enabled.

`tools/parity.py` runs the `tools/test_*_parity.py` harnesses in parallel and reruns only what a change can affect:

```sh
python3 tools/parity.py              # harnesses whose inputs changed, or that never passed
python3 tools/parity.py food light   # these harnesses, whatever their state
python3 tools/parity.py --failed     # the ones that failed last time
python3 tools/parity.py --all        # everything, ignoring earlier passes
python3 tools/parity.py --all --skip-named-only  # the daily’s automatic set
python3 tools/parity.py --fresh food # don't reuse vanilla oracle results
python3 tools/parity.py --repeat 3 food   # run it three times; the runs must agree
python3 tools/parity.py --hosts macmini,playsunrise   # with other machines (default: tools/parity_hosts.json's default list, empty)
python3 tools/parity.py --list       # what would run
python3 tools/parity.py --status     # last result of each harness
python3 tools/parity.py --changed origin/main      # those a branch's changes can affect
python3 tools/parity.py --changed-files paths.txt  # the same for listed paths, in a copy without Git (tools/pr_ready.py)
```

CI runs the automatic harnesses only in the [daily on main's head](release-flow.md#parity-on-main), not on pull requests or rollups. The daily posts harness names and outcomes; its logs stay private on the runner. One selection, several shares: the plan job computes the selection once from the tested commit and splits it (`tools/ci/parity_shards.py` names the count), each runner runs exactly its share (`tools/parity.py --harness-file`), and `tools/ci/parity_verdict.py --merge` turns the shares into the one file the daily summary reads, refusing a run whose shares miss a planned harness or run one twice. A commit of main from before the shares runs as one job, as it did then.

The Linux daily also runs `save_kill` and `save_corruption` through this runner, like
`dedicated_crash_save`. To run them directly on Linux with prepared resources, use
`python3 tools/test_save_kill_parity.py` and `python3 tools/test_save_corruption_parity.py`.
They boot a dedicated server in disposable worlds and exercise its real save and load paths.
The kill check uses `SUNRISE_CRASH_FIXTURE` to stop a writer at a named publish boundary,
then sends SIGKILL and reopens the world. This tests process death, not disk power loss.
The corruption check changes copies of the saves and checks recovery, refusal, or chunk regeneration.
Neither runs in the PR or rollup gate.

Each harness runs unmodified under a tracer that records the files it depended on: its Python modules, its Java oracle and the oracle sources javac pulled in, other repository files it read, and the Rust sources of the probes it ran (from Cargo's `.d` dep-info files). A pass stands until one of those files, the pinned vanilla artifacts, `Cargo.lock` or the Rust/Java toolchains change. State and logs are in `.local/parity-runner/`; each harness's own outputs stay in its `.local/<name>/` directory.

Vanilla oracle runs are reused as well. `tools/parity_shim.py` stands in for `java` and `javac`: it records each oracle run's outputs and the content of every file under the paths its arguments name, and replays the run while those are unchanged. After a Rust-only change a harness pays only for its Rust side (`block_update`: 53 s to 5 s). Files an oracle writes outside its named paths aren't tracked, so a harness that fails while reusing oracle results is rerun fresh, and stops reusing them if it then passes. `--fresh` never reuses them; `--all` allows reuse only when
`PARITY_ORACLE_KEEP` is set, naming a folder that outlives the checkout, which the shim keeps runs in besides the
checkout's own cache. The daily names its CI host's keep (`$DYNA_CI_KEEP/oracles`). Replay applies only to the launches the
shim supports (`java` and `javac` found on PATH, stdin not a pipe) and to the input files it tracks: those under the
paths the arguments and the oracle variables name. A run is replayed only for the same command, JVM option and oracle
variables and JDK content on byte-identical tracked inputs, while every harness's Rust side runs. With a keep, only
`$JAVA_HOME/bin/java` and `javac` themselves are reused, from either cache: one reached through a wrapper or JDK selector
always runs, since the JDK behind it can change. A kept run is replayed only where none of its outputs exists yet. A harness
that failed with reuse enabled and then passed fresh runs fresh for 30 days.

Probes build into `.local/parity-target/` with release optimisation but no LTO and 16 codegen units. On ludodesktop that rebuilds every `sunrise-world` probe in 3 seconds after a one-line change, against 122 seconds with the release profile. Rust gives these settings identical results (no fast-math; overflow checks as in release), and the full harness suite passes under them. Probes that harnesses build in the dev profile get `opt-level = 3` too, keeping their overflow checks and debug assertions. Harnesses must locate binaries through `cargo metadata` or `lab.cargo_target()`, never a hard-coded `target/`.

The probes of `sunrise-world` and of `sunrise-client` are one binary per crate, `world_probe` and `client_probe` (`cargo build --release -p sunrise-world --example world_probe`): a harness runs `world_probe <name> [arguments]`, and `<name>` is a module in `crates/<crate>/examples/probe/` (add it to the `mod` list and the `probes!` list in its `main.rs`; read arguments with `crate::args()` or `crate::args_os()`, which drop the name). Code several probes share goes in `examples/probe/support/` or a module of its own. Linking one binary instead of ~250 examples is most of what a probe build used to cost. Each crate's probe has its own target name (set in its `Cargo.toml`, `protocol_probe` too): Cargo copies every crate's examples into one `<profile>/examples/` directory, and with one shared name a harness could run the binary another harness had just copied there. The runner counts only the probes a harness ran, the probes and support modules they name, their own subdirectories and `dispatch.rs` as its dependencies, not `main.rs`, which only lists the probes: editing one probe reruns only its harnesses, and adding one reruns none.

Passes are shared between worktrees. A harness's key depends only on the content of its dependencies, not on where they are, so every pass is also written to `parity-shared/` beside the shared vanilla data (`PARITY_SHARED` overrides it). A worktree whose files match those a harness passed with, in any worktree, counts that pass without running the harness: after merging the integration branch, an agent's gate only runs the harnesses its own changes affect.

All runners on a machine share `PARITY_MACHINE_JOBS` slots (default: half the CPU threads), so several agents' runs together can't overload it. Parallel harnesses are also admitted by memory: each harness's recorded peak is reserved, within three quarters of the memory available at the start, and a harness starts only if the memory free at that moment still leaves 2 GB. `-j` caps the count here (default: `PARITY_LOCAL_JOBS`, else 3 while other machines help or a third of the CPU threads alone, within `$FLEET_LOAD_JOBS`).

### Harnesses that start the game

The game logs into `SUNRISE_HOME` and falls back to the player's own game folder when it is unset, so a harness that starts the game, `sunrise-ctl`, the launcher or the dedicated server builds the environment with `lab.game_env(**extra)`: no audio, and fresh scratch folders for `SUNRISE_HOME`, `SUNRISE_LAUNCHER_HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, removed when the harness exits (a folder you pass, or that the environment already has, is kept). The runner also gives each harness its own `SUNRISE_HOME` under `.local/parity-runner/game-home/`. `tests/test_game_home.py` fails for a script in `tools/` that starts the game without it.

`frame_identity` (`tools/test_frame_identity_parity.py`) holds the picture where a change shouldn't move it. Its oracle is the base build, not vanilla: pictures of the game hold Minecraft's textures and can't be committed. It builds `BASE_SHA` (else the merge-base with `origin/main`) in a worktree and target folder of its own under `.local/frame-identity/`, kept between runs so a later base rebuilds incrementally, draws fixed scenes offscreen with each build under `SUNRISE_STILL=1` (HDR off; the Golden Hour pack in SDR and with offscreen HDR output; HDR turned off in a running game), and requires identical pixels, writing `diff-<scene>.png` where they aren't. Both builds draw on lavapipe where Mesa's Vulkan driver is installed, else on the machine's default adapter. When the change touches only `tools/`, `tests/`, `.github/` or Markdown outside `crates/`, the base reuses the change's binaries (`--build-base` builds it anyway), so the run checks that two runs of one build agree. It is `named_only` in `tools/parity_hosts.json`, since it fails any change that moves the picture on purpose; a change that must leave the picture alone runs it by name with `python3 tools/parity.py frame_identity`.

### Other machines

Harnesses can also run on the Mac mini, the desktop and playsunrise (`tools/parity_hosts.json` describes them; `desktop` and `playsunrise` use their CI runner's environment and vanilla data, the desktop's entry describes its retired setup and refuses work; its `default` list is empty, so pass `--hosts macmini,playsunrise` or set `PARITY_HOSTS`, after copying `shared-local/` to `~/code/sunrise-parity/shared-local` on each host). The runner mirrors the worktree there with rsync (not `.git`, `.local`, `target` or ignored files) into `~/code/sunrise-parity/remote/<host><path>`, whose `.local` links to the vanilla data in `~/code/sunrise-parity/shared-local`, and starts `tools/parity.py --worker` over ssh. The worker builds the probes (about 80 s the first time) and runs what it is sent under the same tracer, admitting harnesses against its own free memory. One queue, longest first, feeds every machine, remote ones first: the Mac takes up to 14 harnesses, the desktop 8, playsunrise 3 (on CPUs 0-3: its CI runner has 4-7). Each pass comes back with the content hash of every file it depended on, so it is keyed on exactly what ran, and logs come back to `.local/parity-runner/logs/`; state, sharing and `gate.py` work as before. If a host can't be reached, or drops out, its harnesses run here.

Harnesses a host can't run are listed in the file: `local_only` (`save_roundtrip` runs the game on a virtual screen) and each host's `exclude`. A host's `exclude` also holds when `parity.py` runs on that host itself, as its CI runner does: the harness is skipped there with its reason (playsunrise skips `remote_fireball` and `remote_fireworks`, which its CPU-starved CI slice fails even on bare main), unless named on the command line. The daily uses Dyna’s Linux runner instead. The Mac excludes nothing: vanilla's `Math.sin`, `pow`, `log` and the like don't round the same on an Apple silicon Mac as on Linux (HotSpot uses fdlibm there, Intel's stubs on x86-64), and Rust's float functions round like neither (they call the platform libm), so code that mirrors a vanilla `Math` call uses `sunrise_core::jmath`, which computes what HotSpot computes on the host architecture. The ThinkPad no longer runs harnesses (2026-10-26): playsunrise is the Linux host that is always on.

The file's `solo` list names harnesses that drive game windows (`remote_fireworks`, `server_player_sculk`, `world_reopen`, ...). On a machine where `parity.py` has 4 CPUs or fewer, such as playsunrise's CI runner, each of them runs alone, after the others: there frames are drawn on the CPU, and beside other harnesses a screenshot or a control request took more ticks than the check allows. `PARITY_SOLO=1` or `0` forces this on or off.

### Any command on playsunrise: `tools/remote.sh`

For Linux work that doesn't need a whole parity run (a crate's tests, a few harnesses, a server build), `tools/remote.sh <worktree> <command...>` runs the command on playsunrise at the worktree's commit and streams its output back; its exit status is the command's. Commit and push first: the host fetches the commit from GitHub, and nothing else leaves this machine (no files, no credentials, no environment). Each worktree keeps one checkout there, `~/remote-work/<name>` with its own `target/`, so the next run only rebuilds what changed, and one run per name goes at a time. The command runs niced in the host's CI slice, pinned to CPUs 0-3 (`--cpus`): the CI runner has 4-7, so these runs do not slow the other runner tasks. A benchmark asks for its CPUs with `--cpus 0-3 --exclusive` (and says so in its pull request): it starts only when no other run uses them, and later runs wait for it to end. The command gets scratch game and launcher homes and the vanilla data linked as CI links it; a command naming `parity`, `lan_e2e` or `sunrise-prepare` also gets `.local/sunrise-client`, prepared as CI prepares it. Whatever the command leaves running (a game, a daemon) is stopped when it ends, and stopping `remote.sh` (or losing the connection) stops the command on the host. Logs stay in `~/remote-work/<name>.logs/`; `--list` shows the checkouts, `--gc [DAYS]` removes old ones.

```sh
tools/remote.sh .claude/worktrees/my-fix cargo test -p sunrise-world --lib   # 33 s, warm
tools/remote.sh . python3 tools/parity.py food                               # 19 s warm (4 min the first time); here: 188 s, 229 CPU-seconds
```

The host is lent by a partner who keeps hypervisor access: send it no secret, token or credential, and never take a result from it as a CI result.

### Shards and Rust scopes

A harness can declare, at the top level of its script:

- `PARITY_SHARDS = N`: it runs as `NAME#1` to `NAME#N`, in parallel, each told its share in `PARITY_SHARD=i/N` (`test_datafix_parity.py` upgrades every sixth world). `NAME` on the command line or in `--known-failures` means all of them.
- `PARITY_RUST_SCOPE = [...]`: the Rust sources its probes can reach, as path prefixes. Cargo's dep-info names every file of every crate a probe links, so without it a probe of `sunrise-world` depends on all 350 of its files. The runner keeps only the scope's files if the scope is closed: nothing in it names a module, crate-root re-export or macro outside it (`crate::`, `super::`, other workspace crates; comments and strings ignored), declares a module it doesn't cover, or includes source, and nothing outside it implements anything for a type it defines. Crate roots then count without their declarations of other modules, so adding a module elsewhere doesn't rerun the harness. If the scope isn't closed the runner says why and keeps every file. `datafix` depends on 41 files this way, not 354.
- `PARITY_MANUAL = "<how to run it>"`: the script is a proof someone runs by hand with arguments (a Java side, then a native side against its reference), not a harness. `parity.py` leaves it out of every run, the daily included, and names that text when it is asked for it by name.

### Which harnesses a change affects

`--changed` and `--changed-files` narrow each harness's Rust sources to those its probes can reach (`tools/parity_reach.py`), so a change to one module of `sunrise-client` no longer reruns every harness whose probe links the crate. The narrowing works on items, not files: from the probe modules a harness ran (or the binary it built), a free item (fn, type, const, static) is reached when a reached item names it in its own module, through a path or an import of a module that is named, or through a glob import; an inherent impl's methods count one by one once their type is reached and their name is mentioned; a trait impl counts once its trait and its type are reached (std's traits and types always are); a macro counts once invoked. Names match by spelling, so the result only over-approximates. A reached file's parent modules count through their `mod` declarations of it, compared with the merge base; Rust files a change adds take part, a dependency that was deleted counts as changed, code under `#[cfg(test)]` never counts, and a harness whose probe can't be found keeps every source. Parses are cached by content in `reach-cache.json` of the shared parity folder (beside `passes/`), so a new worktree starts warm; a harness's search stops as soon as it reaches a changed file, and after `PARITY_REACH_BUDGET` seconds (default 180) the remaining harnesses count every source their probes link, so choosing never holds a slow machine for long. `--by-crate` turns the narrowing off. With `--changed`, `Cargo.lock` and `Cargo.toml` changes that only add, remove or move workspace crates (a crate split: no registry or git package, version, feature, dependency or build setting changed across them, compared with the merge base) don't count: the moved code counts through its sources. A workspace crate's feature that such a change defines or turns on is allowed only where every source naming it changed too (the new crate's own files). #2160's split went from 352 harnesses to 123 this way (its own changes to the runner still select them all). On the last 20 pull requests merged on 2026-10-25, the harnesses picked went from 2255 to 1061 (2384 to 1592 harness-minutes; game-binary harnesses, which link all of `game/`, set the floor), and every harness a CI parity run failed in the week before was still picked; every source file a sample of probes executed (LLVM `-C instrument-coverage`) is among those kept. Building the graph takes 15 s cold, under a second warm, and up to a minute and a half on a change that reaches many harnesses.

## The gate

`tools/gate.py` is what a branch must pass before it is integrated: the public-tree check, rustfmt, clippy with warnings denied, the Rust and Python tests, then the harnesses whose inputs changed. It stops at the first failure and writes `.local/gate.json`. `--known-failures a,b` lets harnesses that already fail on the integration branch keep failing. `--quick` checks formatting, lints and Python tests only; `--no-parity` skips the harnesses.

The gate also looks for flaky harnesses: a harness whose script, oracle (Java under `mods/sunrise/src/test`) or probe (a crate's example or binary) is new or changed since the merge base with `--flake-base` (default `parity-base`) must pass three runs in a row. The later runs reuse the first one's oracle results, so they cost only the Rust side, and run on the same machine; a harness whose runs disagree is reported `FLAKY` and fails the gate. A pass records how many runs it held over, so a later gate doesn't repeat it.

## Parallel work: worktrees and the integration branch

Parity work happens in one worktree and branch per area, next to an integration worktree that has the integration branch (`parity-base`, pushed as `t3/achieve-full-parity`) checked out. All worktrees share the vanilla data through `.local/` links to a `shared-local/` directory beside them.

```sh
python3 tools/supervise.py status                       # branches, progress, unpushed work, last gate
python3 tools/supervise.py new dispensers               # new worktree and branch from the integration branch
python3 tools/supervise.py save                         # commit and push unfinished work before agents stop
python3 tools/supervise.py check                        # which branches merge cleanly, alone and together
python3 tools/supervise.py integrate parity/dispensers  # merge, gate, push; undo the merge if the gate fails
python3 tools/supervise.py integrate --ready            # every parity/* branch with new commits, one gate
```

An agent working an area merges the integration branch first, uses `tools/parity.py` on its harnesses while it works, runs `tools/gate.py` before its last commit and pushes its branch. Integration happens only through `supervise.py integrate`, which runs the gate with the harnesses already failing on the integration branch as known failures: a branch may leave those failing, or fix them, but must not make any other harness fail.

`check` and `integrate` start by trial-merging the candidates with `git merge-tree`, in the object database, without touching any worktree: each branch alone, every pair, then the largest set without conflicts, merged in turn. That takes about a second for ten branches and names the conflicting files. `integrate` merges that set, gates it once (formatting, lints and tests before the long harness run; the flake check covers everything the branches change) and pushes. If the gate fails it tries each half of the set in turn, so the branches that pass still land; `--no-bisect` turns that off.

On ludodesktop, source `~/code/sunrise-parity/env.sh` first: it sets the ALSA build path and the Cargo job cap. Debug builds carry line tables only (`[profile.dev]` in `Cargo.toml`), which keeps backtraces readable while keeping each worktree's build small; a fresh worktree builds `sunrise-client` and its tests in under a minute.

## sunrise-client fixtures

A harness that needs a sunrise-client fixture runs it from the `sunrise-parity-fixtures` program, which
`lab.parity_fixtures()` builds (`default_features=False` for the vanilla build) and returns. Its command line and
report are libtest's: filters, `--exact`, `--list`; a filter that selects nothing exits 101. Its package,
`crates/sunrise-parity-fixtures`, a dev tool, compiles sunrise-client's sources as the program with `cfg(test)` set,
so test-only code compiles while the `#[test]` functions stay out. It takes sunrise-client's dependencies and
features with its dev-dependencies, which sunrise-client itself may not ship (docs/layers.md); its manifest is
written by `tools/ci/parity_fixtures_manifest.py`, which you rerun after changing sunrise-client's. A harness never builds the `sunrise-client` lib test: its one
rustc outgrows a parity runner's memory, and `tests/test_dyna_ci.py` refuses a harness that does.

A fixture is a `#[test]` under `game` whose body is one call to a function beside it named `run_<test>`;
the package's `build.rs` finds every such test and lists it, so nothing is registered by hand. List them with
`$(python3 -c 'from tools import lab; print(lab.parity_fixtures())') --list`.

## Server execution in game fixtures

`SUNRISE_COMMAND_SCRIPT` runs on the integrated server's `Server thread`, with the
headless client joined through the normal memory connection. Commands use the connected
owner's body and dimension. The script owns the clock: each `tick` calls `tick_server`,
while login and portal arrival pump connection-only passes. The client acknowledges
terrain and teleports without predicting movement, matching the command oracle's stationary
player. Script and save errors return through server shutdown; output keeps the existing
styled messages and `CommandResultCallback` schema.

`SUNRISE_DISPENSE_FIXTURE` and `SUNRISE_SCULK_FIXTURE` are component probes. They construct
a server `Game` on a server thread and retain the oracles' synthetic levels, seeded random
streams and explicit entity/block phases. A synthetic sculk player controls warning levels,
not client input; dispenser probes call one activation without running unrelated world ticks.
Keep those controls when extending the fixtures. Player interactions belong in connected-client
coverage alongside the components:

```sh
python3 tools/test_command_exec_parity.py
python3 tools/test_dispenser_parity.py
python3 tools/test_sculk_parity.py
python3 tools/lan_e2e.py --check machines --check fireball --check sculk_container
```

The LAN checks open and fill machines through the guest's inventory packets, observe a
powered dispenser's projectile on both clients, and open a chest to activate a sculk sensor
and shrieker. They supplement the deterministic component comparisons.

### Randoms that repeat

Vanilla seeds every unseeded random (`RandomSource.create()`: each entity's own, the level's, a raid's...) from `RandomSupport.generateUniqueSeed`, a shared uniquifier XORed with `System.nanoTime()`, and numbers entities from `ServerLevel.ENTITY_COUNTER`; Sunrise ports both (`random_source::unique_seed`, `Game.server.next_entity`). A command script can make what they decide comparable exactly: a dropped item's position, primed TNT's kick, a dispensed item's motion, which tick of four a resting item moves on.

- `{"op": "sync_randoms", "uniquifier": U, "entity_id": E, "level_seed": S}`, run on the server thread of both engines, sets that thread's uniquifier to `U`, the entity-id counter to `E` (the next entity is `E + 1`) and the random of the player's level to seed `S`. Startup creates a different number of randoms in each engine, so sync right before what you compare, and give each case its own values (Sunrise keeps an entity's UUID by id).
- A script with this step runs vanilla under `tools/seed_agent.py`'s agent (`SeedAgent.java`, built with the JDK's class-file API): `generateUniqueSeed` steps a uniquifier of the calling thread's own, from vanilla's start and by vanilla's factor, without the clock. Sunrise runs with `SUNRISE_TEST_SEEDS=1`, which does the same in `unique_seed`; the step fails without it. Per thread, so worker threads and Sunrise's client don't shift the server thread's seeds. The agent jar is an oracle input (`tools/parity_shim.py`).
- `tools/seed_sync_corpus.py` is the example and the proof, a harness of its own (`python3 tools/parity.py seed_sync`): a script that syncs runs on fixed seeds to its end, so the groups after a sync would draw from them, and a group that passes on most seeds could fail on those every time. `--unsynced-randoms` leaves the steps out, and the same fields then differ run to run.

One draw too many or too few shifts every later value, so read the first field that differs. Not covered: randoms on other threads (worldgen and chunk loading: keep scenarios in loaded chunks), the levels other than the player's, randoms created before the step (the server's, the level's sound seeds) except the level's own, `Level.randValue` (random-tick positions) and other `ThreadLocalRandom` or `Math.random` draws, and anything a tick draws from the level's random before what you read (each tick shuffles the chunks near players with it, even with `spawn_mobs` off).

### State digest

A build with `--features sunrise-client/state-digest` (with `-p sunrise-game`, or `-p sunrise-client --features state-digest`; `sunrise-game` does not forward `state-digest`) and `SUNRISE_STATE_DIGEST=<file>` appends one line per level tick: hashes of the entities (join order), their own randoms, the block entities (ticker order) and the scheduled ticks, plus the level random, `Level.randValue`, the entity-id counter and the join counter. Off by default and compiled out. `digest-sync U E S` on a dedicated server's console is `sync_randoms` (needs `SUNRISE_TEST_SEEDS=1`), and `SUNRISE_STATE_DIGEST_DETAIL=1` adds a raw line per entity and block entity. It is how a refactor proves it changes no behaviour: the same scene on the base and on the branch, identical on every tick.

- `tools/state_digest_scene.py BINARY RESOURCES TEMPLATE OUT` runs a fixed dedicated-server scene (frozen before the first tick, `tick step N`) and writes `OUT/digest.txt`; `--generate` makes the forceloaded flat template once. Random ticks are off unless `--random-ticks`. `SUNRISE_SCENE_GATE` names an optional script to wait on before each run.
- `tools/state_digest_players.py BINARY RESOURCES TEMPLATE OUT BOTS` is the same scene with two scripted `sunrise-bots` players (`as_guest`, the level swaps); `--nether` sends one to the Nether, and `--compare A B` compares two runs level by level. The players settle on a wall-clock number of frozen ticks, so the scene pins the randoms again after the settle and only then summons what they can reach; their final `players.txt` still depends on the settle and is reported, not compared; `--integrated` (a LAN guest on a server thread) has no `digest-sync`, so its digest is not comparable run to run either.
- `--coverage` on either scene tool adds commands for code the default scene never runs (foxes, dolphins and allays that pick up items, a fox that spits out what it held, mobs that pick up loot, tridents, a furnace, a brewing stand, a village with five claimed beds for the cat, a saved wandering-trader delay that runs out inside the window, the siege spawner, and the gamerules and night time they need) and makes `--ticks` default to 2500; the default scene's commands are unchanged, so earlier digests stay comparable. With `state_digest_players.py` Alice stands in the village and gets a Loyalty trident back, and her pickup is placed beside her. After the run the tool writes `OUT/coverage.txt` (each spot reached or missed, the saved trader delay and chance, Alice's pickup stats) and fails when a spot was missed; `state_digest_compare.py` and `--compare` compare that file too. The two-player `--coverage` run is not reproducible run to run (two runs of one binary part within a few hundred ticks, as the bots' moves land on wall-clock ticks over the long window), so it checks its spots and its digest is reported, not compared; the playerless `--coverage` run is reproducible and is the one compared.
- For gameplay that moved to `sunrise-gameplay` or `sunrise-client-gameplay` (#2210), the digest checks only what it hashes, in the dedicated-server scenes above: entity motion, health and sampled random states, block-entity data, counters and scheduled ticks, plus `coverage.txt`. It does not see the client (rendering, occlusion and particle culling, prediction), sounds, packets, saved stats beyond `coverage.txt`, or the LAN-host run, which cannot pin its randoms; check those with their own harnesses, `lan_e2e` or the game. Harness selection takes candidates from the crate graph (both crates are path dependencies of `sunrise-client`), but an older dependency record that names the old path keeps only the recorded paths when its probe cannot be found (`tools/parity_reach.py`), so read the selected list (`tools/parity.py --changed-files paths.txt --list`) or name the harness.
- `tools/state_digest_compare.py A/digest.txt B/digest.txt` names the first differing step per group, the game time (`tick`) and the level (`dim`) of each step included. Compare runs from one toolchain: the hashes are not stable across Rust versions.
- Keep the measured window free of chunk loading, generation and lighting, and of trial spawners, ominous item spawners and `/item modify` (clock-seeded UUIDs and loot random).

### Fuzzing scenarios

`tools/parity_fuzz.py` runs generated scenarios in vanilla and Sunrise and reports each divergence group with a shrunk reproduction. A batch of scenarios runs as one command-exec script (`test_command_exec_parity.py --groups-file`). It is a separate tool: `parity.py` never selects it.

```sh
python3 tools/parity_fuzz.py --smoke --binary target/fast/sunrise-client  # fixed scenarios that must match
python3 tools/parity_fuzz.py persist --seed 1                            # summon or place, unload and reload, read
python3 tools/parity_fuzz.py spawn --seed 1                              # each spawn mechanism, read tick by tick
python3 tools/parity_fuzz.py --replay scenarios.json                      # run saved scenario steps, not a regenerated seed
python3 tools/parity_fuzz.py --promote group.json --name case-name         # save passing reproductions
python3 tools/parity_fuzz.py --corpus                                     # replay all saved cases
```

- **Scenarios.** A scenario stores its seed and steps: summon or place with NBT, run a command, tick, activate a dispenser, reload a chunk, or read saved state. Each gets a forceloaded chunk away from the player. Both engines tick to the same game time before syncing their randoms ("Randoms that repeat"). A reload removes the forced ticket and ticks with chunk I/O enabled. A guard checks that the chunk is no longer loaded and vanilla has released its pending unload before it is forced back in; Sunrise's own unload is not read directly, it is inferred from the `/execute unless loaded` result. The load waits for terrain, lighting and entities without advancing game time. Summoned entities get fixed UUIDs.
- **Families** (`tools/parity_fuzz_scenarios.py`). `persist` reads an entity or block entity with changed fields before and after a reload. It discovers vanilla's saved fields and caches them as `defaults.json` in the work directory; a crashed discovery is not cached. The default field order and set of types affect generation, so keep the steps when sharing a case. `spawn` uses its own tables, then reads the new entity across ticks. Item-drop reads observe all items in the scenario area as an unordered collection, preserving duplicate stacks: the order of entities in an area query is not something the scenarios pin down, so an order difference is not reported here.
- **Comparison.** A `/data get` read is parsed as SNBT and compared field by field. Key order and the order of `attributes` records do not matter. Other commands compare like the harness (feedback, styles, results). Each batch also runs the same script in vanilla alone: a read that differs between the two vanilla runs is counted as `vanilla-unstable`, not compared against Sunrise.
- **Grouping and shrinking.** Each diverging read is grouped by family, subject type, whether the read came after a reload, and first differing field. Each group's smallest scenario is shrunk (steps and NBT keys dropped, tick counts halved) while that group's divergence stays, trying all edits of a round in one batch. `report.json` in the work directory lists the groups, largest first, each with its shrunk and original scenario, rendered commands, and differences. This report includes vanilla output; do not publish it.
- **Promotion and corpus.** After fixing a divergence, save its group from the private report as JSON and promote it in the fix PR. The tool replays both the shrunk and original inputs, refuses anything that does not match, and writes one JSON file under `tools/fuzz_corpus/` with only the input scenarios and provenance. You can also promote a standalone saved scenario; it supplies both inputs and has no known divergence signature. `--corpus` discovers the files in that directory and replays both inputs of each case together. It exits nonzero and names any cases that did not match. The corpus is report-only, not a `parity.py` harness or release gate.
- **Daily report.** On a writing run of main's head, `parity-fuzz.yml` runs after vanilla parity. It runs smoke, the promoted corpus, the scenarios in open fuzz issues, and a fresh seed window for each family derived from the UTC date. It shrinks only new groups, within a cap. The fuzz issues are leads: replay the input and check the field and random stream before changing the game. A passing replay closes an existing issue unless the run still sees the same signature. The raw report and logs stay on the runner under `fuzz/` in its keep folder (`$DYNA_CI_KEEP`, else `keep/` under `$DYNA_CI_BUILD`); runs older than 14 days are removed; the `fuzz-scenarios` artifact has only scenario inputs and signatures. Fuzz results never hold the SDK or Nightly.

### Fuzzing the shader-pack translator

`tools/shader_fuzz.py` is a robustness fuzzer of `crates/sunrise-shaders`: no reference picture, only invariants every load must keep. The work is done by the `shader_fuzz` example of `sunrise-render-shaderpack`, the crate that holds the game's own pack inputs (`pack_atlas::game_inputs`, which `Renderer::pack_inputs` also calls). It adds no dependency.

```sh
python3 tools/shader_fuzz.py --smoke                                  # the seed programs unmutated, which must pass
python3 tools/shader_fuzz.py --seed 1 --count 500                     # cases from the committed seed pack only
python3 tools/shader_fuzz.py --corpus ~/shaderpacks --budget 1200 --jobs 4   # as many cases as 20 minutes make
python3 tools/shader_fuzz.py --replay 1234567 --corpus ~/shaderpacks --time-limit 20 --memory-limit 2048 --size-limit 16  # a group's case again, as the run prints it
```

- **Run controls.** `--budget` alone generates cases until the budget ends, `--count` alone that many, both until the first is reached; with neither, 200 cases.
- **Corpus.** The hand-written seed pack (`crates/sunrise-render-shaderpack/examples/shader_fuzz/seeds/`), and with `--corpus` every pack (zip or folder) of a local folder. Third-party packs are read, never committed. The run only lists the corpus (names and paths, as `sunrise_shaders::candidates` lists a `shaderpacks` folder). Each of the `--jobs` case processes opens and reads the pack of each case it makes, within `--memory-limit` and `--time-limit`: a pack that exhausts memory, hangs or crashes the loader while it is read is that case's failure (step `sample`), and the run goes on; a pack that doesn't open, a dimension without programs, or a pack the case process can't write to its folder (a file name the run can't write, a file beside a folder of the same name; printed) makes no case: each is printed with why, and counted under `skipped` in the summary and `report.json`. A run in which no case ran fails (exit 1), as one with failures does. A case takes one pack, whole and as it is, one dimension's folder (as the game picks it), one or two of its programs (both stages, or compute shaders only) for the steps to edit, and flat or grid terrain inputs. Unmutated, it loads exactly as the pack does: every file the loader reads is there, and so is what the other programs decide for the case's programs (a shadow pass a reader of the shadow map turns on, terrain drawn in two parts, a fallback, buffer formats). The steps edit the programs' stage files and what they include, as the loader's preprocessor finds it. Terrain inputs follow the pack's `separateAo`, as the game sets them.
- **Steps.** One to six mutations, each named and drawn from its own SplitMix64 seed: `declaration` (mixed declarators, `struct S {…} a, b;`, anonymous structs, interface, uniform and buffer blocks, arrays on the type or the name), `include` (chains past the preprocessor's depth, cycles, diamonds, paths out of the pack), `preprocessor` (deep and unbalanced conditionals, recursive and doubling macros, odd directives), `array_size`, `collide` (names of the game's inputs, the translator's `hz_` names, later keywords such as `sample`, `filter`, `input`), `numeric` (huge, NaN, negative, malformed literals), `truncate`, `encoding` (BOM, CRLF, invalid UTF-8, NUL), `duplicate`, `reorder`, and `properties` (`shaders.properties` values and keys; never `program.*`, which turns programs off without a note by design).
- **Invariants.** Each case loads twice through `load_post` as the game loads a pack, in an eval worker process of its case process: no `panic`; within `--time-limit` (`time`) and `--memory-limit` (the worker's allocator refuses past it: `memory`; any other death is `crash`); every stage file of the dimension's folder within `--size-limit` once preprocessed by itself (`size`); every loaded program's modules validate in naga as the translator validates them (`naga`); every note starts with a subject (a non-empty `` `key` ``, or a word without spaces before its first `: `; whether the word is one of the pack's names is not checked; the pack-wide `no program for …` note is accepted), and a program the unmutated case loaded, when only its own stage files changed, loads or has a note naming it (`refusal`); both loads give the same modules, interfaces, settings and notes (`determinism`).
- **Grouping and shrinking.** A failure's group is its invariant, the first step after which it fails (`base` when the unmutated case already does) and its detail (panic location, naga error, program). Each group's smallest case is shrunk while the same failure stays: steps dropped, then files, then runs of lines halved down to single lines, then files again, within `--shrink-seconds`. For a `refusal`, the stage files of the programs that must load are never removed or emptied: without them the program would be missing, which needs no note. The shrunk case is a pack folder in the new run's `groups/` directory under `.local/shader-fuzz/runs/` (or `<work-dir>/runs/` when calling the Python driver with `--work-dir`) with `case.txt`; `report.json` lists every group with its cases (a generated case by its seed, a fixed smoke case as `smoke:N`) and the arguments that replay its smallest case alone (`replay_args`, printed as a command): `--replay` with that case, the run's `--seed-pack` when it had one (the smoke cases are its programs, and it is the first pack a generated case draws from), the corpus of a generated case (a smoke case takes the seed pack only), and the run's `--time-limit`, `--memory-limit` and `--size-limit`, which decide its failures. `report.json` also lists how each case process and its eval worker ended once their cases did: closed, and waited for. Only `time` depends on timing: a case that takes about the limit can land either side of it.

## Reading vanilla

Game jars, decompiled sources and reports live in the gitignored `.local/`, rebuilt from your own copy of the game;
nothing of Mojang's is committed or downloaded by the tools ([README](native-client.md#run-from-a-checkout)):

```sh
python3 tools/lab.py prepare --server-jar ~/Downloads/server.jar   # checks the pinned jars (research-lock.json)
python3 tools/lab.py decompile    # Vineflower: .local/minecraft/26.3/sources/{client,server}/
python3 tools/lab.py status       # coverage reports under .local/minecraft/26.3/reports/
```

`tools/rea_vanilla.py` queries the shipped bytecode through [REA](https://github.com/morluto/rea) (a headless JADX)
for what text search misses: every caller of a method including lambdas and bridges, every overload, or a second
decompiler's view of a method whose Vineflower output looks wrong:

```sh
python3 tools/rea_vanilla.py class net.minecraft.world.level.block.DispenserBlock
python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom
python3 tools/rea_vanilla.py callers net.minecraft.server.level.ServerLevel tickChunk
python3 tools/rea_vanilla.py --jar server-game classes DedicatedServer
```

REA and JADX are pinned in `research-lock.json` and downloaded into `.local/tools/` on first use; each query takes
about five seconds. Decompiled bodies are a reading aid: the bytecode and the oracle harnesses are the reference.

## Player-facing integration and deterministic fixtures

`python3 tools/lan_e2e.py` runs every LAN check with an integrated server thread,
the same path as normal single player. `python3 tools/test_friends.py --client
target/fast/sunrise-client --relay target/fast/sunrise-relay --baseline` covers
plain LAN, direct invitations, a local relay and ended invitations in that mode.
Run Linux games under `xvfb-run -a` when no display is available. Each
process gets scratch game, launcher, XDG data and config homes; audio
and launcher self-update are disabled.

`lan_integration` and `friends_integration` register these suites with `parity.py`.
`friends_integration` (under a minute) is selected like any harness when the
drivers or game implementation change. `lan_integration` is the whole LAN suite
(~40 minutes on a loaded runner, and single checks flake under load), so
`tools/parity_hosts.json` lists it under `named_only`: no automatic selection
(`--changed`, `--failed`, `--flake-check`, or the daily) picks it; it runs
when named or explicitly selected with local `--all`. LAN writes `results.json`
with the host mode and each result beside its screenshots and logs. A skipped
case is recorded explicitly and does not count as passing coverage.

A harness that fails while reusing oracle results runs once more from scratch, and stops reusing
them if that run passes; it counts as failed either way, and its first failure is its outcome.
`parity.py` prints the fresh run's outcome on its own line and a `RETRIED` line with both, and the
daily keeps both runs' logs in the private runner logs. No rerun turns a failure into a pass: a
harness that fails now and then is a daily red, fixed at its cause.

The game-binary fixture audit keeps these complementary direct rule tests:

- `command_harness.rs`: command feedback, styled spans and result callbacks,
  held player position, explicit ticks and portal completion matching
  `VanillaCommandExecOracle`. LAN `commands`, `feedback`, `styles`, `levels` and
  `portals` cover command input, packet feedback and travel across the boundary.
- `dispense_fixture.rs`: seeded synthetic worlds, block/event traces, item
  use/mining and combat hooks matching `VanillaDispenserOracle`. These isolate
  rule results and RNG order; LAN machines, crafting, clicks, fireball, mining
  and combat exercise real input and server handling separately. The new
  `dispenser_input_host` / `dispenser_input_guest` checks press a button and
  verify authoritative inventory consumption and exactly one drop on both clients.
- `sculk_fixture.rs`: scripted events and exact per-tick sculk/warden traces
  matching `VanillaSculkOracle`; introducing wall-clock network ticks would
  destroy that deterministic oracle contract. `sculk_input_host` and
  `sculk_input_guest` separately exercise real footsteps reaching a sensor;
  failures remain visible rather than changing the oracle trace.
- `net_harness.rs`: already a real network client of a dedicated server; keep
  its handshake, world loading, chat and command packet assertions.

Library probes, `lan_bot`'s protocol hub fixture, generation/save tests and
streaming/mesh tests remain fast deterministic coverage. They do not become
full game launches. Setup and authoritative reads use the owning server;
interactions stay on the client. `server-hold on|off` provides a prediction-test
barrier between server ticks, including player packet handling, while transport
threads and control calls remain responsive. `stall MILLISECONDS` stalls only
the window to test recovery from a hung frame without process signals interrupting
socket reads. Neither mechanism changes normal gameplay.
