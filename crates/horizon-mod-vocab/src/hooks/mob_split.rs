//! The `mob_split` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A slime, magma cube or sulfur cube is dying and about to split into
    /// smaller ones (NeoForge's `MobSplitEvent`, vanilla's
    /// `AbstractCubeMob.remove`): `dimension` is the level it dies in (a
    /// `PendingMobSpec` for a new child names it), `parent` its entity id, `kind` its type,
    /// `size` its size and `child_size` the size of the children (half of
    /// it). The children vanilla made (2 to 4 for a slime or magma cube, 2
    /// for a sulfur cube; a primed one makes none and isn't asked) are
    /// already constructed, in order, and wait in an engine-owned list that
    /// `edits.children()` names and `edits` changes: read it with `edits.len` and
    /// `edits.child`, then edit a child (`edits.set_position`,
    /// `edits.set_yaw`, `edits.set_baby`, `edits.set_size`), replace one
    /// (`edits.replace`), remove one (`edits.remove`) or append a new one
    /// (`edits.construct`, then `edits.append`; at most 64 children). Each
    /// child is a complete mob that is in no level yet: it has no entity
    /// number, no join and nothing a player or another mod can see, and
    /// every handle (`edits.children()` and the children `edits.child` gave) is
    /// borrowed for one listener only; kept past it, or used by a later listener, it answers `Expired`.
    /// Set `cancel` (or leave the list empty) to spawn no children: the
    /// children's construction draws stay spent and the parent still dies.
    /// What the list holds when the hook returns is spawned once, in order,
    /// through the usual join. A listener that changes the world in a way that
    /// stops the split from being valid, or a list that ends up invalid,
    /// spawns nothing. Only for a parent above size 1, once its children are
    /// made. With no listener nothing is allocated for the list. On the
    /// server.
    #[notes = r#"## Mob split

- **`mob_split(dimension, parent, kind, size, child_size, edits, cancel)`** (NeoForge's `MobSplitEvent`): fires when a slime, magma cube or sulfur cube above size 1 dies and splits, after vanilla has constructed its children (`AbstractCubeMob.remove`: the count, then each half-size child and the parent's yaw draw for it). `edits` (`&dyn PendingChildrenEdits`) changes an engine-owned, ordered list of those children: `edits.children()` returns its lease (`PendingChildrenRef`), which every operation takes and which lasts one listener. See [Design: pending children](../concepts/pending-children.md) for the ownership rules shared with `baby_entity_spawn`.
- The editor offers `len`, `child`, `construct` (a `PendingMobSpec`, returning an `UnattachedMob`), `append`, `replace`, `remove` and, per child, `set_position`, `set_yaw`, `set_baby` and `set_size`. A child is read with the `UnspawnedMobRef` it gives through `UnspawnedMobEdits`-style reads (`mob.kind(host)`, `mob.position(host)`). Every handle is valid for one listener only; a retained one, or one used by the next listener, answers `Expired`, a token already appended answers `Ownership`, a 65th child answers `Capacity`, and an unknown kind, a non-finite position or an invalid size answers `Invalid`. A refused call changes nothing.
- Cancelling, or removing every child, spawns none. The construction draws stay spent and the parent still dies. Listeners see the earlier listeners' edits.
- Not fired for a parent of size 1, nor for a primed sulfur cube (it makes no children). With no listener the split runs exactly as vanilla and allocates nothing for the list."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "MobSplitEvent",
        paper = "SlimeSplitEvent",
        cancellable = true
    )]
    fn mob_split(
        dimension: &str,
        parent: u32,
        kind: &str,
        size: i32,
        child_size: i32,
        edits: &dyn PendingChildrenEdits,
        cancel: &mut bool,
    );
}
