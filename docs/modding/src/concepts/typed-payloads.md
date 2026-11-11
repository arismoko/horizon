# Registered payloads and the typed-payload design

The registered **raw** transport implements [#2882](https://github.com/arismoko/horizon/issues/2882).
Typed registration/token binding, configuration snapshots, and attachment migration remain work for
[#813](https://github.com/arismoko/horizon/issues/813). They are not implemented by the raw API.
The transport contract is detailed in [Registered raw custom payload transport](../../../design/raw-custom-payload-transport.md).

## Register raw payloads once

Register during `Hooks::register`, in dependency order. `payload_handlers().registrar("1")`
returns a `PayloadRegistrar` view. Versions are exact, nonblank strings, not semver ranges.
Main-thread handling and required peer support are the defaults.

```rust,ignore
use mod_api::{IPayloadContext, IPayloadHandler, PayloadBytes, PayloadId, Registrar};
use std::sync::Arc;

struct Update;
impl IPayloadHandler for Update {
    fn handle(&self, bytes: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        // Decode this application's bytes here. Network handlers must enqueue
        // main-thread work before requesting a world Host.
        let _ = (bytes, context);
        Ok(())
    }
}
fn register(registrar: &mut dyn Registrar) {
    registrar.payload_handlers().registrar("1")
        .play_to_server(PayloadId("example:update".into()), Arc::new(Update));
}
```

The registrar has `play_*`, `configuration_*`, and `common_*` methods for
`to_client`, `to_server`, and `bidirectional`. Common occupies both protocols.
A client handler may be supplied at registration or through
`RegisterClientPayloadHandlersEvent` before the installed table freezes.
`optional()`, `versioned(...)`, and `executes_on(HandlerThread::Network)` return
modified views; they do not modify the original view. Duplicate protocol/id
registrations, invalid ownership or versions, and missing receiving handlers are
refused. An id grants no authority merely because a caller constructs a `PayloadId`.

Native and wasm use the same declarations, opaque `PayloadBytes`, handlers, and
results. A callback receives the real connection's `IPayloadContext`, not a
broadcast event. Dispatch selects the one registered owner.

## Admission, sending, and context

Configuration exchanges declarations and freezes the compatible peer intersection.
A missing required declaration refuses admission. Matching protocol/id declarations
must agree on version and direction even when optional; mismatches refuse with the
id, reason and both declarations. Only one-sided optional absence permits admission,
and send then returns `UnsupportedPeer`
without a packet. When neither peer declares payloads, no mod manifest is exchanged.
There is no per-tick scan of loaded mods.

Clients send through `ClientHost::send_payload_to_server`; servers use
`PlayerRef::send_payload`. `IPayloadContext` offers `player`, `reply`, `disconnect`,
`enqueue_work`, and sided Host access. Configuration has no play player. A NETWORK
handler has no direct world access: enqueue work to obtain the receiving main-thread
context. Wrong-thread and wrong-side requests return explicit errors.

`Queued` means accepted locally, not acknowledged by the peer. Sending reserves
physical/provider credit before acceptance. Root commit publishes staged sends;
rollback publishes none. Replies enter a subsequent receiving turn, including
integrated play. Play delivery shares the connection's ordinary gameplay inbox, so
movement/actions before a raw message are applied before its MAIN handler, and
later inputs wait until that handler's root transaction finishes.

Work and prepared sends bind to a physical connection and protocol incarnation.
Reconfiguration invalidates old queued work; returning to Play cannot revive an old
prepared packet. Original input credit remains held through queued work and derived
actor effects, not merely until dequeue. Queue pressure returns `Backpressure` or
refuses the flooding session; there is no 32-message-per-tick rule or silent truncation.

## Capacity and interop

Registered application bytes use their own namespaced vanilla custom-payload channel.
There is **no application envelope**. Unknown unrelated plugin channels retain
vanilla's discard limits: 32,767 serverbound and 1,048,576 clientbound body bytes.
Those numbers do **not** cap a registered body.

A network frame must fit 2,097,151 encoded bytes. Compressed transport also limits
the complete uncompressed packet to 8,388,608 bytes. Both include packet/channel
framing; highly incompressible data may hit the encoded limit first. Private memory
connections do not inherit network framing caps. Resource quotas still apply and
are resource refusals, not a smaller codec body limit. Frame preparation is reused
for writing rather than recompressing solely for validation.

`ClientHost::send_packet` still supports unrelated vanilla/plugin interoperability.
It validates the packet and reserves outgoing credit, but publication waits for root
commit and the same physical protocol incarnation. It refuses every locally owned
registered channel and engine-reserved channel; it cannot bypass negotiation,
direction, or service authority. The former envelope hooks and tick-capped send
methods are removed, without aliases or fallback handlers. Private custom books
advertise their own version on `horizon:recipe_book_settings`; book support does not
depend on a mod transport flag.

## Stage 1: capacities, tokens and send services

`crates/horizon-mod-vocab/src/content/payloads.rs` holds the API-owned types the later stages build on. Token values, including Wire-decoded ones, are untrusted claims. Only admission against the engine-owned frozen declaration table grants a slot; there is no send implementation yet.

| Item | Contract |
| --- | --- |
| `SERVERBOUND_PAYLOAD_CAPACITY`, `CLIENTBOUND_PAYLOAD_CAPACITY` | Discard-fallback body limits only, not registered-payload caps |
| `PayloadPhase` (`Configuration`, `Play`, `Common`), `ConnectionPhase` | `permits` says whether a declaration may travel in the connection's phase |
| `PeerPolicy` (`Required`, `Optional`) | As under [Negotiation and absent mods](#negotiation-and-absent-mods) |
| `PayloadTokenDirection` (`ToServer`, `ToClient`) | The direction of a staged token, distinct from raw registration's bidirectional `PayloadDirection` |
| `PayloadToken` | The erased token: slot, phases, direction and the load session that issued it. Private fields, issued by the game. `admit(direction, load_session, phase, declarations)` returns the slot or `StaleToken`, `WrongDirection`, `WrongPhase`, after exact comparison against the engine-owned frozen slot table, before any codec or handler runs |
| `ToServer<T>`, `ToClient<T>` | A `PayloadToken` bound to its value type and direction. Binding a token of the other direction fails |
| `ServerboundPayloads::send_erased(token, body)`, `ClientboundPayloads::send_erased(player, token, body)` | The two object-safe send services, one erased operation each. A constant in the module stops compiling if either stops being usable behind `dyn` |
| `PayloadTokenSendError` | `PayloadTooLarge`, `UnsupportedPeer`, `Backpressure`, `StaleToken`, `WrongDirection`, `WrongPhase`, `NotConnected`; has a Wire form |

The typed send helper (stage 2, with the codec) is generic and lives on the token: `ToServer<T>` encodes a value and calls `ServerboundPayloads::send_erased`, `ToClient<T>` calls `ClientboundPayloads::send_erased`. A `ToClient<T>` has no method taking the serverbound service, which is the compile-time refusal the acceptance matrix asks for. The connection epoch is the connection's, not the token's: the send service checks it with the token.

## Stage 2: the value codec

`crates/horizon-mod-vocab/src/content/payload_codec.rs` holds the pure codec half of stage 2, exported from `mod_api`, and `#[derive(PayloadCodec)]` writes a type's codec from its fields. It is the part that touches neither registration nor delivery, so it stands apart from the raw transport of [#2882](https://github.com/arismoko/horizon/issues/2882). Nothing calls it yet: the typed handler adapter that decodes a received body, and the typed send helper that encodes one, arrive with registration.

| Item | Contract |
| --- | --- |
| `PayloadCodec` | The trait a payload's value type implements: `encode(&self, &mut PayloadWriter)` and `decode(&mut PayloadReader) -> Result<Self, PayloadDecodeError>`. Both see only the buffer, never a `Host`. Implemented for `bool`, `u8`, `i8`, `i16`, `u16`, `i32`, `i64`, `f32`, `f64`, `VarInt`, `VarLong`, `String`, `Vec<T>`, `Option<T>` and `Box<T>` |
| Encoding | Vanilla's network encoding, so a ported payload keeps its exact bytes: big-endian numbers, VarInt and VarLong, a VarInt byte length before a string or byte array, a VarInt count before a list, one boolean byte before an optional value |
| `decode_payload::<T>(body)` | The whole body as one `T`; a byte left over is `Trailing { bytes }` |
| `encode_payload(&value, limit)` | The body, or `PayloadEncodeError`; the byte limit is the caller's (the transport's frame bound), checked before the buffer grows |
| `PAYLOAD_MAX_DEPTH` | 64. Each list, each present option and each `nested` level counts one; level 65 is `TooDeep` |
| `PAYLOAD_MAX_STRING_BYTES`, `PAYLOAD_MAX_LIST_ENTRIES` | 4096 each, the defaults of `String` and `Vec<T>`. A schema with another bound calls `str_within(limit)`, `bytes(limit)` or `list(limit, read)` itself, so these are not a transport ceiling |
| `PAYLOAD_DECODE_BUDGET` | 8,388,608 bytes of allocation for one value. A list is charged `count * size_of::<T>()` before it is reserved, an owned string its length, so a small body cannot ask for a large allocation |
| `#[derive(PayloadCodec)]` | The generated codec, in `crates/horizon-mod-api-macros/src/payload.rs`. A struct is its fields in declaration order with nothing between them. An enum is a VarInt tag, the variant's position from 0 whatever its discriminant, then that variant's fields; a tag no variant has is `InvalidDiscriminant`. Each field goes through its own `PayloadCodec`, so the bounds above hold inside a derived type, and it adds no nesting level of its own. An enum with no variants, a union and a field without a codec do not compile |
| `PayloadDecodeError` | `UnexpectedEnd`, `VarIntTooLong`, `NegativeLength`, `LengthOverLimit`, `Utf8`, `TooDeep`, `InvalidDiscriminant`, `NonFinite`, `BudgetExhausted`, `Trailing`, `InvalidValue`; has a Wire form |
| `PayloadEncodeError` | `TooLarge`, `LengthOverLimit`, `TooDeep`, `NonFinite`; has a Wire form. An encoder refuses what the peer's decoder would refuse |

A length is read as a signed VarInt and checked for sign and bound before anything is reserved or copied; strings and byte arrays are borrowed from the body. A boolean other than 0 or 1 is refused (vanilla reads any non-zero byte as true). `finite_f32` and `finite_f64` are how a schema declares a finite float. No decode path indexes, and none panics on any input.

Two things this stage does not do. The bounded field kinds of [#811](https://github.com/arismoko/horizon/issues/811) (`DatapackFieldKind`) are not merged with this reader: they describe JSON entries the game decodes, are named for datapacks, and their issue is in flight; a shared name for the two belongs to whoever lands second. And the budget counts what the reader's own primitives allocate, not what a hand-written codec allocates beside them; native code has to obey the contract, as the section above says.


Strings use a 4096 UTF-8 byte default, not a character count. A schema may pass 3N bytes to `str_within` for a vanilla N-character limit. Each box, list and present option consumes one of 64 nesting levels; boxes charge their inner allocation size before decoding.

## Remaining typed work in #813

The earlier accepted design is amended by the raw transport's actual codec and
framing contracts above: remove its generic registered-body ceilings, typed
application envelope/numeric-slot proposal, and 32-per-tick acceptance row.
Keep typed registration/helpers outside object-safe Host dispatch; delegate to the
same raw transport rather than adding another route. A typed token must bind its
checked declaration/type, phase, direction, and load identity. Forged or stale
wasm tokens must refuse without invoking a handler.

Share schema/value codec primitives with #811. Wire is an ABI representation, not
permission to serialize Rust pointers, memory layouts, or `Any`. Supported values
need explicit Wire forms on both targets, or a named refusal at load. Structured
codecs must check signed lengths, overflow, remaining bytes, cumulative allocation,
depth, discriminants, and registry references before allocating; require complete
consumption. Per-schema bounds must not silently reinstate a lower generic transport
ceiling. Native arbitrary allocation/process-aborting panics cannot be sandboxed.
Errors and wasm traps must abort the active transaction and discard staged sends.

Configuration snapshots still need revisioned atomic publication and acknowledgement
before admission, without overwriting local config files. Attachment migration still
needs #796's world/dimension, holder incarnation, residency join, schema/revision,
tracker authorization, full-before-delta ordering, bounded assembly, and rollback
proofs. Both use this transport; neither gets a separate networking implementation.

Acceptance for those future layers must exercise native and wasm over the real
integrated, LAN, and dedicated routes: exact typed decoding, mismatched/optional
peers, invalid tokens, malformed structured values, sender identity, ordered
application, next-turn replies, root rollback/traps, stale epochs, and resource
pressure with progress after drain. Measure wasm copying/allocation honestly; Wire
transfer is not zero-copy. Raw transport tests do not prove the unfinished typed,
configuration-snapshot, or attachment contracts.
