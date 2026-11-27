//! `player_login_profile` on the server thread: a login's properties as the mods leave them
//! go back to its connection, and a list the login packet can't carry is refused whole.
use mod_api::{AccountProfileProperty, Hooks, ModEntry, PlayerLoginProfileEvent, ServerHost};
use std::sync::mpsc::sync_channel;
use sunrise_core::profile::AccountProperty;
use sunrise_sim::lan::login_profile::LoginProfileRequest;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// Replaces the textures with its own, and adds one property per name it is given.
struct Skin(usize);
#[mod_api::__subscriptions]
impl Hooks for Skin {
    fn player_login_profile(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerLoginProfileEvent<'_>,
    ) {
        event
            .properties
            .retain(|property| property.name != "textures");
        event.properties.push(AccountProfileProperty {
            name: "textures".into(),
            value: format!("skin of {}", event.name),
            signature: None,
        });
        for n in 0..self.0 {
            event.properties.push(AccountProfileProperty {
                name: format!("extra{n}"),
                value: String::new(),
                signature: None,
            });
        }
    }
}

fn answer(extra: usize) -> Vec<AccountProperty> {
    let mods = vec![ModEntry::new("skin", Box::new(Skin(extra)))].into();
    let mut sent = Vec::new();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (reply, answer) = sync_channel(1);
            game.answer_login_profile(LoginProfileRequest {
                uuid: 7,
                name: "Alex".into(),
                properties: vec![AccountProperty {
                    name: "textures".into(),
                    value: "signed skin".into(),
                    signature: Some("sig".into()),
                }],
                reply,
            });
            sent = answer.try_recv().unwrap();
        });
    });
    sent
}

#[test]
#[ignore = "daily"]
fn a_login_sends_the_properties_its_listeners_leave_when_they_fit() {
    if run_fresh_cases(
        "cut_layout::game::server_side::lan::login_profile_tests::a_login_sends_the_properties_its_listeners_leave_when_they_fit",
        "SUNRISE_LOGIN_PROFILE_CASE",
        &["edit"],
    ) {
        return;
    }
    let edited = answer(0);
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].name, "textures");
    assert_eq!(edited[0].value, "skin of Alex");
    assert_eq!(edited[0].signature, None);
    // 1 + 16 properties are more than GAME_PROFILE_PROPERTIES carries: the login keeps its own.
    let refused = answer(16);
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].value, "signed skin");
    assert_eq!(refused[0].signature.as_deref(), Some("sig"));
}
