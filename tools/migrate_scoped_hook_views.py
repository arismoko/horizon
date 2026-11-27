#!/usr/bin/env python3
"""Convert the 7 remaining resident subject-scoped hooks to item 1's scoped-view pattern.

Run once from the pre-conversion tree after the scoped-view vocab types and
generator scope-routing arms are in place. Rewrites the 7 hook declarations in
`crates/sunrise-mod-vocab/src/hooks/` and removes the legacy `ClientSnapshot`
cache in `crates/sunrise-level/src/block_entities.rs`.
"""
from pathlib import Path

HOOKS = Path("crates/sunrise-mod-vocab/src/hooks")
BLOCK_ENTITIES = Path("crates/sunrise-level/src/block_entities.rs")


def rewrite(path: Path, transform) -> None:
    old = path.read_text()
    new = transform(old)
    if old == new:
        raise ValueError(f"no migration applied to {path}")
    path.write_text(new)


def cut(text: str, start: str, end: str, replacement: str = "") -> str:
    i = text.index(start)
    j = text.index(end, i + len(start))
    return text[:i] + replacement + text[j:]


def replace_exact(text: str, old: str, new: str) -> str:
    if old not in text:
        raise ValueError(f"expected snippet not found: {old[:80]!r}")
    return text.replace(old, new, 1)


def main() -> None:
    # 1. block_entity_client_tick (v1 -> v2)
    rewrite(
        HOOKS / "block_entity_client_tick.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntityTicker from EntityBlock.getTicker with the client level; NeoForge adds no event)"
    )]
    fn block_entity_client_tick(
        pos: [i32; 3],
        block_entity: BlockEntityRef,
        state: BlockState,
        entity: &ModBlockEntity,
    );''',
            '''    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "subject",
        neoforge = "none (vanilla BlockEntityTicker from EntityBlock.getTicker with the client level; NeoForge adds no event)",
        version = 2
    )]
    fn block_entity_client_tick(subject: mod_api::BlockEntityClientScope<'_>);''',
        ),
    )

    # 2. block_entity_data_received (v1 -> v2)
    rewrite(
        HOOKS / "block_entity_data_received.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (post-merge notice; IBlockEntityExtension.onDataPacket and handleUpdateTag are related loading paths, not counterparts)"
    )]
    fn block_entity_data_received(
        entity: BlockEntityRef,
        source: BlockEntityDataSource,
        tag: &sunrise_core::nbt::Compound,
    );''',
            '''    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "subject",
        neoforge = "none (post-merge notice; IBlockEntityExtension.onDataPacket and handleUpdateTag are related loading paths, not counterparts)",
        version = 2
    )]
    fn block_entity_data_received(
        subject: mod_api::BlockEntityClientReadScope<'_>,
        source: BlockEntityDataSource,
        tag: &sunrise_core::nbt::Compound,
    );''',
        ),
    )

    # 3. block_entity_on_load (v1 -> v2)
    rewrite(
        HOOKS / "block_entity_on_load.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        scope = "block_entity",
        key = "id",
        neoforge = "IBlockEntityExtension.onLoad"
    )]
    fn block_entity_on_load(id: BlockEntityTypeKey, entity: BlockEntityRef);''',
            '''    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        scope = "block_entity",
        key = "id",
        neoforge = "IBlockEntityExtension.onLoad",
        version = 2
    )]
    fn block_entity_on_load(
        id: BlockEntityTypeKey,
        subject: mod_api::BlockEntitySideScope<'_>,
    );''',
        ),
    )

    # 4. block_entity_load_additional (v2 -> v3)
    rewrite(
        HOOKS / "block_entity_load_additional.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        event_per_listener = true,
        scope = "block_entity",
        key = "id",
        target = "sunrise_client::game::native_hooks::block_entity_load_additional::GameLevel<'_>::native_block_entity_load_additional",
        neoforge = "none (vanilla BlockEntity.loadAdditional; NeoForge adds no event for this moment)",
        version = 2
    )]
    fn block_entity_load_additional(
        id: mod_api::BlockEntityTypeKey,
        entity: BlockEntityRef,
        pos: [i32; 3],
        level: LevelRef,
        tag: &Compound,
    );''',
            '''    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        event_per_listener = true,
        scope = "block_entity",
        key = "id",
        target = "sunrise_client::game::native_hooks::block_entity_load_additional::GameLevel<'_>::native_block_entity_load_additional",
        neoforge = "none (vanilla BlockEntity.loadAdditional; NeoForge adds no event for this moment)",
        version = 3
    )]
    fn block_entity_load_additional(
        id: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityLevelReadScope<'_>,
        tag: &Compound,
    );''',
        ),
    )

    # 5. general_entity_tick (v1 -> v2)
    rewrite(
        HOOKS / "general_entity_tick.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "kind",
        neoforge = "none (vanilla Entity.tick has no part-scoped counterpart; NeoForge's EntityTickEvent is the global pre/post pair)"
    )]
    fn general_entity_tick(kind: &str, entity: mod_api::EntityRef);''',
            '''    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "kind",
        neoforge = "none (vanilla Entity.tick has no part-scoped counterpart; NeoForge's EntityTickEvent is the global pre/post pair)",
        version = 2
    )]
    fn general_entity_tick(kind: &str, subject: mod_api::EntityServerScope<'_>);''',
        ),
    )

    # 6. can_rider_interact (v1 -> v2)
    rewrite(
        HOOKS / "can_rider_interact.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        target = "sunrise_client::game::native_hooks::client_pick::Game<'_>::native_can_rider_interact",
        dispatch = "api_carrier",
        side = "client",
        contract = "collect",
        out = "allowed",
        scope = "entity",
        key = "entity",
        neoforge = "IEntityExtension.canRiderInteract"
    )]
    fn can_rider_interact(entity: EntityRef, allowed: &mut bool);''',
            '''    #[hook(
        target = "sunrise_client::game::native_hooks::client_pick::Game<'_>::native_can_rider_interact",
        receiver = "associated",
        dispatch = "api_carrier",
        side = "client",
        contract = "collect",
        out = "allowed",
        scope = "entity",
        key = "subject",
        neoforge = "IEntityExtension.canRiderInteract",
        version = 2
    )]
    fn can_rider_interact(subject: mod_api::EntityClientScope<'_>, allowed: &mut bool);''',
        ),
    )

    # 7. on_entity_item_update (v1 -> v2)
    rewrite(
        HOOKS / "on_entity_item_update.rs",
        lambda s: replace_exact(
            s,
            '''    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onEntityItemUpdate"
    )]
    fn on_entity_item_update(stack: &ItemStack, item: ItemEntityRef, canceled: &mut bool);''',
            '''    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onEntityItemUpdate",
        version = 2
    )]
    fn on_entity_item_update(
        stack: &ItemStack,
        subject: mod_api::ItemEntityServerScope<'_>,
        canceled: &mut bool,
    );''',
        ),
    )

    # Drop legacy ClientSnapshot cache from sunrise-client-render/src/block_entities.rs.
    rewrite(
        BLOCK_ENTITIES,
        lambda s: replace_exact(
            cut(
                cut(
                    cut(
                        s,
                        "    /// The API snapshots a client's mod block entities are ticked with\n",
                        "}\n",
                    ),
                    "/// A client copy of a mod block entity as its client tick shows it to mods:",
                    "/// Storage-owned identity passed to attachment residency observers.",
                ),
                "    /// The client tick snapshots, lent for one pass over the tickers:",
                "    /// Whether the ticking table has been built.",
            ),
            "        self.client_snapshots.clear();\n",
            "",
        ),
    )


if __name__ == "__main__":
    main()
