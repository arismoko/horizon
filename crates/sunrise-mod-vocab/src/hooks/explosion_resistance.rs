//! The `explosion_resistance` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, the explosion resistance of the block `state` at `pos`
    /// against `explosion` (`IBlockExtension.getExplosionResistance`), asked as a ray of the
    /// blast enters the block (`ExplosionDamageCalculator.getBlockExplosionResistance`). The
    /// answer replaces only the block's own value (`BlockProperties.explosion_resistance`);
    /// vanilla combines it with the cell's fluid via `max` (`0.0` for empty fluid, `100.0` for
    /// water or lava). What vanilla applies after the block's value keeps winning: a respawn
    /// anchor's water cell, a TNT minecart's rails and the blocks under them (never asked) and
    /// a dangerous wither skull's 0.8 cap. Any finite answer is accepted (negative answers
    /// combine with the cell's fluid resistance via `max`); a non-finite answer is refused and
    /// the block's own value is used; a panicking part is contained and the block's own value
    /// is used. A ray asks once per block it enters, so keep the answer a pure function of the
    /// inputs. The level is read-only. With no part attached to the state, the cost is one
    /// table lookup. Diverges from upstream: new hook.
    #[notes = r#"## Block explosion resistance

- **`BlockPart::explosion_resistance(level, event)`** (`ExplosionResistanceEvent { pos, state, explosion }`): answer `Some(resistance)` to replace the block's own explosion resistance (NeoForge's `IBlockExtension.getExplosionResistance`) against the blast `event.explosion` describes, the same `ExplosionStart` the global `explosion_start` hears. A ray of the blast asks as it enters the block; its next steps in the same block reuse the answer, so keep it a pure function of the inputs. The answer replaces only the block's static value: vanilla combines it with the cell's fluid via `max` (`0.0` for empty fluid, `100.0` for water or lava), and the overrides vanilla applies after the block's value keep winning (a respawn anchor's water cell, a TNT minecart's rails and the blocks under them, which are never asked, and a dangerous wither skull's 0.8 cap on a block that is not `#wither_immune`). `None` leaves it to another part and then the block's value. Any finite answer is accepted; a non-finite answer is refused with a warning, and a panicking part is contained; the block's own value is used for both. Authoritative servers only; the level is read-only. With no part attached to the state, one table lookup. Diverges from upstream: new hook."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "resistance",
        scope = "block",
        key = "state",
        version = 3,
        target = "sunrise_gameplay_common::game::native_hooks::explosion_resistance::GameLevel<'_>::native_explosion_resistance",
        neoforge = "IBlockExtension.getExplosionResistance"
    )]
    fn explosion_resistance(
        pos: BlockPos,
        state: BlockState,
        explosion: &ExplosionStart<'_>,
        resistance: &mut Option<f32>,
    );
}
