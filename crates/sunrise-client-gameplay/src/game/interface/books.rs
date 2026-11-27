//! Books (`BookEditScreen`, `BookSignScreen`, `BookViewScreen`,
//! `LecternScreen`): writing pages of a book and quill (14 lines of 114
//! pixels, 1024 characters, 100 pages), signing it into a written book,
//! reading written books, and lecterns (placing a book, turning its pages
//! with a redstone pulse, the comparator output and taking the book).
use crate::game::ClientGame;
#[cfg(feature = "client")]
use crate::game::client_side::screen_info;
use sunrise_client::{
    hud::{HudBatch, rgba},
    ui,
    widgets::{self, Narratable, ScreenInfo},
};
use sunrise_game_data::text::Text;
use sunrise_items::{item_data::StackData, items::ItemStack};
use sunrise_level::containers::BlockEntity;
use sunrise_protocol::packets::play::{self, Serverbound as S};
use sunrise_sim::first_person_sim::Hand;
use winit::keyboard::KeyCode;

pub(in crate::game) const BLACK: u32 = 0xFF00_0000;
pub(in crate::game) const PAGE_CHARS: usize = 1024;
pub(in crate::game) const MAX_PAGES: usize = 100;
pub(in crate::game) const LINES: usize = 14;

pub(in crate::game::client_side) fn book_title_box() -> ui::EditBox {
    let mut edit = ui::EditBox::new(0.0, 0.0, 114.0, 9.0);
    edit.bordered = false;
    edit.max_length = 15;
    edit.text_color = BLACK;
    // BookSignScreen sets the title box centred.
    edit.centered = true;
    edit.focused = true;
    edit
}
pub(in crate::game::client_side) fn book_page_box() -> ui::EditBox {
    let mut edit = ui::EditBox::new(0.0, 0.0, 114.0, 9.0);
    edit.bordered = false;
    edit.multiline = true;
    edit.max_length = PAGE_CHARS;
    edit.text_color = BLACK;
    edit.focused = true;
    edit
}

/// Where a book's text is, from its top left corner, above its page buttons.
pub(in crate::game) const BOOK_TEXT: [f32; 4] = [36.0, 16.0, 120.0, 136.0];

pub enum BookScreen {
    Edit {
        /// Inventory slot of the book (the selected hotbar slot).
        slot: usize,
        pages: Vec<String>,
        page: usize,
        signing: bool,
        title: String,
        /// BookSignScreen's title box (`EditBox` of 15).
        title_edit: ui::EditBox,
        /// BookEditScreen's page (`MultiLineEditBox`).
        page_edit: ui::EditBox,
    },
    View {
        pages: Vec<Text>,
        page: usize,
        lectern: Option<(i32, i32, i32)>,
    },
}
impl BookScreen {
    pub(in crate::game::client_side) fn begin_signing(&mut self) {
        if let Self::Edit {
            signing,
            title,
            title_edit,
            ..
        } = self
        {
            *signing = true;
            title_edit.set_value(title.as_str());
            title_edit.reset_blink();
        }
    }

    /// The id the screen hooks see (`ScreenView.id`).
    pub fn id(&self) -> &'static str {
        match self {
            Self::Edit { signing: true, .. } => "book_sign",
            Self::Edit { .. } => "book_edit",
            Self::View {
                lectern: Some(_), ..
            } => "lectern",
            Self::View { .. } => "book_view",
        }
    }
}
/// The pages a book screen reads: a written book's components (so their
/// click events stay), else a writable book's text.
pub(in crate::game::client_side) fn view_pages(data: Option<&StackData>) -> Vec<Text> {
    let Some(data) = data else { return Vec::new() };
    let Some(w) = &data.written else {
        return data.writable_pages.iter().map(Text::untrusted).collect();
    };
    let tagged = w.page_tags.len() == w.pages.len();
    w.pages
        .iter()
        .enumerate()
        .map(|(i, page)| match tagged.then(|| &w.page_tags[i]) {
            // A component object, or a component list (`ComponentSerialization`
            // takes both; servers behind ViaVersion send the list form).
            Some(tag @ (sunrise_core::nbt::Tag::Compound(_) | sunrise_core::nbt::Tag::List(_))) => {
                sunrise_sim::entity_nbt::name_text(tag)
            }
            _ => Text::untrusted(page),
        })
        .collect()
}
#[cfg(feature = "client")]
impl ClientGame<'_> {
    pub(in crate::game::client_side) fn book_lines(&self, page: &str) -> Vec<Text> {
        self.page_lines(&Text::untrusted(page))
    }
    pub(in crate::game::client_side) fn page_lines(&self, page: &Text) -> Vec<Text> {
        self.client.gui.atlas.font.split(page, 114.0)
    }
    /// `BookViewScreen.mouseClicked`: the click event of the text under the
    /// cursor on the page.
    pub(in crate::game::client_side) fn book_link_at(
        &self,
        page: &Text,
        left: f32,
    ) -> Option<sunrise_game_data::text::ClickEvent> {
        self.book_style_at(page, left)?.click
    }
    /// `ActiveTextCollector` over the page: the style of the character
    /// under the cursor, for its click and hover events.
    pub(in crate::game::client_side) fn book_style_at(
        &self,
        page: &Text,
        left: f32,
    ) -> Option<sunrise_game_data::text::Style> {
        let (mx, my) = self.client.cursor;
        let row = ((my - 32.0) / 9.0).floor();
        if row < 0.0 || row >= LINES as f32 {
            return None;
        }
        let lines = self.page_lines(page);
        let line = lines.get(row as usize)?.resolve_legacy();
        let mut x = mx - (left + 36.0);
        if x < 0.0 {
            return None;
        }
        for (c, style) in line.chars() {
            x -= self.client.gui.atlas.font.advance(c, style);
            if x < 0.0 {
                return Some(style.clone());
            }
        }
        None
    }
    /// `BookViewScreen.handleClickEvent`: a page link turns to its page
    /// (`forcePage`), a web link goes through the confirm-link flow.
    pub(in crate::game::client_side) fn book_follow_link(
        &mut self,
        link: sunrise_game_data::text::ClickEvent,
        count: usize,
        lectern: Option<(i32, i32, i32)>,
    ) {
        use sunrise_game_data::text::ClickEvent;
        match link {
            ClickEvent::ChangePage(target) => {
                let target = (target - 1).clamp(0, count.saturating_sub(1) as i32);
                let Some(BookScreen::View { page, .. }) = self.client.book.as_mut() else {
                    return;
                };
                if let Some(key) = lectern {
                    // LecternScreen.forcePage: the menu's button 100 + page.
                    if target != *page as i32 {
                        self.lectern_button(key, 100 + target);
                    }
                } else if target != *page as i32 {
                    *page = target as usize;
                }
            }
            ClickEvent::OpenUrl(url) => self.click_url(url),
            // closeContainerOnServer, then clickCommandAction with no screen
            // after the command: the book closes and the command runs. A
            // lectern's screen goes with its menu; a book is still the screen
            // a confirmation returns to.
            ClickEvent::RunCommand(command) => {
                let book = if lectern.is_none() {
                    self.client.book.take()
                } else {
                    None
                };
                self.close_book();
                self.click_command(&command);
                if let Some(book) = book {
                    self.command_prompt_returns_to(super::remote_chat::Behind::Book(Box::new(
                        book,
                    )));
                }
            }
            // Screen.defaultHandleClickEvent: copied, the book stays open.
            ClickEvent::CopyToClipboard(value) => {
                use sunrise_client::clipboard::Clipboard;
                self.client.clipboard.0.set(&value);
            }
            // defaultHandleGameClickEvent: the dialog opens over the book.
            ClickEvent::ShowDialog(dialog) => self.show_dialog_ref(&dialog),
            // Screen.defaultHandleGameClickEvent: custom actions retain the book.
            ClickEvent::Custom { id, payload } => {
                if let Some(remote) = &self.client.remote {
                    remote
                        .session
                        .send_custom_click(&id, payload.map(std::sync::Arc::unwrap_or_clone));
                }
            }
            // SuggestCommand's insertText does nothing on a book; a server's
            // text never opens a file.
            ClickEvent::SuggestCommand(_) | ClickEvent::OpenFile(_) | ClickEvent::Profiler(_) => {}
        }
    }
    /// The screen a book in hand opens: `BookEditScreen` for a book and
    /// quill, `BookViewScreen` for a written book.
    pub(in crate::game::client_side) fn book_screen(
        &self,
        stack: &ItemStack,
    ) -> Option<BookScreen> {
        Some(match &*stack.item {
            "minecraft:writable_book" => {
                let mut pages = stack
                    .data()
                    .map(|d| d.writable_pages.clone())
                    .unwrap_or_default();
                if pages.is_empty() {
                    pages.push(String::new());
                }
                let mut page_edit = book_page_box();
                page_edit.set_value(&pages[0]);
                BookScreen::Edit {
                    slot: self.common.held_slot(),
                    pages,
                    page: 0,
                    signing: false,
                    title: String::new(),
                    title_edit: book_title_box(),
                    page_edit,
                }
            }
            "minecraft:written_book" => BookScreen::View {
                pages: view_pages(stack.data()),
                page: 0,
                lectern: None,
            },
            _ => return None,
        })
    }
    /// `MultiPlayerGameMode.useItem`'s own use of a book and quill on a
    /// server's client: its editor opens here at once.
    pub fn remote_use_book(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:writable_book" {
            return false;
        }
        if let Some(screen) = self.book_screen(stack) {
            self.open_book(screen);
        }
        true
    }
    /// `ClientPacketListener.handleOpenBook`: the written book in that hand.
    pub(in crate::game::client_side) fn remote_open_book(&mut self, hand: i32) {
        let hand = if hand == 1 { Hand::Off } else { Hand::Main };
        let screen = self.with_hand(hand, |g| {
            let stack = g.held().filter(|s| &*s.item == "minecraft:written_book")?;
            g.book_screen(&stack.clone())
        });
        if let Some(screen) = screen {
            self.open_book(screen);
        }
    }
    /// Opens `screen` (through `open_screen_overlay`: a mod may keep it shut).
    pub fn open_book(&mut self, screen: BookScreen) {
        self.open_screen_overlay(screen.id(), |game| {
            game.client.book = Some(screen);
            game.clear_device_keys();
            game.set_mapping_held("key.use", false);
            game.capture(false);
        });
    }
    /// `BookEditScreen` to `BookSignScreen` and back, each a
    /// `Minecraft.setScreen` (through `open_screen_overlay`).
    pub(in crate::game::client_side) fn set_book_signing(&mut self, on: bool) {
        let id = if on { "book_sign" } else { "book_edit" };
        self.open_screen_overlay(id, |game| {
            if let Some(book @ BookScreen::Edit { .. }) = game.client.book.as_mut() {
                if on {
                    book.begin_signing();
                } else if let BookScreen::Edit { signing, .. } = book {
                    *signing = false;
                }
            }
        });
    }
    pub(in crate::game) fn close_book(&mut self) {
        // A server's lectern: LecternScreen's menu closes (closeContainer).
        if matches!(
            self.client.book,
            Some(BookScreen::View {
                lectern: Some(_),
                ..
            })
        ) && self.client.remote.is_some()
        {
            self.remote_close_container();
        }
        if self.client.book.take().is_some() {
            self.refresh_screen_widgets();
        }
        self.capture(self.client.screen == super::Screen::Playing && !self.screen_over_playing());
    }
    /// `BookEditScreen.saveChanges` and `BookSignScreen`'s Sign and Close:
    /// trailing empty pages dropped, the book in hand updated here and, on a
    /// server's client, sent (ServerboundEditBookPacket).
    pub fn save_book(&mut self, sign: bool) {
        let Some(BookScreen::Edit {
            slot, pages, title, ..
        }) = self.client.book.take()
        else {
            return;
        };
        let mut pages = pages;
        while pages.len() > 1 && pages.last().is_some_and(String::is_empty) {
            pages.pop();
        }
        let title = sign.then(|| title.trim().to_owned());
        if self.client.remote.is_some() {
            self.send(S::EditBook(play::EditBook {
                slot: slot as i32,
                pages: pages.clone(),
                title: title.clone(),
            }));
        }
        let author = self
            .client
            .remote
            .as_ref()
            .map(|r| r.profile_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.common.options.player_name());
        self.common
            .write_book_with_author(slot, pages, title, author);
        self.refresh_screen_widgets();
        self.capture(true);
    }
    pub(in crate::game::client_side) fn book_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        if let Some(shortcut) = shortcut
            && matches!(self.client.book, Some(BookScreen::Edit { .. }))
        {
            self.book_shortcut(shortcut);
            return true;
        }
        self.book_input(Some(key), None)
    }
    /// A key and the text it typed in the open book (the keyboard, or a
    /// controller's on-screen keyboard).
    pub fn book_input(&mut self, key: Option<KeyCode>, text: Option<&str>) -> bool {
        match self.client.book.as_mut() {
            Some(BookScreen::View { .. }) => match key {
                Some(KeyCode::Escape) => {
                    self.close_book();
                    true
                }
                Some(KeyCode::PageUp) => {
                    self.turn_page(false);
                    true
                }
                Some(KeyCode::PageDown) => {
                    self.turn_page(true);
                    true
                }
                _ => false,
            },
            Some(BookScreen::Edit { signing: true, .. }) => match key {
                // `BookSignScreen` overrides neither `onClose` nor
                // `shouldCloseOnEsc`: Escape is `Screen.onClose`, which closes
                // the whole book and saves nothing.
                Some(KeyCode::Escape) => {
                    self.close_book();
                    true
                }
                // `BookSignScreen.keyPressed` signs whenever the title box is
                // not empty, blank or not, and trims the title on send.
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                    let empty = self.client.book.as_ref().is_some_and(
                        |b| matches!(b, BookScreen::Edit { title, .. } if title.is_empty()),
                    );
                    if !empty {
                        self.save_book(true);
                    }
                    !empty
                }
                Some(key) if self.book_title_key(key) => true,
                _ => {
                    if let Some(typed) = text {
                        self.book_title_insert(typed);
                        true
                    } else {
                        false
                    }
                }
            },
            Some(BookScreen::Edit { .. }) => match key {
                // `BookEditScreen` overrides neither `onClose` nor
                // `shouldCloseOnEsc` either: Escape is `Screen.onClose`, and
                // only Done calls `saveChanges`.
                Some(KeyCode::Escape) => {
                    self.close_book();
                    true
                }
                Some(KeyCode::PageUp) => {
                    self.turn_page(false);
                    true
                }
                Some(KeyCode::PageDown) => {
                    self.turn_page(true);
                    true
                }
                Some(KeyCode::ArrowUp) => {
                    self.book_move_line(-1);
                    true
                }
                Some(KeyCode::ArrowDown) => {
                    self.book_move_line(1);
                    true
                }
                // MultilineTextField.keyPressed: Home and End seek the caret's
                // line; with Ctrl they go to the page's start and end.
                Some(edge @ (KeyCode::Home | KeyCode::End)) if !self.has_control_down() => {
                    self.book_line_edge(edge == KeyCode::End);
                    true
                }
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                    self.page_insert("\n");
                    true
                }
                Some(key) if self.book_page_key(key) => true,
                _ => {
                    if let Some(typed) = text {
                        self.page_insert(typed);
                        true
                    } else {
                        false
                    }
                }
            },
            None => false,
        }
    }
    /// Typing for `sunrise-ctl type` (`\n` is a line break).
    pub(in crate::game::client_side) fn book_type(&mut self, text: &str) {
        let text = text.replace("\\n", "\n");
        match self.client.book.as_mut() {
            Some(BookScreen::Edit { signing: true, .. }) => self.book_title_insert(&text),
            Some(BookScreen::Edit { .. }) => self.page_insert(&text),
            _ => {}
        }
    }
    /// `BookSignScreen.keyPressed` on the title box.
    pub(in crate::game::client_side) fn book_title_key(&mut self, key: KeyCode) -> bool {
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let Some(BookScreen::Edit {
            title, title_edit, ..
        }) = self.client.book.as_mut()
        else {
            return false;
        };
        if !title_edit.key(key, ctrl, shift) {
            return false;
        }
        title.clone_from(&title_edit.value);
        true
    }
    pub(in crate::game::client_side) fn book_title_insert(&mut self, text: &str) {
        let Some(BookScreen::Edit {
            title, title_edit, ..
        }) = self.client.book.as_mut()
        else {
            return;
        };
        title_edit.insert_text(text);
        title.clone_from(&title_edit.value);
    }
    /// `BookEditScreen.keyPressed` on the page, apart from the line moves.
    pub(in crate::game::client_side) fn book_page_key(&mut self, key: KeyCode) -> bool {
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let before = match self.client.book.as_ref() {
            Some(BookScreen::Edit { page_edit, .. }) => Some((
                page_edit.value.clone(),
                page_edit.cursor,
                page_edit.highlight,
            )),
            _ => return false,
        };
        let handled = if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
            page_edit.key(key, ctrl, shift)
        } else {
            false
        };
        if handled {
            self.page_keep(before);
        }
        handled
    }
    /// One character at a time, so a page stops at the last character that fits.
    pub(in crate::game::client_side) fn page_insert(&mut self, text: &str) {
        for c in text.chars() {
            self.page_change(|edit| edit.insert_text(&c.to_string()));
        }
    }
    /// Ctrl+A/C/V/X: the title box while signing, else the page.
    pub(in crate::game::client_side) fn book_shortcut(
        &mut self,
        shortcut: sunrise_client::clipboard::EditShortcut,
    ) {
        let signing = matches!(
            self.client.book,
            Some(BookScreen::Edit { signing: true, .. })
        );
        if signing {
            if let Some(BookScreen::Edit {
                title, title_edit, ..
            }) = self.client.book.as_mut()
            {
                title_edit.shortcut(shortcut, &mut self.client.clipboard.0);
                title.clone_from(&title_edit.value);
            }
            return;
        }
        let before = match self.client.book.as_ref() {
            Some(BookScreen::Edit { page_edit, .. }) => Some((
                page_edit.value.clone(),
                page_edit.cursor,
                page_edit.highlight,
            )),
            _ => None,
        };
        if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
            page_edit.shortcut(shortcut, &mut self.client.clipboard.0);
        }
        self.page_keep(before);
    }
    /// Edit the page box, then keep it only within 14 lines and 1024 characters.
    pub(in crate::game::client_side) fn page_change(
        &mut self,
        f: impl FnOnce(&mut ui::EditBox),
    ) -> bool {
        let before = match self.client.book.as_ref() {
            Some(BookScreen::Edit { page_edit, .. }) => Some((
                page_edit.value.clone(),
                page_edit.cursor,
                page_edit.highlight,
            )),
            _ => return false,
        };
        let ran = if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
            f(page_edit);
            true
        } else {
            false
        };
        ran && self.page_keep(before)
    }
    /// Write the page, or put the previous text back when it no longer fits.
    pub(in crate::game::client_side) fn page_keep(
        &mut self,
        before: Option<(String, usize, usize)>,
    ) -> bool {
        let next = match self.client.book.as_ref() {
            Some(BookScreen::Edit { page_edit, .. }) => page_edit.value.clone(),
            _ => return false,
        };
        let ok = next.chars().count() <= PAGE_CHARS && self.book_lines(&next).len() <= LINES;
        let Some(BookScreen::Edit {
            pages,
            page,
            page_edit,
            ..
        }) = self.client.book.as_mut()
        else {
            return false;
        };
        if ok {
            pages[*page].clone_from(&page_edit.value);
        } else if let Some((value, cursor, highlight)) = before {
            page_edit.value = value;
            page_edit.cursor = cursor;
            page_edit.highlight = highlight;
        }
        ok
    }
    /// ArrowUp and ArrowDown move a visual line, keeping the column.
    pub(in crate::game::client_side) fn book_move_line(&mut self, dir: isize) {
        let shift = self.has_shift_down();
        let (text, cursor) = match self.client.book.as_ref() {
            Some(BookScreen::Edit {
                pages,
                page,
                page_edit,
                ..
            }) => (pages[*page].clone(), page_edit.cursor),
            _ => return,
        };
        let rows = self.page_rows(&text);
        let (x, line) = self.caret_on(&text, &rows, cursor);
        let next = (line as isize + dir).clamp(0, rows.len() as isize - 1) as usize;
        let (start, end) = rows[next];
        let chars: Vec<char> = text.chars().collect();
        let slice: String = chars[start..end].iter().collect();
        // MultilineTextField.LINE_SEEK_PIXEL_BIAS: vanilla biases this seek, not the drawn caret.
        let pos = start + self.client.gui.atlas.font.fit(&slice, x + 2.0);
        if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
            page_edit.set_cursor(pos, shift);
        }
    }
    /// Home and End: the start or end of the visual line holding the caret
    /// (`getCursorLineView`'s beginIndex and endIndex).
    pub(in crate::game::client_side) fn book_line_edge(&mut self, end: bool) {
        let shift = self.has_shift_down();
        let (text, cursor) = match self.client.book.as_ref() {
            Some(BookScreen::Edit {
                pages,
                page,
                page_edit,
                ..
            }) => (pages[*page].clone(), page_edit.cursor),
            _ => return,
        };
        let rows = self.page_rows(&text);
        let (_, line) = self.caret_on(&text, &rows, cursor);
        let (start, stop) = rows[line];
        if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
            page_edit.set_cursor(if end { stop } else { start }, shift);
        }
    }
    /// Wrapped rows of a page as character ranges, the same breaks as `font.split`.
    pub(in crate::game::client_side) fn page_rows(&self, page: &str) -> Vec<(usize, usize)> {
        let font = &self.client.gui.atlas.font;
        let chars: Vec<char> = page.chars().collect();
        let mut rows = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut end = chars.len();
            let mut next = chars.len();
            let mut last_space = None;
            for i in start..chars.len() {
                if chars[i] == '\n' {
                    end = i;
                    next = i + 1;
                    break;
                }
                if chars[i] == ' ' {
                    last_space = Some(i);
                }
                let slice: String = chars[start..=i].iter().collect();
                if font.width(&slice) > 114.0 {
                    match last_space {
                        Some(s) if s > start => {
                            end = s;
                            next = s + 1;
                        }
                        _ => {
                            end = i.max(start + 1);
                            next = end;
                        }
                    }
                    break;
                }
            }
            rows.push((start, end));
            start = next;
            if start == chars.len() && chars.last() == Some(&'\n') {
                rows.push((chars.len(), chars.len()));
            }
        }
        if rows.is_empty() {
            rows.push((0, 0));
        }
        rows
    }
    /// GUI x of the caret and the row it sits on.
    pub(in crate::game::client_side) fn caret_on(
        &self,
        page: &str,
        rows: &[(usize, usize)],
        cursor: usize,
    ) -> (f32, usize) {
        let chars: Vec<char> = page.chars().collect();
        for (i, &(start, end)) in rows.iter().enumerate() {
            if cursor <= end || i + 1 == rows.len() {
                let take = cursor.saturating_sub(start).min(end.saturating_sub(start));
                let prefix: String = chars.iter().skip(start).take(take).collect();
                return (self.client.gui.atlas.font.width(&prefix), i);
            }
        }
        (0.0, 0)
    }
    pub(in crate::game::client_side) fn page_index_at(
        &self,
        page: &str,
        local_x: f32,
        local_y: f32,
    ) -> usize {
        let rows = self.page_rows(page);
        let line = (local_y / 9.0).floor().max(0.0) as usize;
        let line = line.min(rows.len().saturating_sub(1));
        let (start, end) = rows[line];
        let chars: Vec<char> = page.chars().collect();
        let slice: String = chars[start..end].iter().collect();
        start + self.client.gui.atlas.font.fit(&slice, local_x.max(0.0))
    }
    /// A click on the centred title (`EditBox.onClick`).
    pub(in crate::game::client_side) fn book_title_click(&mut self, field_x: f32, mx: f32) {
        let shift = self.has_shift_down();
        let Some(BookScreen::Edit {
            title, title_edit, ..
        }) = self.client.book.as_mut()
        else {
            return;
        };
        let tx = field_x + ((114.0 - self.client.gui.atlas.font.width(title)) / 2.0).floor();
        let rel = (mx - tx).max(0.0);
        let pos = self.client.gui.atlas.font.fit(title, rel);
        title_edit.set_cursor(pos, shift);
    }
    /// A click on the open page (`MultiLineEditBox.onClick`): a double click
    /// selects the word before the cursor and leaves the cursor where it is.
    pub(in crate::game::client_side) fn book_page_click(
        &mut self,
        origin_x: f32,
        origin_y: f32,
        mx: f32,
        my: f32,
        double: bool,
    ) {
        if double {
            if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
                page_edit.select_word_at_cursor();
            }
            return;
        }
        let shift = self.has_shift_down();
        let text = match self.client.book.as_ref() {
            Some(BookScreen::Edit { pages, page, .. }) => pages[*page].clone(),
            _ => return,
        };
        let pos = self.page_index_at(&text, mx - origin_x, my - origin_y);
        if let Some(BookScreen::Edit { page_edit, .. }) = self.client.book.as_mut() {
            page_edit.set_cursor(pos, shift);
        }
    }
    /// The page buttons (`pageForward` adds a page while editing).
    pub(in crate::game::client_side) fn turn_page(&mut self, forward: bool) {
        let mut lectern_page = None;
        match self.client.book.as_mut() {
            Some(BookScreen::Edit {
                pages,
                page,
                page_edit,
                ..
            }) => {
                let before = *page;
                if forward {
                    if *page + 1 >= pages.len() && pages.len() < MAX_PAGES {
                        pages.push(String::new());
                    }
                    if *page + 1 < pages.len() {
                        *page += 1;
                    }
                } else if *page > 0 {
                    *page -= 1;
                }
                if *page != before {
                    let text = pages[*page].clone();
                    page_edit.set_value(&text);
                }
            }
            Some(BookScreen::View {
                lectern: Some(key), ..
            }) => lectern_page = Some(*key),
            Some(BookScreen::View { pages, page, .. }) => {
                if forward && *page + 1 < pages.len() {
                    *page += 1;
                } else if !forward && *page > 0 {
                    *page -= 1;
                }
            }
            None => return,
        }
        // LecternScreen's page buttons click the menu's (1 back, 2 forward);
        // the page turns with the server's data slot and level event 1043.
        if let Some(key) = lectern_page {
            self.lectern_button(key, if forward { 2 } else { 1 });
            return;
        }
        self.play_audio("minecraft:item.book.page_turn", None, 1.0, 1.0);
    }
    /// `LecternBlock.openScreen`: reading the lectern's book.
    #[allow(dead_code)]
    pub(in crate::game) fn open_lectern(&mut self, pos: [i32; 3]) -> bool {
        assert!(
            !self.is_logical_server(),
            "open_lectern called on Server role"
        );
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Lectern(l)) = self.common.world.block_entities.get(&key) else {
            return false;
        };
        let Some(book) = &l.book else { return false };
        let pages = view_pages(book.data());
        let page = (l.page.max(0) as usize).min(pages.len().saturating_sub(1));
        self.open_book(BookScreen::View {
            pages,
            page,
            lectern: Some(key),
        });
        true
    }
    /// LecternScreen's Take Book: the menu's button 3. The book leaves the
    /// lectern, so the screen closes (the server closes the menu once the
    /// book is gone; a server's client closes at once rather than wait).
    pub(in crate::game::client_side) fn take_lectern_book(&mut self, key: (i32, i32, i32)) {
        if self.client.remote.is_none() {
            return;
        }
        self.lectern_button(key, 3);
        if self.may_build() && self.lectern_book_open() {
            self.close_book();
        }
    }
    /// A `LecternMenu` button of the open lectern screen: on a server it is
    /// sent (`handleInventoryButtonClick`); here the lectern's menu takes it
    /// with the player's inventory, and the screen follows the block entity.
    pub(in crate::game) fn lectern_button(&mut self, key: (i32, i32, i32), id: i32) {
        let _ = key;
        if self.client.remote.is_some() {
            let _ = self.remote_button_click(id);
        }
    }
    #[inline(always)]
    pub(in crate::game::client_side) fn lectern_book_open(&self) -> bool {
        matches!(
            self.client.book,
            Some(BookScreen::View {
                lectern: Some(_),
                ..
            })
        )
    }
    /// The `book` control command: a button of the open book or lectern
    /// screen (the page arrows, Take Book, Done), then its page.
    pub fn book_control(
        &mut self,
        action: Option<&str>,
    ) -> sunrise_game_data::Result<serde_json::Value> {
        let lectern = match &self.client.book {
            Some(BookScreen::View { lectern, .. }) => *lectern,
            Some(BookScreen::Edit { .. }) => None,
            None => return Err("no book is open".into()),
        };
        match action {
            None => {}
            Some("next") => self.turn_page(true),
            Some("back") => self.turn_page(false),
            Some("take") => match lectern {
                Some(key) => self.take_lectern_book(key),
                None => return Err("not a lectern".into()),
            },
            // The book and quill's buttons: Sign, then Sign and Close; Done
            // (which saves), then Cancel.
            Some("sign") => match self.client.book.as_mut() {
                Some(BookScreen::Edit {
                    signing: true,
                    title,
                    ..
                }) if !title.trim().is_empty() => self.save_book(true),
                Some(BookScreen::Edit { .. }) => self.set_book_signing(true),
                _ => return Err("not a book and quill".into()),
            },
            Some("done") => match self.client.book.as_mut() {
                Some(BookScreen::Edit { signing: true, .. }) => {
                    self.set_book_signing(false);
                }
                Some(BookScreen::Edit { .. }) => self.save_book(false),
                _ => self.close_book(),
            },
            Some(other) => return Err(format!("unknown book button {other}").into()),
        }
        Ok(match &self.client.book {
            Some(BookScreen::View {
                pages,
                page,
                lectern,
            }) => serde_json::json!({
                "page": page, "pages": pages.len(), "lectern": lectern.map(|k| [k.0, k.1, k.2]),
            }),
            Some(BookScreen::Edit { pages, page, .. }) => {
                serde_json::json!({ "page": page, "pages": pages.len(), "lectern": null })
            }
            None => serde_json::json!({ "closed": true }),
        })
    }
    /// A server's `minecraft:lectern` menu: LecternScreen, its book and
    /// page to come (`remote_lectern_book`, `remote_lectern_page`).
    pub(in crate::game::client_side) fn remote_open_lectern(&mut self, key: (i32, i32, i32)) {
        self.open_book(BookScreen::View {
            pages: Vec::new(),
            page: 0,
            lectern: Some(key),
        });
    }
    /// `LecternScreen.bookChanged`: the menu's book slot.
    pub(in crate::game::client_side) fn remote_lectern_book(&mut self, book: Option<ItemStack>) {
        if let Some(BookScreen::View {
            lectern: Some(_),
            pages,
            page,
        }) = self.client.book.as_mut()
        {
            *pages = view_pages(book.as_ref().and_then(ItemStack::data));
            *page = (*page).min(pages.len().saturating_sub(1));
        }
    }
    /// `LecternScreen.pageChanged`: the menu's page data slot.
    pub fn remote_lectern_page(&mut self, value: i32) {
        if let Some(BookScreen::View {
            lectern: Some(_),
            pages,
            page,
        }) = self.client.book.as_mut()
        {
            *page = (value.max(0) as usize).min(pages.len().saturating_sub(1));
        }
    }
    pub(in crate::game::client_side) fn book_click(&mut self, size: [u32; 2]) -> bool {
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let (w, left) = (b.width, ((b.width - 192.0) / 2.0).floor());
        let (mx, my) = self.client.cursor;
        // MouseHandler: a click within 250 ms of the last one on this screen is a double click.
        let now = std::time::Instant::now();
        let double = self
            .client
            .book_last_click
            .is_some_and(|at| now.duration_since(at).as_millis() < 250);
        self.client.book_last_click = Some(now);
        let inside =
            |x: f32, y: f32, bw: f32, bh: f32| mx >= x && my >= y && mx < x + bw && my < y + bh;
        let click = |game: &mut Self| game.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
        match &self.client.book {
            Some(BookScreen::Edit {
                signing: true,
                title,
                ..
            }) => {
                let blank = title.trim().is_empty();
                if inside(w / 2.0 - 100.0, 196.0, 98.0, 20.0) && !blank {
                    click(self);
                    self.save_book(true);
                    return true;
                } else if inside(w / 2.0 + 2.0, 196.0, 98.0, 20.0) {
                    click(self);
                    self.set_book_signing(false);
                    return true;
                } else if inside(left + 36.0, 50.0, 114.0, 9.0) {
                    self.book_title_click(left + 36.0, mx);
                    return true;
                }
            }
            Some(BookScreen::Edit { page, .. }) => {
                let page = *page;
                if inside(left + 116.0, 159.0, 23.0, 13.0) {
                    self.turn_page(true);
                    return true;
                } else if page > 0 && inside(left + 43.0, 159.0, 23.0, 13.0) {
                    self.turn_page(false);
                    return true;
                } else if inside(w / 2.0 - 100.0, 196.0, 98.0, 20.0) {
                    click(self);
                    self.set_book_signing(true);
                    return true;
                } else if inside(w / 2.0 + 2.0, 196.0, 98.0, 20.0) {
                    click(self);
                    self.save_book(false);
                    return true;
                } else if inside(left + 36.0, 32.0, 114.0, 14.0 * 9.0) {
                    self.book_page_click(left + 36.0, 32.0, mx, my, double);
                    return true;
                }
            }
            Some(BookScreen::View {
                pages,
                page,
                lectern,
            }) => {
                let (page, count, lectern) = (*page, pages.len(), *lectern);
                // The text's click event first, then the buttons.
                if let Some(link) = pages.get(page).and_then(|t| self.book_link_at(t, left)) {
                    self.book_follow_link(link, count, lectern);
                    return true;
                } else if page + 1 < count && inside(left + 116.0, 159.0, 23.0, 13.0) {
                    self.turn_page(true);
                    return true;
                } else if page > 0 && inside(left + 43.0, 159.0, 23.0, 13.0) {
                    self.turn_page(false);
                    return true;
                } else if let Some(key) = lectern.filter(|_| self.may_build()) {
                    if inside(w / 2.0 - 100.0, 196.0, 98.0, 20.0) {
                        click(self);
                        self.close_book();
                        return true;
                    } else if inside(w / 2.0 + 2.0, 196.0, 98.0, 20.0) {
                        click(self);
                        self.take_lectern_book(key);
                        return true;
                    }
                } else if inside((w - 200.0) / 2.0, 196.0, 200.0, 20.0) {
                    click(self);
                    self.close_book();
                    return true;
                }
            }
            None => {}
        }
        false
    }
    pub(in crate::game::client_side) fn draw_book(&mut self, batch: &mut HudBatch) {
        let mut hovered = None;
        let Some(book) = &self.client.book else {
            return;
        };
        let atlas = &self.client.gui.atlas;
        let w = batch.width;
        let (mx, my) = self.client.cursor;
        let hover =
            |x: f32, y: f32, bw: f32, bh: f32| mx >= x && my >= y && mx < x + bw && my < y + bh;
        self.render_transparent_background("book", batch);
        let left = ((w - 192.0) / 2.0).floor();
        let top = 2.0;
        if let Some(mut r) = atlas.sprites.get("gui/book").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 192.0 * du;
            r.v1 = r.v0 + 192.0 * dv;
            batch.rect_uv(left, top, 192.0, 192.0, r, u32::MAX);
        }
        // The page, or the signing title on it: a field to a controller, which opens its keyboard.
        if matches!(book, BookScreen::Edit { .. }) {
            widgets::target_as(
                [
                    left + BOOK_TEXT[0],
                    top + BOOK_TEXT[1],
                    BOOK_TEXT[2],
                    BOOK_TEXT[3],
                ],
                widgets::Kind::Field,
            );
        }
        let tr = |key: &str, fallback: &str| self.translate_with_fallback(key, fallback);
        let page_buttons = |batch: &mut HudBatch, back: bool, forward: bool| {
            for (show, x, name) in [
                (forward, 116.0, "page_forward"),
                (back, 43.0, "page_backward"),
            ] {
                if show {
                    let sprite = if hover(left + x, top + 157.0, 23.0, 13.0) {
                        format!("widget/{name}_highlighted")
                    } else {
                        format!("widget/{name}")
                    };
                    batch.sprite(atlas, &sprite, left + x, top + 157.0);
                }
            }
        };
        let indicator = |batch: &mut HudBatch, page: usize, count: usize| {
            // BookViewScreen.getPageNumberMessage: 26.3's en_us template is
            // positional ("Page %1$s of %2$s").
            let text = sunrise_protocol::text::format_java(
                &tr("book.pageIndicator", "Page %1$s of %2$s"),
                &[(page + 1).to_string(), count.max(1).to_string()],
            );
            let tw = HudBatch::text_width(atlas, &text);
            batch.text(atlas, &text, left + 148.0 - tw, top + 16.0, BLACK, false);
        };
        // PageButton: a button named for where it turns, there while it can;
        // the view adds the forward one first, the editor the back one.
        let page_narratables = |back: bool, forward: bool, view: bool| {
            let back = (back, 43.0, "book.page_button.previous");
            let forward = (forward, 116.0, "book.page_button.next");
            let order = if view {
                [forward, back]
            } else {
                [back, forward]
            };
            for (show, x, key) in order {
                if show {
                    widgets::narratable(|| {
                        let rect = [left + x, top + 157.0, 23.0, 13.0];
                        Narratable::button(screen_info::tr(key, &[]), rect, true)
                            .hovered(hover(rect[0], rect[1], rect[2], rect[3]))
                            .order(i32::from(view))
                    });
                }
            }
        };
        let button = |batch: &mut HudBatch, x: f32, bw: f32, label: &str, active: bool| {
            ui::button(
                batch,
                atlas,
                x,
                196.0,
                bw,
                20.0,
                &Text::literal(label),
                active,
                hover(x, 196.0, bw, 20.0),
            );
        };
        let action_button = |batch: &mut HudBatch, action, x: f32, bw: f32, label: &str| {
            ui::action_button(
                batch,
                atlas,
                action,
                x,
                196.0,
                bw,
                20.0,
                &Text::literal(label),
                true,
                hover(x, 196.0, bw, 20.0),
            );
        };
        match book {
            BookScreen::Edit {
                signing: true,
                title,
                title_edit,
                ..
            } => {
                // BookSignScreen: its title, and the title box focused.
                widgets::begin_screen(|| {
                    ScreenInfo::new("book/sign", screen_info::tr("book.sign.title", &[]))
                });
                widgets::narratable(|| {
                    let rect = [((w - 114.0) / 2.0).floor() - 3.0, 50.0, 114.0, 20.0];
                    Narratable::edit_box(
                        screen_info::tr("book.sign.titlebox", &[]),
                        title_edit.value.clone(),
                        rect,
                    )
                    .focused(title_edit.focused)
                });
                let edit = tr("book.editTitle", "Enter Book Title:");
                let tw = HudBatch::text_width(atlas, &edit);
                batch.text(
                    atlas,
                    &edit,
                    left + 36.0 + (114.0 - tw) / 2.0,
                    34.0,
                    BLACK,
                    false,
                );
                // The title box: an unbordered, centred EditBox of 114 with
                // black text and no shadow; the value alone is centred, the
                // cursor follows it.
                let value = &title_edit.value;
                let x = left + 36.0;
                let tx = x + ((114.0 - HudBatch::text_width(atlas, value)) / 2.0).floor();
                batch.text(atlas, value, tx, 50.0, BLACK, false);
                if title_edit.cursor != title_edit.highlight {
                    let (a, b) = (
                        title_edit.cursor.min(title_edit.highlight),
                        title_edit.cursor.max(title_edit.highlight),
                    );
                    let pre: String = value.chars().take(a).collect();
                    let sel: String = value.chars().skip(a).take(b - a).collect();
                    let sx = tx + HudBatch::text_width(atlas, &pre);
                    batch.text_highlight(
                        atlas,
                        (sx, 49.0, HudBatch::text_width(atlas, &sel), 11.0),
                        true,
                    );
                }
                if title_edit.cursor_visible() {
                    let pre: String = value.chars().take(title_edit.cursor).collect();
                    batch.fill(
                        atlas,
                        tx + HudBatch::text_width(atlas, &pre),
                        49.0,
                        1.0,
                        9.0,
                        BLACK,
                    );
                }
                // BookSignScreen: `owner.getName()`, the account's name on a server.
                let owner = self
                    .client
                    .remote
                    .as_ref()
                    .map_or(super::chat_common::PLAYER_NAME, |r| r.profile_name.as_str());
                let by = tr("book.byAuthor", "by %1$s").replace("%1$s", owner);
                let tw = HudBatch::text_width(atlas, &by);
                batch.text(
                    atlas,
                    &by,
                    left + 36.0 + (114.0 - tw) / 2.0,
                    60.0,
                    rgba(64, 64, 64, 255),
                    false,
                );
                let warning = tr(
                    "book.finalizeWarning",
                    "Note! When you sign the book, it will no longer be editable.",
                );
                for (i, line) in atlas
                    .font
                    .split(&Text::literal(&warning), 114.0)
                    .iter()
                    .enumerate()
                {
                    batch.text_styled(
                        atlas,
                        line,
                        left + 36.0,
                        82.0 + i as f32 * 9.0,
                        BLACK,
                        false,
                    );
                }
                button(
                    batch,
                    w / 2.0 - 100.0,
                    98.0,
                    &tr("book.finalizeButton", "Sign and Close"),
                    !title.trim().is_empty(),
                );
                action_button(
                    batch,
                    widgets::Action::Cancel,
                    w / 2.0 + 2.0,
                    98.0,
                    &tr("gui.cancel", "Cancel"),
                );
            }
            BookScreen::Edit {
                pages,
                page,
                page_edit,
                ..
            } => {
                widgets::begin_screen(|| {
                    screen_info::book_edit(*page, pages.len(), &screen_info::tr)
                });
                // The page's MultiLineEditBox (no message), focused, then the
                // page buttons.
                widgets::narratable(|| {
                    let rect = [((w - 114.0) / 2.0).floor() - 8.0, 28.0, 122.0, 134.0];
                    Narratable::edit_box("", pages[*page].clone(), rect).focused(page_edit.focused)
                });
                page_narratables(*page > 0, true, false);
                indicator(batch, *page, pages.len());
                let text = &pages[*page];
                let lines = self.book_lines(text);
                for (i, line) in lines.iter().take(LINES).enumerate() {
                    batch.text_styled(
                        atlas,
                        line,
                        left + 36.0,
                        top + 30.0 + i as f32 * 9.0,
                        BLACK,
                        false,
                    );
                }
                let rows = self.page_rows(text);
                let (sel_a, sel_b) = (
                    page_edit.cursor.min(page_edit.highlight),
                    page_edit.cursor.max(page_edit.highlight),
                );
                if sel_a != sel_b {
                    let chars: Vec<char> = text.chars().collect();
                    for (i, &(start, end)) in rows.iter().enumerate().take(LINES) {
                        let from = sel_a.max(start);
                        let to = sel_b.min(end);
                        if from >= to {
                            continue;
                        }
                        let pre: String = chars[start..from].iter().collect();
                        let sel: String = chars[from..to].iter().collect();
                        batch.text_highlight(
                            atlas,
                            (
                                left + 36.0 + atlas.font.width(&pre),
                                top + 29.0 + i as f32 * 9.0,
                                atlas.font.width(&sel),
                                9.0,
                            ),
                            true,
                        );
                    }
                }
                if page_edit.cursor_visible() {
                    let (x, row) = self.caret_on(text, &rows, page_edit.cursor);
                    batch.fill(
                        atlas,
                        left + 36.0 + x,
                        top + 29.0 + row as f32 * 9.0,
                        1.0,
                        9.0,
                        BLACK,
                    );
                }
                page_buttons(batch, *page > 0, true);
                button(
                    batch,
                    w / 2.0 - 100.0,
                    98.0,
                    &tr("book.signButton", "Sign"),
                    true,
                );
                action_button(
                    batch,
                    widgets::Action::Done,
                    w / 2.0 + 2.0,
                    98.0,
                    &tr("gui.done", "Done"),
                );
            }
            BookScreen::View {
                pages,
                page,
                lectern,
            } => {
                widgets::begin_screen(|| {
                    let text = pages.get(*page).map(Text::plain).unwrap_or_default();
                    screen_info::book_view(*page, pages.len(), &text, &screen_info::tr)
                });
                // createMenuControls comes before createPageControlButtons
                // (forward, then back).
                page_narratables(*page > 0, *page + 1 < pages.len(), true);
                indicator(batch, *page, pages.len());
                if let Some(text) = pages.get(*page) {
                    for (i, line) in self.page_lines(text).iter().take(LINES).enumerate() {
                        batch.text_styled(
                            atlas,
                            line,
                            left + 36.0,
                            top + 30.0 + i as f32 * 9.0,
                            BLACK,
                            false,
                        );
                    }
                }
                page_buttons(batch, *page > 0, *page + 1 < pages.len());
                // LecternScreen.createMenuControls: Done and Take Book only with
                // build permission, else BookViewScreen's wide Done.
                if lectern.is_some() && self.may_build() {
                    action_button(
                        batch,
                        widgets::Action::Done,
                        w / 2.0 - 100.0,
                        98.0,
                        &tr("gui.done", "Done"),
                    );
                    button(
                        batch,
                        w / 2.0 + 2.0,
                        98.0,
                        &tr("lectern.take_book", "Take Book"),
                        true,
                    );
                } else {
                    action_button(
                        batch,
                        widgets::Action::Done,
                        (w - 200.0) / 2.0,
                        200.0,
                        &tr("gui.done", "Done"),
                    );
                }
                // HoveredTextEffects.TOOLTIP_AND_CURSOR: the hovered words' hover
                // event, drawn as the chat draws it (componentHoverEffect).
                hovered = pages.get(*page).and_then(|t| self.book_style_at(t, left));
            }
        }
        if let Some(book_id) = self.client.book.as_ref().map(BookScreen::id) {
            self.draw_overlay_mounted_widgets(batch, book_id);
        }
        if let Some(style) = hovered {
            if let Some(cursor) = link_cursor(&style) {
                sunrise_client::widgets::request_cursor(cursor);
            }
            self.component_hover_effect(batch, &style, (mx, my));
        }
    }
}
#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod title_box_tests {
    use super::book_title_box;

    /// `BookSignScreen` sets its 15 character title box centred and unbordered.
    #[test]
    fn the_sign_title_box_is_centred() {
        let edit = book_title_box();
        assert!(edit.centered);
        assert!(!edit.bordered);
        assert_eq!(edit.max_length, 15);
    }
}
#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod lectern_books_tag_tests {}
impl BookScreen {
    /// `BookViewScreen.isPauseScreen`: the class inherits `Screen`'s true, so
    /// a book read in hand stops a single-player world. `LecternScreen`
    /// overrides it false, so the same pages read on a lectern leave the
    /// world ticking (its menu is an `AbstractContainerScreen`, which does
    /// not pause either).
    pub(in crate::game::client_side) fn pauses_world(&self) -> bool {
        !matches!(
            self,
            Self::View {
                lectern: Some(_),
                ..
            }
        )
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod pause_tests {
    use super::BookScreen;
    use sunrise_client::ui;
    use sunrise_game_data::text::Text;

    fn view(lectern: Option<(i32, i32, i32)>) -> BookScreen {
        BookScreen::View {
            pages: Vec::<Text>::new(),
            page: 0,
            lectern,
        }
    }

    #[test]
    fn a_lecterns_book_does_not_pause_the_world() {
        // BookViewScreen keeps Screen.isPauseScreen's true; LecternScreen
        // overrides it false although both read the same pages.
        assert!(view(None).pauses_world());
        assert!(!view(Some((10, 64, -3))).pauses_world());
    }

    #[test]
    fn a_book_being_edited_or_signed_pauses() {
        // BookEditScreen and BookSignScreen do not override isPauseScreen.
        let edit = |signing| BookScreen::Edit {
            slot: 0,
            pages: Vec::new(),
            page: 0,
            signing,
            title: String::new(),
            title_edit: ui::EditBox::new(0.0, 0.0, 0.0, 0.0),
            page_edit: ui::EditBox::new(0.0, 0.0, 0.0, 0.0),
        };
        assert!(edit(false).pauses_world());
        assert!(edit(true).pauses_world());
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod sign_tests {
    use super::{BookScreen, book_page_box, book_title_box};

    /// `BookSignScreen`'s constructor sets its title box to `titleValue`, what
    /// Cancel kept; reopening the sign screen shows the title typed before,
    /// not an empty box beside a stale `title`.
    #[test]
    fn the_sign_screen_opens_with_the_title_it_kept() {
        let mut book = BookScreen::Edit {
            slot: 0,
            pages: vec!["page".into()],
            page: 0,
            signing: false,
            title: "My Book".into(),
            title_edit: book_title_box(),
            page_edit: book_page_box(),
        };
        assert_eq!(book_title_value(&book), "");
        book.begin_signing();
        let BookScreen::Edit {
            signing,
            title,
            title_edit,
            ..
        } = &book
        else {
            panic!("the book stays open");
        };
        assert!(signing);
        assert_eq!(title.as_str(), "My Book");
        assert_eq!(title_edit.value, "My Book");
        assert_eq!(title_edit.cursor, 7);
    }

    fn book_title_value(book: &BookScreen) -> String {
        match book {
            BookScreen::Edit { title_edit, .. } => title_edit.value.clone(),
            BookScreen::View { .. } => String::new(),
        }
    }
}

pub(in crate::game::client_side) fn link_cursor(
    style: &sunrise_game_data::text::Style,
) -> Option<sunrise_client::widgets::Cursor> {
    style
        .click
        .is_some()
        .then_some(sunrise_client::widgets::Cursor::PointingHand)
}
#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod link_cursor_tests {
    use super::link_cursor;
    use sunrise_client::widgets::Cursor;
    use sunrise_game_data::text::{ClickEvent, Style, Text};

    /// `HoveredTextEffects.TOOLTIP_AND_CURSOR`: a clickable word asks for the
    /// pointing hand; a hover event alone only shows its tooltip.
    #[test]
    fn only_a_clickable_book_word_asks_for_the_hand() {
        let link = Style {
            click: Some(ClickEvent::OpenUrl("https://example.com".into())),
            ..Style::default()
        };
        let hover_only = Style {
            hover: Some(std::sync::Arc::new(Text::literal("tip"))),
            ..Style::default()
        };
        assert_eq!(link_cursor(&link), Some(Cursor::PointingHand));
        assert_eq!(link_cursor(&hover_only), None);
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
