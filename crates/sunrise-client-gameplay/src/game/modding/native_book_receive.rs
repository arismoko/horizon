//! A remote catalogue supplies recipe membership, never the client's local recipe JSON.
use std::collections::{BTreeMap, BTreeSet};
use std::result::Result;
use sunrise_gameplay_common::game::native_book_displays::Resources;
use sunrise_sim::recipe_book::{DisplayResources, ModBookIndex, placement_sync as sync};
struct RemoteResources<'a, 'w> {
    inner: Resources<'a, 'w, crate::game::ClientGame<'w>>,
    recipes: BTreeMap<String, String>,
    types: BTreeSet<String>,
}
impl DisplayResources for RemoteResources<'_, '_> {
    fn book(&self, key: &mod_api::RecipeBookTypeKey) -> bool {
        self.inner.book(key)
    }
    fn item(&self, id: &str) -> bool {
        self.inner.item(id)
    }
    fn stack(&self, stack: &mod_api::ItemStack) -> bool {
        self.inner.stack(stack)
    }
    fn tag(&self, id: &str) -> bool {
        self.inner.tag(id)
    }
    fn component(&self, id: &str) -> bool {
        self.inner.component(id)
    }
    fn trim_pattern(&self, id: &str) -> bool {
        self.inner.trim_pattern(id)
    }
    fn menu(&self, id: &str) -> bool {
        self.inner.menu(id)
    }
    fn menu_layout(
        &self,
        id: &str,
    ) -> Option<sunrise_sim::recipe_book::menu_layout::MenuLayoutMetadata> {
        self.inner.menu_layout(id)
    }
    fn recipe_type(&self, id: &str) -> bool {
        self.types.contains(id) || self.inner.recipe_type(id)
    }
    fn recipe(&self, id: &str, kind: &str) -> bool {
        self.recipes.get(id).is_some_and(|value| value == kind)
    }
}
impl crate::game::ClientGame<'_> {
    fn load_remote_mod_book_catalogue(
        &self,
        data: &sync::CatalogBytes,
    ) -> Result<ModBookIndex, sync::Refusal> {
        let declarations = sync::decode_catalogue(&data.bytes)?;
        let mut recipes = BTreeMap::new();
        for category in &declarations {
            if category.target_menu != data.identity.menu.as_str()
                || category.book.as_str() != data.identity.book.as_str()
            {
                return Err(sync::Refusal::Invalid);
            }
            for entry in &category.displays {
                sync::ResourceId::try_from(entry.recipe.clone())
                    .map_err(|_| sync::Refusal::Invalid)?;
                recipes.insert(entry.recipe.clone(), category.recipe_type.clone());
            }
        }
        let types = mod_api::RecipeBookLayout::registered_menus()
            .iter()
            .filter_map(|menu| mod_api::RecipeBookLayout::registered(menu).map(|(_, kind, _)| kind))
            .collect();
        let index = ModBookIndex::build(
            declarations,
            &RemoteResources {
                inner: Resources(self, std::marker::PhantomData),
                recipes,
                types,
            },
        )
        .map_err(|error| {
            log::error!(
                "remote recipe-book catalogue {} refused: {error:?}",
                data.identity.menu.as_str()
            );
            sync::Refusal::Invalid
        })?;
        Ok(index)
    }
    pub fn remote_mod_book_placement(&mut self, frame: sync::Frame) {
        match frame {
            sync::Frame::Ready { epoch, schema } => {
                if self.common.native_net.placement.client.ready(epoch, schema) {
                    self.ensure_mod_book_catalog();
                } else {
                    log::warn!(
                        "[recipe book] ignored mismatched placement ready for epoch {epoch}"
                    );
                }
            }
            sync::Frame::Start {
                identity,
                layout,
                storage,
                total,
            } => {
                if self.native_book_placement_window().as_ref()
                    != self.common.native_net.placement.client.window()
                {
                    log::warn!(
                        "[recipe book] ignored placement catalogue start for inactive window {}",
                        identity.window
                    );
                    return;
                }
                let expected = mod_api::RecipeBookLayout::registered(identity.menu.as_str())
                    .map(|(_, _, layout)| layout);
                let matches = expected.as_ref() == Some(&layout)
                    && Resources(self, std::marker::PhantomData)
                        .menu_layout(identity.menu.as_str())
                        == Some(storage);
                let epoch = identity.epoch;
                let started = self
                    .common
                    .native_net
                    .placement
                    .client
                    .start(identity, layout, total);
                match started {
                    Ok(()) if matches => {}
                    Ok(()) => {
                        self.common.native_net.placement.client.invalidate(epoch);
                        self.common.native_net.placement.view = None;
                        self.reset_mod_book_placement_ui();
                        log::error!(
                            "remote recipe-book catalogue load refused: incompatible menu metadata"
                        );
                    }
                    Err(reason) => {
                        if matches!(reason, sync::Refusal::TooLarge | sync::Refusal::Invalid) {
                            self.common.native_net.placement.client.invalidate(epoch);
                            self.common.native_net.placement.view = None;
                            self.reset_mod_book_placement_ui();
                        }
                        log::warn!("remote recipe-book catalogue start refused: {reason:?}");
                    }
                }
            }
            sync::Frame::Part {
                epoch,
                number,
                offset,
                bytes,
            } => {
                if self.native_book_placement_window().as_ref()
                    != self.common.native_net.placement.client.window()
                {
                    log::warn!(
                        "[recipe book] ignored placement catalogue part for inactive epoch {epoch}"
                    );
                    return;
                }
                let data = match self
                    .common
                    .native_net
                    .placement
                    .client
                    .part(epoch, number, offset, bytes)
                {
                    Ok(Some(data)) => data,
                    Ok(None) => return,
                    Err(reason) => {
                        log::warn!(
                            "[recipe book] ignored placement catalogue part ({epoch}/{number}): {reason:?}"
                        );
                        return;
                    }
                };
                match self.load_remote_mod_book_catalogue(&data) {
                    Ok(index) => {
                        if self
                            .common
                            .native_net
                            .placement
                            .client
                            .accept_catalog(data.identity.clone())
                        {
                            self.common.native_net.placement.view =
                                Some(sunrise_gameplay_common::game::native_book_placement::View {
                                    identity: data.identity,
                                    index,
                                });
                            self.reset_mod_book_placement_ui();
                            self.build_screen();
                        }
                    }
                    Err(reason) => {
                        log::error!(
                            "remote recipe-book catalogue {} load refused: {reason:?}",
                            data.identity.menu.as_str()
                        );
                        self.common.native_net.placement.client.invalidate(epoch);
                        self.common.native_net.placement.view = None;
                        self.reset_mod_book_placement_ui();
                    }
                }
            }
            sync::Frame::Ghost {
                catalog,
                recipe,
                display,
            } => {
                if self.native_book_placement_window().as_ref()
                    != self.common.native_net.placement.client.window()
                    || !self
                        .common
                        .native_net
                        .placement
                        .client
                        .accepts_ghost(&catalog)
                {
                    log::warn!(
                        "[recipe book] ignored stale ghost for {} in window {}",
                        recipe.as_str(),
                        catalog.window
                    );
                    return;
                }
                self.install_mod_book_ghost(&catalog, &recipe, &display);
            }
            sync::Frame::Refused {
                epoch,
                number,
                window,
                nonce,
                reason,
            } => {
                if self
                    .common
                    .native_net
                    .placement
                    .client
                    .refused(epoch, number, window, nonce)
                {
                    self.common.native_net.placement.view = None;
                    self.reset_mod_book_placement_ui();
                    log::error!("mod recipe-book placement load refused: {reason:?}");
                } else {
                    log::warn!(
                        "[recipe book] ignored stale placement refusal ({epoch}/{number}): {reason:?}"
                    );
                }
            }
            sync::Frame::Invalidate { epoch } => {
                if self.common.native_net.placement.client.invalidate(epoch) {
                    self.clear_mod_book_placement_screen();
                    self.ensure_mod_book_catalog();
                } else {
                    log::warn!(
                        "[recipe book] ignored stale placement invalidation for epoch {epoch}"
                    );
                }
            }
            other => {
                log::warn!(
                    "[recipe book] ignored unexpected clientbound placement frame: {other:?}"
                );
            }
        }
    }
}
