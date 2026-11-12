# Block fluids

Look up `level.capability(host, &FLUIDS, pos, side)` to access a block's
`ResourceHandler<FluidResource>`. Transfer fluids inside a transaction with
`move_resources`, as you transfer items. `FLUIDS` corresponds to NeoForge's
`Capabilities.Fluid.BLOCK`. Amounts are millibuckets (mB); one bucket is 1000 mB.

## Resources and tanks

`FluidResource` identifies a fluid and its data components without an amount.
Use `resource.to_stack(amount)` to create a `FluidStack`, and
`FluidResource::of(&stack)` to obtain its resource identity. A tank accepts fluid
when empty or when its contents match both the fluid and components.

| Game provider | Rules |
| --- | --- |
| Mod block tanks (`block_entity_fluids`) | Access depends on side. Saved tanks reject fluids with components. |
| Water cauldron | One bucket in three levels; insertion and extraction use whole levels. |
| Lava cauldron | One bucket, inserted or extracted whole. |
| Registered cauldron content | Uses the whole levels specified through `register_cauldron_fluid_content`. |
| Powder snow cauldron | No fluid handler. |

Cauldrons are position-based providers, not block entities. They accept registered
fluids regardless of components and discard those components; extracted fluid is
plain. An empty cauldron accepts any fluid with registered cauldron content.

If a destination accepts only part of a transfer, `move_resources` rolls back and
retries with that amount. A full water cauldron asked for a bucket when the
destination has room for 500 mB supplies one level, 334 mB, without overanswering.

## Item containers

Use `place.capability(host, &ITEM_FLUIDS)` for the fluid handler at an `ItemPlace`.
Register a provider with `r.item_capability(&ITEM_FLUIDS, "mymod:canister", …)`.
Changes exchange one item, splitting the changed item from a larger stack.

Vanilla buckets expose one 1000 mB tank. Empty buckets accept plain water or lava
and become filled buckets; filled buckets drain whole and become empty buckets.
`move_resources` transfers between item and block handlers: a full cauldron can
fill a bucket in a player slot, but a cauldron holding two levels cannot.

## Player interaction

Call `fluid_util::interact_with_fluid_handler(host, player, hand, level, pos, side)`
from your block's `use_item_on` part, and return success when it returns `true`.
This corresponds to NeoForge's `FluidUtil.interactWithFluidHandler`.

The helper looks up the block's `FLUIDS` and the held item's `ITEM_FLUIDS` through
`ItemPlace::PlayerHand`. It first tries filling the item, then emptying it if no
fluid moves. It transfers the first fluid that can move, as much as fits in one
operation. From three buckets, it fills and splits off one; a creative player's
hand stack is unchanged.

After a successful transfer, the helper emits the sound and game event at the
block's centre:

| Operation | Game event | Fluid sound |
| --- | --- | --- |
| Fill the item | `minecraft:fluid_pickup` | `fluid_pickup_sound` |
| Empty the item | `minecraft:fluid_place` | `fluid_empty_sound` |

A tank-only fluid emits the event without a sound.
`fluid_util::interact_with_handler` performs the interaction against a handler you
supply. `fluid_util::trigger_sound_and_game_event` emits only the sound and event.
These effects are emitted before the call returns: call these helpers outside
an enclosing transaction that you might abort.
