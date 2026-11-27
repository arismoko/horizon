# Resource packs: what works

Java Edition resource packs load in Sunrise: put the folder or `.zip` in `resourcepacks/`, pick it on the Resource Packs
screen (the same screen as vanilla's) or in game, and every loader reloads from it. This page lists everything a
26.3 resource pack can change and whether Sunrise honours it, with the proof.

## How it works

`packs.rs` finds the packs (`FolderRepositorySource`: folders and zips, plus the built-in Programmer Art and High
Contrast, which only the asset index holds: preparation copies them, `sunrise-prepare packs` repairs older resources),
reads `pack.mcmeta` (format range, overlays, filters) and resolves one shared resource view over the
prepared vanilla resources, later packs winning. Folders and open zip files use the same lookup and listing
interface; archive entries stay in their archives. Files vanilla reads as a resource *stack* (language files,
`sounds.json`, font definitions, atlas definitions) are merged layer by layer as vanilla merges them.
Every loader reads that view; `game/world/packs/mod.rs` (`reload_packs`) prepares a new generation on a worker and
publishes it after loading succeeds, as `Minecraft.reloadResourcePacks` does. Readers of the previous
archive generation retain its open files. Prepared registry tables and disposable decoding caches have
separate filesystem locations.

Vanilla 26.3's reload listeners (`Minecraft.<init>`, `Gui`, `Hud`): languages, textures, shaders, sounds, atlases,
fonts, grass / foliage / dry foliage colour maps, models (block states, block models, item definitions), equipment
assets, paletted textures, entity and block entity renderers, particles, the game renderer (post effects), the level
renderer, clouds, GPU warn list, regional compliance, splashes and waypoint styles. Every row below comes from one
of them.

## The table

**Works**: Sunrise reads it from the pack and draws or plays it like vanilla. **Partly**: some of it.
**No**: ignored (the vanilla version stays). Proof: a parity harness in `tools/` (compares with the pinned 26.3
client), a unit test, or the loader reading the resolved resources (vanilla's files go through the same loader, and the
harness named checks that loader against vanilla).

| Feature | Files | State | Proof |
| --- | --- | --- | --- |
| Pack metadata: description, icon, `min_format`/`max_format`, old `supported_formats`, compatibility warning | `pack.mcmeta`, `pack.png` | works | `test_pack_screen_parity`, `test_pack_selection_parity` |
| Overlays per format | `pack.mcmeta` `overlays` | works | `test_resource_pack_parity` (an overlay for 26.3 applied, an older one not) |
| Filters | `pack.mcmeta` `filter` | works | `test_resource_pack_parity` (`sunrise-game-data/src/pattern.rs` regex subset) |
| Folder and zip packs, order, built-in packs | `resourcepacks/` | works | `test_resource_pack_parity`, `test_pack_selection_parity` |
| Server resource packs: the prompt, the server list's Server Resource Packs setting, download (250 MiB, SHA-1, `downloads/` keeps 20), download toasts, a progress screen while the packs reload, applied on top of the player's packs, removed on leaving | the server's `resource_pack_push`/`_pop` | works | `tools/server_pack_e2e.py` (against vanilla's server and Sunrise's), unit tests in `server_packs.rs`; `test_server_resource_pack_parity` for Sunrise's server |
| Other namespaces | `assets/<namespace>/` | works | `test_resource_pack_parity` |
| Block states: variants, multipart, weights, rotations, uvlock | `blockstates/` | works | `test_blockstate_model_parity`, `test_model_choice_parity`, `test_blockstate_transform_parity` |
| Block models: parents, elements, rotations, faces, cullface, tint, shade, light emission, ambient occlusion | `models/block/` | works | `test_model_resolution_parity`, `test_model_baking_parity`, `test_section_mesh_parity` |
| Item definitions and item models (all model types, display transforms, tints) | `items/`, `models/item/` | works | `test_item_model_parity` |
| Custom entity models (OptiFine/EMF): a `.jem` reshapes a mob's model layer, with its animations and texture, or a block entity's model (chests, shulker boxes, banners, heads, bells, books, conduits, decorated pots) with its texture but no animations yet; Entity Model Features' files win over OptiFine's | `optifine/cem/`, `emf/cem/` | partly | `mobs::cem`, `mobs::cem_anim`, `block_entity_cem` and `block_entity_render` unit tests, `sunrise-optifine` `cem::files` (which file wins); trapped and ender chests read only their own files and copper chests keep vanilla's model; no sign, hanging sign or bed models, no per-bone textures, no random models, no textures under `optifine/` (vanilla's own entity models are code in 26.3, which `test_entity_model_parity` checks) |
| Random entity textures (OptiFine/ETF): numbered variants and their rules (biome, height, name, profession, colour, baby, health, moon, time, weather, size, blocks), chosen per entity from its UUID and rechecked once a second | `optifine/random/`, `optifine/mob/`, `mcpatcher/mob/`, `etf/random/`, `textures/entity/` with a `.properties` | partly | `sunrise-optifine` `random` and `random_textures` tests, `mobs::random_textures` tests; no `nbt.*` or ETF's extra conditions (teams, items...), no block entities |
| Connected textures (OptiFine/Continuity): every method (`ctm`, `ctm_compact` with `ctm.N`, `horizontal`, `vertical`, `horizontal+vertical`, `vertical+horizontal`, `top`, `random`, `repeat`, `fixed`, `overlay`, `overlay_ctm`/`_random`/`_repeat`/`_fixed`, and Continuity's `overlay_horizontal`/`_vertical`/`_h+v`/`_v+h`), `matchTiles`/`matchBlocks` and the file-name forms, `tiles` (ranges, names, paths, `<skip>`, `<default>`), `weight`, `faces`, `biomes`, `heights` (and the old `minHeight`/`maxHeight`), `connect`, `connectTiles`/`connectBlocks`, `innerSeams`, `weights`, `randomLoops`, `symmetry`, `linked`, `width`/`height`, `tintIndex`/`tintBlock`, `layer`; Continuity's `orient`, `prioritize`, `optifineOnly` and its stacked-pane culling; tiles a rule picked matched again by another; panes drawn with OptiFine's unmirrored UVs | `optifine/ctm/**/*.properties` (and `mcpatcher/ctm/`) in any namespace, with their tiles | partly | `sunrise-optifine` `ctm` and `sunrise-connected` tests (the tile tables read off OptiFine's template images), `connected` and `mesher::quarter_tests` tests; `name=` rules are not loaded (the client meshing chunks never receives block entities' custom names, so they could never match); `random` picks from its own fixed hash, so random tiles land in other places than in OptiFine (same weights); tiles glow with their `_e` overlays, sharing the emissive row's table and limits (block overlays come first) |
| Emissive textures (OptiFine/ETF): `<texture>_e.png` (or the pack's `suffix.emissive`) drawn full bright over mobs, their layers and armour, armour stands, the elytra, the dragon, thrown tridents, block entities, placed blocks and connected-texture tiles | `<texture>_e.png` beside its texture (also random variants' and blink frames'); the suffix from `optifine/emissive.properties`, `etf/emissive.properties` or `textures/emissive.properties` | partly | `sunrise-optifine` `emissive` tests, `mobs::render::emissive_tests`, `block_entity_render` and `native_emissive` tests; shares the 128-entry GPU emissive uniform table (`Globals.emissive_layers`) with mod-registered overlays (no private 32/96 split), cutout; none on translucent faces, nor on blocks whose texture is pixel-identical to another's (they share a layer); not over items, armour trims, other projectiles, minecarts or end crystals |
| Blinking textures (ETF): `<texture>_blink.png` and `_blink2.png`, `blinkLength`/`blinkFrequency` | `textures/entity/`, random variants' folders | works | `sunrise-optifine` `blink` tests, `mobs::render` blink test; sleeping villagers keep their eyes shut |
| Textures, any resolution (16x to 512x) | `textures/` | works | atlases stitched from the overlay; Faithful 32x, Default HD 128x and FPBR checked |
| Texture animation: frames, frame time, frame size | `*.png.mcmeta` `animation` | works | `test_sprite_animation_parity` |
| Texture animation: `interpolate` | `*.png.mcmeta` `animation` | works | `sprites.rs` blends each frame towards the next (`interpolated_animations_blend_towards_the_next_frame`) |
| Mipmap strategy, alpha cutoff bias | `*.png.mcmeta` `texture` | works | `test_mipmap_parity` |
| `blur` / `clamp` | `*.png.mcmeta` `texture` | works | `hud.rs` `TextureFlags` reads them from the overlay, and `screen_effects.rs` samples the screen overlays with them |
| Atlases and sprite sources: `directory`, `single`, `filter`, `unstitch`, `paletted_permutations` | `atlases/` | works | `test_resource_pack_parity` (merged source lists), `atlas_sources.rs` tests |
| GUI sprites | `textures/gui/sprites/` | works | `test_hud_parity`, `test_screen_layout_parity` |
| GUI sprite scaling: `nine_slice` border | `*.png.mcmeta` `gui` | works | `ui.rs` reads the border; vanilla's buttons, sliders and frames use it |
| GUI sprite scaling: `width`/`height`, `stretch_inner`, `tile`, `stretch`, and metadata that doesn't decode (the sprite draws as the missing one) | `*.png.mcmeta` `gui` | works | `test_gui_sprite_scaling_parity` (the codec and the tiled copies, against the client); `ui.rs` draws every whole GUI sprite through it, but the sub-region `blitSprite` overload still draws its region stretched |
| Fonts: `bitmap`, `space`, `unihex`, `reference` | `font/*.json`, `textures/font/` | works | `test_resource_pack_parity` (merged provider lists) |
| Fonts: `ttf` | `font/*.json` | works | `font.rs` rasterises them with `ab_glyph` (`TrueTypeGlyphProvider`) |
| Fonts picked by text components (`"font": "..."`, custom fonts) | `font/` | works | `component.rs` reads the `font` key and `font.rs` (`glyph_styled`) draws with the font it names |
| Languages, translations | `lang/*.json` | works | `test_resource_pack_parity` (key by key, as `ClientLanguage`) |
| New languages | `pack.mcmeta` `language` | works | `lang.rs` reads every pack's `language` section into the language list |
| Sounds and sound events (`replace`, weights, streams, event references) | `sounds.json`, `sounds/` | works | `test_resource_pack_parity`, `test_sound_event_parity`, `test_sound_engine_parity` |
| Splashes | `texts/splashes.txt` | works | `title.rs` reads it from the overlay |
| Credits, end poem | `texts/credits.json`, `end.txt`, `postcredits.txt` | works | `game/win.rs` (`WinScreen`): after the End, and from Options → *Credits & Attribution* |
| Grass, foliage and dry foliage colours | `textures/colormap/` | works | `biome.rs` reads the overlay; `test_section_mesh_parity` checks the tints |
| Entity textures and variants | `textures/entity/` | works | `test_entity_model_parity`, loaded from the overlay |
| Block entities: chests, signs, beds, banners, shields, shulker boxes, decorated pots | `atlases/`, `textures/entity/` | works | `test_block_entity_render_parity` |
| Villager hats | `*.png.mcmeta` `villager` | **no** | the type layer's hat is always drawn; vanilla hides it under a profession hat marked `full`/`partial` |
| Equipment (armour, horse/llama/wolf/nautilus armour, wings, dyeable layers) | `equipment/`, `textures/entity/equipment/` | works | `game/entities/armor_stands.rs`, `mobs/render.rs` read the overlay |
| Armour trims on items | `atlases/items.json`, `textures/trims/items/` | works | `paletted_permutations` (`atlas_sources.rs` tests) |
| Armour trims on worn armour | `textures/trims/entity/`, `textures/palettes/`, `palette` metadata | works | `trims.rs` (`PalettedTextureManager`), drawn by `game/armor_stands.rs` and the mob renderer |
| Particles | `particles/`, `textures/particle/` | works | `test_particle_providers_parity` |
| Sky: sun, moon phases, stars, end sky, clouds, rain and snow | `atlases/celestials.json`, `textures/environment/` | works | `test_environment_render_parity`, `test_cloud_render_parity`, `test_weather_render_parity`; a `clouds.png` vanilla reads past (taller than wide, with a cell in its first or last column: `NativeImage.getPixel` throws) fails the load as vanilla's does, dropping every selected pack (`clouds::load`, `packs::clouds_reload_tests`) |
| Title panorama | `textures/gui/title/background/` | works | `gfx/panorama.rs` loads it from the overlay |
| Screen overlays: vignette, pumpkin, powder snow, nausea, underwater, world border | `textures/misc/` | works | `test_world_border_render_parity`, `test_hud_parity`; loaded from the overlay |
| Enchantment glint | `textures/misc/enchanted_glint_*` | works | `item_render/gui.rs`, `hud/cit_glint.rs`, `entity.wgsl` |
| Maps: background, decorations | `textures/map/`, `atlases/map_decorations.json` | works | loaded from the overlay |
| Paintings, mob effect icons | `atlases/paintings.json`, `textures/mob_effect/` | works | loaded from the overlay |
| Locator bar styles | `waypoint_style/` | works | `locator_bar.rs` loads `waypoint_style/*.json` (`WaypointStyleManager`); `game/locator_bar.rs` draws the bar |
| Post effects (spectating a creeper, spider or enderman; glowing outlines) | `post_effect/`, `shaders/post/` | works | `post_chain.rs` reads `post_effect/*.json` and `post_glsl.rs` translates the `shaders/post/*.vsh`/`.fsh` with naga (their imports resolved as the core shaders' are), with depth inputs, persistent targets and `TextureInput`; a pass binds only what its stages read. One rule for a refused pass, whatever refuses it (more blocks, samplers or `uniform` keywords than the device binds to both stages, checked before any analysis; more than a stage's limit read; the translator; its configuration; or the device, for its modules, layout or pipeline): a pass naming vanilla's shaders runs on the built-in stand-ins, and any other refuses its effect, named in the log |
| Core shaders | `shaders/core/`, `shaders/include/` | partly | `core/text` draws the GUI's text (HUDs moved by its vertex shader, text effects) and the world's text on vanilla's normal, polygon-offset and see-through paths (signs, text displays and their backgrounds, name tags), translucent world text under Improved Transparency through `OIT_TEXT`'s three stages (`gfx/core_text/oit.rs`); a native mod's world canvas draws with native shaders, `core/rendertype_lines` the block outline (both strokes of a high contrast one; hitboxes, chunk borders and other debug lines still draw natively), `core/sky` and `core/stars` the overworld's sky disc and stars, `core/lightmap` (with `core/screenquad`) the 16x16 light texture from `LightmapInfo`, and `core/clouds` the clouds from the faces `CloudRenderer.buildMesh` encodes (`CloudFaces`, read as a storage buffer) and `CloudInfo`, under Improved Transparency through `OIT_CLOUDS`' three stages with each stage's defines and samplers (natively under a shader pack or HDR output's cloud marks); a pack's core shader that any pipeline vanilla requires of it can't be made from (each of these shaders is translated for every such pipeline, the grayscale text and the Improved Transparency stages of the text and the outline included, and the device makes every drawn one, on a frame of linear light too) fails the load as vanilla's `ShaderManager.apply` does, dropping every selected pack (`hud/pack_text.rs` `load_shaders`, `packs::core_shaders_reload_tests`); a vanilla block a pack's shader declares larger than 26.3's (an older vanilla's) is bound at the shader's size, the bytes past 26.3's reading zero, up to the 256 bytes each block has, past which that shader draws natively, and a pack replacing only an include one of those pipelines keeps changes the shader; both the pack's and the native clouds take the layer's bottom from `EnvironmentAttributes.CLOUD_HEIGHT` probed at the camera each frame (`minecraft:visual/cloud_height`, 192.33 by default; on a server, the dimension type the server synchronized sets it, a data pack's Overworld included; the packs' Polytone dimension modifiers apply over it only for the three vanilla type IDs, `minecraft:overworld`, `minecraft:the_nether` and `minecraft:the_end`, each with its own file, and a mod's type is drawn without them); each of these but the clouds takes the packs' Polytone expression uniforms on its own shaders (`polytone/shader_modifiers`, `polytone_post.rs`, `gfx/expression_blocks.rs`), translated by naga (`sunrise-client-render` `core_shader.rs`, `hud/pack_text.rs`, `gfx/core_lines.rs`, `gfx/core_sky.rs`, `gfx/core_lightmap.rs`, `gfx/core_clouds.rs`, `gfx/core_text.rs`); the other core shaders aren't run (#13218) |
| OptiFine / CIT Resewn custom item textures (`type=item`: `texture`, `model`, `texture.<name>`, `model.<name>`; `type=armor`: `texture`, `texture.<layer>` with the pre-1.21.2 layer names such as `diamond_layer_1`, `leather_layer_2_overlay`; `type=elytra`: `texture`; `type=enchantment` layers on GUI items; `useGlint=false` in `cit.properties`, which removes vanilla's glint from items) | `optifine/cit/`, `mcpatcher/cit/`, `citresewn/cit/` | partly | `item_render/cit.rs`, `cit_armor.rs`, `cit_glint.rs` and `hud/cit_glint.rs` tests; enchantment layers on held, dropped, framed and worn items, and `anim/` glints, come later; special renderers (heads, shields, banners) keep their vanilla look; only players and armour stands wear CIT armour and elytra, other mobs keep vanilla's; worn trims are unchanged; armour textures above 1024x512 (16 times vanilla's 64x32) draw vanilla |
| GPU warn list, regional compliance | `gpu_warnlist.json`, `regional_compliancies.json` | n/a | OpenGL driver warnings and play-time notices for some regions; not used |

Summary: 50 rows; 43 work, 6 partly (OptiFine/EMF entity models, random entity textures, connected textures,
emissive textures, CIT, core shaders), 1 doesn't apply. Every vanilla loader in the list above reads the pack; what
is left is the core shaders, plus the OptiFine-format extras.

### Polytone expression uniforms on core shaders

A pack's `assets/<ns>/polytone/shader_modifiers/<shader>.json` (`minecraft/.../core/lightmap.json` is the shader
`minecraft:core/lightmap`) maps uniform block names to expressions (wiki "Expression Uniforms on Any Shader"). The
packs' lightmap (`core/lightmap` with `core/screenquad`), sky (`core/sky`), stars (`core/stars`), block outline
(`core/rendertype_lines`) and text (`core/text`: the GUI's, the world's, the see-through text and Improved
Transparency's stages, each pipeline with its own blocks) take them, each from
its own shaders' files (`gfx/expression_blocks.rs`):

- A block one of its stages reads, declared `uniform Name { float value; };`, holds its expression's value each
  frame. The sky, stars, outline and text each use a single modifier file (the text's for all three of its
  programs). Only the lightmap uses two files: for a block read by both stages or defined in both files, it looks in
  the fragment shader's (`core/lightmap`) first, then in the vertex shader's (`core/screenquad`). Otherwise, it uses
  the file for the stage that reads the block. A block no stage reads is not bound, and a stage that only declares
  it doesn't decide its shape. A float block with no expression reads zero. If a block a stage reads is not
  one float, the pipeline can't give it and the load fails, as for any required pipeline.
- Each block, vanilla's and `LightmapInfo` included, of each of these pipelines is bound only to the stages that read it, and counts against
  the device's limit of uniform blocks per stage (`max_uniform_buffers_per_shader_stage`, 12 by default) for those
  stages only: a shader over it in either stage draws natively, as does one passing more varyings than the device's
  `max_inter_stage_shader_variables` (16 by default), or writing a colour target with another base type or fewer
  components than its format has, and the pack stays.
- The expression context is the frame's, as the packs' post chains and particles see it: the weather, the game and
  day time, the dimension and its sky, the player's place and motion, the camera's light, biome and climate, and
  the packs' config entries (`polytone/config_entries`). Fullbright skips only the lightmap's expressions.
- Limits: a file is read up to 64 KiB, with at most 16 uniforms, each a GLSL identifier of at most 64 characters
  not starting `gl_`. An expression has at most 1024 tokens; its tree is at most 64 levels high and its parse
  nests at most 16 levels deep (a statement, its expression, each bracket and each unary operator), both checked as
  it is parsed. A file over any of these, or whose path names no shader, is refused whole, named by its path in the
  log.
- Where no world runs (the title screen and its menus, a loading screen, the file fixer, the saving screen), the
  text's expression uniforms and `PolyGlobals` are 0: the game gives the text none of them, and the frame writes
  zeros.
- The programs are resolved once when the pack's shaders load. Each frame evaluates them for the installed pack
  pipelines; each pipeline uploads 16 bytes per expression uniform when it prepares a draw.
- Polytone's `PolyGlobals` block (wiki "Extra Shader Uniforms": the projection and model-view matrices, the sun's
  angle, the day time, the frame's ticks, and the player's block and position) is given to each of these shaders
  and to every post effect's pass that reads it, with the same values, in the same std140 layout (`sunrise-polytone`
  `globals.rs`). A pack declaring the block itself reads the same values as in Java with
  Polytone 26.3: on a core sky shader and a post chain, its pixels are within the spread a control pack without
  the block shows between the two. It is bound only to the stages
  that read it, before the expression uniforms, and counts against the same limit.
- `#moj_import <polytone_globals.glsl>` (or `#include`) declares the block with the `polyPlayerPos` and
  `polyPositionFromDepth` helpers where the packs don't have that file: a built-in that follows the public wiki.
  Polytone 26.3 (6.8.6, the only build for 26.3) has no such file, so a pack importing it loads here and not in that
  Java build. Polytone 6.11 on 26.2 has it (through `#moj_import`) and gives the same block position, offset, sun
  angle, delta time and projection form in the same scene; its `polyPositionFromDepth` takes the depth with OpenGL's
  convention (`depth * 2 - 1`), which on 26.3's reverse-Z depth buffer (0 to 1) gives other positions. Sunrise
  follows the wiki's contract (the position from the screen uv and the depth buffer's value) and gives the
  geometric hit. Its day time read 6001 in a scene where Sunrise gives 6000 (both suns at 6000's angle); not
  matched. A pack's own file is read instead of the built-in, and one that can't be read refuses the shader by its
  path.
- The game builds the block only when something reads it, at most once a frame: when an installed pack core shader
  reads it (the lightmap's not under fullbright, which keeps the light texture white; the text's only where a world
  runs), or when a post effect that runs has a pass reading it. Each reading core pipeline writes its 176 bytes to
  the GPU when it uploads its blocks for its draw; a post pass writes them when it draws (zeros on a frame without
  them, as on the title screen).
- The block is the game's, on the core and post shaders alike: a shader declaring it other than that layout (each
  member of its type at its offset; a leading part of the block is fine), a `shader_modifiers` file giving it, a
  post chain's expression uniform of that name, or a pass's `uniforms` giving it, is refused, named in the log. A
  `shader_modifiers` file giving another block the pipeline fills itself (the lightmap's `LightmapInfo`) is refused
  the same way.

## Gaps and their work

| Gap | Size | Where |
| --- | --- | --- |
| Core shaders but the text (GUI, polygon-offset world, see-through), the block outline, the sky, the stars, the lightmap and the clouds | large | [#13218](https://github.com/horizon-rust/horizon/issues/13218) |

## Testing a pack

`tools/test_resource_pack_parity.py` builds small packs (folder and zip, overlays, a filter, textures, languages,
sounds, fonts, atlases), and a Java oracle answers what vanilla's loaders read from them; Sunrise's loaders must
give the same answers. Never commit third-party packs: build test packs in the harness or the test.
