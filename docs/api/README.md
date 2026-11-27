# Sunrise APIs

Use this index to find the guides and references for building mods, running Sunrise, or integrating it with other tools.
See [stability and versioning](stability.md) for how to read the stability labels in the API pages.

| Surface | What it covers | Guides and references |
| --- | --- | --- |
| [Mods](../modding/src/introduction.md) | Content and native mods both ship as `.smod` packages. Native mods use `mod_api` and build against the SDK for an exact game build. | [Manifest](../modding/src/reference/manifest.md), [SDK](../modding/src/reference/sdk.md) |
| [Agent control](agent-control.md) | Drive a running game with `sunrise-ctl`, its MCP tools, or the local control socket. | [sunrise-ctl commands](../sunrise-ctl-commands.md) |
| [Servers](server.md) | Host and manage a dedicated server with `sunrise-server`, `server.toml`, and the `/api/v1` control API. | [Server reference](server-reference.md) |
| [Launcher](launcher.md) | Manage installations and play through the `sunrise-launcher` CLI or MCP. | [Launcher commands](https://github.com/sunrise-org/sunrise-launcher/blob/main/docs/launcher-commands.md) |
| [Launch](launch.md) | Integrate another launcher with `sunrise-prepare` and `sunrise-client`: resource preparation, launch options, session data, and exit codes. | [Launch guide](launch-guide.md), [Launch reference](launch-reference.md) |
| [Data formats](data-formats.md) | Read files such as benchmark reports, server settings, and signed release checksums. | Format links and version fields on that page |

## How these APIs are designed

Vanilla Minecraft: Java Edition 26.3 is the specification for game behaviour. Mod API hooks use NeoForge 26.3
as the design reference for names and shapes where a counterpart exists; see the
[mod API reference](../modding/src/reference/api.md) for the exact Sunrise contracts and any differences.
Single player runs on an integrated server thread and uses the same server-side gameplay code as LAN and
dedicated servers.

Content mods provide data and assets without running code. Native mods run in the game process with the player's
permissions; see the [modding book](../modding/src/introduction.md) for how the two kinds of package differ.

## References are generated

Update the source, then regenerate the reference when its contract changes:

| Page | Generated from | Update it with |
| --- | --- | --- |
| [Server reference](server-reference.md) | Help text, `config::TEMPLATE`, and `api::ROUTES` in `crates/sunrise-server` and `crates/sunrise-server-launch` | `SUNRISE_WRITE_DOCS=1 cargo test -p sunrise-server reference` |
| [sunrise-ctl commands](../sunrise-ctl-commands.md) | The command table in `crates/sunrise-client/src/bin/sunrise-ctl/commands.rs` | `sunrise-ctl help --markdown > docs/sunrise-ctl-commands.md` |
| [Launcher commands](https://github.com/sunrise-org/sunrise-launcher/blob/main/docs/launcher-commands.md) | `src/cli.rs` in [sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) | `SUNRISE_WRITE_DOCS=1 cargo test the_launcher_reference` in that repository |
| [Launch reference](launch-reference.md) | `crates/sunrise-gameplay-common/src/game/runtime/launch_api.rs` | `SUNRISE_WRITE_DOCS=1 cargo test -p sunrise-gameplay-common --lib launch_api` |

## Each surface's page

Start with the overview or guide for examples, then use the reference to look up commands, arguments, or file formats.
These pages are published at [playsunrise.net/docs/api](https://playsunrise.net/docs/api) by the website in
`sunrise-org/sunny`.
