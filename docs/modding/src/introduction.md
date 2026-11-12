# Modding Horizon

Horizon is Minecraft: Java Edition 26.3 rewritten in Rust. A Horizon mod is one `.hmod` package. It is one of:

| Kind | Holds | Runs |
| --- | --- | --- |
| **Content mod** | `assets/` and `data/`: blocks, items, recipes, loot, tags, models, textures | read when the game starts |
| **Native mod** | a Rust crate (`Cargo.toml`, `src/`) beside the content, using the mod API | compiled into a game the launcher builds for the player |
| **WebAssembly mod** | a `mod.wasm` beside the content, built from the same Rust source | in the game's one sandboxed runtime, with no rebuild |

Native and WebAssembly mods are written against one API, `mod_api`, and share one dependency model: the only
difference between them is the build target.

## How this book is organised

The book follows [Diátaxis](https://diataxis.fr): each page is one of four kinds, and says so by where it lives:
**Start** (a tutorial), **Concepts** (explanation), **How-to** and **Reference**. See
[where each doc lives](contributing.md).

An [agents page](agents.md) and `llms.txt` give a coding agent the rules in one place.

A native mod that has to change the game's own code, beyond what hooks and [injections](injections.md) reach,
uses [the native route](native-route.md): `[[native.mixin]]`, `[[native.loop]]`, `[[native.bridge]]` and
`[[native.module]]`, each with its own page.

## Where things are

- The API reference is `cargo doc -p horizon-mod-api`: every hook, event and `Host` method, with its side,
  its NeoForge 26.3 counterpart and its cancel contract.
- The end-to-end example mod is
  [horizon-machine-sample](https://github.com/arismoko/horizon-machine-sample).
- Design and milestones: [#26](https://github.com/arismoko/horizon/issues/26) and
  [#44](https://github.com/arismoko/horizon/issues/44).
