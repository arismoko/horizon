# Design: a mod's container items report their fluid

This answers [#3089](https://github.com/arismoko/horizon/issues/3089). The `neoforge:fluid_container` item model
(#2707) draws the fluid a stack holds, but only vanilla's buckets could say which one. A mod's own tank, canteen or
bucket needs one seam to say it.

## The seam: an `ItemPart` hook, `contained_fluid`

One catalog hook, `contained_fluid` (`side = "client"`, `contract = "listen"`, `scope = "item"`, `key = "stack"`).
The generator turns it into `ItemPart::contained_fluid(&self, host: &dyn ClientHost, event: &mut
ContainedFluidEvent<'_>)`, its dispatch keyed by the stack's item, its Part table entry and its wasm Wire call: one
mechanism for both targets. The event carries `stack: &ItemStack` and `fluid: Option<FluidKey>`, which arrives
`None`.

Why not the alternatives:

- **The `ITEM_FLUIDS` item capability.** It is NeoForge's `Capabilities.Fluid.ITEM`, the source
  `FluidUtil.getFirstStackContained` reads, but its providers take `&mut dyn ServerHost` and an `ItemPlace` and
  hand back a transactional handler. Rendering has neither, and building a handler per drawn stack allocates. The
  hook is the read the model needs, nothing more.
- **A `register_*` hook in the item model catalog style** (as `register_item_tint_sources`). Those register a name
  that a model JSON refers to. Here the model type is fixed (`neoforge:fluid_container`) and the question belongs to
  the item, so it attaches where the item's other answers live: its Parts.
- **A texture or tint answer.** Ruled out by the issue: the model keeps drawing the fluid's own still sprite through
  its mask, tinted as a stack holding that fluid (#2707). The answer is a registered fluid, a `FluidKey`.

## Where it runs

`ItemEnv::stack_fluid` is the resolver's one question for a fluid container node, and the game's `Env` answers it:

1. An item in `minecraft:` answers from vanilla's closed set (`fluid_models::held_fluid`: water and lava buckets).
   It never asks a part, even one attached to it by key or tag.
2. Another item asks only if its parts subscribe: `item_listens(ContainedFluid, item)` is a lookup by the item in
   the Part table built at load. With no subscriber nothing is converted and nothing allocates.
3. A subscribed item gets the stack's owned snapshot (as the item model property hooks do) and its parts' answer.
4. The key becomes the fluid's id through the load-resolved registry (`fluid_id`, a `&'static str`). A key no fluid
   is registered under draws the stack as holding none and is logged once per item: the model's declared fluid
   untinted, or no fluid layer when that is `minecraft:empty`. A registered fluid with no look in an item (a
   tank-only fluid with no `horizon/fluid` data) is drawn like any held fluid, so the resolver draws the missing
   texture through the mask, as `DynamicFluidContainerModel` does for a contained fluid it has no model for: the
   mod author sees the mistake instead of an unexplained empty tank (fork-lead's ruling on #3342).

`stack_fluid` now returns `Option<&str>`: both answers are `&'static str` ids, so the resolver no longer allocates a
`String` for every held bucket.

## Cost

The resolver asks only for stacks whose model is a fluid container, once per resolve: at most once per stack per
frame. The renderer resolves item models per draw and has no per-stack state cache to put the answer in; it caches
the baked layers (`baked_with`) and the GUI vertices of a resolved state (`gui_drawn`), and both stay valid since
the answer only picks which baked fluid layer is drawn. Caching the answer by stack would mean hashing the stack's
component patch on every draw, which costs more than the lookup it saves. Vanilla items pay one prefix check;
unsubscribed mod items one table lookup.

## Failure

Native and wasm answers cross the same generated dispatch. A wasm answer's key is checked when the Wire boundary
decodes it, a native key can only come from registration, and the game checks the id again before drawing: an
answer whose key names no registered fluid draws as holding none and never panics in the game.
