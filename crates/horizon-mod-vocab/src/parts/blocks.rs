use super::*;
/// Whether this hook has a prepared subscriber for an API block key.
/// The caller owns validation of the routing key.
pub fn block_key_listens(hook: HookId, key: crate::BlockKey) -> bool {
    parts(hook, "block routing")
        .for_hook(hook)
        .is_some_and(|table| !table.block_key(Some(key)).is_empty())
}

impl HookParts {
    /// Prepared subscribers for an API block key; missing keys have no subscribers.
    pub fn block_key(&self, key: Option<crate::BlockKey>) -> &[Attached<dyn BlockPart>] {
        key.and_then(|key| self.block_keys.get(&key))
            .map_or(&[], |parts| parts.as_slice())
    }
    /// Parts for a checked API state. Key lookup does not depend on world state numbers.
    pub fn block_value(&self, state: crate::BlockState) -> &[Attached<dyn BlockPart>] {
        let Some(blocks) = crate::__private::block_definitions() else {
            return &[];
        };
        let Some(definition) = blocks.definitions.get(state.block().0 as usize) else {
            return &[];
        };
        if definition.property(state.ordinal(), "").is_err() {
            return &[];
        }
        self.block_key(Some(state.block()))
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to block `block` (`mymod:shaft`, `minecraft:oak_log`).
    pub fn block_part<P: BlockPart>(&mut self, block: crate::BlockKey, part: P) {
        self.attach_block_part(block, type_name::<P>(), Arc::new(part));
    }
    /// Attach `part` to every block in tag `tag` (`minecraft:logs`, without `#`), as each game's
    /// tags have it when its world opens and after `/reload`.
    pub fn block_part_tag<P: BlockPart>(&mut self, tag: &str, part: P) {
        self.attach_block_part_tag(tag, type_name::<P>(), Arc::new(part));
    }
}

/// Registrar operations for blocks.
pub trait BlocksRegistrar {
    /// Request a block by canonical registry id. All mods' declarations are checked at load.
    fn block_key(&mut self, id: &str) -> crate::BlockKey;

    /// Attach `part`, of kind `kind` (its type's name), to block `block`.
    fn attach_block_part(
        &mut self,
        block: crate::BlockKey,
        kind: &'static str,
        part: Arc<dyn BlockPart>,
    );

    /// Attach `part`, of kind `kind`, to every block in tag `tag`.
    fn attach_block_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn BlockPart>);
}
