# Config files and settings

A mod keeps its options in one of two ways:

| | Config files | Settings |
| --- | --- | --- |
| Lives in | The game's `config/` folder (on a server, `serverconfig/` per world) | The mod's data folder, one file per key |
| Edited by | The player, in a text editor | The player, on the mod's page in **Options > Mods** |
| You declare it with | `Hooks::register_configs` | `Registrar::register_setting` |
| You read it in | `config_loading` and `config_reloading` | `Host::setting` and the typed `setting_*` reads |
| NeoForge counterpart | `ModContainer.registerConfig`, `ModConfigEvent` | `ModConfigSpec` entries on the config screen |

## Config files

Config files sit in the game's shared `config/` folder, beside `options.txt` (beside `server.properties` on a
dedicated server), as Fabric and NeoForge mods' do. Your mod picks its own names: a folder `config/<mod id>/`, or
files `config/<mod id>.<ext>` and `config/<mod id>-<rest>`.

1. In `register_configs`, push each file with its kind (`Client`, `Common` or `Server`) and the default text to
   write when the file is missing. Push all of them every time: each game reads only its own kinds.
2. The game reads each file and calls `config_loading` with the mod id, the file and its text, which your mod
   parses (TOML by convention). Every mod hears every config, so check the mod id.
3. When the file changes on disk, `config_reloading` brings the new text. The game checks modification times once a
   second; a save with the same text, a removed file or an unreadable one fires nothing and keeps the last text.
4. A `Server` config is per world: it loads before `server_about_to_start` from the world's `serverconfig/`, and
   `config_unloading` drops it after `server_stopping`. `Client` and `Common` configs load once and stay until the
   game quits.

These hooks run in the Config [load phase](../concepts/load-phases.md): settings reads and config files are
available, world handles and transactions are not. A file outside `config/`, or one already registered, is
skipped.

For files the hooks don't manage, `Host::config_files()` gives your mod its own files in `config/`. The service is
bound to the mod whose hook or Part is running: you never pass your id, and you cannot reach another mod's files.

## Settings

Register each setting from `Hooks::register` with `Registrar::register_setting(SettingSpec { … })`: a key
(`a-z`, `0-9`, `_`, `-` and `.`, not starting with `.`), translation keys for its label and tooltip, and its kind
with the default. A setting the spec check refuses, or a key you registered already, comes back as the error.
Settings are client settings and appear on your mod's page in **Options > Mods**, opened from its **Config**
button.

- Read a setting with `host.setting(mod_id, key)` or a typed read. Typed reads are cached per data folder and key.
- `host.set_setting(mod_id, key, value)` keeps a value. A successful write updates the cache; a failed one leaves
  it. This write does not fire `setting_changed`.
- When the player edits a setting, the value is kept first, then `setting_changed` tells your mod alone.

A mod reads and writes only its own settings: another mod's id, or a key its owner never registered, reads
`None` and writes nothing.
