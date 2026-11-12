# Installing and using mods

Two kinds of mod run in Horizon:

| Kind | File | What it is | Where it runs |
|---|---|---|---|
| **Content mod** | `.hmod` | Data and assets: blocks, items, recipes, loot, tags, models, textures, structures. No code. | Loaded at startup; assets-and-data-only switches can reload live |
| **Native mod** | `.hmod` | A flat Rust project using the typed hook API, with optional raw injections. | In a game the launcher builds for the player ([native mods](../how-to/native-mods.md)) |

Fabric and NeoForge jars do **not** run as they are: they target Minecraft's Java runtime, and
Horizon has none. What carries over is the data and assets inside a jar ([modpacks.md](https://github.com/arismoko/horizon/blob/main/docs/modpacks.md)).

Mods are **off by default**. A fresh install has no `horizon-mods` folder and plays exactly like vanilla 26.3.

## Playing with mods

1. Install a mod from the launcher's Mods page ([mod-browser.md](https://github.com/arismoko/horizon/blob/main/docs/mod-browser.md)) or the game's Options > Mods >
   **Browse Mods** (content-only packages, checked against the same registry), or put its files by hand in the
   game folder (the one holding `options.txt`): a content mod's `.hmod` (or a folder holding `horizon-mod.toml`) in
   `horizon-mods/`. Native mods go in `horizon-native-mods/`, and the launcher builds them
   ([native mods](../how-to/native-mods.md)).
2. The **Mods** button on the title screen (also Options > Mods) opens the mod list, laid out as NeoForge's: Minecraft
   and Horizon first, then each installed mod with its icon, name and version. Search the list, sort it A-Z or Z-A, and
   pick a mod to read its id, version, license, authors, credits and description. Homepage and Issues open the mod's
   links after a confirmation, and Config opens the settings page of a native mod that has one. The screen has no
   switch. A mod starts off: the launcher (`horizon-launcher mods enable <id>`) or `horizon-mods.txt` (`mod.<id>:true`)
   turns it on, and a mod that is not on says so in the list.
3. A turned-on content mod with only assets and reloadable data (textures, models, sounds, languages, recipes, tags,
   loot, advancements and functions) is picked up live when a mod is installed from Browse Mods: the game reloads its
   resources, then its world's data as `/reload` does, including for LAN guests. Content that adds registry entries or
   changes world generation still needs a restart. Packages with jar layers also wait for a restart, because those
   layers may contain world generation. On another server all content waits: the server owns its data.
4. A mod that can't load shows why, in red, in its panel.

Two turned-on content mods never own the same namespace: the first by file name loads, and the other stays out with
the reason in the log.

Benchmarks (`--benchmark`) never load mods.

## Horizon's official mods

Horizon's own mods are native. They live in
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) under `native/<name>/`:

| Mod | Folder | Replaces |
|---|---|---|
| FPS Counter | `native/fps-overlay` | Extras > FPS counter |
| Keystrokes | `native/keystrokes` | Extras > Keystrokes |
| Zoom | `native/zoom` | Extras > Zoom |
| Minimap | `native/minimap` | Extras > Minimap |
| Recipes | `native/recipes` | The Recipes mod (item panel and recipe pages, on hook points version 2) |
| Coordinates HUD | `native/coords-hud` | A small example mod (position and facing, `/where`, `/coords`) |

The game keeps the parts that must be fast, such as drawing the minimap's map, and a native mod picks the options
([extras.md](https://github.com/arismoko/horizon/blob/main/docs/extras.md)). Far View is not a mod any more: its distance and detail are Graphics > LOD Distance and Far
Detail ([graphics.md](https://github.com/arismoko/horizon/blob/main/docs/graphics.md)).
