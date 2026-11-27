# What a special item renderer costs a draw

A registered renderer extracts a typed argument and submits baked geometry on each draw. The game
hands extraction a borrowed stack rather than hashing it or copying its patch, and it does not
cache a draw recipe. The cost of component reads and custom submission depends on the renderer.
The `built_in_scenes_cost_nothing_more_without_a_listener` regression test compares vertices and
allocation counts with and without an unmatched registration.

## Measuring direct submission

The `special_item_draw_cost` benchmark times `ClientGame::resolve_item` and `submit_item_unlit` for:

- `built_in_model`: stone's own `minecraft:model`.
- `special_direct_submission`: a registered renderer submitting the same stone geometry from
  a bake-time lease.

It checks that the vertex counts match and reports median, p95 and p99 after warm-up. Run it with
prepared resources on an optimized build:

```sh
SUNRISE_TEST_RESOURCES=$PWD/.local/sunrise-client/26.3 cargo test --profile fast -p sunrise-client --lib -- --ignored --nocapture --exact cut_layout::game::client_side::native_special_renderers::cost_tests::special_item_draw_cost
```

## Historical: cached recipes

The [original measurements](https://github.com/sunrise-org/sunrise/blob/ff508096661f41601fdb3905f48618a3a216bb28/docs/performance/special-item-renderers.md#measurements)
compared the built-in cube with `special_kept` (a cache hit) and `special_first_extraction`
(a fresh recipe). Both special cases measured the removed recipe path; neither measures the
current typed extraction and direct submission path.
