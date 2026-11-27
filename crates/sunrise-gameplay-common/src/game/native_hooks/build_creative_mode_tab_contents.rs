//! One demand-driven creative catalog per logical side, and the scoped edit event.
use super::*;
use crate::game::api_values::{
    TryIntoApi, TryIntoEngine, project_stack, stack_read_metadata, stack_value,
};
use mod_api::{
    CreativeModeTabInfo, CreativeModeTabs, CreativeTabEditError, CreativeTabList,
    CreativeTabOutput, CreativeTabPermissions, CreativeTabPlacement, CreativeTabReadError,
    CreativeTabVisibility, ItemStackView,
};
use std::cell::OnceCell;
use sunrise_items::creative_catalog::{
    Catalog, Holders, Kind, Parameters, TabContents, normalize_components,
};

use sunrise_items::items::Items;

pub(in crate::game) type Contents = Arc<[Arc<TabContents>]>;

pub(in crate::game) struct BuildKey {
    pub(in crate::game) features: Arc<[String]>,
    pub(in crate::game) permissions: CreativeTabPermissions,
}

impl BuildKey {
    pub(in crate::game) fn matches(&self, other: &Self) -> bool {
        self.permissions == other.permissions
            && (self.features == other.features
                || (other
                    .features
                    .iter()
                    .all(|flag| self.features.contains(flag))
                    && self
                        .features
                        .iter()
                        .all(|flag| other.features.contains(flag))))
    }
}

pub struct Published {
    pub(in crate::game) key: BuildKey,
    pub lists: Contents,
    pub(in crate::game) api: OnceCell<Arc<CreativeModeTabs>>,
    pub holders: Arc<Holders>,
    pub items: Arc<Items>,
}

pub(in crate::game) struct ActiveBuild {
    pub(in crate::game) key: BuildKey,
    pub(in crate::game) partial: Contents,
    pub(in crate::game) items: Arc<Items>,
}

/// Replaced with the holder provider, never with tags or presentation. Thus the
/// cache key need not carry a registry pointer that changes on a tag-only update.
#[derive(Default)]
pub struct CreativeCache {
    pub published: Option<Published>,
    pub(in crate::game) active: Option<ActiveBuild>,
}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Lend the live tab output after the generator and before feature filtering.
    fn native_build_creative_mode_tab_contents(
        &mut self,
        tab: &str,
        features: &[String],
        permissions: mod_api::CreativeTabPermissions,
        output: &mut dyn mod_api::CreativeTabOutput,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_build_creative_mode_tab_contents(
            self,
            tab,
            features,
            permissions,
            output,
        );
    }

    fn creative_permissions(&self) -> CreativeTabPermissions {
        if self.can_use_game_master_blocks()
            && crate::game::Side::operator_items_tab(self.side_state())
        {
            CreativeTabPermissions::Operator
        } else {
            CreativeTabPermissions::Normal
        }
    }

    fn build_creative(
        &mut self,
        permissions: CreativeTabPermissions,
    ) -> Result<Contents, CreativeTabReadError> {
        // A connection may exist in configuration without a live world provider.
        if self.native_lifecycle.epoch().is_none() {
            return Err(CreativeTabReadError::Unavailable);
        }
        let key = BuildKey {
            features: self.enabled_feature_ids().clone(),
            permissions,
        };
        if let Some(active) = &self.creative_contents.active {
            if active.key.matches(&key) {
                return Ok(active.partial.clone());
            }
            return Err(CreativeTabReadError::Reentrant);
        }
        if let Some(published) = &self.creative_contents.published
            && published.key.matches(&key)
        {
            return Ok(published.lists.clone());
        }
        let holders = self
            .side_creative_holders()
            .ok_or(CreativeTabReadError::Unavailable)?;
        let items = self.shared_items();
        let catalog = self.side_creative_catalog().clone();
        let prior = self
            .creative_contents
            .published
            .as_ref()
            .map(|p| p.lists.clone());
        let listen = mod_api::__private::listened(mod_api::HookId::BuildCreativeModeTabContents);
        self.creative_contents.active = Some(ActiveBuild {
            key: BuildKey {
                features: key.features.clone(),
                permissions,
            },
            partial: Arc::from([]),
            items: items.clone(),
        });
        let lists = catalog.build(
            Parameters {
                features: &key.features,
                operator: permissions == CreativeTabPermissions::Operator,
                holders: &holders,
                items: &items,
            },
            prior.as_deref(),
            |tab, partial, current| {
                if listen {
                    let active = self
                        .creative_contents
                        .active
                        .as_mut()
                        .expect("active creative build");
                    active.partial = Arc::from(partial);
                    self.native_build_creative_mode_tab_contents(
                        &tab.id,
                        &key.features,
                        permissions,
                        &mut CreativeTabEditor {
                            contents: current,
                            items: &items,
                        },
                    );
                }
            },
        );
        self.creative_contents.active = None;
        // The platform reads the same search-tab order (after mods' edits) the sorter ranks by, refreshed with each build.
        self.side_publish_creative_order(&lists);
        self.creative_contents.published = Some(Published {
            key,
            lists: lists.clone(),
            api: OnceCell::new(),
            holders: Arc::new(holders),
            items,
        });
        Ok(lists)
    }

    /// The last completed build's catalog, for a reader that cannot build: unavailable outside
    /// a live world's phase, and `None` before the side has built once.
    fn built_creative_tabs(&self) -> Result<Option<Arc<CreativeModeTabs>>, CreativeTabReadError> {
        if self.native_lifecycle.epoch().is_none() {
            return Err(CreativeTabReadError::Unavailable);
        }
        Ok(self.creative_contents.published.as_ref().map(|published| {
            published
                .api
                .get_or_init(|| {
                    snapshot(
                        self.side_creative_catalog(),
                        published.key.permissions,
                        &published.lists,
                    )
                })
                .clone()
        }))
    }

    /// The search tab's canonical display list (not the client-only UI additions).
    fn creative_search_items(&mut self) -> Vec<ItemStack> {
        if self.native_lifecycle.epoch().is_none() {
            return Vec::new();
        }
        let Ok(lists) = self.build_creative(self.creative_permissions()) else {
            return Vec::new();
        };
        self.side_creative_catalog().search_display(&lists).to_vec()
    }

    /// Read the published order or preview the stock lists without firing callbacks.
    fn stock_creative_lists(&self) -> Arc<[Arc<TabContents>]> {
        if let Some(published) = &self.creative_contents.published {
            return published.lists.clone();
        }
        self.side_creative_catalog().build(
            Parameters {
                features: self.enabled_feature_ids(),
                operator: false,
                holders: &self.side_creative_holders().unwrap_or_default(),
                items: self.items(),
            },
            None,
            |_, _, _| {},
        )
    }

    fn creative_stack_key(&self, stack: &ItemStack) -> Option<ItemStack> {
        self.native_lifecycle.epoch()?;
        // Use the very same prototypes as the generator during a reentrant read.
        let items = self
            .creative_contents
            .active
            .as_ref()
            .map_or_else(|| self.items(), |active| &active.items);
        comparison_key(stack.clone(), items).ok()
    }

    fn creative_tabs(
        &mut self,
        permissions: CreativeTabPermissions,
    ) -> Result<Arc<CreativeModeTabs>, CreativeTabReadError> {
        if self.native_lifecycle.epoch().is_none() {
            return Err(CreativeTabReadError::Unavailable);
        }
        let lists = self.build_creative(permissions)?;
        let catalog = self.side_creative_catalog();
        if self.creative_contents.active.is_some() {
            return Ok(snapshot(catalog, permissions, &lists));
        }
        let published = self
            .creative_contents
            .published
            .as_ref()
            .expect("complete creative build");
        Ok(published
            .api
            .get_or_init(|| snapshot(catalog, permissions, &lists))
            .clone())
    }

    fn reset_creative_contents(&mut self) {
        self.creative_contents = CreativeCache::default();
        self.side_reset_creative_screen();
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

struct CreativeTabEditor<'a> {
    contents: &'a mut TabContents,
    items: &'a Items,
}

impl CreativeTabOutput for CreativeTabEditor<'_> {
    fn entries(
        &self,
        list: CreativeTabList,
        visit: &mut dyn FnMut(ItemStackView<'_>),
    ) -> Result<(), CreativeTabEditError> {
        let entries = match list {
            CreativeTabList::Parent => &self.contents.display,
            CreativeTabList::Search => &self.contents.search,
        };
        entries
            .iter()
            .try_for_each(|stack| lend_entry(stack, &mut *visit))
    }

    fn insert(
        &mut self,
        placement: CreativeTabPlacement,
        stack: mod_api::ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError> {
        if stack.count() != 1 {
            return Err(report_edit_refusal(CreativeTabEditError::Count));
        }
        let mut stack = stack
            .try_into_engine(self.items)
            .map_err(report_edit_refusal)?;
        normalize_components(&mut stack, self.items);
        let anchor = match &placement {
            CreativeTabPlacement::Before(anchor) | CreativeTabPlacement::After(anchor) => {
                let anchor = stack_value(anchor)
                    .and_then(|anchor| comparison_key(anchor, self.items))
                    .map_err(report_edit_refusal)?;
                Some(anchor)
            }
            _ => None,
        };
        for &list in target_lists(visibility) {
            let entries = list_items(self.contents, list);
            let index = match &anchor {
                Some(anchor) => {
                    entries
                        .iter()
                        .position(|entry| entry.same_item_same_components(anchor))
                        .ok_or_else(|| {
                            report_edit_refusal(CreativeTabEditError::MissingAnchor(list))
                        })?
                        + usize::from(matches!(placement, CreativeTabPlacement::After(_)))
                }
                None if matches!(placement, CreativeTabPlacement::First) => 0,
                None => entries.len(),
            };
            if entries
                .iter()
                .any(|entry| entry.same_item_same_components(&stack))
            {
                return Err(report_edit_refusal(CreativeTabEditError::Duplicate(list)));
            }
            entries.insert(index, stack.clone());
        }
        Ok(())
    }

    fn remove(
        &mut self,
        stack: &mod_api::ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError> {
        let stack = stack_value(stack)
            .and_then(|stack| comparison_key(stack, self.items))
            .map_err(report_edit_refusal)?;
        for &list in target_lists(visibility) {
            list_items(self.contents, list)
                .retain(|entry| !entry.same_item_same_components(&stack));
        }
        Ok(())
    }

    fn remove_if(
        &mut self,
        visibility: CreativeTabVisibility,
        predicate: &mut dyn FnMut(ItemStackView<'_>) -> bool,
    ) -> Result<(), CreativeTabEditError> {
        for &list in target_lists(visibility) {
            let mut result = Ok(());
            list_items(self.contents, list).retain(|entry| {
                if result.is_ok() {
                    match lend_entry(entry, &mut *predicate) {
                        Ok(remove) => return !remove,
                        Err(error) => result = Err(error),
                    }
                }
                true
            });
            result?;
        }
        Ok(())
    }
}

fn lend_entry<R>(
    stack: &ItemStack,
    visit: impl FnOnce(ItemStackView<'_>) -> R,
) -> Result<R, CreativeTabEditError> {
    let metadata = stack_read_metadata(stack)
        .map_err(|error| report_edit_refusal(error.into_write_error()))?;
    let project = |metadata| {
        project_stack(stack, metadata)
            .map_err(|error| report_edit_refusal(error.into_write_error()))
            .ok()
    };
    Ok(visit(ItemStackView::resident(metadata, &project)))
}

fn report_edit_refusal(error: impl Into<CreativeTabEditError>) -> CreativeTabEditError {
    let error = error.into();
    let owner = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
    log::warn!("mod {owner} hook build_creative_mode_tab_contents edit refused: {error:?}");
    error
}

pub(in crate::game) fn api_stacks(stacks: &[ItemStack]) -> Vec<mod_api::ItemStack> {
    stacks
        .iter()
        .filter_map(|item| item.try_into_api().ok())
        .collect()
}

pub fn snapshot(
    catalog: &Catalog,
    permissions: CreativeTabPermissions,
    lists: &[Arc<TabContents>],
) -> Arc<CreativeModeTabs> {
    let tabs = catalog
        .tabs()
        .iter()
        .zip(lists)
        .map(|(tab, stacks)| {
            CreativeModeTabInfo::new(
                tab.id.clone(),
                tab.kind == Kind::Category,
                display_name(&tab.id, &tab.title),
                api_stacks(&stacks.display),
                api_stacks(&stacks.search),
            )
        })
        .collect();
    Arc::new(CreativeModeTabs::new(permissions, tabs))
}

/// `CreativeModeTab.getDisplayName`: the tab's `itemGroup.*` key with its title as fallback.
pub fn display_name(id: &str, title: &str) -> mod_api::Text {
    let key = sunrise_items::creative_catalog::item_group_key(id).map_or_else(
        || sunrise_items::creative_catalog::mod_item_group_key(id),
        str::to_owned,
    );
    mod_api::Text::new(mod_api::text::TextContent::Translatable {
        key,
        fallback: Some(title.to_owned()),
        arguments: Vec::new(),
    })
}

// Comparison ignores count and resident maxima, but damage must fit the component codec.
pub(in crate::game) fn comparison_key(
    mut stack: ItemStack,
    items: &Items,
) -> Result<ItemStack, mod_api::StackWriteError> {
    if i32::try_from(stack.damage).is_err() {
        return Err(mod_api::StackWriteError::InvalidComponents {
            reason: mod_api::StackError::InvalidDamage,
        });
    }
    normalize_components(&mut stack, items);
    Ok(stack)
}

pub(in crate::game) fn target_lists(
    visibility: CreativeTabVisibility,
) -> &'static [CreativeTabList] {
    use CreativeTabList::{Parent, Search};
    match visibility {
        CreativeTabVisibility::Parent => &[Parent],
        CreativeTabVisibility::Search => &[Search],
        CreativeTabVisibility::Both => &[Parent, Search],
    }
}

pub(in crate::game) fn list_items(
    contents: &mut TabContents,
    list: CreativeTabList,
) -> &mut Vec<ItemStack> {
    match list {
        CreativeTabList::Parent => &mut contents.display,
        CreativeTabList::Search => &mut contents.search,
    }
}

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

use crate::game::tables::GameItems as _;

use crate::game::tables::WorldTablesExt as _;
