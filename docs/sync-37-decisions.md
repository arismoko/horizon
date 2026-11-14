# Sync 37 decisions

Base: `cbeae4649b2f8321528d2e63f75c55da536555e7`.
Upstream pin: `2726656f2ae851f2830bdfd58ca49956218f895f`.
Its own main-push checks succeeded in run `43456290314`.

## Deferred move restoration

The move-script refinement precedes the mechanical moves and fork reapplications in separate commits.
Twenty destination rows complete upstream's earlier Game extraction. Fork edits follow the moved code,
including the current main `&HurtSource` durability signatures and menu environment construction.
The reverse window-handler move, fluid-aware ray helper and three renamed counterparts are included.
The census script precedes the renderer extraction. All nine census methods remain active; only their
obsolete held-mesh accounting is removed. No replacement raw world-hold API is introduced.

The name-only Game measurement against the frozen pin reports **9 matches**, not zero:
`key`, `level` and `sea_level` are unrelated `Dim` methods; `render_with`, `in_own_process`,
`all`, `stage`, `native` and `exists` match unrelated methods outside upstream's Game directory.
These are name-only matches, not game.rs moves: zero genuine move remnants remain.
Measured afresh against the frozen pin, the base has 99 name-only matches occupying 4,882 function
lines; the restoration has 9 occupying 983 function lines. Game shrinks from 15,507 to 12,270
physical lines (264 to 167 functions; 12,893 to 10,061 function lines). Upstream's game.rs has
10,128 function lines. These are actual base/head measurements, not reused earlier-sync figures.
The generated baseline has zero allowances; no baseline was raised.

Newer main overlaps were inspected against the reference replay: generic `Clicks<Option<ItemStack>>`,
`menu_env()` in the moved menu click handler and custom drowning bubbles in guest ticks are preserved.

## Diverges from upstream

The fork retains its typed mod API, generated topic layout, native build policy and launcher repository.
The upstream raw `native_world_hold` route remains deliberately unavailable by maintainer ruling.
Without a listener upstream's default hold covers nothing; deleting the unreachable infrastructure
therefore does not change default non-mod gameplay. The following seven isolated tests of the deleted
hold type are removed, not claimed as passing:

- `a_hold_covers_sections_by_their_middle`
- `bad_circles_are_left_out_and_there_are_at_most_max_areas`
- `a_hold_starts_and_lets_go_when_no_one_asks`
- `a_hold_ends_by_itself_and_stays_ended_until_let_go`
- `smaller_circles_let_go_what_they_leave`
- `what_waits_is_counted_and_capped`
- `columns_going_and_the_sections_cleared_drop_what_waits`

The upstream waypoint optimization is imported while retaining fork-owned guest state and interfaces.
No protected SDK export, prebuilt package layout or injection routing design changes are part of this sync.

## Main merge after rollup 79

Merged fresh main `159024296f4` without rewriting history or changing the frozen upstream pin.
The only conflicted file was `game/runtime/window_handler.rs`: main added title paging inside
`handle_window_event_inner`, which this sync had already restored to `game.rs`. Keep that restored
owner and port main's `page_missing_mods` key arm into it; do not reintroduce the duplicate method.
Main's accepted-mouse `mod_mappings_button` and raw-key `key_input` changes merge unchanged in the
runtime adapter. All other main carries merge unchanged, including the generated hotspot
classification supplied by main. The four corrected source-extraction test paths remain intact.

Fresh measurement of the resolved tree using main's imported measurement tool reports zero genuine
remnants and the same nine classified name-only matches occupying 983 lines. Its parser reports
12,274 physical lines, 168 functions and 10,083 function lines in `game.rs`; these counts use the
new main tool and are not directly comparable with the earlier tool's function counts above.
No measurement tool or classification was edited by this conflict resolution.

Python guards for the four repaired source-fixture modules plus input-key, mouse-button and hotspot
measurement tests passed (15 tests). Rustfmt ran only on the two resolved Rust files. No local Cargo
check, clippy or tests were run for this merge.
