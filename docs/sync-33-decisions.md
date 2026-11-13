# Upstream sync 33 decisions

## Pin and verification boundary

The integration initially started at fork main `00561622057353a5e3fb83a557da68c3e2f856a3` and merges upstream `f24ee1accf1c816736e6e067939d04f3853f474e` directly. Fork main `fe4a761329fe866327bd29f9a8d097b40fb6e90b`, including the slow-fixture follow-up, was merged unchanged. The immediate pre-push reread then found rollup 52 on main `21d47922bd0287c60b67d7aaa33c7d7990f7aa4c`; it too was merged unchanged and without conflicts. Rollup 52 is the initial compile-only validation base. The final CI repair merges rollup 55 main `eda0b04851` unchanged, without conflicts, and regenerates the baseline against that actual merged tree. Upstream's own `ci.yml` push-to-main run [43207155875](https://github.com/horizon-rust/horizon/actions/runs/43207155875) passed. The previous synced upstream pin is `ba91f75cb21a74026319d1393c17fc5061dbd725`.

The pre-push upstream CI reread still found `f24ee1accf1c816736e6e067939d04f3853f474e` as the newest successful push-to-main head. A fresh fetch confirmed the validation base above is current fork main and already an ancestor of the integrated sync.

No local tests, clippy, harnesses or game runs are permitted for this sync. Only touched-file rustfmt and the full workspace/all-target compile-only check are used locally. Dependency metadata resolution does not compile or execute the new suites. Runtime claims require CI results, and resource-backed ignored assertions require the main slow shards.

## Dependency graph

- `crates/horizon-client-render/Cargo.toml`: union fork `tempfile` and `mod_platform` test dependencies with incoming `horizon-perf`; retain incoming format dependency and allocation-count feature forwarding.
- `crates/horizon-world/Cargo.toml`: retain fork `horizon-assets` and add upstream `horizon-threads`.
- `crates/horizon-server/Cargo.toml`: take upstream's removal of direct `horizon-bench`; the server probes already use `horizon_platform::bench`. Retain incoming direct anticheat/chat-filter dependencies.
- `crates/horizon-threads/Cargo.toml`: upstream `alloc-census = ["alloc-count"]`; retain both fork allocation-test features.
- `crates/horizon-client/Cargo.toml`: incoming allocator feature propagation and allocation counting; retain fork Windows prebuilt allocator documentation and mod test edges.
- `Cargo.lock` and `launcher/Cargo.lock`: preserve the fork's locked registry versions, resolve incoming additional packages, and keep the newly introduced protobuf packages at upstream's exact `0.14.4` versions. No pre-existing locked registry package/version is removed. Both `object` versions coexist where the profiler and fork native machinery require different versions.
- Automatic manifest merges retain the three new perf workspace members and their library/platform/official edges without removing mod dependencies.
- The launcher drops its direct `horizon-bench` dependency; `launcher/src/perf.rs` uses `horizon_platform::bench`, taking upstream's own dependency and import changes.

## Workflow divergence

**Diverges from upstream: perf-wasm has no pull_request trigger, per the fork's gate policy.** Main pushes retain upstream's path filters; manual dispatch remains. The workflow uses `ubuntu-latest`, a GitHub-hosted runner, not a Dyna runner. No dispatch was made during integration. This does not add a sixth PR gate or request additional Dyna capacity.

The following exact block was removed from `.github/workflows/perf-wasm.yml`, between the push filters and `workflow_dispatch`. Restoring this block reverses the trigger divergence in one edit:

```yaml
  pull_request:
    types: [opened, synchronize, reopened]
    paths:
      - 'crates/horizon-perf/**'
      - 'crates/horizon-perf-format/**'
      - 'crates/horizon-bench/**'
      - 'xtask/**'
      - 'Cargo.lock'
      - 'Cargo.toml'
      - '.github/workflows/perf-wasm.yml'
```

## Conflict resolution inventory

Each row names the exact conflicted path, including retired paths that remain deleted. “Fork” means the typed ownership, generated discovery and native/wasm architecture; “incoming” means the upstream performance and Horizon-layer behavior.

| Conflicted path | Resolution |
|---|---|
| `Cargo.lock` | Fork locked graph plus incoming dependencies; retain upstream prost family 0.14.4 pins. |
| `crates/horizon-client-core/src/text.rs` | Fork fallible NBT serialization retained; add upstream local Profiler click and styled inline-object propagation. |
| `crates/horizon-client-render/Cargo.toml` | Union fork test dependencies with incoming perf dependencies and allocation feature forwarding. |
| `crates/horizon-client-render/src/gfx.rs` | Fork non-warp pipelines, prepared entities and staged mod draws retained; add upstream pass splits, timing and counters. |
| `crates/horizon-client-render/src/hud.rs` | Fork Mods icon fallback retained; add upstream Horizon mark alongside controller art. |
| `crates/horizon-client-render/src/streaming.rs` | Fork resumable planning, ownership and scratch reuse retained; port queue census and timing laps into corresponding phases. |
| `crates/horizon-client/Cargo.toml` | Incoming system-allocator forwarding and alloc-count feature; retain fork dependencies and allocator notes. |
| `crates/horizon-client/assets/horizon/lang/en_us.json` | Retain 48 incoming additions; restore the two deliberately different fork values from `9f9e2e4af39`: Open Mods Folder and the tooltip without a nonexistent toggle. Whole-file shared-key/history audit found no other changed shared values. |
| `crates/horizon-client/src/client_lib.rs` | Union fork module declarations with incoming perf modules and mark_sprite export. |
| `crates/horizon-client/src/entry.rs` | Fork typed launch context retained; incoming profiler startup is adapted at the typed launch boundary, not by restoring string argument parsing. |
| `crates/horizon-client/src/game.rs` | Fork generated module discovery, points and typed ownership retained; integrate upstream profile state, GPU timing, work attribution and benchmark fields. |
| `crates/horizon-client/src/game/administration/commands/mod.rs` | Incoming syntax_failure visibility widened within game for canonical module callers. |
| `crates/horizon-client/src/game/block_entity_ticks.rs` | Keep deleted; port work attribution into blocks/block_entity_ticks.rs published loop, retaining slot rechecks and all fork variants. |
| `crates/horizon-client/src/game/block_ticks.rs` | Keep deleted; port random block work clearing into canonical game.rs loop. |
| `crates/horizon-client/src/game/interface/chat.rs` | Fork command registrations retained; add incoming profiler tree, trusted click dispatch and namespace collision test. |
| `crates/horizon-client/src/game/interface/dedicated/mod.rs` | Union imports and capture shutdown with fork watchdog RAII and lifecycle. |
| `crates/horizon-client/src/game/interface/dedicated/report.rs` | Incoming .hzperf reporting replaces retired JSON bodies; adapt canonical typed active/parked counts and shared mod metadata. |
| `crates/horizon-client/src/game/interface/dialogs.rs` | Fork Arc unwrapping retained; refuse incoming local Profiler click in dialogs. |
| `crates/horizon-client/src/game/item_entities.rs` | Keep deleted; port attribution and batch clearing into canonical typed game.rs item loop. |
| `crates/horizon-client/src/game/lan_actions.rs` | Keep deleted; route incoming perf payload through network/lan_actions/packets.rs. |
| `crates/horizon-client/src/game/lan_actions/horizon_channels.rs` | Keep deleted; perf routing uses canonical packet handling, not retired plugin-channel helper. |
| `crates/horizon-client/src/game/launch_options.rs` | Keep deleted; adapt --profile and benchmark options at typed selection/context.rs boundary and canonical game options. |
| `crates/horizon-client/src/game/mobs/fangs.rs` | Fork typed lift and after_take test hook retained; add upstream work clearing and canonical perf count access. |
| `crates/horizon-client/src/game/native_hooks.rs` | Keep deleted; incoming change is documentation only, no executable behavior to restore. |
| `crates/horizon-client/src/game/natural_spawning.rs` | Keep deleted; port spawn chunk attribution and clearing into mobs/natural_spawning.rs. |
| `crates/horizon-client/src/game/remote_poll.rs` | Keep deleted; port perf replies and chunk tracing into network/remote/mod.rs, retaining residency argument. |
| `crates/horizon-client/src/game/runtime/test_pos_guest_tests.rs` | Fork resource-borrowing fixture retained; incoming memory test adapted to pass resource slot. |
| `crates/horizon-client/src/game/window_handler.rs` | Keep deleted; port profile completion and redraw profiling into runtime/window_handler.rs. |
| `crates/horizon-client/src/game/world/integrated.rs` | Fork server lifecycle wrapper retained; finish capture after lifecycle, without duplicate native hooks. |
| `crates/horizon-client/src/game/world/levels.rs` | Incoming parked perf accessors, work context and TickTiming shape; canonical typed counts retained, retired native timing hook omitted. |
| `crates/horizon-client/src/game/world/tick_profile.rs` | Incoming shared span user bits and explicit AtomicU8 import. |
| `crates/horizon-client/src/game/world_update.rs` | Keep deleted; port LOD, terrain-choice and upload-queue tracing into canonical game.rs update_world. |
| `crates/horizon-client/src/lan/conn.rs` | Incoming queue metrics; omit obsolete sender fake flags because fork guest kinds own identity. |
| `crates/horizon-client/src/lan/io_pool.rs` | Incoming queued metrics retained; packet tally APIs and storage remain retired with native_diagnostics. |
| `crates/horizon-client/src/main.rs` | Keep deleted; executable ownership stays with fork; apply GameAllocator to canonical gameplay library src/lib.rs. |
| `crates/horizon-client/src/mobs/spawn_selection.rs` | Union fork limit/overflow tests with incoming attempt-counter publication test. |
| `crates/horizon-command/src/tests.rs` | Union custom argument tests with incoming profiler parsing, completion and namespace ownership tests. |
| `crates/horizon-server/Cargo.toml` | Take upstream's bench dependency removal and platform API routing, with incoming anticheat/chat-filter references required by server CLI. |
| `crates/horizon-server/src/main.rs` | Fork thin executable retained; incoming report help ported to canonical server src/lib.rs. |
| `crates/horizon-threads/Cargo.toml` | Incoming alloc-census depends on alloc-count; retain fork allocation probe/count test flags and incoming platform dependencies. |
| `crates/horizon-threads/src/lib.rs` | Union modules; incoming sampling/GameAllocator routing with fork test allocator guards and mutually exclusive fallback. |
| `crates/horizon-world/Cargo.toml` | Union fork horizon-assets with incoming horizon-threads dependency. |
| `crates/horizon-world/src/level/tick.rs` | Incoming chunk/block attribution retained; fork fluid.tick_id() remains authoritative, not registry index. |
| `docs/layers.md` | Fork optional-edge explanation retained over obsolete exception table; incoming capture hook, addresses and perf-share layering retained. |
| `docs/native-mods.md` | Fork prebuilt SDK and typed service contracts retained; reject obsolete source-rebuild/versioned native ABI catalog. |
| `launcher/Cargo.lock` | Fork locked graph plus incoming dependencies; preserve upstream prost family 0.14.4 pins. |
| `launcher/src/native_mods/api.rs` | Fork slot/template generation retained, not upstream native ABI; generated allocator uses GameAllocator. |
| `tests/test_game_home.py` | Union native dylib probe and profile client fixture exemptions. |
| `tools/layers.json` | Incoming bench/perf classification and portable threads; retain fork discovery/mod crates and optional edges; remove obsolete bench exception. |
| `xtask/src/main.rs` | Union fork mod-api/mixins-tool commands with incoming perf-wasm task and help. |

### Dead-file and automatic-merge sweep

Retired gameplay paths listed above remain deleted; their behavior lives in the canonical area modules rather than duplicate implementations. New `perf_link`, `perf_remote`, `profile_capture` and `profile_lines` modules move into `interface/`; `lan_op` moves into `administration/`. The incoming integrated capture child moves beside `world/integrated.rs`. Generated discovery keeps their logical game module identities. Streaming job statistics move from the former inline module to `streaming/job_stats.rs`.

Automatic-merge review includes prepared entity rendering, mod primitive stage ordering, dynamic lighting, queue metrics, text styling, book click refusal, GPU debug detail, minecart attribution, creature tick profiling and shared capture modules. Mob work attribution is ported into the live resumable `creatures/borrowed_tick.rs` implementation, not left only in the inactive incoming `creatures/mob_group.rs` path; paused ticks clear attribution and resumed ticks restore it. Canonical shutdown finishes the client capture after saving. Spike span ownership uses the shared user bit so finishing a report does not disable slow-ring recording. Saving remains engine-owned; explicit Open/Share remains Horizon-owned through `Hooks::captures`, without moving upload services into the engine.

Capture producers use canonical loaded-mod inventory for id/version and native/wasm provenance. The format enum and viewer vocabulary include Wasm; unknown code provenance remains unspecified rather than being guessed. Active and parked entity counts use canonical typed stores, custom registry keys and mod block entity variants. Special families include cushions, fangs, one dragon singleton, breath clouds and lightning; dragon hitbox parts are not independently counted. Network profiler authorization remains tied to connected hub guests; it does not replace general entity or player ownership gates with remote-only checks.

### Developer-tool and workflow sweep

All incoming Python changes were swept for canonical source paths and executable names: `tests/test_game_home.py`, `tests/test_pacing_bench.py`, `tests/test_server_bench.py`, `tools/lan_e2e.py`, `tools/pacing_bench.py`, `tools/perf_upload_e2e.py`, `tools/profile_client_e2e.py`, `tools/server_bench.py`, `tools/server_perf_e2e.py` and `tools/server_profile_e2e.py`. The profile client fixture's default build selects `horizon-game`, not the library-only `horizon-client` package. Other callers use canonical game/control/perf binaries and resource discovery. Pacing now queries captures rather than parsing retired spike logs; server benchmarking can attach run records to captures.

The incoming `perf-wasm.yml` workflow keeps main and manual triggers for the reader ABI test and wasm asset build. The fork removes its pull-request trigger as recorded in the named workflow divergence above; no workflow was dispatched during integration.

### Tests and resource requirements

Retained incoming coverage includes command parsing/completion and namespace collisions; report CLI/capture selection and privacy; text styling; allocation counters, heap sampling, stack/context conversion, aggregation and work attribution; queue and level timing; GPU timing/counters and draw equivalence; format/reader round trips and synthetic fixtures; loopback sharing and upload limits. Synthetic `.hzperf` fixtures remain included; they are not vanilla assets. Linux sampling cases require platform sampling/unwind support, and GPU cases require compatible adapters/query features.

The ignored producer tests `a_capture_of_a_server_with_names_text_and_a_seed_holds_none_of_them` in dedicated/report.rs and `a_servers_report_names_its_entities_and_players_and_leaves_the_rest_unattributed` in runtime/memory.rs require private prepared resources through `HORIZON_TEST_RESOURCES`. The incoming `slow_ticks_enter_the_ring_and_not_the_log` fixture in world/levels.rs also requires `HORIZON_TEST_RESOURCES`. Their resource factories are adapted to the fork's production ResourceRoot, ResourceSource and execution-role helpers. Self-contained format, reader and recorder suites do not prove these ignored producer contracts. Real-game profile and LAN fixtures require prepared 26.3 resources, scratch homes and built game/control/perf binaries; GPU client cases additionally need offscreen rendering support. The vanilla comparison is optional when its licensed server jar is absent. Viewer wasm builds require `wasm32-unknown-unknown`. These requirements are an inventory, not evidence of execution or runtime correctness.

### Generated baseline justification

The generated baseline was originally a separate commit, `98a17c12d6`, relative to main `fe4a761329`; final lifecycle and provenance corrections did not change it. After integrating fresh rollup 52 main `21d47922bd`, the generator wrote a separate regeneration commit, `003849ebaf`. After rollup 55 landed, main `eda0b04851` was merged cleanly and the tool regenerated the baseline again in a separate commit. The same three growth amounts remain justified; the final absolute sizes reflect main's reductions. Its increases correspond to retained incoming behavior in canonical modules, not restored obsolete implementations:

| Canonical file | Before → after | Reason |
|---|---|---|
| `crates/horizon-client/src/game.rs` | 16,811 → 16,848 (+37 code lines) | Profiler state/options and work tracing within the existing canonical game implementation; no resurrected implementation block. |
| `crates/horizon-client/src/game/entities/minecarts.rs` | 3,405 → 3,407 (+2 code lines) | Work attribution and clearing around canonical minecart processing. |
| `crates/horizon-client/src/game/network/remote/mod.rs` | 11,863 → 11,874 (+11 code lines) | Profiler reply routing and chunk tracing ported from the retired remote polling path. |

### CI layer corrections

Upstream `a86ab9d4bc` moved `horizon-bench` into the engine layer and exposed its probes through `horizon_platform::bench`. The server source already took that platform route, but the conflict resolution incorrectly retained the old direct manifest edge. Remove that dependency and its lockfile edge; no layer exception is added.

Main's `c32e27f3b00` introduced `horizon-threads`' dependency on the shared guest-stack policy. Upstream `a86ab9d4bc` then classified threads as portable. The stack-policy leaf is now classified portable alongside it, rather than allowing a forbidden edge: it has no workspace dependencies and no window, GPU, sound, gamepad, clipboard or HTTP library, only target-gated libc and standard-library process/thread plumbing. Native stack queries are target-gated, and unsupported targets refuse clearly. This satisfies the documented desktop-free portability contract, just like threads' existing native CPU/sampler plumbing. The leaf remains allocator-independent for executable/build-script consumers; moving it into threads would reverse that dependency and pull in allocator machinery. The portable list and its documentation are updated together, with no `allowed` entry.

### CI profiler fixture correction

The first verify run's single failure was `profile::tests::a_live_session_cuts_its_samples_into_blocks_of_block_s`, not the separate stack-dump test. Its 450 ms workload budgets thread CPU time while the profiler buckets wall elapsed time in 100 ms blocks. Descheduling and drain latency can legitimately yield the observed buckets 1 through 8, so the upper bound of 6 was invalid. The profiler, sampler and original fixture are byte-identical to the pinned upstream source; the actual upstream verify log for run 43207155875 reports this exact test passing, not just a green run badge. This establishes a scheduling-dependent fixture hazard, not a measured fork interaction or a profiler implementation defect. The fixture uses standard threads, not the fork guest-stack worker builder.

The carry retains the live timer sampler, ring draining and production elapsed/block calculation. A `cfg(test)` elapsed clock advances only after the immediate `Aggregate::cpu_samples` counter increases, and a channel notifies completion. Our first carry incorrectly watched finalized `counts`, whose CPU keys remain pending until a tick marker, a 4,096-sample flush or finish; this caused the next CI timeout. The immediate counter includes pending samples and guarantees real sampled work in every logical bucket. The test stops and joins its workload before checking the bounded wait result, then asserts exact buckets 0 through 5. No sleep, retry or status-poll loop is used; the CPU loop is the sampled workload. Production retains Instant without clock callbacks or indirection. This repair received rustfmt only, with no local build or test execution; CI is the runtime check. An upstream follow-up is recorded separately, without filing an issue.

### Kernel-dependent CPU sampling test corrections

The next verify run used `dyna-ci-lt-24-005` rather than the previous Arch runner. Its WSL2 kernel `6.6.114.1-microsoft-standard-WSL2` has `CONFIG_HZ=250` and `CONFIG_TICK_CPU_ACCOUNTING=y`; the other checked runners use 1,000 Hz and virtual CPU accounting. Root verified these kernel settings and filed the CPU-timer resolution follow-up as horizon-rust/horizon#15721. CPU samples arrived near 250 Hz despite the requested 1 kHz (53 samples over 200 ms CPU, 2,538 over 10 seconds), while a shared workload yielded 54 CPU versus 203 wall-running samples. The sampler and allocator dependencies did not change between those heads; portable classification changed metadata only.

The tests now independently measure raw CPU-timer deliveries on a dedicated unregistered thread, using the existing once-installed signal handler and a thread-local calibration counter, without replacing handlers. This cached measurement scales CPU count bounds, not wall-clock counts or behavioral ratios. Fixed-capacity ring-pressure fixtures calibrate CPU exposure while retaining their count/drop requirements. The support feature is enabled only for Linux tests; normal production builds have no calibration branch. No retry, skip or nominal-rate fallback is introduced. Threads' slow-share and render's slow-share checks assert 0.75 ± 0.1, render entity attribution retains its 3:1 assertion, and equal CPU-time span sections assert a named share of 0.5 ± 0.15. All tick, unit, stack classification and exact time-block assertions remain. Both test patches received independent static review and rustfmt only; they were not executed locally.

This sync does not change the mod wasm runtime, wire boundary, loader or fixture inputs relative to the merged main. `horizon-perf/src/wasm.rs` is a profiler report reader, not the mod runtime; the package selection change only admits the game's `--profile` argument. The profiling wasm workflow remains main/manual-only as recorded above. No additional mod-runtime fixture dispatch is requested by this correction.

### Compile-only corrections and retired contracts

The first compile-only pass exposed a Text mutation error; canonical `map_styles` was corrected in `c27cd1ffb3`. The second exposed string-based game-rule reads, missing refusal of the local Profiler click, and obsolete network fields. Game rules now use typed `vanilla` indices and `int_at` / `bool_at`; the string lookup API stays retired. The mod text boundary refuses Profiler like OpenFile.

The packet tally APIs and storage stay deleted: their only upstream consumer was the retired `native_diagnostics` callback, not server_perf. The profiler continues to capture connected byte traffic and queue metrics without inventing a packet observation API. Connectionless fake actors have zero connection queue metrics and do not count as online participants or operator lookup targets. They remain entities where their typed owner admits them.

Server gauges sum typed collection lengths without building registry-name maps. One shared counting view covers active and parked families, while start snapshots build richer registry counts. Client and server entity gauges use the same all-family meaning. **Upstream bug taken:** `tick_profile.rs` keeps upstream's deep clone of `MobType` and address-keyed sampled rows. This is deliberately unchanged in the sync; the lane lead tracks the correction separately.

Tick timing discards between-tick span events and captures only events inside the measured tick, preserving the upstream `TickDetail` contract. The retired native timing hook does not justify including outside work in capture or slow-ring records.

The final integrated head `003849ebaf`, including fresh rollup 52 main, passed `CARGO_INCREMENTAL=0 cargo check --workspace --all-targets --keep-going -j 4` with zero compiler errors and zero warnings. All test targets were checked for compilation only; no tests, clippy, harnesses or game runs were executed. Producer classification and reader wasm-name coverage were added but not run locally. CI and resource-backed ignored runtime results remain separate evidence and are not claimed here.
