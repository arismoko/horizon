# Sunrise extras

Optional conveniences built into the client: fullbright, dynamic lights, your own name tag, hiding block-break particles, auto-reconnect,
Mouse Tweaks-style inventory controls, inventory sorting, a smooth chat and a music page. **Every one is off by default**: a fresh
install looks and behaves exactly like vanilla 26.3, and turning an extra off
again restores vanilla's behaviour completely.

## Where they live

| Part | Code |
|---|---|
| Settings, parsing and saving | `crates/sunrise-client-render/src/extras.rs` |
| FPS and click counters (for native mods) | `extras.rs` (pure logic, unit tested) |
| Dynamic lights: item glow, nearest sources | `crates/sunrise-client-render/src/dynamic_light.rs` (unit tested), `game/interface/dynamic_lights.rs`, `terrain.wgsl` and `entity.wgsl` (`dynamic_block_light`) |
| HUD drawing (mods' panels, corner stacking) | `crates/sunrise-client/src/extras_hud.rs` |
| Auto-reconnect: its setting, its row, the policy and the backoff | crate `sunrise-reconnect`, which the official executable registers ([forking.md](forking.md#modding-and-rendering)); the engine shows its countdown and connects again when it asks (`Hooks::retry`, `game/network/remote/connection.rs`) |
| Game glue (input hooks, per-frame update) | `crates/sunrise-client-gameplay/src/game/interface/extras.rs`; each extra's own glue in a child module, `crates/sunrise-client-gameplay/src/game/interface/extras/` |
| Mouse Tweaks-style controls, sort buttons | `crates/sunrise-client-gameplay/src/game/interface/extras/mouse_tweaks.rs`, `crates/sunrise-client-gameplay/src/game/interface/extras/sorting.rs` |
| Sort order and its clicks | `crates/sunrise-client/src/inv_sort.rs` (pure, unit tested) |
| The music page's rows, and the pools it filters | `options_ui.rs` (`Page::Music`), `crates/sunrise-client/src/music.rs` (`track_groups`, `MusicManager::set_off`), `sound_events.rs` (`sound_except`) |
| The settings page | `extras_ui.rs`, shown as **Options → Sunrise Settings... → Gameplay** (`sunrise_ui.rs`); fullbright, dynamic lights and hiding block-break particles are on its Quality page instead (`graphics_ui.rs`), the shader pack key on its Shaders page, No AI on its AI Agent page (Tools) |

The settings are grouped by extra, under a header each, one to a row, and the strip
under the list says what the row in focus or under the pointer does. An
extra's own settings (its key, corner, size...) are greyed out while its switch
is off (`Item::parent`).

`game.rs` only gets one-line hooks (key, wheel, mouse, frame, HUD), so the
extras stay out of the way of other input and options work.

## Options storage

`sunrise-extras.txt` beside `options.txt`, in the same `key:value` format as
`sunrise-graphics.txt`. `options.txt` is untouched, so a vanilla client can share
the folder. Unknown or broken lines fall back to the defaults.

Extras' keys (the sort key) are Sunrise settings stored in that file and bound on
Sunrise Settings' Gameplay page, not in vanilla's Key Binds screen: Sunrise has no Sunrise category
in Controls, and adding one would put a non-vanilla mapping into `options.txt`.

## The extras

### Chunk pregeneration (`pregen`, default off)
- Makes `/pregen` available in single-player and hosted LAN worlds, with the existing cheat and
  permission checks. It does not start generation: run `/pregen start` explicitly.
- Saved with the other Extras settings and applied immediately; no launch argument or restart is
  needed. `--pregen` still enables it for the whole run. Dedicated servers use their own property.
- Turning it off hides the command without cancelling an existing task; turn it on again to manage
  that task, or cancel it first. See [pregeneration](pregeneration.md).

### Fullbright (`fullbright`, default off)
- Displays blocks and entities with a fully bright lightmap, including in dark caves.
- Turning it off restores the normal lightmap using the current brightness,
  time of day and effects. The brightness slider keeps its normal range.
- Held off on another's server (#3823): a multiplayer server or another player's LAN world draws the normal
  lightmap and the graphics screen hides the row, while the saved choice stays for the next world. Single
  player, a LAN host and Replay Studio keep it (`fullbright_held`,
  `game/options.rs`).

### Dynamic lights (`dynamicLights`, default off)
- A light source held by a player or mob (main or off hand), dropped as an item, or an entity on
  fire lights the blocks, entities and hand around it, as dynamic-light mods do. Not
  in vanilla; the idea is ported, not a mod's code (issue #814).
- An item glows like its block's default state (`getLightEmission`): torches 14, lanterns,
  glowstone, sea lanterns and jack o'lanterns 15, soul torches 10, a redstone torch 7; a lava bucket
  15. An entity on fire gives 15.
- Client-only, at render time: the light engine, meshes, saves and other players never see it.
  Each frame the game picks the 16 sources nearest the camera (within 96 blocks,
  `dynamic_light::select`); terrain.wgsl and entity.wgsl raise each vertex's block light to the
  source's level minus its distance in blocks when that is brighter, then light it through the
  lightmap as usual. The light doesn't stop at walls.
- Off: nothing is gathered and the shaders' loop runs zero times (the count is 0), so it costs
  nothing measurable (numbers in the pull request, #814).

### Mod audio streams (`modAudioStreams`, default on)
- Mods can play audio by supplying a native source to `ClientHost::open_audio_stream`
  ([the how-to](modding/src/how-to/client/audio-streams.md)).
  Off, the mixer still consumes each stream's audio at its own pace, but none of it is heard.
- The one extra that is on by default; its line is written to `sunrise-extras.txt` only while it is off. It lives in
  `crates/sunrise-official/src/mod_audio.rs` and sets the engine's mod-stream gain (`sunrise_platform::sound`).

### Moved to native mods

The **FPS counter**, **Keystrokes** and **Zoom** are official native mods now (in
horizon-rust/horizon-mods under `native/<name>/`, [mods.md](modding/src/players/installing-mods.md#sunrises-own-mods)), with the same look and
options. Players install them from the launcher's Mods page, and the launcher builds them into the game
([modding/src/how-to/native-mods.md](modding/src/how-to/native-mods.md)). The game keeps
the hook points they use (`crates/sunrise-gameplay-common/src/game/native_hooks/`, `crates/sunrise-gameplay/src/game/native_hooks/`, and `crates/sunrise-client-gameplay/src/game/native_hooks/`). Sunrise Settings' Gameplay page keeps a **Zoom** group while the
mod is built in: its switch and Settings button.

### Own name tag (`ownNameTag`, default off)
- In third person (F5, either view), the player's own name shows above its head,
  as other players see it: the profile name in its team's prefix, suffix and
  colour, the below-name score under it within 10 blocks, faded and without the
  see-through copy while crouching. Vanilla never names the camera entity
  (`LivingEntityRenderer.shouldShowName`).
- Hidden in first person, while sleeping, in spectator mode, while invisible, and
  whenever the team's name tag visibility would hide it, exactly as the other
  players' tags (`game/interface/names.rs`, `own_name_tag`). Client-only: nothing is sent.

### Hide block-break particles (`hideBlockParticles`, default off)
- Hides the debris particles that burst from a block when it breaks
  (`ClientLevel.addDestroyBlockEffect`) and the crumbs off the face you mine
  (`addBreakingParticles`). Other particles (critical hits, smoke, rain...) are
  unchanged, and the block still breaks and drops as usual. Client-only:
  nothing is sent (`game.rs`, `destroy_particles` and `crack_particle`).

### Auto-reconnect (`autoReconnect`, default off)
- On the "Connection lost" screen after a multiplayer connection drops, a
  countdown "Reconnecting in N s" with a Cancel button, then a new connection
  to the same address.
- Backoff 5, 10, 20, 40, 60 s; at most 5 attempts, then it stops. The count
  only resets after staying connected for a minute, so a server that drops
  players at once never causes an endless loop.
- Never after a kick, a ban, a whitelist or version refusal, a duplicate login,
  an online-mode refusal (no account signed in), or a session refusal from the
  account's login (invalid session, banned from playing online, multiplayer
  disabled, the server failing to verify the username). Unreachable session
  servers, on either side, are retried with the same backoff.
- Online-mode servers: the retry joins with the account the game signed in
  with, held in memory for the whole run (`auth::current()`); nothing is
  written to disk.
- Only in the official executable (`sunrise-official`), which registers it: sunrise-client's own executable has
  no such row or setting, and never connects again by itself.

### Keep channels for UI sounds (`uiSoundReserve`, default off)
- Positional sounds (a block breaking, a mob, an explosion) start only while at
  least 16 of the 247 static sound channels are free; sounds played to the
  listener (`relative`, or no attenuation: UI clicks, toasts) may take those 16.
  A flood of world sounds (a big explosion, `/fill … destroy`, a mod breaking
  hundreds of blocks at once) then can't stop the pause menu's clicks.
- Vanilla 26.3 gives all 247 to whatever starts first
  (`Library.acquireChannel` returns null once the pool is full); with the
  extra off Sunrise does exactly that. Below 231 sounds at once nothing
  changes either way. Streaming sounds (music, records) are untouched
  (`sound_engine.rs`, `listener_reserve`). Client-only.

### Mouse Tweaks controls (`mouseTweaks`, default off; `mouseTweaksWheel`, default on)
In the player inventory, chest-like containers (chests, barrels, ender chests,
shulker boxes, hoppers, dispensers and droppers), furnaces (also blast furnaces
and smokers) and crafting tables, in singleplayer and on servers. Anvils,
enchanting tables, merchants, beacons, brewing stands, crafters and the other
workstations keep vanilla's controls:
- Right-drag holding a stack: one item goes into each slot entered, at once
  (slots holding another item, or full, are skipped).
- Left-drag holding a stack: each slot entered that holds the same item is
  gathered into the hand, when it all fits. This takes over only while
  vanilla's drag covers one slot, so a left-drag across empty slots still
  splits the stack as vanilla does.
- Shift + left-drag: every slot entered is quick-moved (with a stack in hand,
  only the slots of its item).
- The wheel over a slot, with an empty hand (`mouseTweaksWheel`): down moves
  one item where a Shift click would send it (vanilla's `quickMoveStack`: fuel
  to the fuel slot, a smeltable to the input, into the crafting grid, across a
  container, between hotbar and main inventory, an armour piece onto the
  player), up brings one of the same item from the other side.
- Nothing goes into a result slot. The wheel down over one takes one craft, or a
  furnace's whole output, to the inventory when it all fits (a result can't be
  split or put back); the wheel up over one does nothing, and it isn't a source
  for the wheel up elsewhere. A right-drag skips it, and a right press on it is
  vanilla's.

Every action is ordinary slot clicks (`ContainerClick` on a server), so a
vanilla server sees nothing unusual. With the extra off, the hooks return at
once and the screens click exactly as vanilla's (`tools/test_menu_click_parity.py`
and the container screen tests are unchanged).

### Sorting (`sort`, default off; `sortKey`, unbound)
- On every menu screen, by its slots' roles (`native_slot_roles`): a small
  button over the top right of its storage slots (chests, barrels, ender
  chests, shulker boxes, hoppers, dispensers, a mount's chest, a mod menu
  type's linked slots) and one right of the "Inventory" line (the main
  inventory; the hotbar is left alone), crafting tables and workstations
  included. The key, when bound on the Gameplay page, sorts the part under the
  cursor (the storage when over no slot).
- Partial stacks of one item merge, then the stacks go in creative tab order
  (the search tab's), then by name; full stacks before the partial one,
  empty slots last. It needs an empty hand.
- Done with ordinary left clicks planned by `inv_sort::plan`, so a vanilla
  server accepts it; a unit test replays the planned clicks through the menu
  engine on random chests.

### Shader pack key (`shaderPackKey`, `K` by default; `lastShaderPack`, none picked)

- The key on Sunrise Settings' Shaders page for the shader pack pick: press it and the running pack turns off, press it again and
  the same pack runs again. Click the row, then press a key to rebind it (Escape unbinds); it is not a vanilla
  key binding, so it never collides with one (Replay Studio keeps physical `K` for its camera keyframe).
- Turning off writes the pick to `lastShaderPack`, so the pack stays off across restarts until the key is
  pressed again. The menu's second click on the picked pack does the same through the same pick.
- A pack whose owning mod is off can't run: the key reports that instead of claiming it turned on.

### Smooth chat (`chatAnimation`, default off; `chatIndicator`, default on)
- A new chat line slides in from the left while it fades in over a quarter of
  a second (5 ticks, `INTRO_TICKS`/`INTRO_WIDTH` in `Chat::layout`), instead of
  appearing whole on the tick it arrives. Its background and its indicator bar
  move with it, and the fractional tick from `Game::clock` drives the motion, so
  it moves every frame and not 20 times a second.
- Only the unfocused chat (the HUD's) animates: with the chat screen open the
  history is drawn as vanilla draws it, so clicking a line lands on its text.
- `chatIndicator` off removes the coloured bar at the left of a line. It hides
  the bar only, never the message.

### Scroll wheel changes settings (`wheelCyclesListSettings`, default off)
- Off, as in vanilla, the wheel scrolls the list under the cursor, on every settings page, Video's
  included (`AbstractScrollArea.mouseScrolled`, `OptionsUi::wheel`, `GraphicsScreen::wheel_step`). Scrolling
  through a long page can't change an option that passes under the cursor (#5455, #9633). A cycle
  button outside a list, such as a world's Difficulty, still steps
  (`CycleButton.mouseScrolled`).
- On, the wheel over a button in a list changes the button's value too, and a button focused with
  the keyboard or a pad steps on the wheel wherever the cursor is. Client-only.

### Local multiplayer

Splitscreen is temporarily unavailable; it returns as an official mod.

### Music page (`musicPage`, default off; `musicOff`, none off by default)
- An extra like the rest: off, Music & Sounds is exactly vanilla's page and every
  pool plays as vanilla picks. On, it gains a **Music...** button.
- The page lists every track the loaded `sounds.json` can play
  (`SoundRegistry::tracks`): the built-in situations' own tracks first (menu,
  overworld, creative, underwater, the End, boss, credits), then the tracks only a
  biome's pool names, under **Overworld biomes** and **Nether biomes** (vanilla
  shares those between several biomes, so no heading claims one). A track a few
  pools share is listed once, from the first pool that names it, so every row has
  a pool its "play now" can use.
- **Left click** a track plays it now: it takes the music manager's slot (the track
  already playing stops), and the situation's own music waits until it ends.
- **On/Off** at the right of a row takes a track out of every pool it plays in, so
  a situation with all its tracks off plays nothing. Every track is on by default,
  so vanilla's random selection is unchanged; the off ones are `musicOff` in
  `sunrise-extras.txt`.
- The pools are filtered where they are picked (`sound_except` in
  `sound_events.rs`), the manager is `music.rs`, and the page is in `options_ui.rs`.

### Modded splashes (`skipModdedSplashesIfVanillaModified`, default on)
- NeoForge's client setting of the same name, with no Gameplay page entry. On, a
  resource pack or a mod that provides `minecraft:texts/splashes.txt` replaces
  the title screen's whole splash pool; off, every namespace's
  `texts/splashes.txt` joins it (`crates/sunrise-client/src/splashes.rs`). Read
  on each resource reload.

## Shared rules

- All extras are hidden with F1 and drawn in the HUD layer: under screens, over
  the world, like vanilla's HUD. Extras in the top left corner give way to the
  debug screen (F3), which shows the FPS anyway.
- Corners stack: extras placed in the same corner stack downward (top corners)
  or upward (bottom corners) in the order the PvP HUD widgets, then mods' panels. The top
  right corner starts below the status effect icons.

## Perf budget

Off: a flag check per frame per extra (no allocations, no world access).
