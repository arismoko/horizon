//! Escape on the two book screens, and Enter on the sign screen's title.
//!
//! `BookEditScreen` and `BookSignScreen` override neither `onClose` nor
//! `shouldCloseOnEsc`, so Escape is `Screen.onClose` (`setScreen(null)`,
//! `Screen.java:121-125,197-203`) and saves nothing: only the Done button
//! calls `BookEditScreen.saveChanges`. `BookSignScreen.keyPressed`
//! (`BookSignScreen.java:79-87`) signs on Enter whenever the title box is not
//! empty — `!getValue().isEmpty()`, not blank — and trims the title on send
//! (`:70`).
use sunrise_client_gameplay::game::client_side::books::BookScreen;
use sunrise_items::items::ItemStack;
use winit::keyboard::KeyCode;
use {
    crate::game::{Audio, Settings, load_resources},
    sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress,
    sunrise_gameplay_common::game::execution_role,
};

/// A game whose selected hotbar slot holds a book and quill, its editor open,
/// `text` typed on the page.
fn editing<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    text: &str,
) -> sunrise_client_gameplay::game::ClientGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::Client;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        res,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root,
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    *game.held_mut() = Some(ItemStack::new("minecraft:writable_book", 1));
    let stack = game.held().expect("a book in hand").clone();
    assert!(game.remote_use_book(&stack), "the editor opened");
    game.book_input(None, Some(text));
    game
}

/// The pages saved into the stack in hand (empty: nothing was written to it).
fn saved_pages(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> Vec<String> {
    game.held()
        .and_then(|stack| stack.data())
        .map(|data| data.writable_pages.clone())
        .unwrap_or_default()
}

/// `BookEditScreen` has no `onClose`: Escape is `Screen.onClose`, which closes
/// the screen and sends nothing, so the typed page is lost.
#[test]
#[ignore = "daily"]
fn escape_in_the_editor_saves_nothing() {
    let mut resources = None;
    let mut game = editing(&mut resources, "hello");
    game.book_input(Some(KeyCode::Escape), None);
    assert!(game.client.book.is_none(), "Escape left the editor open");
    assert!(saved_pages(&game).is_empty(), "Escape saved the page");
}

/// `BookSignScreen` has no `onClose` either: Escape closes the whole book
/// rather than going back to the editor, and saves nothing.
#[test]
#[ignore = "daily"]
fn escape_on_the_sign_screen_closes_the_book() {
    let mut resources = None;
    let mut game = editing(&mut resources, "hi");
    game.book_control(Some("sign")).expect("the Sign button");
    assert!(matches!(
        game.client.book,
        Some(BookScreen::Edit { signing: true, .. })
    ));
    game.book_input(Some(KeyCode::Escape), None);
    assert!(
        game.client.book.is_none(),
        "Escape went back to the editor instead of closing the book"
    );
    assert!(saved_pages(&game).is_empty(), "Escape saved the page");
}

/// `BookSignScreen.keyPressed` signs on Enter for any title that is not empty,
/// so three spaces sign a book whose title trims to nothing; an empty title
/// stays unsignable.
#[test]
#[ignore = "daily"]
fn enter_signs_a_title_of_spaces_but_not_an_empty_one() {
    let mut resources = None;
    let mut game = editing(&mut resources, "hi");
    game.book_control(Some("sign")).expect("the Sign button");
    game.book_input(Some(KeyCode::Enter), None);
    assert!(
        game.client.book.is_some(),
        "Enter signed a book with an empty title"
    );
    game.book_input(None, Some("   "));
    game.book_input(Some(KeyCode::Enter), None);
    let held = game.held().expect("a book in hand");
    assert_eq!(
        &*held.item, "minecraft:written_book",
        "Enter ignored a title of spaces"
    );
    let title = held
        .data()
        .and_then(|data| data.written.as_ref())
        .map(|book| book.title.clone());
    assert_eq!(title.as_deref(), Some(""), "the title is trimmed on send");
}
