# `Game` fields grouped into `ServerState` and `ClientState`

This change (#2210) groups server-only fields of `sunrise_client::game::Game<'a>` into a trailing `server: ServerState` field (`sunrise_client::game::server_state::ServerState`) and client-only fields into a trailing `client: ClientState` field (`sunrise_client::game::client_state::ClientState`). Shared, mode, and drop-order-sensitive fields remain directly on `Game<'a>`. The authoritative field-to-owner table is `tools/game_field_owners.tsv`; compute current counts per owner with:

```sh
awk -F'\t' 'NR > 1 { counts[$3]++ } END { for (owner in counts) print owner, counts[owner] }' tools/game_field_owners.tsv
```

The stable `mod_api` surface and published loops are unchanged. Native mods that reach `Game` internals through `sunrise-internals`, `[[native.mixin]]`, `[[native.loop]]`, or `[[native.bridge]]` see `internal:field:sunrise_client::game::Game<'_>::<field>` entries for every `server` or `client` row in `tools/game_field_owners.tsv` move to `ServerState` or `ClientState` (with `Game<'_>::server` and `Game<'_>::client` added; all changes are advisory in `internals.json`).

Migrate native mod helpers and branches by updating moved field accesses from `game.<field>` to `game.server.<field>` or `game.client.<field>` according to `tools/game_field_owners.tsv`, updating any `[[native.bridge]]` on a moved field to target `sunrise_client::game::server_state::ServerState` or `sunrise_client::game::client_state::ClientState`, and running `cargo sunrise-mod internals sync`. `tools/game_field_owners.py` supports `--list` (printing each site with its justification), `--check`, and `--fix`; outside `crates/sunrise-client/src/game*` it fails closed, leaving files untouched and reporting candidate `Game` field accesses on stderr for manual update.
