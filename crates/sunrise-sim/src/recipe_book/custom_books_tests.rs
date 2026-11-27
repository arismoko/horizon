//! The design's storage, persistence, copy and replace acceptance cases (#2415).
use super::*;
use crate::recipe_book::{BookType, Settings};
use sunrise_core::nbt::{self, JavaString, Limits, NamedTag};

const POT: &str = "demo:cooking_pot";
const OVEN: &str = "demo:oven";
const OPEN: BookFlags = BookFlags {
    open: true,
    filtering: false,
};
const FILTERING: BookFlags = BookFlags {
    open: false,
    filtering: true,
};

/// A saved `recipeBook` compound: the vanilla lists, the furnace book open and `extension`.
fn saved(extension: Option<Tag>) -> Tag {
    let mut entries = vec![
        ("recipes", Tag::List(vec![])),
        ("toBeDisplayed", Tag::List(vec![])),
        ("isFurnaceGuiOpen", Tag::Byte(1)),
    ];
    entries.extend(extension.map(|tag| (EXTENSION, tag)));
    Tag::compound(entries)
}

fn entry(open: Tag, filtering: Tag) -> Tag {
    Tag::compound([("open", open), ("filtering", filtering)])
}

fn id(text: &str) -> ResourceId {
    ResourceId::try_from(text.to_owned()).unwrap()
}

#[test]
fn a_fresh_book_reads_closed_and_one_book_leaves_another_alone() {
    let mut books = CustomBooks::default();
    assert_eq!(books.get(POT), BookFlags::default());
    assert_eq!(books.get(OVEN), BookFlags::default());
    assert!(books.set(&id(POT), OPEN));
    assert_eq!(books.get(POT), OPEN);
    assert_eq!(books.get(OVEN), BookFlags::default());
    assert!(!books.set(&id(POT), OPEN), "an unchanged pair");
    assert!(!books.set(&id(OVEN), BookFlags::default()));
    assert_eq!(books.len(), 1, "a default pair adds no entry");
}

#[test]
fn a_saved_book_loads_exactly_and_the_vanilla_pairs_stay() {
    let mut settings = Settings::read(Some(&saved(None)));
    settings.filtering[BookType::Crafting.index()] = true;
    settings.custom.set(&id(POT), OPEN);
    let vanilla = (settings.open, settings.filtering);
    let loaded = Settings::read(Some(&settings.write()));
    assert_eq!(loaded.custom.get(POT), OPEN);
    assert_eq!((loaded.open, loaded.filtering), vanilla);
    assert_eq!(loaded.custom, settings.custom);
}

#[test]
fn both_flags_are_written_for_each_book_including_zero() {
    let mut books = CustomBooks::default();
    books.set(&id(POT), FILTERING);
    books.set(&id("demo:anvil"), OPEN);
    let Some(Tag::Compound(written)) = books.write() else {
        panic!("an extension");
    };
    let ids: Vec<_> = books.iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["demo:anvil", POT]);
    assert_eq!(written.len(), 2);
    assert_eq!(
        written.get(&JavaString::from("demo:anvil")),
        Some(&entry(Tag::Byte(1), Tag::Byte(0)))
    );
    assert_eq!(
        written.get(&JavaString::from(POT)),
        Some(&entry(Tag::Byte(0), Tag::Byte(1)))
    );
    assert_eq!(
        CustomBooks::default().write(),
        None,
        "no books, no extension"
    );
}

#[test]
fn a_book_without_its_mod_is_kept_through_save_and_load() {
    // Nothing is registered here: storage keeps every well-formed entry, so the pot's flags
    // are back when its mod is installed again.
    let tag = saved(Some(Tag::compound([(
        POT,
        entry(Tag::Byte(1), Tag::Byte(0)),
    )])));
    let loaded = Settings::read(Some(&tag));
    assert_eq!(loaded.custom.get(POT), OPEN);
    let again = Settings::read(Some(&loaded.write()));
    assert_eq!(again.custom.get(POT), OPEN);
}

#[test]
fn a_copy_is_independent_and_a_replace_drops_what_only_the_destination_had() {
    let mut a = Settings::default();
    a.custom.set(&id(POT), OPEN);
    let b = a.clone();
    a.custom.set(&id(POT), FILTERING);
    assert_eq!(b.custom.get(POT), OPEN, "the copy is a value");
    let mut destination = Settings::default();
    destination.custom.set(
        &id(OVEN),
        BookFlags {
            open: true,
            filtering: true,
        },
    );
    destination = b.clone();
    assert_eq!(destination.custom.iter().collect::<Vec<_>>(), [(POT, OPEN)]);
    assert_eq!(destination.custom.get(OVEN), BookFlags::default());
}

#[test]
fn a_book_without_the_extension_reads_empty() {
    assert!(Settings::read(Some(&saved(None))).custom.is_empty());
    assert!(Settings::read(None).custom.is_empty());
    assert!(
        Settings::default().write().get(EXTENSION).is_none(),
        "a vanilla-only book writes what it wrote before"
    );
}

#[test]
fn lists_that_do_not_decode_keep_the_mods_flags() {
    let tag = Tag::compound([(
        EXTENSION,
        Tag::compound([(POT, entry(Tag::Byte(1), Tag::Byte(0)))]),
    )]);
    let loaded = Settings::read(Some(&tag));
    assert!(loaded.recipes.is_empty(), "no lists: vanilla reads nothing");
    assert_eq!(loaded.custom.get(POT), OPEN);
}

#[test]
fn a_malformed_entry_is_skipped_and_the_rest_read() {
    let extension = Tag::compound([
        (POT, entry(Tag::Byte(1), Tag::Byte(0))),
        ("demo:two", entry(Tag::Byte(2), Tag::Byte(0))),
        ("demo:int", entry(Tag::Int(1), Tag::Byte(0))),
        ("demo:half", Tag::compound([("open", Tag::Byte(1))])),
        ("demo:flat", Tag::Byte(1)),
        ("Demo:Upper", entry(Tag::Byte(1), Tag::Byte(1))),
        ("no_namespace", entry(Tag::Byte(1), Tag::Byte(1))),
    ]);
    let books = CustomBooks::read(Some(&saved(Some(extension))));
    assert_eq!(books.iter().collect::<Vec<_>>(), [(POT, OPEN)]);
}

#[test]
fn large_saved_books_round_trip_with_a_malformed_sibling() {
    // Every book is from an absent mod. Their names alone exceed the old
    // extension byte budget, without any padding or malformed bytes on disk.
    let prefix = format!("absent:{}", "cook/".repeat(100));
    let ids: Vec<_> = (0..1_100).map(|i| format!("{prefix}{i:04}")).collect();
    let initial = Tag::compound(
        ids.iter()
            .map(|id| (id.as_str(), entry(Tag::Byte(1), Tag::Byte(0))))
            .chain([(
                "absent:malformed",
                Tag::compound([("nested", Tag::compound([("flag", Tag::Byte(1))]))]),
            )]),
    );
    let player_bytes = |book| {
        nbt::write_named(
            &NamedTag {
                name: "".into(),
                tag: Tag::compound([("recipeBook", book)]),
            },
            Limits::saved(),
        )
        .unwrap()
    };
    let raw = player_bytes(saved(Some(initial)));
    assert!(raw.len() > 512 * 1024);
    let loaded = load(&raw);
    for settings in [loaded.clone(), load(&player_bytes(loaded.write()))] {
        assert!(settings.open[BookType::Furnace.index()]);
        let actual: Vec<_> = settings.custom.iter().collect();
        assert_eq!(
            actual,
            ids.iter().map(|id| (id.as_str(), OPEN)).collect::<Vec<_>>()
        );
    }
}

/// A saved player root in raw NBT: `recipeBook` with empty lists, the furnace book open and an
/// `EXTENSION` compound whose entries `books` writes.
fn raw_player(books: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = vec![10, 0, 0];
    key(&mut out, 10, "recipeBook");
    for list in ["recipes", "toBeDisplayed"] {
        key(&mut out, 9, list);
        out.extend([8, 0, 0, 0, 0]);
    }
    key(&mut out, 1, "isFurnaceGuiOpen");
    out.push(1);
    key(&mut out, 10, EXTENSION);
    books(&mut out);
    out.extend([0, 0, 0]);
    out
}

fn key(out: &mut Vec<u8>, id: u8, name: &str) {
    out.push(id);
    out.extend((name.len() as u16).to_be_bytes());
    out.extend(name.as_bytes());
}

fn raw_book(out: &mut Vec<u8>, id: &str, open: u8) {
    key(out, 10, id);
    key(out, 1, "open");
    out.push(open);
    key(out, 1, "filtering");
    out.push(0);
    out.push(0);
}

fn load(raw: &[u8]) -> Settings {
    let (named, used) = nbt::read_named(raw, Limits::saved()).expect("the file reads");
    assert_eq!(used, raw.len());
    Settings::read(named.tag.get("recipeBook"))
}

#[test]
fn a_saved_extension_uses_compound_tags_last_duplicate_and_keeps_other_books() {
    let settings = load(&raw_player(|out| {
        raw_book(out, POT, 0);
        raw_book(out, OVEN, 1);
        raw_book(out, POT, 1);
    }));
    assert_eq!(
        settings.custom.iter().collect::<Vec<_>>(),
        [(POT, OPEN), (OVEN, OPEN)]
    );
    assert!(settings.open[BookType::Furnace.index()]);
}

#[test]
fn a_truncated_extension_still_fails_the_file() {
    let mut raw = raw_player(|out| raw_book(out, POT, 1));
    raw.truncate(raw.len() - 4);
    assert!(nbt::read_named(&raw, Limits::saved()).is_err());
}

#[test]
fn explicit_java_fields_load_and_save_flat_without_extension_duplicates() {
    let fields = vec![(
        POT.to_owned(),
        (
            "isModCookingGuiOpen".to_owned(),
            "isModCookingFilteringCraftable".to_owned(),
        ),
    )];
    for (open, filtering) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut java = saved(Some(Tag::compound([
            (POT, entry(Tag::Byte(1), Tag::Byte(1))),
            (OVEN, entry(Tag::Byte(0), Tag::Byte(1))),
        ])));
        let Tag::Compound(map) = &mut java else {
            unreachable!()
        };
        if open {
            map.insert("isModCookingGuiOpen".into(), Tag::Byte(1));
        }
        if filtering {
            map.insert("isModCookingFilteringCraftable".into(), Tag::Byte(1));
        }
        let loaded = Settings::read_with_fields(Some(&java), &|_| true, &fields);
        assert_eq!(loaded.custom.get(POT), BookFlags { open, filtering });
        assert_eq!(loaded.custom.get(OVEN), FILTERING);
        assert!(loaded.open[BookType::Furnace.index()]);
        let written = loaded.write_with_fields(&fields);
        assert_eq!(
            written.get("isModCookingGuiOpen"),
            open.then_some(&Tag::Byte(1))
        );
        assert_eq!(
            written.get("isModCookingFilteringCraftable"),
            filtering.then_some(&Tag::Byte(1))
        );
        let extension = written.get(EXTENSION).unwrap();
        assert!(extension.get(POT).is_none());
        assert_eq!(
            extension.get(OVEN),
            Some(&entry(Tag::Byte(0), Tag::Byte(1)))
        );
        let again = Settings::read_with_fields(Some(&written), &|_| true, &fields);
        assert_eq!(again.custom, loaded.custom);
    }
}

#[test]
fn installed_field_declaration_roundtrips_the_player_book() {
    struct Cooking;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Cooking {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.recipe_book_type_with_fields(
                "demo:cooking_pot",
                "isModCookingGuiOpen",
                "isModCookingFilteringCraftable",
            )
            .unwrap();
        }
    }
    let mods = [mod_api::ModEntry::new("demo", Box::new(Cooking))];
    mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
    assert_eq!(
        mod_api::RecipeBookTypeKey::registered_save_fields(),
        vec![(
            POT.to_owned(),
            (
                "isModCookingGuiOpen".to_owned(),
                "isModCookingFilteringCraftable".to_owned()
            )
        )]
    );
    let java = Tag::compound([
        ("recipes", Tag::List(vec![])),
        ("toBeDisplayed", Tag::List(vec![])),
        ("isModCookingGuiOpen", Tag::Byte(2)),
        ("isModCookingFilteringCraftable", Tag::Int(1)),
    ]);
    let loaded = Settings::read_valid(Some(&java), &|_| true);
    assert_eq!(
        loaded.custom.get(POT),
        OPEN,
        "nonzero bytes are true; non-bytes are false"
    );
    let written = loaded.write();
    assert_eq!(written.get("isModCookingGuiOpen"), Some(&Tag::Byte(1)));
    assert!(written.get("isModCookingFilteringCraftable").is_none());
    assert!(written.get(EXTENSION).is_none());
    let raw = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: written,
        },
        Limits::saved(),
    )
    .unwrap();
    let (parsed, _) = nbt::read_named(&raw, Limits::saved()).unwrap();
    assert_eq!(
        Settings::read_valid(Some(&parsed.tag), &|_| true)
            .custom
            .get(POT),
        OPEN
    );
}
