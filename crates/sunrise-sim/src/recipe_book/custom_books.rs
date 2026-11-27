//! The open and filter flags of mods' own recipe books (#2415), beside the four vanilla
//! `RecipeBookType` pairs of [`Settings`](super::Settings): one map from a book's namespaced id
//! to its two flags. A book missing from it reads `(false, false)`. The map is saved in the
//! player's `recipeBook` compound under [`EXTENSION`] unless explicit flat fields were
//! declared. Extension entries are kept while their mod is absent, so reinstalling the mod
//! finds them again.
use super::placement_sync::ResourceId;
use std::collections::BTreeMap;
use sunrise_core::nbt::Tag;

/// The key of the map inside `recipeBook`.
pub use mod_api::item::recipe_book::saved_fields::EXTENSION;

/// A mod book's two flags, as each vanilla book has: whether its screen shows the book
/// (`open`), and whether it lists only what can be crafted (`filtering`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookFlags {
    pub open: bool,
    pub filtering: bool,
}

/// The flags of mods' books, by id, in id order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomBooks {
    books: BTreeMap<String, BookFlags>,
}

impl CustomBooks {
    /// The flags of book `id`: `(false, false)` for a book that has none.
    pub fn get(&self, id: &str) -> BookFlags {
        self.stored(id).unwrap_or_default()
    }
    /// The exact saved entry, distinguishing an explicit closed book from an absent one.
    pub fn stored(&self, id: &str) -> Option<BookFlags> {
        self.books.get(id).copied()
    }
    /// Restore a journaled entry, including absence, without publishing a settings change.
    pub fn restore(&mut self, id: String, flags: Option<BookFlags>) {
        match flags {
            Some(flags) => self.books.insert(id, flags),
            None => self.books.remove(&id),
        };
    }
    /// Sets the validated book's flags; returns whether they changed.
    pub fn set(&mut self, id: &ResourceId, flags: BookFlags) -> bool {
        if let Some(old) = self.books.get_mut(id.as_str()) {
            return std::mem::replace(old, flags) != flags;
        }
        if flags == BookFlags::default() {
            return false;
        }
        self.books.insert(id.as_str().to_owned(), flags);
        true
    }
    /// Every book with saved flags and its flags, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, BookFlags)> {
        self.books.iter().map(|(id, flags)| (id.as_str(), *flags))
    }
    /// How many books have saved flags.
    pub fn len(&self) -> usize {
        self.books.len()
    }
    /// Whether no book has saved flags.
    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }
    /// The map saved in `recipe_book` (the player's `recipeBook` compound). A malformed
    /// entry is skipped independently: an invalid id, a value that is not a compound,
    /// or a flag that is not the byte 0 or 1. Valid entries need not be installed to load.
    pub fn read(recipe_book: Option<&Tag>) -> Self {
        let Some(extension) = recipe_book.and_then(|tag| tag.get(EXTENSION)) else {
            return Self::default();
        };
        let Tag::Compound(entries) = extension else {
            log::warn!("Sunrise: {EXTENSION} is not a compound; mods' recipe books read as closed");
            return Self::default();
        };
        let mut books = BTreeMap::new();
        for (id, value) in entries {
            let id = id.to_string_lossy();
            match read_flags(value) {
                Some(flags) if mod_api::Identifier::parse_namespaced(&id).is_some() => {
                    books.insert(id, flags);
                }
                _ => log::warn!("Sunrise: skipped the malformed {EXTENSION} entry {id:?}"),
            }
        }
        Self { books }
    }
    /// Read explicitly declared flat fields instead of extension entries for those books.
    pub(super) fn read_fields(&mut self, tag: Option<&Tag>, fields: &[(String, (String, String))]) {
        for (id, (open, filtering)) in fields {
            let flag =
                |name: &str| matches!(tag.and_then(|t| t.get(name)), Some(Tag::Byte(b)) if *b != 0);
            let flags = BookFlags {
                open: flag(open),
                filtering: flag(filtering),
            };
            if flags == BookFlags::default() {
                self.books.remove(id);
            } else {
                self.books.insert(id.clone(), flags);
            }
        }
    }

    /// The [`EXTENSION`] NBT compound to save, with both flags of each book
    /// written (including zero). Returns `None` when no book has flags.
    pub fn write(&self) -> Option<Tag> {
        self.write_excluding(&[])
    }

    pub(super) fn write_excluding(&self, fields: &[(String, (String, String))]) -> Option<Tag> {
        let entries: Vec<_> = self
            .books
            .iter()
            .filter(|(id, _)| !fields.iter().any(|(named, _)| named == *id))
            .map(|(id, flags)| {
                let entry = Tag::compound([
                    ("open", Tag::Byte(i8::from(flags.open))),
                    ("filtering", Tag::Byte(i8::from(flags.filtering))),
                ]);
                (id.as_str(), entry)
            })
            .collect();
        (!entries.is_empty()).then(|| Tag::compound(entries))
    }
}

/// A book's saved flags: a compound whose `open` and `filtering` are each the byte 0 or 1.
fn read_flags(value: &Tag) -> Option<BookFlags> {
    let flag = |key: &str| match value.get(key)? {
        Tag::Byte(0) => Some(false),
        Tag::Byte(1) => Some(true),
        _ => None,
    };
    Some(BookFlags {
        open: flag("open")?,
        filtering: flag("filtering")?,
    })
}

#[cfg(test)]
#[path = "custom_books_tests.rs"]
mod tests;
