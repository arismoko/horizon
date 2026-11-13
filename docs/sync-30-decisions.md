# Upstream sync 30 decisions

## Pin and scope

- Fork base: `ffb27f0e287c818a3b11bd66e25cb051ec82a793` (sync 29 merged).
- Upstream pin: `911aa6ddc6ec1684eaa312c7650e938789cf2193`.
- Selection evidence: newest completed-success push-to-main `.github/workflows/ci.yml` run `43183045877` at selection, completed 2026-11-12 08:59:31 UTC. Newer cancelled or unfinished runs do not qualify.
- Previous integrated upstream ancestor: `c94cdb0c76d2e06e13240eee7d9e1f807977318f`.
- Incoming delta: 21 files. Initial conflicts: 9, comprising 7 content conflicts and 2 modify/delete conflicts.

## Resolution principles

Keep typed component, equipment, entity, hook and list ownership. One-side merge wins are not evidence that behavior was integrated. Compare old incoming roots with compiled owners before removal; move tests onto the production helpers rather than reimplementing behavior in fixtures. Main CI files and the parallel mob dispatch gate remain unchanged.

## Tooling and launcher sweep

The incoming LAN port ledger now names `WorldOptions::port_tooltip` and describes invalid-port reasons while retaining the fork's relocated `game/network/lan_screen.rs` paths. The incoming launcher diagnostics distinguish a pinned version manifest without its client jar; paths are normalized for display, a wrong jar still takes precedence, and the existing missing-client case remains. Its scratch-directory regression checks both direct and Modrinth-style manifest paths and verifies no download request. Incoming diagnostic-string style is retained from upstream.

The command corpus adds TNT minecart float-field/default/clamping and stationary explosion scenarios. It remains a developer harness, not a local test executed for this sync. Its fields must correspond to the compiled minecart NBT and blast owners.

## Typed integration decisions

- Arrow saved Air and shared entity fields go through the actual projectile codec. Wolf/cat collar loading narrows to the legacy signed byte before out-of-range white fallback; typed equipment decoding remains authoritative.
- TNT minecart power and speed factor load/save on the existing inline Minecart owner; stale default fields are removed before nondefault emission. The incoming dead `game/minecarts/nbt.rs` is deleted only after porting behavior. The real lending-aware explosion owner uses the incoming power formula without bypassing hooks, residency, destruction guards or RNG publication.
- Guest equipment keeps its typed BODY/SADDLE storage; a distinct eight-slot cache owns conversions. Fixed slot comparisons include component state, removals and selected hand changes. GuestList updates own cache mutation; registry reload invalidates it before advancement early exits. Tick-time effect: unchanged stacks avoid network-stack conversion; the existing outgoing equipment snapshot allocation remains. No runtime benchmark was run.
- Dead incoming `lan/hub_tests.rs` is mapped to compiled inline hub tests, not resurrected. Added cache tests use the actual guest-equipment conversion and reload paths; existing tracking tests inspect decoded packets.
- Incoming render data is named `spectated_camera_snapshot`, not `camera_entity`: the latter is already the typed epoch-bound EntityRef owner. Rendering, aim, FOV and perspective use the snapshot; native typed getter/lifecycle validation stays intact. Server camera packets support armor stands, but the existing typed setter still rejects them; this sync does not claim full typed setter support.
- Fox defense goes through the existing validated target hook rather than a duplicate raw setter. Village shelter behavior is retained alongside sync 29 effective-component eligibility, remainder and ordered consumption behavior.

## Test and static-review sweep

Whole-file upstream inventory: 287 declarations across 13 changed test-bearing paths. NBT/minecart families map all 166 declarations; camera/fox map all 43; launcher retains all 21 and LAN tooltip all 8. Of 49 network declarations, 48 map to compiled owners, including one renamed typed client-update test. The historical raw-channel registration test remains omitted because its legacy API is absent; the fork instead owns typed payload negotiation. This is an existing deliberate divergence, not newly removed behavior, and no exhaustive equivalence claim is made.

Independent static review found no remaining integration blocker after the camera-name correction. Initial compile attempts exposed that name collision and two stale fox goal fixture fields; the fixtures now use the actual typed TestLevel context with unchanged seeds and assertions. Static fixture sweeps were not sufficient to establish compilation; the final all-target check remains authoritative.

## Verification status

`cargo check --workspace --all-targets` completed successfully with zero warnings before the first push. Touched Rust was formatted. No local tests, clippy, parity harnesses or game runs. Exact-head CI and the root-ordered final static glance remain pending.

Fresh main advanced to `c96494f612b64b87fa66444a30c042f5b57136a8` immediately before the first push; merged once, cleanly. The complete integrated `cargo check --workspace --all-targets` then passed with zero warnings.
