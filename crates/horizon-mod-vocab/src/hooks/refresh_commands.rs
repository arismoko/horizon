//! Command refresh guidance carried from upstream; the command service owns the method.

pub trait Notes {
    #[notes = r#"`refresh_commands()` (`Host::refresh_commands()` for trait mods) asks every mod's `register_commands` again at the start of the next server tick, rebuilds the commands players may run, and sends every player on the server their tree again after `player_commands_sent`, as vanilla's `PlayerList.sendPlayerPermissionLevel` (`Commands.sendCommands`) does after `/reload`. Nothing happens where no world is open to players. Bundles need "hooks": @API:refresh_commands@.

On the fork this method is `host.commands().refresh_commands()`, on the server-side
`Commands` service. It rebuilds the existing typed command dispatcher as well as
the client trees. It is not a second Host method or a registration hook."#]
    fn refresh_commands();
}
