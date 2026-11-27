//! Authoritative custom displays, with slot examples resolved in the server's context.
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};
use std::result::Result;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_sim::recipe_book::{DisplayResources, placement_sync as sync};

impl crate::game::ServerGame<'_> {
    fn concretize_mod_book_slot(&self, slot: &mut Slot) -> Result<(), sync::Refusal> {
        if matches!(slot, Slot::Empty | Slot::ItemStack(_)) {
            return Ok(());
        }
        let stacks = self
            .resolve_mod_book_declaration_slot(slot)
            .into_iter()
            .map(|stack| {
                stack
                    .try_into_api()
                    .map(Slot::ItemStack)
                    .map_err(|_| sync::Refusal::Invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        *slot = Slot::Composite(stacks);
        Ok(())
    }
    fn concretize_mod_book_display(&self, display: &mut Display) -> Result<(), sync::Refusal> {
        match display {
            Display::Shaped {
                ingredients,
                result,
                station,
                ..
            }
            | Display::Shapeless {
                ingredients,
                result,
                station,
            }
            | Display::Custom {
                slots: ingredients,
                result,
                station,
            } => {
                for slot in ingredients {
                    self.concretize_mod_book_slot(slot)?;
                }
                self.concretize_mod_book_slot(result)?;
                self.concretize_mod_book_slot(station)?;
            }
            Display::Furnace {
                ingredient,
                fuel,
                result,
                station,
                ..
            } => {
                for slot in [ingredient, fuel, result, station] {
                    self.concretize_mod_book_slot(slot)?;
                }
            }
            Display::Stonecutter {
                input,
                result,
                station,
            } => {
                for slot in [input, result, station] {
                    self.concretize_mod_book_slot(slot)?;
                }
            }
            Display::Smithing {
                template,
                base,
                addition,
                result,
                station,
            } => {
                for slot in [template, base, addition, result, station] {
                    self.concretize_mod_book_slot(slot)?;
                }
            }
        }
        Ok(())
    }

    pub(in crate::game::server_side) fn server_mod_book_display(
        &self,
        catalog: &sync::CatalogIdentity,
        display: &sync::DisplayId,
    ) -> Result<Display, sync::Refusal> {
        let index = self
            .common
            .recipe_tables
            .books
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .ok_or(sync::Refusal::Invalid)?;
        if index.generation() != catalog.generation {
            return Err(sync::Refusal::Stale);
        }
        let mut resolved = index
            .categories()
            .filter(|category| {
                category.target_menu == catalog.menu.as_str()
                    && category.book.as_str() == catalog.book.as_str()
            })
            .flat_map(|category| &category.displays)
            .find(|entry| entry.recipe == display.recipe.as_str())
            .and_then(|entry| entry.displays.get(display.source as usize))
            .cloned()
            .ok_or(sync::Refusal::UnknownDisplay)?;
        self.concretize_mod_book_display(&mut resolved)?;
        Ok(resolved)
    }

    /// Whether `conn` still has the exact open, valid mod menu identified by `(window, instance, menu)`.
    pub(in crate::game::server_side) fn mod_book_opening_matches(
        &self,
        conn: u32,
        window: i32,
        instance: u64,
        menu: &str,
    ) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return false;
        };
        if !host.hub.here(guest)
            || guest.spectator()
            || guest.dead
            || !self.guest_menu_still_valid(conn)
        {
            return false;
        }
        let Some(open) = guest.menu.as_ref() else {
            return false;
        };
        guest.menu_generation == instance
            && open.window == window
            && open.registry_id.as_ref() == menu
            && open.mod_type.is_some_and(|kind| kind.id == menu)
    }
    pub(in crate::game::server_side) fn server_mod_book_catalogue(
        &mut self,
        conn: u32,
        frame: sync::Frame,
    ) -> Result<(sync::Frame, Vec<u8>), sync::Refusal> {
        use sync::Refusal;
        let sync::Frame::Catalog {
            epoch,
            number,
            window,
            nonce,
            menu,
            book,
        } = frame
        else {
            return Err(Refusal::Invalid);
        };
        let settings = self
            .common
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .ok_or(Refusal::Unsupported)?;
        if settings.epoch != epoch {
            return Err(Refusal::Stale);
        }
        if !settings.contains(book.as_str()) {
            return Err(Refusal::Unavailable);
        }
        let (bound_book, _, _) =
            mod_api::RecipeBookLayout::registered(menu.as_str()).ok_or(Refusal::Unavailable)?;
        if bound_book.as_str() != book.as_str() {
            return Err(Refusal::Unavailable);
        }
        let (instance, known) = {
            let host = self.server.lan.as_ref().ok_or(Refusal::Unavailable)?;
            let guest = host.hub.guest(conn).ok_or(Refusal::Unavailable)?;
            if !host.hub.here(guest)
                || guest.spectator()
                || guest.dead
                || !self.guest_menu_still_valid(conn)
            {
                return Err(Refusal::Unavailable);
            }
            let open = guest.menu.as_ref().ok_or(Refusal::Unavailable)?;
            if open.window != window || open.registry_id.as_ref() != menu.as_str() {
                return Err(Refusal::Stale);
            }
            if open.mod_type.is_none_or(|kind| kind.id != menu.as_str()) {
                return Err(Refusal::Unavailable);
            }
            (guest.menu_generation, guest.recipe_book.recipes.clone())
        };
        self.ensure_mod_book_displays();
        let index = self
            .common
            .recipe_tables
            .books
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .ok_or(Refusal::Invalid)?;
        let layout = index
            .layout(menu.as_str())
            .ok_or(Refusal::Unavailable)?
            .declaration()
            .clone();
        let generation = index.generation();
        let mut declarations = Vec::new();
        for category in index.categories().filter(|category| {
            category.target_menu == menu.as_str() && category.book.as_str() == book.as_str()
        }) {
            let mut entries = Vec::new();
            for entry in &category.displays {
                if !known.contains(&entry.recipe) {
                    continue;
                }
                let mut displays = entry.displays.clone();
                for display in &mut displays {
                    self.concretize_mod_book_display(display)?;
                }
                entries.push(sync::BorrowedDisplayEntry {
                    recipe: entry.recipe.as_str(),
                    displays,
                    placement: entry.placement.as_ref(),
                });
            }
            declarations.push(sync::BorrowedCategory {
                category,
                displays: entries,
            });
        }
        let bytes = sync::encode_borrowed_catalogue(&declarations)?;
        let same_authority = self
            .common
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .is_some_and(|authority| authority.epoch == epoch && authority.contains(book.as_str()));
        if !same_authority || !self.mod_book_opening_matches(conn, window, instance, menu.as_str())
        {
            return Err(Refusal::Changed);
        }
        let identity = sync::CatalogIdentity {
            epoch,
            number,
            window,
            nonce,
            instance,
            generation,
            menu,
            book,
        };
        let storage = super::native_book_displays::Resources(self, std::marker::PhantomData)
            .menu_layout(identity.menu.as_str())
            .ok_or(Refusal::Unavailable)?;
        let displays = declarations
            .iter()
            .flat_map(|category| &category.displays)
            .map(|entry| (entry.recipe.to_owned(), entry.displays.len() as u32))
            .collect();
        drop(declarations);
        let authority = self
            .common
            .native_net
            .placement
            .servers
            .get_mut(&conn)
            .ok_or(Refusal::Unsupported)?;
        authority.peer.publish(identity.clone())?;
        authority.displays = displays;
        let start = sync::Frame::Start {
            identity,
            layout,
            storage,
            total: bytes.len() as u32,
        };
        Ok((start, bytes))
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_book_displays::NativeBookDisplaysGameExt1 as _;
