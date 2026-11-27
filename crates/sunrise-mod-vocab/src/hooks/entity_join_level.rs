//! The `entity_join_level` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a player's tick on both logical sides (NeoForge 26.3
    /// `PlayerTickEvent.Pre`). Non-cancellable. On the client, fires through death tick 20,
    /// but not after the local player is removed.
    /// `player` identifies the ticked player; on the server their state is
    /// acting. Fires before sleep, cooldowns and body work. Players still tick
    /// under `/tick freeze`, but not while the level is paused.
    /// On the server, neither player tick hook fires for a spectator whose
    /// bounds touch an unloaded chunk; that player's body tick is skipped.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_player_tick_pre",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        neoforge = "PlayerTickEvent.Pre"
    )]
    fn player_tick_pre(player: PlayerRef);
    /// After a player's tick on both logical sides (NeoForge 26.3
    /// `PlayerTickEvent.Post`). Non-cancellable. On the client, fires through death tick 20,
    /// but not after the local player is removed.
    /// `player` is the same identity as in `player_tick_pre`; on the server
    /// their state is acting. Runs after body work and server-side item,
    /// arrow and XP pickups, so reads see the completed tick. Client reads
    /// see only the state available to that client, not future server updates.
    /// On the server, neither player tick hook fires for a spectator whose
    /// bounds touch an unloaded chunk; that player's body tick is skipped.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_player_tick_post",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        neoforge = "PlayerTickEvent.Post"
    )]
    fn player_tick_post(player: PlayerRef);
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back from saved entity storage,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `subject` is the joining entity's own handle
    /// (NeoForge's `getEntity`) and `kind` its type (`minecraft:item`). It is already in the
    /// level's lists, but `Host::entity` finds it only in an accessible section: `subject`
    /// reads it, and the listener may veto it, before it is admitted, even into a hidden
    /// section; discovery through the host is unchanged. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. Tells every
    /// kind in the game's entity lists (items, orbs, mobs, boats, minecarts,
    /// armor stands, primed TNT, falling blocks, projectiles, rockets,
    /// tridents, end crystals, the dragon, cushions, paintings, item frames,
    /// block/item/text displays, markers and interactions),
    /// once each; check `kind`. A drop, an orb, a mob, a `/summon` and a chunk's
    /// entities are told at once. Arrows and projectiles in the same spawn list
    /// (including thrown potions and fireballs) are told at spawn with their owner
    /// already set. Fresh falling blocks are also told at spawn, before
    /// the next scheduled block tick or before `LevelRef::spawn_falling_block` returns. A natural fall's source block
    /// replacement is not undone by a veto. Other spawns are batched in insertion
    /// order between tick phases and a veto removes them before they tick again.
    /// Inside a transaction (`Host::transaction`) opened
    /// in the entity's own level the join is told at that transaction's root
    /// commit, not at the spawn call (an entity spawned in another level, as by
    /// a command run `in` it, is told at once): before the
    /// commit the entity exists and `entity_constructing` has run, but no one
    /// is told it joined, and a listener reads the entity when the commit
    /// runs. An aborted transaction tells nothing, and the entity it spawned
    /// is gone. A join cancelled at the commit leaves the spawn's handle
    /// stale. Joins at one commit come in the order the entities were added,
    /// before the removals the transaction asked for take effect. Join
    /// notifications in the same level requested while a commit is announcing
    /// its joins (from a direct spawn or a committed transaction in that level)
    /// are delivered after that announcing batch, in request order.
    /// Live fallback-store entities also notify: displays, markers, interactions,
    /// and summoned or chunk-restored eyes of ender and explicit leash knots.
    /// Gameplay-thrown eyes and naturally tied leash knots live in separate lists
    /// and do not notify yet. Saved data kept for a missing mod is not an entity in the level
    /// and does not notify. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_join_level",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityJoinLevelEvent",
        version = 5
    )]
    fn entity_join_level(
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
        subject: EntityRef,
        canceled: &mut bool,
    );
}
