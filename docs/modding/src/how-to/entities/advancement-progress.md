# Observing advancement progress

Subscribe to `player_advancement_progress` to inspect the whole advancement immediately after
one criterion is granted or revoked, before its completion rewards. `event.progress.criteria`
includes obtained and unobtained criteria; `obtained_at` is the original saved grant date or
`None`. `requirements` lists the OR groups, `completed_requirements` counts satisfied groups,
and `done` is the game's completion result. A player-issued `/reload` mutates and
snapshots the acting player's live progress, including during a nested turn.

Keep `event.progress.clone()` if it is needed later: the owned `AdvancementProgress` snapshot
cannot change the player's progress and is not updated by subsequent events. No-op grants and
revokes do not fire.

To read a server player's live progress on demand, call `player.advancement_progress(host, &id)`.
It takes `&dyn Host` and returns the same `Option<AdvancementProgress>` snapshot, mirroring
`ServerPlayer.getAdvancements().getOrStartProgress(holder)`: a loaded advancement the player has
not touched returns all criteria with `obtained_at: None` and `done == false`, and reads inside
`player_advancement_progress` or `player_advancement_done` observe the post-grant live state.
Unknown ids, stale or offline players, and client hosts return `None`.

To award or revoke an advancement (or one criterion of it) on the server, call
`player.award_advancement(host, &id, criterion)` and `player.revoke_advancement(host, &id, criterion)`.
Both take `&mut dyn ServerHost` and `criterion: Option<&str>`, mirroring
`PlayerAdvancements.award`/`revoke` and `AdvancementCommands.Action.GRANT`/`REVOKE`: pass
`Some(name)` for one criterion or `None` for every remaining (or obtained) criterion. They route
through the same engine path as triggers and `/advancement`, so completion grants rewards,
announces in chat per the display and `show_advancement_messages`, shows the toast, fires
`player_advancement_progress` and `player_advancement_done`, and queues the client progress
update. `Ok(bool)` reports whether progress changed; `Err(AdvancementError)` distinguishes
`NotServer`, `PlayerUnavailable`, `UnknownAdvancement`, `UnknownCriterion`, and `NoBackend`.
Inside a transaction, validation runs immediately and `Ok(bool)` reports whether the change
would alter current progress; the change itself, along with its rewards, messages, hooks and
client updates, happens at the root commit and is dropped on abort.

## Display icons

An advancement's `display.icon` is an item stack, including its count, damage and
supported component patch. The toast, advancement widgets, tabs and hover draw that stack;
LAN sends it through the same item-stack conversion as inventory synchronization, including
explicit component removals resolved through the peer's component registry.
Remote clients convert it once when receiving the advancement, not each frame.

If a remote icon fails the custom-component permission check (including malformed
nested templates), the advancement remains visible with the same item and count.
The **entire component patch** is discarded: every addition, including damage and
glint overrides; every removal; and cached wire bytes. Item defaults apply instead.

### Trying the icon surfaces

The runnable datapack `crates/sunrise-client/tests/fixtures/advancement-icons`
contains two root advancements with wooden-pickaxe icons, count 3 and damage 7:
`sunrise_test:glint` explicitly enables glint; `sunrise_test:plain` disables it.
Its titles and descriptions use vanilla translations.

Copy the fixture directory into a **fresh test world's** `datapacks/` directory,
then run `/reload`. With cheats/operator permission:

```text
/advancement grant @s only sunrise_test:plain
/advancement grant @s only sunrise_test:glint
```

Watch the two toasts: only the second pickaxe shimmers, both show count 3 and a
partially depleted durability bar. Press **L** to open Advancements, select the
fixture's glint tab, then hover its pickaxe widget. The tab, widget and hover must
all shimmer and retain the count and damage bar; the plain tab must not shimmer.
Repeat on a second Sunrise client connected to the test world's LAN server.
To replay the toast, revoke that advancement and grant it again:

```text
/advancement revoke @s only sunrise_test:glint
/advancement grant @s only sunrise_test:glint
```

The native regression
`game::client_side::advancements_client::icons::tests::advancement_icons_reach_local_and_remote_ui_surfaces`
checks actual GUI draw submissions at 640×480 with GUI scale 1, including the
two permission-failure cases in isolated child processes. The resource-free
`lan::advancements::tests::advancement_icons_roundtrip_count_damage_and_opposite_glint_overrides`
checks the LAN packet, remote stack conversion and byte differences when only
the same icon's glint override changes.

## Advancement triggers

Name the custom trigger in an advancement criterion's `trigger` field. The engine binds the
criterion's `conditions` when loading the advancement. The engine decodes `player` using vanilla's
loot condition codec (use `"type"` for an inline condition). A malformed `player` field rejects the
advancement. On the server, call `player.trigger_criterion(host, &trigger, &mut predicate)`.
Implement `CriterionTest::test(&mut self, criterion: &dyn CriterionContext)` on the predicate.
It can borrow the triggering event's data and is not retained by the game.

Read mod-specific fields through `criterion.conditions()`, a borrowed `mod_api::serde_json::Value`.
Use `criterion.item("pipe", &stack)?` to test an item field using the target player's level and tags.
A missing `conditions` value is null. The engine tests `player` after the mod predicate passes and
awards matches through the normal advancement path.

Use methods such as `trigger_consume_item`, `trigger_block_use` and `trigger_item_used_on_block`
for those built-in triggers. Only exact built-in keys are reserved; the namespace alone does not
reserve a key. Block-use triggers read the event block in the target player's level, loading its
chunk if needed. A location predicate checks the position it actually tests; a tool predicate does
not require any additional chunks. If the player's level cannot be entered, the call returns
`PlayerUnavailable`; if the event block cannot be read, the block-use call returns `BlockUnavailable`.
Return `CriterionTestError::Refused(message)` when a custom predicate cannot answer. A refusal or
panic stops the current matcher invocation only. A panic logs and shows an error toast naming the
mod and returns `TestPanicked`; a later call can lend another matcher for the same trigger.
