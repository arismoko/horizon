use std::cell::{Cell, RefCell};
use sunrise_items::items::ItemStack;

thread_local! {
    static CAPTURE: Cell<bool> = const { Cell::new(false) };
    pub static SEEN: RefCell<Vec<(String, Option<ItemStack>)>> = const { RefCell::new(Vec::new()) };
}
pub fn record(name: &str, stack: Option<&ItemStack>) {
    if CAPTURE.get() {
        SEEN.with_borrow_mut(|seen| seen.push((name.to_owned(), stack.cloned())));
    }
}
pub fn counts() -> [usize; 3] {
    SEEN.with_borrow(|seen| {
        [
            "minecraft:default_block_use",
            "minecraft:item_used_on_block",
            "minecraft:any_block_use",
        ]
        .map(|name| seen.iter().filter(|(n, _)| n == name).count())
    })
}

pub struct Recording;

impl Recording {
    pub fn start() -> Self {
        SEEN.with_borrow_mut(Vec::clear);
        CAPTURE.set(true);
        Self
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        CAPTURE.set(false);
        SEEN.with_borrow_mut(Vec::clear);
    }
}
