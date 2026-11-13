# Client effects and tooltips

Use client services to edit tooltips, draw extra camera views and change local sound output. See the [API reference](../../reference/api.md) for the generated signatures.

## Reading the tooltip preference

Read `ClientHost::gui_options().advanced_item_tooltips` for the current
`advancedItemTooltips` preference (F3+H). The snapshot reflects the current client
setting on native and wasm; request a new snapshot after a toggle. This is separate
from the advanced flag supplied to an individual tooltip request.

## Adding tooltip lines

Use `ItemPart::tooltip` for an attached item's own lines at the vanilla
`Item.appendHoverText` position, after the name and before component lines.
Use `Hooks::item_tooltip` for NeoForge's global `ItemTooltipEvent` moment: it
receives every item's finished tooltip, including the parts' earlier lines,
component lines and advanced information. No attached part is needed for a
global listener, so this is also how a mod edits vanilla or other mods' tooltips.
The global event supplies the stack, display settings, player IDs, and the
request's `advanced` and `creative` flags on native and wasm alike.

## Drawing a second world camera

Native internals mods can request extra world cameras through `world_views` and
consume their textures in `world_view_drawn`. These are raw renderer callbacks, not
portable API callbacks: wasm mods cannot use them. Construct a `WorldView` with a
stable ID, camera, yaw, pitch, field of view and pixel size. The hook is asked
once a frame; at most four valid, distinct IDs draw, with sides limited to 4096
pixels and the device limit. Each view culls the world with its own frustum, and
the player's view keeps its own culling.

For a view through a portal, add `with_clip_plane([x, y, z, d])`: nothing behind
that world-space plane is drawn, and the camera must be behind it. In such a view
an entity wins the depth test against terrain up to 1/1024 of its distance in
front of it (about 1 block at 1024 blocks). For a mirror,
give the camera reflected through it and add `with_mirror()`: the picture is
flipped left to right, with face culling flipped to match, so the same faces are
culled as in an unmirrored view. A clip plane with a zero normal, a value that is
not finite, or the camera in front of it leaves the view out.

Use `world_view_drawn` to copy a still to your own buffer or draw into the supplied
target with its encoder. Copy a picture you need to preserve: targets are reused
while requested and retired when absent. Views draw before `world_drawn`, where
you can composite their pictures into the player's frame. Extra cameras increase
rendering work and do not include particles, weather, hands or shader-pack passes.
Extra views also omit the sun, moon, stars, clouds and block outline. Translucent
terrain uses the player camera's sorting. See the generated `world_views` hook notes
for the rendering contract.

## Muffling sounds

`ClientHost::muffle_sounds(amount, lowpass, ticks, exempt)` temporarily muffles the sound output, for example to accompany ringing ears after a nearby blast:

- `amount`, from 0 to 1, is the fraction of every sound's volume taken away.
- `lowpass`, from 0 to 1, dulls the output, from untouched to a 300 Hz low-pass.
- Both effects start at full strength and fade linearly over `ticks` client ticks. The fade waits while the game is paused.
- The sound events in `exempt` remain clear, including ones already playing. Use this for the mod's ringing sound. Other sounds already playing, music and interface clicks are muffled too.

A new call replaces the current muffle; `ticks` of 0 ends it immediately. The effect belongs to the sound output, so every splitscreen player hears it. The call refuses an amount or strength outside 0 to 1, more than 64 exempt events, or an exempt event that is not a namespaced sound ID, without changing the output. Server code cannot call this client-only service.

Play the accompanying sound through `ClientHost::play_local_sound` with a `SoundPlay`.
