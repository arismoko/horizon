//! Prepared cauldron callbacks.
use mod_api::{
    __private::{RetainedCallback, game::discard_registration},
    HookId,
};
pub use std::{cell::RefCell, collections::BTreeSet, sync::Arc};
use sunrise_game_data::Result;
pub use sunrise_items::items::Items;
use sunrise_sim::cauldron::interactions::Dispatcher;

#[derive(Clone)]
pub struct Callback {
    pub handler: Arc<dyn mod_api::Interaction>,
    pub owner: &'static str,
    pub guard: Option<RetainedCallback>,
}
impl Callback {
    pub(in crate::game) fn new(handler: Arc<dyn mod_api::Interaction>) -> Self {
        let owner = mod_api::__private::cauldron_owner();
        let guard = mod_api::__private::mod_index(owner)
            .map(|index| RetainedCallback::new(owner, index, "cauldron_interaction", ""));
        Self {
            handler,
            owner,
            guard,
        }
    }
}
pub type Table = Dispatcher<Callback>;
thread_local! {
    pub(in crate::game) static TABLE: RefCell<Arc<Table>> = RefCell::new(Arc::new(Table::default()));
}
pub fn snapshot() -> Arc<Table> {
    TABLE.with_borrow(Arc::clone)
}
pub fn clear() {
    install(Arc::new(Table::default()));
}
pub fn install(table: Arc<Table>) {
    TABLE.with_borrow_mut(|old| *old = table);
}

pub(in crate::game) fn discard_callback(callback: Callback) {
    discard_registration(
        (
            HookId::RegisterCauldronInteraction,
            "register_cauldron_interaction",
        ),
        Some(callback.owner),
        callback.handler,
    );
}

/// One admitted cauldron registration edit, replayed in registration order at `finish`.
pub(in crate::game) enum Edit {
    Item {
        block: String,
        item: String,
        callback: Callback,
    },
    AllItem {
        item: String,
        callback: Callback,
    },
    Tag {
        block: String,
        tag: String,
        callback: Callback,
    },
    AllTag {
        tag: String,
        callback: Callback,
    },
}
impl Edit {
    pub(in crate::game) fn into_callback(self) -> Callback {
        match self {
            Self::Item { callback, .. }
            | Self::AllItem { callback, .. }
            | Self::Tag { callback, .. }
            | Self::AllTag { callback, .. } => callback,
        }
    }
}
pub struct Registrar<'a> {
    pub(in crate::game) items: &'a Items,
    pub(in crate::game) registry: &'a sunrise_world::registry::BlockRegistry,
    pub(in crate::game) edits: Vec<Edit>,
    pub(in crate::game) error: Option<mod_api::CauldronRegistrationError>,
}
/// A tag as entries keep it: its resource location, without `#`.
pub(in crate::game) fn tag_id(tag: &str) -> Option<String> {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    mod_api::Identifier::parse(tag)
        .ok()
        .map(|id| id.to_string())
}
impl<'a> Registrar<'a> {
    pub fn new(items: &'a Items, registry: &'a sunrise_world::registry::BlockRegistry) -> Self {
        Self {
            items,
            registry,
            edits: Vec::new(),
            error: None,
        }
    }

    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn mod_api::CauldronInteractions) -> bool,
    ) {
        let mut pending = Registrar::new(self.items, self.registry);
        if listen(&mut pending) {
            let Registrar { edits, error, .. } = pending;
            self.edits.extend(edits);
            if let Some(error) = error {
                self.error = Some(error);
            }
        } else {
            for edit in pending.edits {
                discard_callback(edit.into_callback());
            }
        }
    }

    /// Record an admission error, which fails the whole preparation when it finishes.
    pub(in crate::game) fn refuse(
        &mut self,
        error: mod_api::CauldronRegistrationError,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        self.error = Some(error);
        Err(error)
    }
    pub fn finish(self) -> Result<Arc<Table>> {
        let Self { edits, error, .. } = self;
        if let Some(error) = error {
            for edit in edits {
                discard_callback(edit.into_callback());
            }
            return Err(format!("cauldron registration: {error:?}").into());
        }
        let mut table = Table::default();
        let mut blocks = table.blocks();
        let mut extra = BTreeSet::new();
        for edit in &edits {
            if let Edit::Item { block, .. } | Edit::Tag { block, .. } = edit
                && !table.contains_block(block)
            {
                extra.insert(block.clone());
            }
        }
        blocks.extend(extra);
        for edit in edits {
            match edit {
                Edit::Item {
                    block,
                    item,
                    callback,
                } => {
                    if let Some(old) = table.register(block, item, callback) {
                        discard_callback(old);
                    }
                }
                Edit::Tag {
                    block,
                    tag,
                    callback,
                } => {
                    if let Some(old) = table.register_tag(block, tag, callback) {
                        discard_callback(old);
                    }
                }
                Edit::AllItem { item, callback } => {
                    for block in &blocks {
                        if let Some(old) =
                            table.register(block.clone(), item.clone(), callback.clone())
                        {
                            discard_callback(old);
                        }
                    }
                    discard_callback(callback);
                }
                Edit::AllTag { tag, callback } => {
                    for block in &blocks {
                        if let Some(old) =
                            table.register_tag(block.clone(), tag.clone(), callback.clone())
                        {
                            discard_callback(old);
                        }
                    }
                    discard_callback(callback);
                }
            }
        }
        Ok(Arc::new(table))
    }
}
impl mod_api::CauldronInteractions for Registrar<'_> {
    fn register(
        &mut self,
        block: mod_api::BlockKey,
        item: mod_api::ItemKey,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        use mod_api::CauldronRegistrationError as Error;
        let Some(block) = mod_api::__private::block_key_id(block)
            .filter(|id| self.registry.default_state(id).is_some())
        else {
            return self.refuse(Error::UnknownBlock);
        };
        let Some(item) =
            mod_api::__private::item_key_id(item).filter(|id| self.items.items.contains_key(*id))
        else {
            return self.refuse(Error::UnknownItem);
        };
        self.edits.push(Edit::Item {
            block: block.to_string(),
            item: item.to_string(),
            callback: Callback::new(handler),
        });
        Ok(())
    }
    fn register_to_all(
        &mut self,
        item: mod_api::ItemKey,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let Some(item) =
            mod_api::__private::item_key_id(item).filter(|id| self.items.items.contains_key(*id))
        else {
            return self.refuse(mod_api::CauldronRegistrationError::UnknownItem);
        };
        self.edits.push(Edit::AllItem {
            item: item.to_string(),
            callback: Callback::new(handler),
        });
        Ok(())
    }
    fn register_tag(
        &mut self,
        block: mod_api::BlockKey,
        tag: &str,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        use mod_api::CauldronRegistrationError as Error;
        let Some(block) = mod_api::__private::block_key_id(block)
            .filter(|id| self.registry.default_state(id).is_some())
        else {
            return self.refuse(Error::UnknownBlock);
        };
        let Some(tag) = tag_id(tag) else {
            return self.refuse(Error::InvalidTag);
        };
        self.edits.push(Edit::Tag {
            block: block.to_string(),
            tag,
            callback: Callback::new(handler),
        });
        Ok(())
    }
    fn register_tag_to_all(
        &mut self,
        tag: &str,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let Some(tag) = tag_id(tag) else {
            return self.refuse(mod_api::CauldronRegistrationError::InvalidTag);
        };
        self.edits.push(Edit::AllTag {
            tag,
            callback: Callback::new(handler),
        });
        Ok(())
    }
}
