//! The `relocability` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether your block `state` at `pos` may be relocated
    /// (`IBlockExtension.getRelocability`): asked at the last step of `PistonBaseBlock.isPushable`, after
    /// the block's push reaction, for a block that is otherwise pushable,
    /// whenever a piston's structure takes the block in. Left `None`, vanilla's
    /// rule holds: a block with a block entity does not move, and any other
    /// block does. `Some(false)` keeps the block in place at that position.
    /// `Some(true)` lets a piston move it even with a block entity, which
    /// travels with the block: taken out of the world as the move starts, with
    /// its saved data and its attachments and without its removal side effects
    /// (nothing drops), and put where the block lands as a new block entity of
    /// that position, with a new ticker slot; a handle to the old position no
    /// longer resolves. If the moving block is replaced before it lands, the
    /// block entity is removed there as a broken block's is (a container drops
    /// its contents). A save mid-move keeps it in the moving piston's tag. A
    /// block entity, including a chest, moves only after its reaction permits movement
    /// and this hook answers true. While the block moves it draws as its state, without its
    /// block entity's drawing. Asked on both sides, as a client replays the
    /// move on its own copy: answer the same on both, from the arguments,
    /// without touching the world. Only attached block parts are asked; blocks
    /// without a subscribed part pay only the dispatch guards. A block that only needs to move or not, or
    /// to be popped or pushed only, is the push reaction's question, asked
    /// before this one. The order is reaction first, then this shared block-entity
    /// rule for push-pull, ignore-entity and push only; a wrong-direction push-only
    /// answer refuses before this hook. Popped destroys and drops without asking it.
    /// This hook is asked only for an otherwise-pushable block. A live owned payload
    /// is required: data-only or unmaterialized vanilla entities remain refused.
    /// Diverges from vanilla: push only never discards a block entity.
    /// Horizon reads NeoForge's answer as `Option<bool>` and the piston as the
    /// only rule it governs: vanilla 26.3's only per-block move rule is
    /// `PistonBaseBlock.isPushable`; `/clone` moves blocks without asking them
    /// and structure templates copy without removing. Diverges from upstream:
    /// new hook.
    #[notes = r#"## Block relocation

- **`BlockPart::relocability(level, event)`** (`RelocabilityEvent { dimension, pos, state }`, answer `Option<bool>`): whether a piston may move your block (NeoForge's `IBlockExtension.getRelocability`, read by Horizon as a yes or no for pistons, the only per-block move rule vanilla 26.3 has). Asked at the last step of `PistonBaseBlock.isPushable`, for a block that got past its push reaction, where vanilla refuses any block with a block entity. `Some(false)` makes the block immovable at that position; `Some(true)` moves it, with its block entity, its data and its attachments when it has one; `None` keeps vanilla's rule: a block with a block entity does not move, any other block follows its push reaction. Asked on both sides with a read-only `LevelSide`, as a client replays a piston's move on its own copy: answer the same on both, from the event. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        contract = "replace",
        out = "movable",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::relocability::GameLevel<'_>::native_relocability",
        neoforge = "IBlockExtension.getRelocability"
    )]
    fn relocability();
}
