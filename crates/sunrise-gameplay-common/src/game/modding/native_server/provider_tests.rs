//! Native argument completion through the loaded dispatcher and guest registries.
use super::*;
use std::sync::Mutex;
use sunrise_command::{Registries, Suggestion};

/// What one provider call saw: the prefix, the arguments before it, and the sender.
type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>, Option<(u128, String, u8)>)>>>;

fn seen() -> Seen {
    Default::default()
}

/// A command argument type that records every request and answers a fixed list.
struct Declared {
    candidates: Vec<mod_api::Suggestion>,
    seen: Seen,
    panic_on_answer: bool,
}

impl mod_api::CommandArgumentType for Declared {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
        self.seen.lock().unwrap().push((
            request.prefix.clone(),
            request
                .arguments
                .iter()
                .map(|argument| (argument.name.clone(), argument.text.clone()))
                .collect(),
            request.sender.as_ref().map(|sender| {
                (
                    sender.uuid(),
                    sender.name().to_owned(),
                    sender.permission().level(),
                )
            }),
        ));
        if self.panic_on_answer {
            panic!("the mod's completer panicked");
        }
        self.candidates.clone()
    }
}

/// A type that completes from a fixed list by the prefix.
struct Plain;

impl mod_api::CommandArgumentType for Plain {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
        ["survival", "creative"]
            .into_iter()
            .filter(|mode| mode.starts_with(request.prefix.as_str()))
            .map(|mode| request.suggest(mode))
            .collect()
    }
}

fn arguments(declared: Declared) -> CommandArguments {
    HashMap::from([
        (
            "test:warp".to_owned(),
            Arc::new(declared) as Arc<dyn mod_api::CommandArgumentType>,
        ),
        (
            "test:mode".to_owned(),
            Arc::new(Plain) as Arc<dyn mod_api::CommandArgumentType>,
        ),
    ])
}

/// The tree and dispatcher of `warp go <world:test:mode> <name:test:warp>`.
fn warp(declared: Declared) -> (Value, Dispatcher) {
    let arguments = arguments(declared);
    let (tree, _, problems) = crate::game::native_server::command_tree(
        &[mod_api::Command::new(
            "warp go <world:test:mode> <name:test:warp>",
        )],
        &sunrise_command::Dispatcher::new(),
        &arguments,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = Dispatcher::new();
    register_dispatcher(&mut dispatcher, &tree, "native_mod_command", &arguments);
    (tree, dispatcher)
}

fn guest() -> Registries {
    Registries {
        permission: 3,
        sender: Some(sunrise_game_data::registries::CommandSender {
            uuid: 0x5eed,
            name: "Simon".into(),
        }),
        ..Registries::default()
    }
}

fn texts(suggestions: &[Suggestion]) -> Vec<&str> {
    suggestions.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn an_argument_is_asked_for_its_own_prefix_with_the_sender_and_the_arguments_before_it() {
    let seen = seen();
    let (tree, dispatcher) = warp(Declared {
        candidates: vec![
            mod_api::Suggestion::new((0, 2), "press").with_tooltip("the press"),
            mod_api::Suggestion::new((0, 2), "furnace"),
        ],
        seen: seen.clone(),
        panic_on_answer: false,
    });

    // A client that does not know the type asks the server for it.
    let node = &tree["children"]["warp"]["children"]["go"]["children"]["world"]["children"]["name"];
    assert_eq!(node["custom_argument"], "test:warp");
    assert_eq!(node["suggestions"], "minecraft:ask_server");

    let typed = "warp go survival pr";
    let registries = guest();
    let (range, suggestions) = dispatcher.suggestions(
        &dispatcher.parse(typed, &registries),
        typed.len(),
        &registries,
    );
    // The mod's answer stands as it came, in the chat's own order, with its tooltip.
    assert_eq!(texts(&suggestions), ["furnace", "press"]);
    assert_eq!(range, (typed.len() - 2, typed.len()));
    assert_eq!(
        suggestions
            .iter()
            .find(|s| s.text == "press")
            .and_then(|s| s.tooltip.as_deref()),
        Some("the press")
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [(
            "pr".to_owned(),
            vec![("world".to_owned(), "survival".to_owned())],
            Some((0x5eed, "Simon".to_owned(), 3))
        )]
    );
}

/// The same builder builds the server's tree and the client's own: a client command may name the
/// same argument type, and it completes through the same `suggest`.
#[test]
fn a_client_command_completes_the_same_argument_type_through_its_suggest() {
    let seen = seen();
    let arguments = arguments(Declared {
        candidates: vec![mod_api::Suggestion::new((0, 0), "press")],
        seen: seen.clone(),
        panic_on_answer: false,
    });
    let commands = [mod_api::Command::new(
        "warp go <world:test:mode> <name:test:warp>",
    )];
    let (tree, _, problems) = super::client_command_tree(&commands, &arguments, None);
    assert!(problems.is_empty(), "{problems:?}");
    let mut chat = sunrise_command::vanilla::game_dispatcher().clone();
    register_dispatcher(&mut chat, &tree, "native_mod_command", &arguments);
    let registries = guest();
    let typed = "warp go creative ";
    let (_, suggestions) =
        chat.client_suggestions(&chat.parse(typed, &registries), typed.len(), &registries);
    assert_eq!(texts(&suggestions), ["press"]);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn a_panicking_completer_answers_no_suggestion_and_the_rest_of_the_line_still_completes() {
    let seen = seen();
    let (_, dispatcher) = warp(Declared {
        candidates: Vec::new(),
        seen,
        panic_on_answer: true,
    });
    let registries = guest();
    let typed = "warp go survival pr";
    let (_, suggestions) = dispatcher.suggestions(
        &dispatcher.parse(typed, &registries),
        typed.len(),
        &registries,
    );
    assert!(suggestions.is_empty(), "{suggestions:?}");

    // One argument earlier, the other type still completes.
    let typed = "warp go cr";
    let (_, suggestions) = dispatcher.suggestions(
        &dispatcher.parse(typed, &registries),
        typed.len(),
        &registries,
    );
    assert_eq!(texts(&suggestions), ["creative"]);
}
