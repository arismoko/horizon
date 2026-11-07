//! Uses two declared fields and a thunk; the host observes only committed writes.
use mod_internals::view::{Game, HandleChatInputRaw};

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

// The name also verifies that runtime imports do not shadow a raw listener.
fn call(raw: HandleChatInputRaw, game: Option<Game>) {
    let game = game.expect("a method receiver");
    let message = raw.message().unwrap();
    if message.starts_with('x') {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view::Cell};
        // Lower once. The WIT setter borrows these cells, with no per-iteration string clone.
        let small = message.len() == 1;
        let cells = [Cell::Str(*message)];
        let node = game.node();
        if small {
            for _ in 0..2000 {
                assert!(node.set(0, &cells).is_err());
            }
            return;
        }
        loop {
            // `left` is i64: refusal precedes retention, so journal budgets cannot stop this.
            assert!(node.set(0, &cells).is_err());
        }
    }
    if matches!(message.as_str(), "full_journal" | "empty_journal") {
        use mod_internals::runtime::AsNode;
        if message.as_str() == "full_journal" {
            for _ in 0..4096 {
                game.set_left(42).unwrap();
            }
            loop {
                // Miss all pending writes, rather than finding the most recent left write.
                let _ = game.node().get(1).unwrap();
            }
        }
        for _ in 0..2000 {
            let _ = game.node().get(1).unwrap();
        }
        return;
    }
    if matches!(message.as_str(), "fuel_get" | "fuel_set") {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view};
        let node = game.node();
        loop {
            if message.as_str() == "fuel_get" {
                let _ = view::get_many(&[(node, 0), (node, 1)]);
            } else {
                let _ = view::set_many(
                    &[(node, 0, 1), (node, 1, 1)],
                    &[view::Cell::S(9), view::Cell::S(9)],
                );
            }
        }
    }
    if message.as_str() == "lazy_wrong_arity" {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view};
        let args = (0..2)
            .map(|_| view::Arg::Value(vec![view::Cell::Str("x".repeat(2_097_152))]))
            .collect::<Vec<_>>();
        assert!(matches!(
            view::call(2, Some(game.node()), &args),
            Err(view::Refusal::Changed(_))
        ));
        return;
    }
    if message.as_str() == "lazy_multiple_values" {
        use mod_internals::runtime::bindings::horizon::mod_internals::view;
        let args = (0..2)
            .map(|_| view::Arg::Value(vec![view::Cell::Str("x".repeat(786_432))]))
            .collect::<Vec<_>>();
        // mutate_copies has two copy arguments. Their shapes are deliberately
        // wrong, but both values fit their own budgets and must reach that check.
        assert!(matches!(
            view::call(3, None, &args),
            Err(view::Refusal::Changed(_))
        ));
        return;
    }
    if let Some(route) = message.strip_prefix("lazy_string_") {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view};
        let node = game.node();
        let cells = vec![view::Cell::Str("x".repeat(2_097_152))];
        let refused = |result: Result<(), view::Refusal>| {
            assert!(
                matches!(result, Err(view::Refusal::NotHere(why)) if why == "internals budget")
            );
        };
        match route {
            "set" => refused(node.set(0, &cells)),
            "key" => refused(node.key(&cells).map(|_| ())),
            "call" => refused(view::call(2, Some(node), &[view::Arg::Value(cells)]).map(|_| ())),
            "batch" => {
                let results = view::set_many(&[(node, 0, 1)], &cells);
                assert_eq!(results.len(), 1);
                for result in results {
                    refused(result);
                }
            }
            _ => panic!("unknown lazy string route"),
        }
        return;
    }
    if matches!(message.as_str(), "strings_batch" | "strings_single") {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view};
        let node = game.node();
        let cells = || {
            vec![
                view::Cell::Str("x".repeat(524_289)),
                view::Cell::Str("x".repeat(524_289)),
            ]
        };
        if message.as_str() == "strings_batch" {
            let writes = (0..12).map(|_| (node, 0, 2)).collect::<Vec<_>>();
            let all_cells = (0..12).flat_map(|_| cells()).collect::<Vec<_>>();
            assert!(
                view::set_many(&writes, &all_cells)
                    .iter()
                    .all(Result::is_err)
            );
        } else {
            for _ in 0..12 {
                assert!(node.set(0, &cells()).is_err());
            }
        }
        return;
    }
    if message.as_str() == "oversize_cells" {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view};
        let node = game.node();
        let writes = (0..12).map(|_| (node, 0, 65_537)).collect::<Vec<_>>();
        let cells = (0..12 * 65_537)
            .map(|_| view::Cell::None)
            .collect::<Vec<_>>();
        let results = view::set_many(&writes, &cells);
        assert_eq!(results.len(), 12);
        assert!(
            results
                .iter()
                .all(|result| matches!(result, Err(view::Refusal::NotHere(_))))
        );
        return;
    }
    if matches!(message.as_str(), "oversize_get" | "oversize_set") {
        use mod_internals::runtime::{AsNode, bindings::horizon::mod_internals::view};
        let node = game.node();
        if message.as_str() == "oversize_get" {
            let reads = (0..13).map(|_| (node, 0)).collect::<Vec<_>>();
            let _ = view::get_many(&reads);
        } else {
            let writes = (0..13).map(|_| (node, 0, 0)).collect::<Vec<_>>();
            let _ = view::set_many(&writes, &[]);
        }
        panic!("oversized batch did not trap");
    }
    match message.as_str() {
        "panic_thunk" => {
            game.set_left(42).unwrap();
            let _ = game.add(-999);
            game.set_right(999).unwrap();
            return;
        }
        "panic_apply" => {
            game.set_left(-777).unwrap();
            return;
        }
        "budget" => {
            for _ in 0..4097 {
                game.set_left(42).unwrap();
            }
            return;
        }
        _ => {}
    }
    let mut n = 7;
    let mut small = mod_internals::view::Small { count: 2 };
    assert_eq!(
        mod_internals::view::mutate_copies(&mut n, &mut small).unwrap(),
        21
    );
    assert_eq!(
        (n, small.count),
        (12, 9),
        "mutable copies write back into guest values"
    );
    game.set_left(game.left().unwrap() + 2).unwrap();
    game.set_right(game.right().unwrap() + 3).unwrap();
    assert_eq!(game.left().unwrap(), 3, "read sees the journal");
    let total = game.add(5).unwrap();
    assert_eq!(game.left().unwrap(), 8, "thunk sees the applied journal");
    raw.set_message(Box::new(format!("proof{total}"))).unwrap();
    raw.set_cancel(true).unwrap();
}

mod_internals::raw_listeners!();
