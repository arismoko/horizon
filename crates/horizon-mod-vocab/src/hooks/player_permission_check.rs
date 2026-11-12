//! The `player_permission_check` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The permission provider: whether `player` (`name`) holds permission
    /// `node` (`essentials.home`, `worldguard.region.bypass.*`), as a
    /// permissions mod keeps it. Answer `Some(true)` to grant it,
    /// `Some(false)` to deny it, or leave `value` None when the node is not
    /// set for them; the first mod that answers decides. Asked by
    /// `PlayerRef::has_permission` and `PlayerRef::permission_value`, and for
    /// a command that requires a node (`Command::with_node`) when the
    /// player's command tree is built and when they run it. Answer from the
    /// mod's own state: it is asked often and must not change the world.
    /// Server-side.
    #[notes = r#"## Player permission check

- **`player_permission_check(host, event)`**: the permission provider a permissions mod (a LuckPerms port, say) answers. Vanilla has only operator levels; Bukkit's `Permissible.hasPermission(String)` and NeoForge's `PermissionAPI.getPermission` ask a registered handler for a node instead. Here the handler is this hook: the first mod that sets `event.value` decides, and when no mod answers, `PlayerRef::has_permission` falls back to vanilla's operator check (permission level 2 or more, `Commands.LEVEL_GAMEMASTERS`), as Bukkit treats a node with no default (`PermissionDefault.OP`). `PlayerRef::permission_value` gives the answer itself, None for unset, as `Permissible.isPermissionSet` tells set from unset.
- A provider whose answers change for players already on calls `host.commands().refresh_commands()`, so the command trees of nodes (`Command::with_node`) follow."#]
    #[hook(
        side = "server",
        contract = "replace",
        out = "value",
        host = "read",
        neoforge = "PermissionAPI.getPermission (the registered IPermissionHandler)",
        paper = "Permissible.hasPermission / Permissible.isPermissionSet"
    )]
    fn player_permission_check();
}
