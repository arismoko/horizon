# Incoming fragments at 501446443c

## living_death_message — converted

Converted with tools/convert_sync_fragment.py to the actual catalog location,
crates/horizon-mod-vocab/src/hooks (horizon-mod-api/src/hooks does not exist).
A server collect hook edits Option<Text> through the generated ServerHost dispatcher;
no new public value type or Wire implementation is needed. LivingDeathEvent and
LivingDeathPostEvent describe different moments, not the owner message. The game
fires after the owner-online and show-death-messages guards. Removed the unused
incoming library wrapper calling a nonexistent native_client_hooks dispatcher.
No runtime claim; CI must validate generated dispatch.

## line_of_sight

Refused as a hook fragment: it has no hook moments, only read helpers.
Existing typed LivingRef::has_line_of_sight (host/living.rs) and LevelRef::clip
(host/levels.rs, collider and no-fluid choices) own these queries. Upstream's
native_entity_refs.rs clips eye-to-eye within the current level or clips an
empty-context segment. Do not add native_ aliases or a second notification.

Converter boundary: `unsupported fragment fields: ['comments', 'doc_summary', 'helpers', 'rust_summary'] or draw contexts; helpers are host methods now, written by hand in crates/horizon-mod-vocab/src/host/<area>.rs`.

## map_colors_read

Refused as a hook fragment: there are no hooks. Existing LevelRef::map_data
(host/levels.rs) returns the typed MapState with colors and metadata. Upstream's
maps.rs native_map_read reads a resident map or its saved file without loading
it, refusing clients and absent worlds. A second map_colors/map_meta API would
split the existing typed read into raw-id aliases; none was added.

Converter boundary: `unsupported fragment fields: ['doc_summary', 'helpers', 'rust_summary'] or draw contexts; helpers are host methods now, written by hand in crates/horizon-mod-vocab/src/host/<area>.rs`.

## npc_entity_lookup

Refused: no hook moment exists in this helper-only fragment. Upstream's
native_npcs.rs searches placed players and current-level UUID holders, and
entity_nbt_in temporarily enters a loaded dimension. The fork has typed
EntityRef::nbt (host/entities.rs), not dimension-string/id entry points.
The immutable UUID-search helper is not represented by an equivalent typed
query in this scaffold; it is deliberately refused rather than smuggled in as
a notification or a raw-id compatibility alias. No new dispatcher is needed.

Converter boundary: `unsupported fragment fields: ['comments', 'doc_summary', 'helpers', 'rust_summary'] or draw contexts; helpers are host methods now, written by hand in crates/horizon-mod-vocab/src/host/<area>.rs`.

## riding_getters

Merged into existing typed queries, not new hooks: EntityRef::vehicle and
EntityRef::passengers (host/entities.rs, entity_vehicle/entity_passengers).
Upstream native_travel.rs reads actor/guest/mob mount ids and enumerates riders;
the fork already owns resident handle queries with explicit off-level semantics.
No hook moment, duplicate notification, native_ alias, or new type was added.

Converter boundary: `unsupported fragment fields: ['comments', 'doc_summary', 'helpers', 'rust_summary'] or draw contexts; helpers are host methods now, written by hand in crates/horizon-mod-vocab/src/host/<area>.rs`.

## server_settings

Refused: helper-only fragment, no hook moment. Upstream native_server.rs
reads dedicated_lan's online-mode config, actual listening port, and a
velocity/none forwarding classification without disclosing its secret; before
listening or on another side it returns None. No equivalent public typed server
configuration query exists in this scaffold. Converting that read into an event
would invent a notification and not provide the requested query. The converter
rejects helpers; no native_ aliases or internals access were introduced.

Converter boundary: `unsupported fragment fields: ['comments', 'doc_summary', 'helpers', 'rust_summary'] or draw contexts; helpers are host methods now, written by hand in crates/horizon-mod-vocab/src/host/<area>.rs`.
