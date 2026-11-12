<!-- chapter: Server attachments -->
<!-- separator: 1 -->

### Server attachments

A mod can declare an `AttachmentType<T>` once and call its `get`, `set`, `remove`,
`has` and `get_existing` methods with the hook's host and a `Holder`. `T` is
`Clone + Send + Sync + 'static`; `get` returns a clone and inserts the default on
first access. `has` and `get_existing` read without inserting (NeoForge's `hasData`
and `getExistingData`): `has` says whether a value is stored, saved or set, even
before it decodes, and `get_existing` returns `None` instead of the default. Neither
adds anything to the holder's saved data. `set` runs the optional codec before
changing the value, returning an `AttachmentError` if it cannot encode.
Keys are the pair `(namespace, name)`; a key already decoded as another Rust type is
an error. On a server, a missing holder is an error, not a new phantom holder; a client
keeps a copy for any holder it is asked about (see the sync paragraph below).
`AttachmentError` distinguishes `MissingHolder`, `TypeMismatch`, `Decode`, `Encode`,
`InvalidNamespace` (a malformed saved compound), `InvalidCopyPolicy` (death-copy
without a codec) and `InvalidSyncPolicy` (sync without a codec).
Every host is an `AttachmentStore` (and `host.attachments()` gives it): the typed methods
stand on its few erased ones, so they work the same in a native mod and a wasm mod. A
value keeps its own Rust type, shared: reading one clones it out, and copying a holder's
data copies no value.

`AttachmentHolder` keeps an existing typed handle when a system needs to store a holder
identity: `Entity`, `Player`, `BlockEntity`, `Chunk` or `Level`. It is `Copy`, does not keep
anything loaded, and follows each handle's ordinary stale-world and unload rules. Unlike
the storage-relative `Holder`, it carries the handle's world scope.

Import `AttachmentQueries` to use `host.query(&TYPE)` on either host side. The
returned batch is a **snapshot** of membership and stored value bindings in the
host's current dimension. Its `iter()` borrows `&T` without cloning values and
lazily decodes saved entries; malformed or incompatible values produce row errors,
not defaults. Later sets, removals and unloads appear only in fresh queries. This
is a residency view: an entity queued for removal but not yet swept can still
appear. Captured handles do not keep holders loaded, and stale writes fail.
Interior mutability inside `T` is not deep-frozen by a snapshot.

On a server, import `AttachmentQueriesMut` for `host.query_mut(&TYPE)` (`T: Clone`).
Each borrowed row has `holder()`, `value()`, and explicit `set(host, replacement)` /
`remove(host)` methods. These validate residency against the host's current
dimension and then use the ordinary attachment journal. There is no `&mut T`,
implicit transaction, or commit on drop: wrap a system in `host.transaction` for
all-or-nothing writes. Unload or replacement produces `AttachmentRowError::StaleHolder`.
Rows retain captured values even after successful write-through updates.

Traversal follows dense index order: insertion appends, and removal fills its slot
with the last row. The same ordered mutation history is deterministic, but the
order is neither sorted nor stable across structural changes. Retaining a batch
can make the next live mutation copy row metadata and shared pointers once;
retaining many batches keeps their old bindings alive. Native acquisition clones
only the bucket Arc. Erased values from an incompatible
type domain report `TypeMismatch` rather than falling back to saved decoding.

```rust
use mod_api::{AttachmentCodec, AttachmentError, AttachmentType, Compound, Holder, Tag};

static COUNTER: AttachmentType<i32> = AttachmentType {
    namespace: "example", name: "counter", default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("count".into(), Tag::Int(*n))].into()),
        decode: |data: &Compound| match data.get(&"count".into()) {
            Some(Tag::Int(n)) => Ok(*n), _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};
// Inside a server hook, or mixin code, with `host: &mut dyn ServerHost`:
if !COUNTER.has(host, Holder::Chunk([0, 0]))? { /* chunk carries no counter yet */ }
let old = COUNTER.get(host, Holder::Chunk([0, 0]))?;
COUNTER.set(host, Holder::Chunk([0, 0]), old + 1)?;
Ok::<(), AttachmentError>(())
```

For mixin code, see [The typed API from mixin code](005-the-typed-api-from-mixin-code.md#the-typed-api-from-mixin-code).

Use `Holder::Level`, `Holder::Chunk([x, z])`, `Holder::BlockEntity([x, y, z])`,
`Holder::Entity(id)`, or `Holder::Player(uuid)`. Entity IDs cover every entity in
the level apart from players: mobs, items, experience orbs, projectiles, tridents,
falling blocks, primed TNT, fireworks, boats, minecarts, armor stands, cushions, item
frames, paintings, End crystals, the ender dragon, ominous item spawners and the
server's `OtherEntity` records. An entity keeps its UUID through saves, so its saved
attachments load back with it; an orb saved with a `count` above one loads as several
orbs, and only the first keeps the attachments. Player UUIDs cover online guests and
the acting player. Mutable level
hosts can access their level, loaded chunks and block entities; read-only level hosts
cannot access attachments. Unserialized values disappear on unload or restart. A saved
value is stored under `horizon:attachments`, then the mod namespace and attachment name,
as `{value: <compound>, copy_on_death: 1b, sync: 1b}` (a flag is omitted when false).
Unknown mods' saved entries survive load and save without being decoded. Level
attachments use the dimension's `data/minecraft/horizon_attachments.dat` SavedData
file, not `level.dat`.

A type with `sync: true` sends its value to Horizon clients when `get` creates the
default, on `set` and `remove`, and on demand with `AttachmentType::sync_data(host,
holder)`; changes made in one tick are sent once, at its end. The recipients are the
players who track the holder: guests who were sent a chunk for a chunk or block entity,
who track an entity for an entity, the player and those tracking them for a player, and
everyone in the level for the level. A client also gets all of a holder's synced values
when it starts seeing it: a chunk sent (with its block entities), a level joined or
re-entered, an entity tracked, and its own player. With no native
mod loaded nothing is sent. The wire form is the codec's compound, so `sync` requires
a codec.

Only Horizon clients are sent values. A message with a namespace or name over 256
characters, a value that cannot be written or a payload past 1 MiB is not sent.

On a client, `Host::attachments` returns a copy of what the server synced, which `get`
decodes; `set` and `remove` change only that copy, which is dropped when the chunk,
entity or player goes. A client keeps no copy for the level hosts (`LevelRandomHost`):
they stay server-only there, though their changes on a server are synced.

`copy_on_death` requires a codec. Player respawn copies opted-in entries via their
encoded representation, including raw entries of an absent mod; other entries are
removed. Living-entity conversion is not supported. This follows the default supplier,
optional server-side persistence, player-respawn option and `sync` with a `StreamCodec`
(with `syncData`) of NeoForge 26.3's public `AttachmentType.Builder` contract, and
`IAttachmentHolder`'s `hasData`/`getExistingData`, but not its holder-aware supplier,
custom copy handler, conditional serialization, the `sendToPlayer` predicate,
`AttachmentSyncHandler` partial updates or a wire codec apart from the disk codec.

`player_clone` supplies `event.original: OriginalPlayerRef` alongside the replacement's
`event.player` UUID. During that hook, `event.original.attachment(host, &COUNTER)` reads
the old value even when `copy_on_death` is false or the type has no codec;
`COUNTER.get(host, Holder::Player(event.player))` reads the replacement's retained value
or fresh default. This works on death and on return from the End, for native and wasm.
The original's `position`, `dimension`, `level`, and `inventory_slot` methods read the
same pre-copy player. Inventory slot numbers are 0–35 for the main inventory,
100–103 for boots through helmet, and -106 for the off hand. Every original read expires
when the hook returns; keeping the handle does not keep the old player alive.
The snapshot retains shared attachment bindings, not a deep freeze of interior mutability.
With no `player_clone` listener, no original snapshot is allocated.

The API owns these public names. Their engine-independent storage and query index
live in `horizon-mod-runtime`, which depends only on the shared vocabulary in
`horizon-core`; world and game storage use the same implementation.
