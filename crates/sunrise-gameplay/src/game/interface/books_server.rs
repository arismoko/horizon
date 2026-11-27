use super::chat_common::PLAYER_NAME;
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_world::level::flags as level_flags;
use {
    sunrise_items::items::ItemStack, sunrise_level::containers::BlockEntity,
    sunrise_level::containers::Lectern, sunrise_sim::first_person_sim::Hand,
    sunrise_sim::menu::Kind, sunrise_sim::menu::Menu, sunrise_sim::mobs::Who,
};

impl crate::game::ServerGame<'_> {
    /// `ServerGamePacketListenerImpl.handleEditBook`: a guest's book in a
    /// hotbar slot or its off hand (`updateBookContents`, `signBook`).
    pub(in crate::game::server_side) fn lan_edit_book(&mut self, conn: u32, p: play::EditBook) {
        if !((0..9).contains(&p.slot) || p.slot == super::shield::OFFHAND_SLOT as i32) {
            return;
        }
        let menu_slot = if p.slot == super::shield::OFFHAND_SLOT as i32 {
            45
        } else {
            36 + p.slot as usize
        };
        let cancelled = self.as_guest(conn, |g| {
            let mut cancel = false;
            let writable = g
                .stack_in(p.slot as usize)
                .is_some_and(|stack| &*stack.item == "minecraft:writable_book");
            if writable
                && let (Some(player), Some(name)) =
                    (g.server.lan_acting_uuid, g.server.lan_acting_name.clone())
            {
                g.native_player_edit_book(
                    player,
                    &name,
                    p.pages.len() as i32,
                    p.title.is_some(),
                    &mut cancel,
                );
            }
            if cancel {
                return true;
            }
            g.write_book(p.slot as usize, p.pages, p.title);
            false
        });
        // A cancelled edit leaves the client's own copy of the book: forget
        // what was sent so the next broadcast resends the server's stack.
        if cancelled == Some(true)
            && let Some(host) = self.server.lan.as_mut()
        {
            let _ = host
                .hub
                .guests
                .update_connection(conn, |g| g.menu_sent[menu_slot] = None);
        }
    }
    /// The lectern menu's book and page back in the block entity: a new page
    /// is `LecternBlock.signalPageChange` (the 2-tick pulse, level event
    /// 1043), a book gone `onBookItemRemove` (`resetBookState`).
    pub(in crate::game) fn lectern_menu_done(&mut self, key: (i32, i32, i32), menu: &Menu) {
        let Some(BlockEntity::Lectern(l)) = self.common.world.block_entities.get_mut(&key) else {
            return;
        };
        let (taken_book, before) = (l.book.clone(), l.page);
        l.book = menu.slot(0).cloned();
        l.page = menu.data[0];
        if let Some(book) = taken_book.filter(|_| l.book.is_none()) {
            self.set_lectern_powered(key, false);
            self.set_lectern_has_book(key, false, None);
            self.container_changed(key);
            if let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            ) {
                self.native_player_take_lectern_book(
                    player,
                    &name,
                    [key.0, key.1, key.2],
                    &book.item,
                );
            }
        } else if menu.data[0] != before {
            self.set_lectern_powered(key, true);
            self.server.lectern_pulses.push((key, 2));
            self.server_level_event_at(1043, [key.0, key.1, key.2], 0);
            self.container_changed(key);
            if let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            ) {
                self.native_player_lectern_page_change(
                    player,
                    &name,
                    [key.0, key.1, key.2],
                    menu.data[0],
                );
            }
        }
    }
    /// The 2-tick pulses of turned lecterns.
    pub(in crate::game) fn tick_lecterns(&mut self) {
        let mut done = Vec::new();
        for (key, left) in &mut self.server.lectern_pulses {
            *left -= 1;
            if *left <= 0 {
                done.push(*key);
            }
        }
        self.server.lectern_pulses.retain(|(_, left)| *left > 0);
        for key in done {
            self.set_lectern_powered(key, false);
        }
    }
    /// `WritableBookItem.use` / `WrittenBookItem.use` (`Player.openItemGui`).
    /// A server's player writes in its own client's editor
    /// (`LocalPlayer.openItemGui`), and reads on its own client once told to
    /// (`ServerPlayer.openItemGui`'s ClientboundOpenBookPacket).
    pub(in crate::game::server_side) fn use_book(&mut self, stack: &ItemStack) -> bool {
        assert!(self.is_logical_server(), "use_book called on Client role");
        let written = match &*stack.item {
            "minecraft:writable_book" => false,
            "minecraft:written_book" => true,
            _ => return false,
        };
        // `ServerPlayer.openItemGui` resolves a written book for the player
        // opening it before the screen reads it.
        let held;
        let stack = if written && self.resolve_held_book() {
            held = self.common.held().cloned();
            held.as_ref().unwrap_or(stack)
        } else {
            stack
        };
        if self.server.lan_acting.is_some() && written {
            let hand = i32::from(self.common.hand == Hand::Off);
            self.server
                .guest_packets
                .push(C::OpenBook(play::OpenBook { hand }));
        }
        // WritableBookItem.use / WrittenBookItem.use count every open.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `LecternBlock.useItemOn`: a book goes onto an empty lectern.
    pub fn use_lectern(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let key = (pos[0], pos[1], pos[2]);
        if &*self
            .common
            .res
            .facts()
            .get(self.common.world.block(pos[0], pos[1], pos[2]))
            .name
            != "minecraft:lectern"
        {
            return false;
        }
        let has_book = matches!(self.world.block_entities.get(&key), Some(BlockEntity::Lectern(l)) if l.book.is_some());
        // `LecternBlock.useItemOn`: `ItemTags.LECTERN_BOOKS`, which a data
        // pack can add its own books to, not only the two vanilla items.
        let Some(book) = held.filter(|h| {
            world_items!(self)
                .item_tags
                .get("minecraft:lectern_books")
                .is_some_and(|books| books.contains(&*h.item))
        }) else {
            return false;
        };
        if has_book {
            return false;
        }
        let mut one = book.clone();
        one.count = 1;
        self.common.world.block_entities.insert(
            key,
            BlockEntity::Lectern(Lectern {
                book: Some(one),
                page: 0,
            }),
        );
        self.set_lectern_has_book(
            key,
            true,
            self.common.is_logical_server().then_some(Who::Player),
        );
        if !self.common.creative {
            self.consume_held();
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.server_play_sound("minecraft:item.book.put", at, 1.0, 1.0);
        self.container_changed(key);
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_insert_lectern_book(player, &name, pos, &book.item);
        }
        true
    }
    /// The book and quill at `slot` takes the pages, or with a title becomes
    /// the written book its writer signed.
    pub fn write_book(&mut self, slot: usize, pages: Vec<String>, title: Option<String>) {
        let author = self
            .server
            .lan_acting_name
            .clone()
            .unwrap_or_else(|| super::chat_common::PLAYER_NAME.to_owned());
        self.common
            .write_book_with_author(slot, pages, title, author);
    }

    /// `ServerPlayer.openItemGui` -> `WrittenBookContent.resolveForItem`: resolve selectors/scores.
    pub(in crate::game::server_side) fn resolve_held_book(&mut self) -> bool {
        let Some(mut book) = self
            .common
            .held()
            .and_then(|s| s.data())
            .and_then(|d| d.written.clone())
        else {
            return false;
        };
        if book.resolved {
            return false;
        }
        let string = |v: &Tag| match v {
            Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        };
        let changed = book.resolve(|tag| {
            let Tag::Compound(c) = tag else { return None };
            if let Some(Tag::String(selector)) = c.get(&"selector".into()) {
                let selector = selector.to_string_lossy();
                return Some(self.resolve_selector_text(&selector));
            }
            let score = c.get(&"score".into())?;
            let name = score.get("name").and_then(string).unwrap_or_default();
            let objective = score.get("objective").and_then(string).unwrap_or_default();
            let name = if name == "*" {
                PLAYER_NAME
            } else {
                name.as_str()
            };
            Some(
                self.common
                    .score
                    .board
                    .score(name, &objective)
                    .map_or_else(String::new, |s| s.value.to_string()),
            )
        });
        if let Some(stack) = self.common.held_mut() {
            stack.data_mut().written = Some(book);
        }
        changed
    }

    pub(in crate::game::server_side) fn set_lectern_powered(
        &mut self,
        key: (i32, i32, i32),
        powered: bool,
    ) {
        self.set_lectern_property(key, "powered", powered);
        let p = BlockPos::new(key.0, key.1, key.2);
        let state = self.common.world.block(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| {
            b.lectern_update_below(level, ticks, p, state);
        });
    }

    /// `LecternBlock.resetBookState`: the property with the block change event.
    pub fn set_lectern_has_book(&mut self, key: (i32, i32, i32), has_book: bool, who: Option<Who>) {
        self.set_lectern_property(key, "has_book", has_book);
        let after = self.common.world.block(key.0, key.1, key.2);
        self.post_block_game_event(
            "minecraft:block_change",
            [key.0, key.1, key.2],
            who,
            Some(after),
        );
    }

    pub fn set_lectern_property(&mut self, key: (i32, i32, i32), name: &str, value: bool) {
        let state = self.common.world.block(key.0, key.1, key.2);
        let registry = self.common.pack.assets.registry;
        let Some(mut s) = registry.state(state).cloned() else {
            return;
        };
        if !s.properties.contains_key(name) {
            return;
        }
        s.properties.insert(name.into(), value.to_string());
        if let Some(new) = registry.id(&s) {
            let p = BlockPos::new(key.0, key.1, key.2);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, p, new, level_flags::UPDATE_ALL)
            });
        }
    }

    /// `LecternMenu` over the lectern at `key`: its book and page.
    pub(in crate::game) fn lectern_menu(&self, key: (i32, i32, i32)) -> Option<Menu> {
        let Some(BlockEntity::Lectern(l)) = self.common.world.block_entities.get(&key) else {
            return None;
        };
        let mut menu = Menu::new(Kind::Lectern);
        menu.set_registry_id(Some("minecraft:lectern"));
        menu.set_slot(&self.menu_env(), 0, l.book.clone());
        menu.data[0] = l.page;
        Some(menu)
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
