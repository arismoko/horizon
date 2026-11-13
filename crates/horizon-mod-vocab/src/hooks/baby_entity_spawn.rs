//! The `baby_entity_spawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Two animals bred and the baby they made is about to join the world
    /// (NeoForge's `BabyEntitySpawnEvent`, vanilla's
    /// `Animal.spawnChildFromBreeding`): `dimension` is the level they are in
    /// (a replacement baby's `PendingMobSpec` names it), `parent` and `partner` are entity
    /// ids, `player` and `player_entity_id` the UUID and entity id of the
    /// player who fed them into love (`getCausedByPlayer`, None when none).
    /// The baby (a mule for a horse and a donkey) is already constructed,
    /// with its inheritance drawn, but not finalized or in any level: it is
    /// the single entry of the engine-owned list `edits.children()` names, which
    /// `edits` changes. Edit the baby in place (`edits.set_position`,
    /// `edits.set_yaw`), replace it with another baby (`edits.construct` a
    /// baby `PendingMobSpec`, then `edits.replace`), or remove it (`edits.remove`)
    /// for none. A second baby is refused with `Capacity`, and a replacement
    /// must be a baby. Set `cancel` for no baby either: the parents still rest and lose their love (their
    /// ages were already set to 6000), but there is no baby, no experience, no
    /// `animals_bred` statistic, no advancement and no heart particles. The
    /// inheritance draws the construction made stay spent. Every handle is
    /// borrowed for one listener only. Not fired for turtles, frogs and sniffers
    /// (an egg, no baby), or their hatching; armadillos take the ordinary animal path and are
    /// included. Villagers do not post this animal-only event, matching NeoForge's documented `Animal.spawnChildFromBreeding` posting site; a rejected villager join instead leaves the child's bed free. On the server.
    #[notes = r#"## Baby entity spawn

- **`baby_entity_spawn(dimension, parent, partner, player, player_entity_id, edits, cancel)`** (NeoForge's `BabyEntitySpawnEvent`): fires after breeding has constructed the baby (`Animal.spawnChildFromBreeding`: `getBreedOffspring`, baby flag and position) and before breeding is finalized and the baby inserted. `edits` (`&dyn PendingChildrenEdits`) changes a list of at most one engine-owned, fully constructed baby, whose lease (`PendingChildrenRef`) `edits.children()` returns; see [Design: pending children](../concepts/pending-children.md). Replace the baby with `edits.construct(children, PendingMobSpec::new(kind, dimension, position, yaw, true))` then `edits.replace(children, 0, baby)` (with `children = edits.children()?`); remove it with `edits.remove`. A second baby answers `Capacity` and a non-baby replacement `Invalid`; each handle is valid for one listener only.
- Cancelling, or removing the baby, breeds no child: both parents keep age 6000 and love 0, and nothing awards experience, the `animals_bred` statistic, an advancement or hearts. The inheritance draws already made stay spent. A replacement is inserted exactly once at the original insertion point, and breeding's own finalization (credit, statistics, experience, the parent's cooldown) reads it as the child.
- Turtle and frog breeding, sniffer egg laying and turtle hatching make eggs, not babies, and do not fire it. Armadillos breed through the ordinary animal path and do. Villagers breed through their own behavior and do not (a separate moment)."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BabyEntitySpawnEvent",
        cancellable = true
    )]
    fn baby_entity_spawn(
        dimension: &str,
        parent: u32,
        partner: u32,
        player: Option<u128>,
        player_entity_id: Option<i32>,
        edits: &dyn PendingChildrenEdits,
        cancel: &mut bool,
    );
}
