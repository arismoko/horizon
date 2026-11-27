//! The recipes a new player's first tick unlocks (ServerPlayer.tick's
//! `CriteriaTriggers.TICK`, then ServerRecipeBook.addRecipes) reach its client
//! in one ClientboundRecipeBookAddPacket, even when the game's own TICK trigger
//! awarded them in the guest's turn before the hub's first tick.
use super::*;
use sunrise_protocol::buf::Codec;

/// Each RecipeBookAdd (its entry count) and UpdateAdvancements sent, in order.
fn adds(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<String> {
    let mut sent = Vec::new();
    while let Ok(out) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = out else {
            continue;
        };
        let mut reader = sunrise_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        if id == ids::UPDATE_ADVANCEMENTS {
            sent.push("advancements".to_owned());
        } else if id == ids::RECIPE_BOOK_ADD {
            let add =
                <sunrise_protocol::packets::recipes::RecipeBookAdd as Codec<_>>::read(&mut reader)
                    .unwrap();
            sent.push(format!("add {} replace {}", add.entries.len(), add.replace));
        }
    }
    sent
}

fn first_tick_after_the_games_turn(unlocked: &[&str]) -> Vec<String> {
    let mut sync = SyncData::empty();
    sync.advancements = crate::lan::advancements::ServerAdvancements::tick_rewarding(
        "test:reward",
        &["test:other"],
    );
    let (mut hub, rx) = hub_with_sync(sync);
    // The guest's turn ran the game's TICK trigger first: the advancement is done, its
    // recipe is in the book and waits in the unlocked list for flushDirty.
    hub.guests.update(0, |g| {
        g.advancements
            .grant("test:tick", "tick", "2020-01-02 03:04:05 +0000");
        for id in unlocked {
            g.recipe_book.recipes.insert((*id).to_owned());
            g.recipes_unlocked.push((*id).to_owned());
        }
    });
    while rx.try_recv().is_ok() {}
    hub.flush_progress(1); // initial snapshot
    hub.flush_progress(1); // next level tick's dirty progress
    adds(&rx)
}

#[test]
fn a_recipe_the_first_tick_announced_is_not_flushed_again() {
    assert_eq!(
        first_tick_after_the_games_turn(&["test:reward"]),
        ["add 1 replace false", "advancements"]
    );
}

#[test]
fn another_recipe_unlocked_before_the_first_tick_is_still_flushed() {
    assert_eq!(
        first_tick_after_the_games_turn(&["test:reward", "test:other"]),
        ["add 1 replace false", "advancements", "add 1 replace false"]
    );
}
