use sunrise_items::item_data::WrittenBook;

impl crate::game::CommonGame<'_> {
    /// Write pages into the book and quill at `slot`, or sign it into a written book by `author`.
    pub fn write_book_with_author(
        &mut self,
        slot: usize,
        pages: Vec<String>,
        title: Option<String>,
        author: String,
    ) {
        let Some(stack) = self
            .stack_in_mut(slot)
            .as_mut()
            .filter(|s| &*s.item == "minecraft:writable_book")
        else {
            return;
        };
        if let Some(title) = title {
            // `signBook` transmutes the carried stack so name and other components stay intact.
            let mut written = stack.transmute("minecraft:written_book", stack.count);
            let data = written.data_mut();
            data.writable_pages = Vec::new();
            data.writable_filtered = Vec::new();
            data.written = Some(WrittenBook {
                title,
                author,
                generation: 0,
                pages,
                page_tags: Vec::new(),
                // Literal pages from `signBook` (`new WrittenBookContent(..., true)`) are resolved.
                resolved: true,
            });
            *stack = written;
        } else {
            let data = stack.data_mut();
            // `updateBookContents`: unfiltered `Filterable` pages while no text filter runs.
            data.writable_filtered = Vec::new();
            data.writable_pages = if pages.iter().all(String::is_empty) {
                Vec::new()
            } else {
                pages
            };
            stack.tidy();
        }
    }
}
