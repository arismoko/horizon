# Sync 27 integration decisions

## CTM sprite deduplication: integrated with an authorized exception

The sync takes upstream's two additive `assets.rs` hunks: load CTM rules and
protect matched sprites before models insert their textures, then pass those
rules into `ConnectedTextures::load`. The connected loader exposes
`matched_sprites` and accepts the already loaded rules. This makes the incoming
`Sprites::keep_apart` implementation active in production.

This is an authorized exception to keeping `assets.rs` identical to main. Its
installation-scoped interner code is untouched: only these two additive CTM
hunks differ from the fork base. `registry.rs` remains identical to main.

## Contextless fireworks service: refused

Upstream adds an audible flag to its contextless fireworks request helper.
The fork removed that helper: `game/modding/native_client_particles.rs` exposes the typed
single-particle request, not a fireworks explosion request, and the typed host
has no corresponding fireworks service. Reintroducing the old numeric service
would violate the clean break. A new typed fireworks service requires an
API-owned explosion payload and its Wire contract, not an engine explosion
slice exposed through the API. This sync does not add that new capability.
Existing engine firework rendering already passes the audible flag based on
rocket silence in `game/entities/fireworks.rs`; that path is retained.

Refs #3472 for an API-owned typed fireworks request and its audible flag.

## UUID and NBT lookup: typed coverage, no legacy services

The upstream shared UUID lookup and dimension-plus-numeric-ID NBT helpers are
not restored. The fork's entity handles carry residency, epoch and level
identity; reads obey that identity rather than accepting an unscoped ID and
swapping arbitrary levels behind a shared reference. The relocated typed
entity tests now cover UUID word ordering, NBT without `id`, refusal outside
the entity's level, and successful reads after returning to its level. This
ports the data contract, not the removed contextless service surface. A new
cross-level UUID query API is outside this sync and is not claimed available.

Refs #3473 for typed cross-level UUID lookup.

## Registry image build context: carried

Refs #3472. Vocabulary generation tracks the engine's native hook declarations.
The registry image now copies `crates/horizon-client/src/game/native_hooks`, and
Railway watches that directory. This addresses the missing build-context input;
a registry image build was not run locally.

## Server settings fragment: helper-only

The upstream fragment names a helper, not a new hook moment. The sync does not
restore a contextless helper or add a duplicate hook; no new capability issue is
needed for this fragment.

## Fork-orphaned upstream lookups: removed

Remove `Entity::dimension_changing_delay` and its overrides,
`Game::entity_portal_delay`, and `Game::player_with_uuid`. The fork retained
its own active portal-delay lookups and creature owner resolution, leaving
these upstream helpers unreferenced here. Removing them preserves the active
behavior without dead code or suppression attributes.

## Main after rollup 45: combined resolutions

Merge main `8a5ab873d5` once. Lighting retains upstream reusable relight
scratch alongside main's dynamic-light-aware section lookup and emission
calculation. Sniffer digging retains upstream animation timing, render-shape
gating, local-sound silence behavior and whole-dig game events, while routing
the block hit sound through main's typed block-sound hook. The hook remains
inside the existing sound window; particle sprite availability gates dust only.
