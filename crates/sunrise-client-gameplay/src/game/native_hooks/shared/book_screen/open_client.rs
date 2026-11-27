use super::*;

impl crate::game::ClientGame<'_> {
    /// Stage the current remote opening's screen for the window. Its slots
    /// are stored under `key`; `extra` was already matched to this opening.
    /// The packet handler checked for required factories before changing menus.
    /// False refuses the opening; an ordinary stock menu needs no mod screen.
    pub fn open_native_screen(
        &mut self,
        key: Pos,
        target: Option<Pos>,
        backing_slots: usize,
        extra: Option<sunrise_protocol::menu_extra::Message>,
    ) -> bool {
        let Some(remote) = self.client.remote.as_ref() else {
            return false;
        };
        let Some((window, name)) = remote.menu.as_ref() else {
            return false;
        };
        let mod_type = sunrise_level::containers::mod_menu_type(name);
        let block = target.and_then(|pos| self.menu_opener_block(pos, name));
        let binding = mod_api::RecipeBookLayout::registered(name);
        let of = block.unwrap_or(name);
        // A matching payload supplies the position, including an explicit absence.
        // Otherwise only a verified clicked opener supplies one, never the slot storage key.
        let pos = extra.as_ref().map_or_else(
            || {
                target
                    .filter(|_| block.is_some())
                    .map(|(x, y, z)| [x, y, z])
            },
            |extra| extra.pos,
        );
        let open = mod_api::MenuScreenOpen {
            window: *window,
            menu: name,
            block,
            pos,
            title: remote.menu_title_snapshot(),
            backing_slots,
            player_slots: mod_type.map_or(36, sunrise_level::containers::ModMenuType::player_slots),
            data_slots: mod_type.map_or(0, |kind| kind.data_slots),
            extra: extra.as_ref().map_or(&[], |extra| extra.data.as_slice()),
        };
        let created = mod_api::__private::create_menu_screen(self, open);
        // Opening bytes are construction-only; the result owns its presentation.
        drop(extra);
        let Some((owner, created)) = created else {
            return true;
        };
        let mut instance = match created {
            Ok(Some(instance)) => instance,
            Ok(None) => {
                log::warn!("mod {owner} declined to create a screen for {of}");
                return false;
            }
            Err(reason) => {
                log::error!("mod {owner} screen for {of}: {reason}");
                return false;
            }
        };
        let screen = instance.layout_mut();
        if let Some((book, _, _)) = &binding
            && screen.recipe_book.as_ref() != Some(book)
        {
            log::error!("screen for {of}: expected the menu's bound recipe book {book:?}");
            return false;
        }
        if ![screen.width, screen.height]
            .into_iter()
            .all(|size| size.is_finite() && size > 0.0)
            || ![screen.inventory_x, screen.inventory_y]
                .into_iter()
                .chain(screen.title_label.into_iter().flatten())
                .chain(screen.inventory_label.into_iter().flatten())
                .chain(
                    screen
                        .recipe_book_toggle
                        .into_iter()
                        .flat_map(|toggle| [toggle.x, toggle.y]),
                )
                .all(f32::is_finite)
        {
            log::error!("screen for {of}: invalid dimensions or coordinates");
            return false;
        }
        if screen.slots.len() > backing_slots {
            log::error!("screen for {of}: layout is longer than its {backing_slots} backing slots");
            return false;
        }
        // Every slot uses its backing index; `None` (including omitted trailing
        // entries) marks an initially inactive slot (`Slot.isActive()`).
        for (index, slot) in screen.slots.iter().enumerate() {
            if let Some(slot) = slot
                && !slot.position.into_iter().all(f32::is_finite)
            {
                log::error!("screen for {of}: nonfinite slot {index} coordinates");
                return false;
            }
        }
        screen.slots.resize_with(backing_slots, || None);
        let title = remote.menu_title_snapshot().cloned();
        self.client.mod_screen.0 = Some(OpenNativeScreen {
            menu: OpenNativeMenu {
                key,
                window: *window,
                generation: remote.menu.generation,
                name: name.clone(),
                title,
                block: block.map(str::to_owned),
                pos,
                mod_type,
                data: Vec::new(),
            },
            slot_moves: Default::default(),
            layout_revision: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            screen: instance,
            shown: false,
            detached: false,
        });
        true
    }
}
