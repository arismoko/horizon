# Layers: engine, platform, Sunrise

Sunrise is three layers:

- a **vanilla engine**;
- a **platform** that makes the vanilla game easier to change, port and run;
- **Sunrise's own layer**: its extras and its product (Play with Friends, Discord, telemetry, the launcher...).
  It has no paid features, shop or ads.

**The target** (Ludovic, 2026-11-09). This repository holds the engine and the platform. Built alone, it is a
complete vanilla-like game. The Sunrise layer moves to its own repository, public if its license needs it, and
ships with every official launcher download (see [The Sunrise layer in its own repository](#the-sunrise-layer-in-its-own-repository)).
The split comes first, inside this repository, one checked step at a time. The repositories separate after that.

Both checks below run in CI's verify job and in `tools/pr_ready.py`, against the fork's baseline: the explicit
rows and reasons in `tools/layers.json` and `tools/addresses.json`, which cover the split mod crates and the separate
launcher, registry and telemetry-server workspaces. No engine/API refactor is implied by importing these tools.

This page says:

- what belongs to each layer and which layer may depend on which;
- where a new feature goes;
- where every built-in outside address lives;
- the plan.

CI checks the rules, so review doesn't have to. Contributors fixing vanilla behaviour never have to touch
Sunrise's extras or a server address. Mods and desktop-class ports (iOS, Android) build on the platform without
editing engine files; ports to machines the desktop client can't run on (a handheld console, a browser) build on
the [portable crates](#portable-crates).

Credits:

- **NikitaMGrimm** (DuckyBertDuck on Discord): the three layers, the dependency rule, the address rule and the plan
  ("3-layer split: engine / platform / Sunrise", #contributors, 2026-11-08), the portable crates (#12997, #13002),
  and the review that made the Sunrise layer linked crates rather than native mods.
- **arismoko:** the mod API owns its types and a CI property check keeps engine types out of it
  ([arismoko/horizon](https://github.com/arismoko/horizon), arismoko/horizon#1608). Also the idea of shipping
  Sunrise's own parts from another repository as a launcher plugin.
- **Martipops** (controllers, splitscreen): the compositor's place in the engine, and where controllers,
  splitscreen, Replay Studio, PvP & HUD and a UI page API go.
- **Contributors at the 2026-11-08 stage:** the single list of addresses, sponsors in the
  Sunrise layer, a platform that exposes what vanilla already uses, and extras as native mods.

## The rule

| Layer | May depend on |
|---|---|
| engine | engine |
| platform | engine, platform |
| Sunrise | platform, Sunrise |
| tool (dev tools) | anything; no shipped crate depends on a tool |

The engine never depends on the platform or on Sunrise. The platform never depends on Sunrise. **The Sunrise layer
never depends on the engine:** it uses only the platform's public API, so it can build from another repository.
Normal, build and optional dependencies all count; dev-dependencies don't, so tests may use anything.

When lower code needs something from above, the lower layer offers an extension point and the higher layer plugs
into it. The extension point is a trait, a callback list, an event or a registry, and it names no type from above.
When Sunrise needs something from the engine, the platform exposes it as public API.

## Engine

Vanilla Minecraft: Java Edition 26.3, and nothing else.

- **Belongs:** gameplay, world data and generation, block behaviour, mobs, items, commands, the network
  protocol, rendering, sound, vanilla's screens, saves, the integrated and dedicated servers. It also holds
  optimizations that keep vanilla's behaviour, and the extension points the platform plugs into.
- **Also the render pipeline's own capabilities that vanilla's path doesn't use**, when a higher layer could only
  get them by rebuilding the pipeline. The one today is the **compositor**: drawing a view into an offscreen target
  and composing several targets into a window's views. The ordinary client does not use the compositor;
  the renderer's `examples/compositor.rs` still exercises it.
- **Examples:** `sunrise-world` (`worldgen/`, `level/redstone.rs`), `sunrise-protocol` (`packets/`),
  `sunrise-items`, `sunrise-command`, `sunrise-core`, `game/creatures.rs`, `game/integrated.rs`, `gfx.rs`,
  `mesher.rs`, `streaming.rs`, `lan/`.
- **Never:** a mod hook's implementation, a shader or pack format vanilla doesn't read, a setting or screen vanilla
  doesn't have, a network address vanilla doesn't contact, Sunrise branding.
- **Network:** only what vanilla contacts: Mojang and Microsoft services, and the servers the player joins.

### Portable crates

`sunrise-core`, `sunrise-world`, `sunrise-protocol`, `sunrise-items`, `sunrise-command`, `sunrise-game-data`
and `sunrise-assets` hold vanilla's rules and data with no desktop in them. `sunrise-threads` provides worker
scheduling and sampling, and `sunrise-java-properties` reads and writes Java-properties files with the standard
library alone. These portable crates have no window, GPU, sound, gamepad or clipboard library, no HTTP client,
and no engine crate outside the portable list. The platform sits on the desktop engine
(`wgpu`, `winit`, `cpal`), which a handheld console or a browser can't run, so a port there builds on these crates
instead. They stay that way: the layer check fails when one takes `winit`, `wgpu`, `cpal`, `gilrs`, `arboard` or
`ureq`, or an engine crate outside the list (#13002). Most gameplay still lives in `sunrise-client`; step P of
the [plan](#plan) moves it into `sunrise-gameplay` (what the server jar holds) and `sunrise-client-gameplay`
(client only) over time. Done so far (#2210, waves 1 and 2): the modules that need no `Game`, such as the mob
families on `MobLevel`, the cat spawner's picks and the wandering trader's state, stats, the block-entity ticker
helpers, the server halves of `lan/` and `dedicated/`, and occlusion and particle culling. Not done: `LanHost`, the
types `World`, `ChunkMap`, `Mob` and `ItemEntity`, level events and the dispatchers, the custom spawners' dispatch and
the village siege, and the block-entity tick loop. `sunrise-gameplay` is not on the list yet: `portable.pending` in `tools/layers.json`
lets it take `sunrise-client-lib` until the types it needs move down, and only ever shrinks.

## Platform

What makes vanilla nicer to change, port and run, with no Sunrise branding and no address of its own. It is the
public API that mods, desktop-class ports and the Sunrise layer build on.

- **Belongs:** the mod API and mod formats, the native mod host, shader pack formats beyond vanilla
  (OptiFine, Polytone, `shaders/`), HDR, controllers, replay recording and playback, the HUD widget
  framework, UI pages and the settings system, agent control (`sunrise-ctl`, MCP),
  modpack porting, the design tokens of non-vanilla screens.
- **Exposes what vanilla already uses.** FOV changes, potion effects, the player's state, screens, rendering
  hooks and so on become public API with stable types (arismoko/horizon#1608's patterns). Then a mod, a port or a
  Sunrise feature builds on them instead of editing or copying engine code.
- **Examples:** `sunrise-shaders`, `sunrise-optifine` (`cem/`), `sunrise-polytone`,
  `sunrise-hdr`, `sunrise-dualsense`, `sunrise-mods`, `sunrise-mcp`,
  `sunrise-theme`. Still in engine crates today: `game/mod_api_host.rs`, `game/native_*.rs`, `shader_ui.rs`,
  `hdr_calibration.rs`.
- **Never:** a built-in address (every address comes from the player, a file or a setting), anything that names
  a Sunrise crate, uploads to playsunrise.net.
- **Network:** allowed, but never to a built-in address.

### Controllers, replays and UI pages

Vanilla has none of these, and the engine stays 1:1 vanilla, so they are platform features on hooks the engine
exposes. Ports to consoles and handhelds need them, and other projects can build on them.

- **Controllers:** the pad backend (gilrs), bindings, deadzones and the look curve, button glyphs, the pad's menu
  navigation and cursor, the controller settings, DualSense output. Today in `sunrise-client-input/src/gamepad.rs`,
  `game/pad.rs`, `game/pad_*.rs` and `game/pad/`, `pad_cursor.rs`, `pad_glyph.rs`, `controller_ui.rs`, and
  `sunrise-dualsense`. The engine's hooks:
  - **Actions.** Every input the game reads is an `Action`, vanilla's key mappings (`sunrise-client-input/src/actions.rs`),
    and code asks whether an action is held (`held.rs`), never a key. A pad presses and holds actions like a
    keyboard does. This hook exists today.
  - **Movement and look.** An analog move vector and a look delta that the player's movement reads instead of
    keys (today `game/pad.rs` calls `gamepad::move_vector` and `Game::look_turn` itself).
  - **Menu focus.** Screens say which widget has focus and take "move focus" and "press" (today the pad's
    cursor drives vanilla's screens through mouse events).

  The last two become a registered input source, a trait in the engine that the platform's pad code implements,
  in M6. Until then the pad code stays in its own modules, so that move is a pure file move.
- **Replays:** recording the session's packets as `.mcpr` (the format other replay tools read) and playing them
  back through the client (`studio/recorder.rs`, `mcpr.rs`, `replay.rs`) is platform: mods, tools and bug reports
  build on it. The Studio on top of it (camera paths, splines, the timeline and its bar: `studio/camera.rs`,
  `spline.rs`, `path_render.rs`, `timeline_ui.rs`, `topbar.rs`, `studio_ui.rs`) is a finished feature, so it
  goes to the Sunrise layer or a native mod (rule 4), built on the platform's replay API.
- **UI pages:** a general API for pages a mod, a port or the Sunrise layer adds (the settings shell, its rows,
  search and undo, today `settings_shell.rs` and `extras_ui.rs`'s pages). The engine exposes screens: open one, draw on
  the GUI layer, take input and focus. The platform gives the page API stable types (as #1608 does for the mod
  API). Sunrise Settings, the controller pages and the PvP & HUD page all become clients of it.
- **PvP & HUD:** the HUD widget framework (placing, sizing and toggling HUD elements: `hud_kit.rs` and `hud_kit/`,
  `extras_hud.rs`, `game/hud_room.rs`, `game/hud_editor.rs`) is platform. `hud_layout.rs` is vanilla's `Hud` as draw
  calls, which the parity fixture compares with the vanilla client: engine, like `hud_overlays.rs`. The PvP presets and Sunrise's own widgets (keystrokes, CPS, reach readout, `pvp.rs`, `pvp_hud.rs`)
  are Sunrise extras, and candidates to become native mods.

## Sunrise

Sunrise's own extras and its product, and every built-in address of an outside service vanilla doesn't contact.

- **Belongs:** Sunrise's extras (zoom, Mouse Tweaks, sorting, smooth chat, the PvP & HUD presets and widgets, the
  music page, keystrokes, auto-reconnect...), Play with Friends and its relay, Discord, the "I'm playing" ping,
  telemetry and benchmark uploads, releases and updates, the hosting supervisor, the launcher.
- **Examples:** `sunrise-addresses`, `sunrise-friends`, `sunrise-relay`, `sunrise-telemetry`,
  `sunrise-perf-share`, `sunrise-perf-overlay`, `sunrise-release-sign`, `sunrise-server`, `sunrise-mod-registry`,
  `sunrise-heartbeat`, `sunrise-official`, `sunrise-discord`, `sunrise-reconnect`, `https://github.com/sunrise-org/sunrise-launcher`. Still in
  engine crates today:
  `game/extras.rs` and `game/extras/`, `extras_ui.rs`, `pvp.rs`.
- **Rules:**
  - It uses the platform's public API only, never the engine.
  - Until it moves, every Sunrise feature in the game sits behind the `sunrise-extras` feature
    ([forking.md](forking.md)), so a plain build compiles without it.

## Tools

Developer tools that no player runs: `xtask`, `sunrise-bots`, `sunrise-mod-api-gen-tests`, and the Python in
`tools/`. They may depend on anything, and no shipped crate depends on them. `cargo-sunrise-mod`, the mod author's
tool, is one: it fetches and checks Sunrise's signed mod SDK, so it reads `sunrise-addresses` and
`sunrise-release-sign` beside the engine's mod crates.

## Where a new feature goes

The rule, in order (duckybertduck's):

1. Vanilla has it: **engine**. Hooks in the engine are fine: the platform needs them.
2. It is directly about Sunrise (server addresses, the website, the name or branding): **Sunrise**.
3. It helps other people build on the project (mods, ports, other clients; dev tooling excepted, see
   [Tools](#tools)): **platform**.
4. Everything else: **Sunrise**, or a native mod that players add themselves, since it can be built on what's below.

A feature often splits along this rule: the part others build on is platform, and the finished feature on top of it
is Sunrise (PvP & HUD and replays, [above](#controllers-replays-and-ui-pages)).

The mod interface follows that split too: `sunrise-mod-vocab` is an engine crate, while
`sunrise-mod-api` preserves the mod-facing paths through re-exports. The build-time generator
and procedural macros are engine dependencies of that interface. The vocabulary holds dispatch and installed
lookups; the platform owns profiling, resource access and Parts authoring. See the
[checked state inventory](layering/mod-vocab-state.md).

| I add... | Layer | Where |
|---|---|---|
| A vanilla behaviour, block, mob, packet, command or screen | engine | the crate that owns the area ([CLAUDE.md](../CLAUDE.md) crate table) |
| A speed-up that keeps vanilla's results | engine | next to the code it speeds up |
| A call to a Mojang or Microsoft service vanilla makes | engine | `auth.rs`, `realms/`, `skins.rs`, `sunrise-server-files` |
| A place mods, ports or Sunrise can hook into | engine + platform | an extension point in the engine crate, naming no type from above; its public face in the platform |
| A mod hook or host interface | engine + platform | `sunrise-mod-vocab` owns the engine vocabulary and dispatch; `sunrise-mod-api` re-exports it for mods and links the native mod to the platform |
| A mod package format | platform | `sunrise-mod-package` |
| A shader or resource pack format vanilla doesn't read | platform | `sunrise-shaders`, `sunrise-optifine`, `sunrise-polytone`, or a new platform crate |
| Controller support: a device, bindings, glyphs, pad menus | platform | the pad modules, on the engine's input hooks ([Controllers](#controllers-replays-and-ui-pages)) |
| Splitscreen | official native mod (planned) | removed from the engine and launcher; the mod will use the existing mod API and native patch route |
| A render pipeline capability vanilla's path doesn't use but a higher layer can't add without rebuilding the pipeline (the compositor) | engine | `gfx.rs` and `gfx/`, doing nothing on vanilla's path |
| A UI page others can add to, a HUD widget framework, replay recording and playback | platform | the page API, `hud_kit.rs` and `hud_kit/`, `studio/recorder.rs`, `mcpr.rs`, `replay.rs` |
| A finished feature built on those (PvP presets, the replay editor) | Sunrise, or a native mod | behind `sunrise-extras` |
| A Sunrise extra (a setting, screen or HUD element vanilla doesn't have) | Sunrise | its own module behind `sunrise-extras`, built on platform API; or an optional native mod that players add themselves |
| Anything that talks to playsunrise.net or a third-party service by itself | Sunrise | a Sunrise crate, its address in `sunrise-addresses` |
| Discord, telemetry, Play with Friends | Sunrise | `sunrise-discord`, `sunrise-telemetry`..., behind `sunrise-extras` |
| A launcher page | Sunrise | `https://github.com/sunrise-org/sunrise-launcher/tree/main/src/` |
| A harness, script or dev tool | tool | `tools/`, `xtask/` |
| A new crate | its layer | give it a layer in `tools/layers.json` in the same PR |

Code that belongs to another layer than the crate it has to live in today (a Sunrise extra in
`sunrise-client-render`, say) goes in its own module, never mixed into an engine module. Sunrise code in the game
also goes behind `sunrise-extras`. That keeps each later move a pure file move.

## The Sunrise layer in its own repository

The target, after the split. It is not built yet: nothing moves before the split below is done.

**What a build of this repository gives.** A complete game: vanilla play, the platform (mods, shader packs,
controllers, replays, HDR, the dedicated server) and no Sunrise layer. That means no extras, telemetry or Discord,
no built-in outside address, and no launcher. Forks and builds from source get exactly
that. sunrise-plain becomes a plain build of this repository.

**The seam.** The Sunrise repository has to plug into the game without changing it:

1. No engine or platform code names a Sunrise crate. The layer check refuses it.
2. The Sunrise layer uses the platform's public API only. The layer check refuses its engine dependencies, and
   today's are on the exceptions list.
3. Sunrise's in-game parts (extras, Discord, Play with Friends, the ping) are crates linked into
   the official game build in CI, plugged in through the platform's hooks. Players never compile them: a native
   mod is source the launcher compiles per instance, so it is not the way. The official executable is a thin
   binary crate in the Sunrise repository that links the platform and the Sunrise crates; this repository keeps
   its own plain executable. The game runs the same without them.
4. Every Sunrise address is in `sunrise-addresses`, which moves with the layer.
5. The launcher is Sunrise's: it moves too. It installs the game from this repository's releases and adds the
   Sunrise layer to it.

**Licence.** The project's licence is being decided (the poll ends around 2026-11-10); the Sunrise repository's
licence follows from it. If this repository becomes GPL, the official game links the Sunrise crates into the same
executable as the engine and the platform, so the Sunrise layer must be GPL-compatible too: no closed native code in
it.

**Every official download includes it.** These paths must ship the Sunrise layer:

| Path | Today |
|---|---|
| The launcher's release builds, every platform and format: Linux `.tar.gz` and AppImage, macOS `.zip` and signed `.dmg`, Windows `.zip` and `sunrise-launcher-setup*.exe` | sunrise-org/sunrise-launcher's release workflow |
| GitHub releases (`launcher@X.Y.Z`) | `.github/workflows/launcher-bridge.yml` (signed here, published there and here) |
| playsunrise.net's download page | `sunrise-org/sunny`, reading `https://github.com/sunrise-org/sunrise-launcher/blob/main/links.json` and the releases |
| The launcher's self-update | `https://github.com/sunrise-org/sunrise-launcher/blob/main/src/backend.rs` (`stage_update`), `https://github.com/sunrise-org/sunrise-launcher/blob/main/src/update_handoff.rs`, `https://github.com/sunrise-org/sunrise-launcher/blob/main/src/nightly.rs` |
| The game builds the launcher installs, so the in-game part reaches every player | `.github/workflows/nightly-build.yml`, `release.yml`, `game-build.yml` |
| The links the launcher and the site read (Discord, website) | `https://github.com/sunrise-org/sunrise-launcher/blob/main/links.json` |

These stay without it: builds of this repository from source, sunrise-plain, and `packaging/server/install.sh`
and `install.ps1` (the dedicated server).

## How a Sunrise crate plugs in

The seam's points 1-3 above, as they work today (layer plan M5).

| Crate | Layer | Holds |
|---|---|---|
| `sunrise-hooks` | engine | The `Hooks` trait, `register`, and the engine's dispatch (`sunrise_hooks::engine`) |
| `sunrise-platform` | platform | `sunrise_platform::hooks`: `Hooks`, `Started`, `register`, without the dispatch |
| `sunrise-game` | platform | The game for an executable to start: `sunrise_game::main()`, the allocator, the mod API mark. The platform's own parts register here as they leave the engine (M6) |
| `sunrise-official` | Sunrise | The official executable: `register()` adds the Sunrise crates' hooks, then it runs `sunrise_game::main()` |

If you embed the game in your own executable, remove calls to and imports of the retired
`sunrise_game::prepare_main_stack`, `sunrise_platform::prepare_main_stack` and
`sunrise_mod_host::prepare_main_stack` re-exports. Register your hooks as before, then call
`sunrise_game::main()`. On Windows, reserve the main stack at your executable's link as the
[game build does](release-flow.md#builds).

A Sunrise crate that runs in the game:

1. depends on `sunrise-platform` (never on an engine crate) and implements `Hooks` on a `static`, only the methods it
   needs: every method has an empty default;

   ```rust
   use sunrise_platform::hooks::{Hooks, Started};

   pub static HOOKS: Ping = Ping;
   pub struct Ping;

   impl Hooks for Ping {
       fn started(&self, game: &Started<'_>) { /* someone is playing: game.version */ }
       fn published(&self, open: bool) { /* the world was opened to LAN or friends, or closed */ }
   }
   ```

2. is added to `sunrise-official`'s dependencies and to its `register()`:
   `sunrise_platform::hooks::register(&my_crate::HOOKS)`.

Registration happens before the game runs: the engine seals the list as it starts (`entry::main`), and a later
`register` returns `TooLate`. Hooks run in the order they registered, on the render thread unless the method says
otherwise, and must return quickly. With nothing registered a dispatch is one load and an empty loop, so the plain
build pays nothing.

The hooks today: `started` (once, when someone plays: not a dedicated server, a benchmark or an automated run),
`published` (the world opened to or closed from LAN or friends, on change), `shutdown` (the window closed, the
world saved) and `tunnel` (the resolver of `friend:` addresses, Play with Friends') and `captures` (Open and Share for saved performance captures, `sunrise-perf-share`: with none registered the game offers neither) and [`client_identity`](../crates/sunrise-hooks/src/lib.rs) (the client brand and optional custom payloads sent during initial network configuration; the first answer wins, and none gives `vanilla` with no payloads. It runs on the network thread, so it must not touch game state. Local joins bypass the hook), and the window's connection to a server: `connecting`, `joined`, `left` and `ended` (each network connection closes in exactly one `left` or `ended`; a world the window plays itself calls none), and `retry`, which the Disconnected screen asks whether to connect again and what its line says (`sunrise_hooks::connection`, `sunrise-reconnect`: with none registered it offers nothing, as vanilla's), plus `frame_timed` (the rendered frame’s timing and scale, `perf::FrameTimes`). **A hook you need and don't find:** add a method with an empty default to `Hooks` and a function in
`sunrise_hooks::engine` that the engine calls where vanilla does the matching thing; give it plain data or a type
the engine owns, never one from above. A hook that needs engine types the platform doesn't expose yet (a draw
context for HUD and chat overlays, a settings page) comes with that type's platform API (M6). An extension point
that belongs to one subsystem (an input source, the compositor) lives in the engine crate that owns it, with the
same shape: registered before the game runs, read-only after.

**What players get.** The release builds (`game-build.yml`, so the nightly and the release)
build `sunrise-official` and ship it as `sunrise-client`, the name the launcher and players know. The fixed client
executable in the prebuilt SDK links the same official layer and calls `sunrise_official::register()` before starting
with native mods. The dedicated server build (`sunrise-engine`)
stays sunrise-client's, with no Sunrise part. sunrise-client's own executable, the one `cargo run -p
sunrise-client`, the harnesses and sunrise-plain build, has none either. The first part that moved this way is the
"I'm playing" ping (`sunrise-heartbeat`).

## Platform registries

The spec for the four registries that let a platform or Sunrise crate add settings, pages, HUD drawing and title
screen buttons without an engine file naming it. Controllers, replays (M6) and Sunrise's extras (M5) move out on
them.
Status: each registry lands in its own PR (layer plan M5, seam): the settings store (#14036), pages (#14070:
`SunriseSettings` and every row but `Slider`; #14110: `Slider` and `tooltip`; `Title` and `Pause` next), the HUD
hook (#14081), `seat()` (#14096), the title screen buttons (#2535).

**One shape for all four.** As with `Hooks` above:

- The trait and its `register` are in `sunrise-hooks` (engine; no dependencies, so it names no type from above).
  `sunrise-platform` re-exports them as `sunrise_platform::{settings, pages, hud, title}`. They take plain data
  (strings, numbers, translation keys) and no engine type. `sunrise-platform` stays light because the launcher
  and the telemetry server build it.
- A crate registers a `&'static` value before the game runs (`sunrise-official`'s `register()` for Sunrise,
  `sunrise_game::main()` for the platform). The engine seals every list as it starts (`entry::main`).
- A registrant keeps its own state: statics in its own crate, never a field of an engine struct. The engine
  only calls it.
- With nothing registered, the engine does no work: a load of an empty slice where it would dispatch. It makes
  no allocation, no draw call and no file. A vanilla build (sunrise-client, sunrise-plain) is unchanged.
- Visible text is a translation key the registrant ships in its lang files ([Lang files](#lang-files-sunrise_platformlang)),
  never English in code. Values a player typed (a name, a path) are the exception.
- `sunrise_platform::hooks::seat()` gives the current local-player index for scoped state.
  It defaults to zero, which is what the stock client uses.
- The enums are `#[non_exhaustive]`: a new row, act or place is an addition, not a break.

### Settings store (`sunrise_platform::settings`)

Each crate keeps its own settings and its own lines in the game folder, next to `options.txt`, which stays
exactly vanilla's.

```rust
pub trait Store: Sync {
    /// Its file in the game folder: `sunrise-extras.txt` or `sunrise-graphics.txt` (shared with the engine's
    /// extras and video settings until they all move) or its own `sunrise-<name>.txt`.
    fn file(&self) -> &'static str;
    /// Whether a `key:value` line of that file is this store's (a fixed list, or a `<name>.` prefix).
    fn owns(&self, key: &str) -> bool;
    /// One of its lines, as the game loads its settings. Unknown values keep the default.
    fn load(&self, key: &str, value: &str);
    /// Its lines, as the game saves its settings.
    fn save(&self, line: &mut dyn FnMut(&str, &str));
    /// Whether everything is at its default: a file that doesn't exist yet is then not written.
    fn is_default(&self) -> bool;
}
```

- The engine routes lines in `Settings::load` and `Settings::save`: once per load or save, never per frame.
  Each file is read once. A store's lines come after the engine's own in a shared file. A line nobody owns
  is kept as it was.
- A setting that moves out of `extras.rs` or `graphics.rs` keeps its key and file, so players keep their
  choice. The engine's `Extras` or `Graphics` stops reading and writing the key in the same PR.
- A page that changes a store's setting (below) needs nothing else: the options screen saves on close, as
  it does for every setting.

### Pages (`sunrise_platform::pages`)

A page lives in the options screen's shell, and its entry sits on one of the game's menus.

```rust
pub enum Place { SunriseSettings, Pause, Extras, Title }
pub trait Page: Sync {
    fn id(&self) -> &'static str;            // stable, for sunrise-ctl and the page stack
    fn title(&self) -> &'static str;         // translation key
    fn place(&self) -> Place;                // where its entry is
    fn rows(&self, rows: &mut Vec<Row>);     // asked when the page is drawn, not every frame elsewhere
    fn act(&self, row: usize, act: Act);     // a click, a reverse click, a slider's value, a key
    fn tooltip(&self, row: usize) -> Option<Label> { None }   // hover text, may follow the value
    fn back(&self) -> &'static str { "gui.done" }            // the leave button of a page drawn as a screen
}
pub enum Row {
    Heading(&'static str),                   // translation key
    Note(String),                            // grey text, a value (a path)
    Toggle { label: &'static str, on: bool },
    Cycle { label: &'static str, value: Label },
    Slider { label: &'static str, value: f32, text: Label },   // value in 0..=1
    Button { label: &'static str, active: bool, half: bool, footer: bool },  // Row::button(..).half()...
    Key { label: &'static str, key: String },                 // a binding: `key.keyboard.z`
    Text { text: String, quiet: bool, groups: bool },         // Row::text(..): white or grey, wrapped
    Space(u8),                                                // GUI pixels, on a page drawn as a screen
    Field { label: &'static str, text: String, max: usize },  // Row::field(..).max(n): an EditBox, on a page drawn as a screen
}
pub enum Label { Key(&'static str), Value(String) }
pub enum Act { Press, PressBack, Slide(f32), Bind(String), Edit(String), Submit }
pub fn connect(address: impl Into<String>);                   // from a Title page's act: join that server
```

- `SunriseSettings`: a row on Sunrise Settings' Extras page, after PvP & HUD's, that opens the page.
  `Title` and `Pause`: a button on the title screen or the pause menu that opens the page. These buttons
  exist only when something registered, so the vanilla menus keep vanilla's look. The page is drawn with the same widgets, tooltips,
  narration, controller focus and `sunrise-ctl` ids as the engine's pages.
- `Button` and `Text` are built with `Row::button` and `Row::text` and their options: `.active(false)` greys a
  button, `.half()` puts two side by side, `.footer()` sets one beside Done on a page drawn as its own screen,
  `.quiet()` greys text, `.groups()` breaks it only between `-`-separated groups (an invite code). In the options
  shell, text is notes and space is nothing.
- `Title` is the title screen's Friends icon slot (vanilla's `CommonButtons.friends`), not a new row: the icon
  keeps its place and look at every GUI scale, and opens the first page registered there as a form drawn as Join a
  Friend is (`multiplayer/page.rs`): the title, the rows from y 53 (a field's grey label, its 200-wide box 11
  below, 41 a field), the buttons in a column from a quarter of the height + 114, the back button last
  (`Page::back`, `gui.cancel` for a form). It shows heading, note, text, space, field and button rows. With no page
  there the icon opens the engine's Join a Friend while an invite-code resolver is registered, else it is
  inactive.
- `Field` is vanilla's `EditBox` on a page drawn as its own screen: focus by click or Tab, typing, the caret keys,
  selection, copy and paste, at most `max` characters (32 by default). The engine keeps its text while the page is
  open and sends each change (`Act::Edit`, before any other act or redraw reaches the page) and Enter
  (`Act::Submit`). The options shell has no field.
- Rows are rebuilt when the page is drawn or acted on, like the engine's own pages: no cost while no page is
  open.
- The engine keeps native mods' pages (`Page::NativeMod`) as they are. Both kinds sit in one module that
  `options_ui.rs` calls.

### HUD drawing (`sunrise_platform::hud`)

```rust
pub enum Corner { TopLeft, TopRight, BottomLeft, BottomRight }
pub trait Canvas {
    fn size(&self) -> (f32, f32);                                  // GUI pixels
    fn tr(&self, key: &str) -> String;                             // the player's language
    fn text_width(&self, text: &str) -> f32;
    fn text(&mut self, x: f32, y: f32, text: &str, argb: u32, shadow: bool);
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, argb: u32);
    /// A box of `w` × `h` in `corner`, stacked with the PvP widgets and mods' boxes;
    /// `None` while the debug screen covers that corner.
    fn place(&mut self, corner: Corner, w: f32, h: f32) -> Option<(f32, f32)>;
}
pub enum Stage { Hud, Top }
pub trait Layer: Sync {
    fn draw(&self, canvas: &mut dyn Canvas);
    fn stage(&self) -> Stage { Stage::Hud }
}
```

- `Stage::Hud` layers draw in the HUD pass where the extras do (`game/interface/extras.rs`, `draw_extras`,
  beside the native mods' HUD boxes), in the order they registered. They are hidden with F1, and a corner the
  debug screen covers is `None`, as for mods' boxes.
- `Stage::Top` layers draw last, over everything: after the debug screen, over menus and the title screen, F1
  included. Nothing is stacked there: `place` is always `None`. The performance overlay is one.
- With no layer registered the engine builds no canvas.
- A layer reads its own state, which its crate keeps up to date through `Hooks` or its own threads. It never
  reads the engine's.
- Sprites, items and the player's state come with their platform API (M6). They are not in this first
  version.

### Look (`sunrise_platform::look`)

What the engine draws differently for a crate above, as plain data: PvP & HUD's knobs today (#14217).

```rust
pub struct Look {       // #[non_exhaustive]; Default is vanilla's
    pub legacy_pose: bool, pub hide_attack_indicator: bool,
    pub mute_attack_sounds: bool, pub hide_attack_particles: bool,
    pub frame: Frame,   // outline, hurt overlay and glint colours
    pub shield_size: u8, pub totem_scale: f32, pub fire_lower: f32,
    pub clear_lava: bool, pub clear_water: bool, pub chat: Chat,
    pub hide_effect_icons: bool, pub time_of_day: Option<i64>, pub weather: Option<(f32, f32)>,
}
pub fn update(change: impl FnOnce(&mut Look));  // the player whose turn it is (`seat()`)
```

- Pushed, not asked: a crate changes its own fields when its settings load or change (several crates share the
  look, each keeping to its fields), and the engine reads it where it draws (`game/look.rs`, the only reader).
  Nothing is called per frame, and a seat nothing changed sees vanilla's.
- A knob is named by what it changes in the picture or the sound, never by the feature that uses it. A new knob
  is a field added with vanilla's value as its default.

### Container screens (`sunrise_platform::containers`)

Input and drawing on any open menu screen: the inventory, chests, furnaces, crafting tables, anvils, merchants,
a mount's inventory, a mod's menu. It is a general seam: its first users are Mouse Tweaks and sorting (M5),
and nothing in it is shaped for them.

```rust
/// The open screen's menu, for the length of one call. The engine implements it.
pub trait Container {
    fn menu(&self) -> &str;                     // "inventory", "minecraft:anvil", a mod's, "sunrise:mount_inventory"
    fn slots(&self) -> Vec<Slot>;               // every slot drawn, in the screen's order
    fn stack(&self, slot: SlotId) -> Option<Stack>;
    fn carried(&self) -> Option<Stack>;         // the stack on the cursor
    fn name(&self, slot: SlotId) -> Option<String>;              // the stack's shown name
    fn takes_one(&mut self, slot: SlotId, of: &Stack) -> bool;  // the menu's rules: mayPlace, the slot's limit
    fn landing(&mut self, slot: SlotId) -> Vec<SlotId>;         // where quickMoveStack would put its items
    fn click(&mut self, slot: SlotId, click: Click) -> bool;    // one ordinary click, as vanilla sends it
    fn spread(&self) -> usize;                  // the slots vanilla's quick craft drag covers now
    fn cursor(&self) -> (f32, f32);             // GUI pixels, from the panel's corner
    fn shift(&self) -> bool;
    fn panel(&self) -> Panel;                   // imageWidth, imageHeight, title and inventory label tops
}
pub struct SlotId(pub u32);                     // opaque, stable while the screen is open
pub struct Slot { pub id: SlotId, pub role: Role, pub x: f32, pub y: f32 }  // top left, from the panel's corner
pub enum Role { Hotbar, Inventory, Storage, Input, Result, Equipment, Other }   // by the open menu
pub struct Stack { pub item: String, pub count: u32, pub max: u32, pub kind: u64, pub id: u64 }
pub enum Button { Left, Right, Middle, Other(u16) }
pub enum Click { Pick { right: bool }, QuickMove, Put { one: bool } }               // Put: a one-slot QUICK_CRAFT drag

// In `Hooks`: `fn containers(&self) -> Option<&dyn ContainerHooks>`; every `ContainerHooks` method
// does nothing by default, so vanilla's handling runs:
fn press(&self, c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool;    // MouseButtonPressed.Pre
fn release(&self, c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool;  // MouseButtonReleased.Pre
fn drag(&self, c: &mut dyn Container, slot: Option<SlotId>, button: Button) -> bool;     // MouseDragged.Pre
fn wheel(&self, c: &mut dyn Container, slot: Option<SlotId>, steps: f32) -> bool;        // MouseScrolled.Pre
fn key(&self, c: &mut dyn Container, slot: Option<SlotId>, key: &str) -> bool;           // KeyPressed.Pre, `key.keyboard.r`
fn draw(&self, c: &dyn Container, canvas: &mut dyn hud::Canvas);                         // Render.Foreground
```

- The hooks run at the screen events' own moments, the ones the mods' `screen_*_pre` and
  `screen_render_foreground` hooks have: an input right after the mods' pre hook let it through and before
  anything else reads it (the recipe book, the slots, vanilla's quick craft), the drawing after the slots and
  under the tooltips and the carried stack.
- `true` means the hook took the input: nothing else reads it. The first hook that returns `true` wins; every
  hook hears `release` and draws. A press a hook took makes the button's release the hooks' too, so vanilla
  never sees half a click; the latch ends at the physical button-up, whoever cancels the release. A drag or
  a release a hook takes from vanilla's own press ends vanilla's quick craft drag without a click.
- `drag` is asked only while a button is physically held. `key` is not asked while a text field has the focus
  (a search box, an anvil's name), nor for a key's repeats. No input is offered while another screen covers
  the menu (a server pack prompt, a dialog).
- Each `click` is an ordinary slot click: the menu engine's on the player's own world, a `ContainerClick` on a
  server. So an extra built on it works wherever vanilla's clicks do, and anticheats see vanilla's packets.
- Every menu screen is offered, with its menu type id; the crate decides which menus it acts in, by `menu()`.
  Creative's tabs are not offered: they have no menu type. Drawing runs from the shared foreground phase,
  so every menu screen that draws one (merchants and beacons too) draws the hooks.
- A slot's role comes from the open menu: a furnace's or station's output, a mod menu's declared results and
  its crafting result are `Result`. A `SlotId` that isn't the open menu's reads nothing and clicks nothing.
- A `Stack`'s `kind` is equal for stacks that merge (`StackData`'s `Hash`, which matches its `PartialEq`); its
  `id` is that exact stack, for `takes_one`.
- The reads that ask the menu's rules take `&mut self`, as the engine's do; drawing gets `&dyn Container`, so
  it reads the slots, stacks and names but clicks nothing.
- The engine builds a `Container` only when a hook is registered and a menu screen gets input or draws.
  Without one there is no cost, and vanilla's handling is unchanged.
- The only text is a stack's shown name. The rest is ids and numbers, so the API names no engine type
  (`ItemStack` stays in the engine).

### Title screen buttons (`sunrise_platform::title`)

The title registry, shaped like the ones above: full-width buttons on the title screen, each on its own row below
Minecraft Realms, as NeoForge adds Mods. The engine's title code names none of them: the platform's mod host adds
Mods (`sunrise-mod-host`'s `mods_button.rs`), shown while one or more mods are installed, with its label in the
same crate's lang files, registered beside the button.

```rust
pub struct Title { pub mods: usize }            // what the screen knows: packages in the mods folder
pub enum Opens { Mods }                         // the engine screen a press opens
pub trait Button: Sync {
    fn label(&self) -> &'static str;            // a translation key
    fn shown(&self, title: &Title) -> bool;     // asked each time the buttons lay out
    fn opens(&self) -> Opens;
}
pub const FIRST_ID: u8 = 200;                   // each registration takes the next main-menu id
pub const MAX_BUTTONS: usize = 56;              // and `register` refuses one with no id left
```

- Rows keep the order buttons registered in, and a press carries the row's id, so it resolves the registration it
  selected without asking `shown` again. The icon row, Options... and Quit Game move down a row for each shown
  button, and lift on a short screen so they stay above the copyright line; a screen too short for the lifted
  block to also clear the logo and edition banner shows fewer rows — the ones that fit.
- With no button shown the title screen is vanilla's.

### Item catalog (`sunrise_platform::items`)

Reads of vanilla's own item catalogs, which any crate may use (an inventory tool, a search, a sorter); the engine
fills them from the data it loaded, and nothing in them is per feature.

```rust
fn creative_order(stack: &containers::Stack) -> Option<usize>;  // its place in vanilla's creative order
```

- This is the stock order, the one the built-in sorter ranks by (the search tab's stacks, then the ungrouped items),
  not the search tab as mods and permissions change it. The place of the same stack (same item and components,
  by `Stack::kind`), else its item's first place; `None` for an item the creative tabs don't show, or before the
  game has built them.
- The window's game publishes the table each time it builds the creative tabs' contents, so a read is two hash
  lookups, never a scan.

### Lang files (`sunrise_platform::lang`)

A crate keeps the text for its keys in its own folder, `crates/<name>/assets/lang/<code>.json`, with the same
shape as the engine's `assets/sunrise/lang` files (flat `"key": "text"` lines, `en_us.json` the reference).
It registers them as `(code, JSON)` pairs, usually `include_str!`s:

```rust
static LANG: &sunrise_platform::lang::Files = &[
    ("en_us", include_str!("../assets/lang/en_us.json")),
    ("fr_fr", include_str!("../assets/lang/fr_fr.json")),
];
sunrise_platform::lang::register(LANG)?;
```

- The game loads them into its `sunrise` namespace right after its own bundled file for the same language, so a
  resource pack still overrides them, and a language a crate lacks falls back to its English.
- A key belongs to one folder. On a key the engine's file also has, the engine's text wins (and the
  `lang.rs` test fails).
- Regional variants read their language's file as the engine's do (`fr_ca` reads `fr_fr`, `de_ch` reads `de_de`
  without `ß`).
- `tools/lang_coverage.py` reports each crate's folder beside the engine's. A person's review of a crate's entry
  is kept in `tools/lang_review/<name>/<code>.json` ([translations.md](translations.md)).
- With nothing registered, a language loads exactly as before.

### Mods screen rows (`sunrise_platform::mod_rows`)

A crate lists something the game ships that is not an installed mod on the Mods screen: a `&'static Row` of
plain data (id, name, version, authors, licence and its link, homepage, issues, description). `authors` and
`description` are translation keys; the rest are proper names, versions and addresses, shown as written.

- The screen's entries are Minecraft, every registered row (registration order is their insertion order) and the
  installed mods; it shows them sorted by name, reversed on request, as `ModListScreen` does. No row replaces
  another.
- With nothing registered (the plain `sunrise-game` and `sunrise-client` executables), the screen lists Minecraft
  and the mods only.
- A row's text belongs to the crate that registers it, in its own lang folder (`sunrise-official` ships the
  Sunrise row's credit and description there).
- This is how a Sunrise-layer crate adds a row: Sunrise crates register through the engine's generic registries
  (umceko, 2026-11-18), not as built-in mods. The registry moves behind the engine's `extension-points` feature
  once that lands (#2510). A mod's own row always comes from its metadata.

## Extras that may leave the game

Sunrise's extras are Sunrise-layer code. Each one is a candidate to become an optional native mod that players add
themselves, or stays a module linked into the official build, as auto-reconnect is (crate `sunrise-reconnect`). The list is here so new work doesn't add to them. See [extras.md](extras.md) for each one:

- zoom (OptiFine-style), keystrokes and FPS overlays, own name tag;
- Mouse Tweaks controls, sorting, scroll-wheel settings, smooth chat;
- UI-sound channels, the music page;
- the PvP & HUD presets and widgets, the replay editor (Replay Studio's camera paths and timeline), the AI agent page;
- the Sunrise Settings menu itself, the advanced menu that hosts them;
- the renderer's extras: fullbright, dynamic lights, hidden block-break particles;
- chunk pregeneration.

Zoom, the FPS counter and Keystrokes already moved to native mods ([extras.md](extras.md#moved-to-native-mods)).
Controllers and replay recording are platform ([above](#controllers-replays-and-ui-pages)).

## The checks

`python3 tools/check_layers.py` runs in CI's verify job and in `tools/pr_ready.py`. It reads the `cargo metadata` of the
four workspaces `tools/layers.json` lists (the game, `https://github.com/sunrise-org/sunrise-launcher`, `crates/sunrise-mod-registry/` and
`crates/sunrise-telemetry-server/`; no build, about a second) and [`tools/layers.json`](../tools/layers.json), which
gives every crate its layer. It fails when:

- a crate has no layer;
- a dependency goes up a layer and neither `allowed` lists it nor `optional` lists it as an optional dependency;
- an `optional` entry's dependency is no longer optional (one mandatory declaration, for any target or as a build
  dependency, is enough);
- a pull request adds an entry to `allowed` or `optional` for a crate that already had a layer (a crate newly split
  out of another may carry the old crate's exceptions, and an `allowed` entry may move to `optional`);
- shipped code depends on a tool;
- a `portable.pending` crate depends on anything beyond its entry, or a pull request adds to an entry.

Each failure names the section of this page that applies. An `allowed` or `optional` entry that has no dependency
left is only reported: take it out when you like. `--list` prints every crate's layer and the upward dependencies
left.

`python3 tools/check_addresses.py` (horizon-rust/horizon#12990) checks the built-in addresses
([below](#built-in-addresses)); it runs in CI's verify job and in `tools/pr_ready.py` too.

### Today's exceptions

The current dependency inventory is in `tools/layers.json`, with fork-specific classifications and reasons in
`_fork_baselines`. It records existing fork dependencies and the restored upstream Horizon layer dependencies. The separate launcher and registry workspaces take two engine crates: `sunrise-mod-package` (the launcher's
package, modpack and index reads; the registry's index format) and `sunrise-system` (the launcher's bug reports).
Those three edges are `allowed` rows like the launcher's other engine dependencies.

## Built-in addresses

An address is built in when the code contacts it without the player typing it. Engine code may hold vanilla's own
endpoints. Every other built-in address belongs to the Sunrise layer, in one list file:
`crates/sunrise-addresses/src/lib.rs` (#12993). It moves with the Sunrise layer. The standalone launcher
has its own address crate. Platform code gets its addresses passed in.

`tools/check_addresses.py` runs in CI's verify job and in `tools/pr_ready.py`. It reads the string literals of shipped
Rust and finds every host. It skips tests, placeholders (`example.com`, `*.test`) and local IPs.
`tools/addresses.json` says where each host may be:

- `homes`: the list file;
- `vanilla`: vanilla's endpoints, allowed anywhere;
- `not_addresses`: strings that look like hosts but are not contacted, each with its reason. A `path host`
  entry applies only in that file;
- `allowed`: today's addresses outside the list file, as "path host".

`allowed` only shrinks. A branch may move an entry with its code, but it may not add a host or grow the list.

Vanilla's endpoints (engine, the same as Java Edition):

| Address | For | Where |
|---|---|---|
| `discovery.minecraftservices.com`, `api.minecraftservices.com` | services discovery, profile keys, player certificates | `auth.rs`, `sunrise-server-files/src/profiles.rs` |
| `textures.minecraft.net` | skins and capes | `skins.rs` |
| `pc.realms.minecraft.net`, `java.frontend.realms.minecraft-services.net` | Realms | `realms/client.rs` |
| `aka.ms/…` | vanilla's help links (accessibility, moderation, EULA, Realms terms), opened only on a click | `game/options.rs`, `game/screens.rs`, `ban_notice.rs`, `realms.rs`, `eula.rs` |
| `libraries.minecraft.net`, `piston-meta.mojang.com`, `launchermeta.mojang.com`, `piston-data.mojang.com`, `resources.download.minecraft.net` | vanilla jars, the Java runtime, libraries and assets | `sunrise-prepare`, launcher |

Sunrise's own and third-party addresses (Sunrise layer):

| Address | For | Where today |
|---|---|---|
| `playsunrise.net/perf`, `playsunrise.net/api/perf/captures` | the performance viewer Open shows a saved capture in, and the upload behind Share (`SUNRISE_PERF_VIEWER`, `SUNRISE_PERF_CAPTURES` override them) | `sunrise-addresses`, used by `sunrise-perf-share` |
| `telemetry.playsunrise.net` | opt-in telemetry, the "I'm playing" ping, benchmark uploads | `sunrise-addresses` (#12993) |
| `stun.cloudflare.com`, `stun.l.google.com` | Play with Friends NAT discovery | `sunrise-addresses` (#12993) |
| `sunrise-org.github.io/sunrise-mods`, `github.com/sunrise-org/sunrise-mods`, `playsunrise.net/mods` | Sunrise's mod registry: index, mirrored release assets, icons and screenshots, report form and project pages (`SUNRISE_MOD_INDEX` overrides the index) | `sunrise-launcher-addresses` in the launcher repository; `cargo-sunrise-mod` reads `sunrise_addresses::MOD_REGISTRY_INDEX` and `MOD_REGISTRY_METADATA`; `sunrise_mod_package::index` (`REGISTRY_REPO`, `FILE_HOST`) supplies the registry release defaults for `sunrise-mod-registry` |
| `api.modrinth.com`, `cdn.modrinth.com` | the in-game resource pack browser, modpack porting | `pack_browser.rs`, `sunrise-port-pack` |
| `github.com`, `raw.githubusercontent.com`, `gitlab.com`, `maven.neoforged.net` | modpack porting's sources | `sunrise-port-pack` |
| `play.playhorizon.dev` | the official Horizon server, which the client identity hook answers for | `sunrise-addresses`, `sunrise-official` |
| `playsunrise.net` | the download link shown with no resources | `no_resources.rs` |
| `api.github.com`, `discord.com` | hosting: updates, Discord webhooks | `sunrise-server` |
| `api.github.com`, `uploads.github.com`, `playsunrise.net` | the registry's submission service | `sunrise-mod-registry` |
| Modrinth, CurseForge, Fabric, Quilt, Forge, NeoForge, GitHub, Microsoft and Xbox sign-in, Tailscale, `discord.gg`, `mclo.gs`, `static.rust-lang.org` | the launcher's browsers, loaders, sign-in, logs and hosting | `https://github.com/sunrise-org/sunrise-launcher/tree/main/src/`, `https://github.com/sunrise-org/sunrise-launcher/blob/main/links.json` |

These are not addresses:

- `sunrise-server/src/discord.rs` lists the hosts a webhook URL may have;
- the Windows, Apple and SOAP schema URLs in manifests and requests, and the UPnP multicast group.

## Reviewer checklist

- The PR names its layer, and new code sits in a crate or module of that layer (the table above).
- No dependency its layer may not use. CI fails on one; a module in an engine crate that calls platform or Sunrise
  code is just as wrong, and so is Sunrise code that reaches into the engine instead of the platform.
- Sunrise features in the game are behind `sunrise-extras`, and nothing new names a Sunrise crate below it.
- No new built-in address outside `sunrise-addresses`.

## Plan

Review first. The design and this plan are open for review in the tracking issue for ~48 hours. The small
mechanical steps (checks, M1–M4) continue meanwhile. M5 and M6 started on 2026-11-09, after the review in #13011. Each step is a PR under ~500 lines. Each move:

- keeps the default build the same for players and costs nothing at run time;
- keeps `--no-default-features` and sunrise-plain building;
- takes its entries out of the allowlists;
- runs the harnesses `tools/parity.py --changed-files … --list` selects when it touches gameplay.

| Step | What | Removes | State |
|---|---|---|---|
| 1 | This page, first version | | #12979, merged |
| 2 | Layer check in CI and pr_ready, with the PR template's checklist | | #12983, merged |
| 3 | Address check in CI and pr_ready | | #12990, merged |
| 2b | The portable crates' rule in the layer check ([Portable crates](#portable-crates)) | | #13002 (NikitaMGrimm), merged |
| M1 | `sunrise-addresses`, the address list file; the Sunrise crates read it | 7 address entries | #12993, merged |
| M2 | `sunrise-system` (engine): the hardware probes leave `sunrise-telemetry`, and the crash report and GPU info read them there; `sunrise-platform`, the platform's public API, re-exports them for the Sunrise layer | prepares M3 | #13058, merged |
| M3 | The renderer's telemetry module leaves `sunrise-client-render` for `sunrise-client-lib`, with Sunrise's other in-game parts; the renderer offers the adapter (`gfx::adapter`) and a crash-report hook (`crash::on_report`) | render → telemetry, bench | #13060, merged |
| M4 | Cosmetics leave `sunrise-client-render` (#13094); Sunrise later removed them, with the player-model hooks they drew through | render → cosmetics | #13094, merged; then removed |
| M5 | Sunrise's in-game parts (extras, Discord, Friends, ping, benchmark upload) leave `sunrise-client` for Sunrise crates linked into the official build through platform hooks | client, client-lib → friends, telemetry, bench | started 2026-11-09 |
| M6 | The engine crates stop depending on platform crates: the renderer's shader-pack, pack-format, HDR and theme code moves behind engine extension points; the mod host and agent control move to platform crates; controllers, replay recording and the UI page API move to platform crates on the engine's input-source and screen hooks; the launcher and `sunrise-server` reach the engine through platform API (done, #13905) | the engine → platform and Sunrise → engine exceptions | started 2026-11-09; HDR's part with NikitaMGrimm, below |
| P | Optional, longer: gameplay into the portable crates. `Game` holds the world and its presentation (renderer, screens, particles, sound): split its state module by module, turn gameplay's direct particle and sound calls into level events as vanilla's server sends them, then move each clean module into `sunrise-gameplay` (server side) or `sunrise-client-gameplay` (client only); waves 1 and 2 of #2210 moved the modules that need no `Game`, and `LanHost`, `World`, `ChunkMap`, `Mob`, `ItemEntity` and the level events wait for wave 3+ | | any time, small PRs |
| R | The Sunrise layer moves to its own repository and ships in every official download (the table above) | | after the split, Ludovic's call |

HDR in M6: moving HDR behind engine extension points moves the frame pipeline (float targets, tone mapping,
present). NikitaMGrimm leads HDR, so that part is planned with them and comes after HDR's step 5 (float targets on
by default), so the pipeline moves once. HDR off stays bit-exact.

The mod API work on arismoko/horizon lands upstream as part of the platform. The API owns its types,
`sunrise-mod-runtime` gets no game dependency, and its CI property check stays as a stricter check inside the
platform.

Engine crates import `sunrise-mod-vocab` as `mod_api`. Their test-only
`mod_platform` dependency reaches `sunrise-mod-api`; dev-dependencies are exempt
from the layer rule because tests may exercise the platform, while shipped code may not.
