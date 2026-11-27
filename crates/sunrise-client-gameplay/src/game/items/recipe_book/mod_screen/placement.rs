//! Placement state is discarded before a new catalogue or menu can be shown.
use super::*;
/// The ghosts to draw, in slot order, without one in a slot the screen hides or lacks or one that
/// resolves to no stack (`GhostSlots.setSlot`). A nonempty display replaces the prior entry for
/// the same physical slot in place; an unresolved empty display does not erase a prior one.
fn shown_ghosts(
    base: usize,
    ghosts: Vec<mod_api::GhostSlot>,
    has_slot: impl Fn(u16) -> bool,
    resolve: impl Fn(&mod_api::RecipeSlotDisplay) -> Vec<ItemStack>,
) -> Vec<super::super::Ghost> {
    let mut shown: Vec<super::super::Ghost> = Vec::new();
    let mut by_slot: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
    for ghost in ghosts {
        if !has_slot(ghost.slot) {
            continue;
        }
        let items = resolve(&ghost.display);
        if items.is_empty() {
            continue;
        }
        let entry = super::super::Ghost {
            slot: base + usize::from(ghost.slot),
            items,
            result: ghost.result,
        };
        match by_slot.entry(ghost.slot) {
            std::collections::hash_map::Entry::Occupied(existing) => {
                shown[*existing.get()] = entry;
            }
            std::collections::hash_map::Entry::Vacant(vacant) => {
                vacant.insert(shown.len());
                shown.push(entry);
            }
        }
    }
    shown
}
pub(super) struct PlacementOverlay {
    pub(super) view: OverlayView,
    pub(super) recipe: String,
    pub(super) sources: Vec<u32>,
}
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn request_mod_book_recipe(&mut self, recipe: String, source: u32) {
        if !self
            .common
            .native_net
            .placement
            .client
            .window()
            .is_some_and(|window| self.native_book_placement_window_matches(window))
        {
            return;
        }
        let Ok(recipe) = recipe.try_into() else {
            return;
        };
        self.client.recipe_ui.0.ghost.clear();
        let use_max = self.has_shift_down();
        if let Some(frame) = self.common.native_net.placement.client.place(
            sunrise_sim::recipe_book::placement_sync::DisplayId { recipe, source },
            use_max,
        ) {
            self.send_mod_book_placement(frame);
            if self
                .batch_for_size()
                .is_some_and(|batch| self.mod_book_overlay(batch.width))
                && let Some((_, filtering)) = self.mod_book_flags()
                && let Some(book) = self.native_screen_book().cloned()
            {
                self.request_custom_book_settings(
                    &book,
                    mod_api::RecipeBookSettings {
                        open: false,
                        filtering,
                    },
                );
            }
        }
    }
    pub(super) fn consume_mod_book_overlay(&mut self, right: bool) -> bool {
        let Some(overlay) = self.client.recipe_ui.0.custom.overlay.take() else {
            return false;
        };
        if !right {
            let columns = if overlay.view.recipes.len() <= 16 {
                4
            } else {
                5
            };
            for (n, source) in overlay.sources.iter().enumerate() {
                let x = overlay.view.x as f32 + 4.0 + 25.0 * (n % columns) as f32;
                let y = overlay.view.y as f32 + 5.0 + 25.0 * (n / columns) as f32;
                if inside(self.client.cursor, x, y, 24.0, 24.0) {
                    self.request_mod_book_recipe(overlay.recipe, *source);
                    break;
                }
            }
        }
        true
    }
    pub(in crate::game::client_side) fn install_mod_book_ghost(
        &mut self,
        catalog: &sunrise_sim::recipe_book::placement_sync::CatalogIdentity,
        recipe: &sunrise_sim::recipe_book::placement_sync::ResourceId,
        display: &mod_api::RecipeBookDisplay,
    ) {
        let Some(view) = self
            .common
            .native_net
            .placement
            .view
            .as_ref()
            .filter(|view| &view.identity == catalog)
        else {
            return;
        };
        let Some(layout) = view.index.layout(catalog.menu.as_str()).cloned() else {
            return;
        };
        let result = match layout.declaration() {
            mod_api::RecipeBookLayout::Crafting { result, .. }
            | mod_api::RecipeBookLayout::Furnace { result, .. }
            | mod_api::RecipeBookLayout::Stonecutter { result, .. }
            | mod_api::RecipeBookLayout::Smithing { result, .. } => Some(*result),
            mod_api::RecipeBookLayout::Mod => None,
        };
        let base = self.native_screen_container_slot_base();
        let fuel_empty = match layout.declaration() {
            mod_api::RecipeBookLayout::Furnace { fuel, .. } => {
                self.slot_mut(base + usize::from(*fuel)).is_none()
            }
            _ => true,
        };
        // The game's own ghosts for the layout (none for `Mod`), then the menu part's say.
        let mut ghosts: Vec<mod_api::GhostSlot> = layout
            .map(display, fuel_empty)
            .unwrap_or_default()
            .into_iter()
            .map(|(slot, display)| mod_api::GhostSlot {
                slot,
                display,
                result: Some(slot) == result,
            })
            .collect();
        let menu = catalog.menu.as_str();
        if mod_api::__private::menu_listens(mod_api::HookId::RecipeBookGhost, menu) {
            self.native_recipe_book_ghost(mod_api::__private::RecipeBookGhostCall {
                menu,
                recipe: recipe.as_str(),
                display,
                ghosts: &mut ghosts,
            });
        }
        let shown = shown_ghosts(
            base,
            ghosts,
            |slot| self.native_screen_slot(usize::from(slot)).is_some(),
            |display| self.resolve_mod_book_declaration_slot(display),
        );
        self.client.recipe_ui.0.ghost = shown;
    }
    pub(in crate::game::client_side) fn reset_mod_book_placement_ui(&mut self) {
        self.client.recipe_ui.0.ghost.clear();
        self.client.recipe_ui.0.custom.prepared = None;
        self.client.recipe_ui.0.custom.presentation = None;
        self.client.recipe_ui.0.custom.overlay = None;
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;

    fn ghost(slot: u16, item: &str, result: bool) -> mod_api::GhostSlot {
        mod_api::GhostSlot {
            slot,
            display: mod_api::RecipeSlotDisplay::Item(item.into()),
            result,
        }
    }

    #[test]
    fn ghosts_without_a_slot_or_a_stack_are_dropped_and_the_rest_keep_their_order() {
        let shown = shown_ghosts(
            300,
            vec![
                ghost(6, "demo:meal", true),
                ghost(40, "demo:a", false),
                ghost(0, "demo:none", false),
                ghost(1, "demo:b", false),
            ],
            |slot| slot < 10,
            |display| match display {
                mod_api::RecipeSlotDisplay::Item(id) if id != "demo:none" => {
                    vec![ItemStack::new(id, 1)]
                }
                _ => Vec::new(),
            },
        );
        let seen: Vec<_> = shown
            .iter()
            .map(|g| (g.slot, g.items[0].item.to_string(), g.result))
            .collect();
        assert_eq!(
            seen,
            [
                (306, "demo:meal".to_owned(), true),
                (301, "demo:b".to_owned(), false)
            ]
        );
    }

    #[test]
    fn late_slots_past_old_cap_and_repeated_slot_replacement_follow_ghost_slots_set_slot() {
        let mut contributions: Vec<_> = (0u16..260)
            .map(|slot| ghost(slot, &format!("demo:early_{slot}"), false))
            .collect();
        contributions.extend([
            ghost(260, "demo:late_260", true),
            ghost(261, "demo:hidden_261", false),
            ghost(262, "demo:late_262", false),
            ghost(5, "demo:replaced_5", true),
            ghost(5, "demo:empty", false),
            ghost(263, "demo:empty", false),
            ghost(264, "demo:late_264", false),
        ]);
        assert!(contributions.len() > 256);

        let shown = shown_ghosts(
            300,
            contributions,
            |slot| slot < 265 && slot != 261,
            |display| match display {
                mod_api::RecipeSlotDisplay::Item(id) if id != "demo:empty" => {
                    vec![ItemStack::new(id, 1)]
                }
                _ => Vec::new(),
            },
        );
        let seen: Vec<_> = shown
            .iter()
            .map(|g| (g.slot, g.items[0].item.to_string(), g.result))
            .collect();
        let mut expected: Vec<_> = (0usize..260)
            .map(|slot| {
                if slot == 5 {
                    (305, "demo:replaced_5".to_owned(), true)
                } else {
                    (300 + slot, format!("demo:early_{slot}"), false)
                }
            })
            .collect();
        expected.extend([
            (560, "demo:late_260".to_owned(), true),
            (562, "demo:late_262".to_owned(), false),
            (564, "demo:late_264".to_owned(), false),
        ]);
        assert_eq!(seen, expected);
    }
}
