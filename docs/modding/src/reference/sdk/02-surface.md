# `surface.json`

The SDK's `surface.json` describes the stable mod API of one build. It is generated from source
(`sunrise-stage surface ROOT OUT`), never kept by hand or committed, and is byte-identical for one commit:
sorted, with no machine paths or timestamps.

## Records

Every record has an id, unique in the document, and a `hash` over its contract. Members (a trait's methods, a type's
fields and variants, a hook's arguments) are records of their own:

{{#sdk-surface}}

An item's path is one a mod writes after `mod_api::`: the path of the module that defines it when that module is
public, so a re-export added later never changes its id; otherwise its shortest public path. A record's `paths`
lists every public path a mod can name it by. Types inside signatures are normalised Rust, with paths relative to
`mod_api` and lifetimes kept.

Hooks are named `hook:<name>` in SDK records. Hidden `HookId` discriminants are process-local dispatch indexes,
never serialized or stable across builds, and native binaries must rebuild for the selected build.

`cargo sunrise-mod hooks <text>` searches the API records offline ([Commands](05-commands.md)).

## What a hash covers

A record's hash covers its contract, not its `doc`, its `paths`, a hook's `engine` (where it fires) or
`counterparts` (its names in NeoForge, Fabric and Paper). Each member has
its own hash, so most additions change no existing hash: a new `Host` method, a new hook, a new hook argument or a
new field on a type change nothing for a mod that doesn't use them. A container's hash covers its members only where
a new member breaks users:

- an exhaustive enum's variants;
- the required methods of a trait mods may implement, and its supertrait bounds;
- a hook's side, kind, scope, whether it is cancellable, its context and host, its owner argument, and whether it has
  an output. Its arguments are member records; its `version` is outside the hash.

A struct's field set is outside its hash. Its `fields_hash` says whether a literal listing every field still builds,
which matters only to a mod that builds it that way. A type's trait impls are a set beside the hash: one gained
changes nothing, one lost is named by the fit test. The document's own `hash` is over every record's id and hash;
`sdk.json` repeats it as `surface.hash`.

`schema` is a number: a reader refuses one it doesn't know and ignores fields it doesn't know.

## What a change is

`sunrise-stage surface-diff OLD NEW` compares two documents by id. A record only in the new one is added, only in the
old one removed; in both with another hash, changed. A rename or a move that changes the record's id appears as a
removal and an addition, even within the same crate. Each change is in one of four classes:

- **Breaking**: it can stop any mod that uses the item from compiling or meaning the same. A record removed; a
  contract changed (a hook's side or kind, a retyped field or argument, a signature); a
  variant added to an exhaustive enum; a required item added to a trait mods implement; an impl a type no longer
  has; a behaviour removed. Removing or changing a published loop is breaking.
- **Conditional**: it breaks only some mods, which the [fit test](04-fit.md) finds from each mod's source (except a
  behaviour's `version`, which no fit report shows: a mod relies on a behaviour without naming it). A field
  added to a struct mods can build, or an argument added to a hook (only a mod that writes it out in full); a struct
  no longer constructible; a public path that goes (only a mod that writes that path); a hook's `version` (only a mod
  whose manifest names it) or a behaviour's `version` (only a mod that relies on it).
- **Additive**: a new API record, behaviour or published loop, or a change that only widens an item:
  an impl gained, a new public path, an item no longer required, an enum no longer `#[non_exhaustive]`.
- **Advisory**: all listed internal changes are reported, not gated, because native internals are pinned to an exact
  build and the [fit test](04-fit.md) tells each mod whether it still fits.

Each change says what changed in words a mod author can act on, for example
``argument `player`: `PlayerRef` → `ServerPlayerRef` ``. `cargo sunrise-mod sdk diff <build> <build>` prints the
same report for two published builds.

## CI and local audits

CI compares the stable API and published loops of its merge base and head. Each revision's own `sunrise-stage`
exports its stable API, so changes to generated types and signatures are visible in the diff. The head's tool reads
both trees' engine catalogs. Loop records are for comparison, never shipped in the SDK's `surface.json`; `loops.json`
and `internals.json` remain separate build data. The audit reads the same declarations:

- A loop's id is `loop:<name>`, using its `[[native.loop]]` name. The report compares its entry path, generic method,
  slot trait and side.
- In a local audit that includes listed internals, an internal's id is `internal:<kind>:<path>` as listed in
  `internals.json`, with ` [<cfg>]` appended for a guarded alternative. The report compares its signature and hash,
  and whether the row is a type shape rather than an exposed internal.

When the `verify` check runs on a pull request that changes Rust, the engine-catalog summary counts additions,
removals and changes, and names removed or changed items, subject to its length cap. The report fails the check
only when:

- the head's tree cannot be exported;
- the base's own `sunrise-stage` cannot be built or cannot export the base, or the head's tool cannot read the
  base's catalogs or compare its exported surface;
- a change note cannot be read, or names an item that did not change;
- a breaking change has no change note: every breaking record the report lists, members included (a removed field is
  its own line), must be named by a note the pull request adds. Conditional, additive and advisory changes need none.
  The gate is `gate = true` in `tools/ci/surface.toml`.

To run the CI report locally, set `SUNRISE_STAGE_TOOL` to the `sunrise-stage` binary built from `HEAD`. If the
compared revisions use different generators, pass `--build-base-tool`:

```sh
cargo build --locked -p sunrise-stage
SUNRISE_STAGE_TOOL=target/debug/sunrise-stage \
  python3 tools/ci/surface_report.py --trees BASE HEAD --build-base-tool
```

`BASE` and `HEAD` are source directories, such as two Git worktrees. The flag always builds and uses `BASE`'s own
`sunrise-stage`, even if `HEAD`'s tool could read `BASE`. Use it only with a whole checkout you trust: it compiles
and runs `BASE`'s code, including build scripts. The build requires POSIX file locking (Linux or macOS).
When both trees use the same generator, omit `--build-base-tool` for a source-only audit with the supplied binary.
Add `--with-internals` to include advisory internal changes without changing the gate.

## Command completion layouts

Command argument completion and `tab_complete` share one `Suggestion` type carrying a byte `range`,
replacement `text`, optional `tooltip`, and optional `int_value`. `CommandArgumentType::suggest`
takes `&SuggestionRequest` (`prefix`, prior `arguments`, and optional `sender`) and returns
`Vec<Suggestion>` on both server and client commands. Range offsets into `request.prefix` must be
ordered, within `request.prefix`, and on UTF-8 character boundaries; invalid entries are logged and
skipped individually. See [Server commands](../../how-to/entities/server-commands.md) for replacement
examples and the `id` argument's `tab_complete` completion route.

## Tooling migrations for 1.0

- Delete `#[sunrise_mod_api_macros::computed_in_mod]` (or `#[computed_in_mod]` if imported) from defaulted
  trait methods, and remove any import of the attribute. Keep the default method body unchanged.
- Replace any `HookId` ordinal cast, comparison, or stored number used outside its own build with the named
  hook identity, for example `hook:player_logged_in`. If you stored ordinals, recover their names from the
  build that wrote them, not from the new build's ordinals. Named `Hooks` methods with `#[sunrise_mod]`
  subscribe the same way; no listener change is needed.

These Sunrise tooling changes have no stable surface IDs.

## Change notes

A change can carry a note: one file per change in `crates/sunrise-mod-vocab/changes/<name>.toml`.

```toml
ids = ["hook:player_logged_in", "type:PlayerLoggedIn.player"]
why = "The hook now fires on the server only."
migrate = "Take `ServerPlayerRef` instead of `PlayerRef`."   # optional
```

Use the exact id printed by `surface-diff`, including a published loop's `loop:` prefix. For a move, give the
removed ids in `ids` and the replacement names in `migrate`.

A note can also say that a merge already changed its ids, without a note, with `landed = "<full merge sha>"`. Its ids are
then checked against that merge's first parent and the merge itself, not against the pull request that adds the note,
and must have changed there: a wrong `landed` fails the report, naming the note and the id. The merge must have two
parents, and both the merge and its first parent must use the native-only surface schema (3 or later). Put notes for
older merges in `changes/history/`. The notes that merge brought itself count as coverage, apart from the late ones; their
ids are not checked again for staleness, since they were when the merge landed. The pull request's own comparison
leaves notes with `landed` out, and a note without `landed` is checked as before.

The reader checks new top-level notes; a note with an unknown key, a missing `ids` or `why`, an empty `ids`, or an id
that did not change is refused. `changes/history/` holds backfilled notes in the same format. The reader does not
scan that directory, so they neither waive the gate nor appear in the SDK diff. CI checks engine notes against the
catalogs it compares, but SDK publishing leaves engine ids out of its API-only comparison. If a note names both
API and engine ids, publishing still checks its API ids and carries the note for those that changed. `sdk.json`
holds these notes since the previous SDK (`surface.notes`); `cargo sunrise-mod pin` prints a changed item's
`migrate` line beside it.

For updating native declarations whose game modules live under `client_side` or `server_side`, see
[Repinning native targets for game side modules](../../how-to/native-mods/16-repinning-side-module-targets.md).

## Behaviours

A few promises are neither a type nor a signature, so no other record can pin them: how `sunrise:add_to` files merge,
or what block entity a block with a vanilla template gets. Each is a `behaviour:<name>` record, generated from one
file in `crates/sunrise-mod-vocab/behaviours/<name>.toml` (the file name is the name; there is no list to keep):

```toml
version = 1
doc = "What the engine promises, stated so that a test could check it."
tests = ["crates/sunrise-client/src/some_tests.rs::the_test_that_pins_it"]
```

`version` starts at 1 and goes up when a mod that relies on the behaviour can see a different meaning. `tests` lists
the tests that pin the promise, each as `<file path from the repository root>::<fn name>`. The export validates the
TOML and link syntax without opening the proof files. The separate `published_behaviour_proofs_exist` test checks
that each file parses as Rust and contains the named function. `doc` and `tests` are outside the record's hash.

The diff treats a behaviour like any record: a new one is additive, a removed one is breaking (so it needs a change
note that names `behaviour:<name>`), and a `version` change is conditional, for the mods that rely on it. Nothing in a
mod's code uses a behaviour, so it never appears in a fit report. Only promises kept across builds belong here, not
diagnostics or behaviour that vanilla fixes.

## Hook versions

A hook's `version` changes when a listener can see a different meaning under the same signature: its answer now
decides the action, or the order or timing it observes changed. It stays when the engine is only corrected to the
contract the hook already documents. Every bump comes with a change note in the same change, so `sdk diff` and
`cargo sunrise-mod pin` say what a listener must now do; the version is outside the hook's hash, so the diff
counts a bump as a conditional change for the mods that name that version.
