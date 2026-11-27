//! The `register_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The commands native mods add to the server's command tree: push each usage with
    /// its executor attached by [`Command::executes`]. Asked whenever the server builds
    /// its command tree, as the reference loader's
    /// `Commands` does: when a world opens (a dedicated server, a LAN host, single player's
    /// server thread), before its functions load, so a function line may name your command;
    /// on each `/reload` and data pack change, so a command you stop pushing is refused by the
    /// next functions loaded; and after a mod calls `host.commands().refresh_commands()`.
    /// Every player's chat box completes them as vanilla's, within their level. A usage
    /// under a root vanilla owns merges into it, as `RegisterCommandsEvent` registers into
    /// vanilla's `CommandDispatcher` (Brigadier's `CommandNode.addChild`): vanilla's usages and
    /// permissions stay, the mod's children are added beside them, and an executor at a vanilla
    /// node replaces vanilla's (see [`Command`]). A lower-level child opens traversal through a
    /// prefix without changing that prefix's own execution permission. When accepted usages
    /// execute at the same node (including optional prefixes), each later accepted registration
    /// overwrites that node's executor (matching Brigadier's `CommandNode.addChild`, which sets
    /// `child.command = node.getCommand()`) together with Sunrise's paired execution permission
    /// level for that node. Only that node's executor runs, attributed to its mod; if the
    /// owner is disabled, execution is refused rather than handed to another mod.
    /// Clients get the command tree, not executors. An argument's kind is one
    /// of `word`, `string` (a word or a quoted phrase), `text`, `integer`, `integer(min,max)`,
    /// `long`, `long(min,max)`, `number`, `boolean`, `player`, `item`, `block_pos`, `vec3` and `vec2`,
    /// or a command argument type a mod
    /// registered (`Registrar::register_command_argument`), named by its namespaced id.
    #[notes = r#"## Command argument kinds beyond the basics

- Besides `word`, `text`, `integer`, `number`, `boolean`, `player`, `item` and `block_pos`, an argument's kind may be `vec3`, `vec3_uncentered`, `vec2`, `vec2_uncentered`, `column_pos`, `rotation`, `dimension` or `nbt`. Each is sent to clients as vanilla's parser, so `~`, `^` and the dimension list complete as in a vanilla command. Neither `vec3`/`vec3_uncentered` nor `vec2`/`vec2_uncentered` can share a position in one command's usages, as they look the same to a client.

- `vec3` and `vec3_uncentered` deliver `CommandArgumentValue::Vec3`; `vec2` and `vec2_uncentered` deliver `CommandArgumentValue::Vec2` (`x` and `z` as `[f32; 2]`, `~` relative parts only); `column_pos` delivers `ColumnPos`, `rotation` delivers `Rotation` (yaw, pitch), `dimension` delivers `Dimension(LevelRef)` and `nbt` delivers `Nbt(Compound)`. `~` and `^` resolve against the source's position and rotation. A `dimension` the world has not loaded is refused with `argument.dimension.invalid` before the executor runs.

- `string` reads one word or one `"quoted"` / `'quoted'` phrase, as Brigadier's `StringArgumentType.string()` (`QUOTABLE_PHRASE`), and other arguments may follow it. A registered argument type reads the same with `CommandArgumentKind::Quotable` (otherwise one word, or the rest of the line with `Greedy`). Word, `string` and greedy text values are strings: a `string` arrives decoded as `CommandArgumentValue::String`, without its quotes and escapes, and a `Quotable` type's `parse` gets that decoded text. Clients receive both as `brigadier:string` mode 1."#]
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_register_commands",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "commands",
        answered_by = "mod",
        neoforge = "RegisterCommandsEvent",
        version = 5
    )]
    fn register_commands(commands: &mut Vec<mod_api::Answered<mod_api::CommandRegistration>>);
}
