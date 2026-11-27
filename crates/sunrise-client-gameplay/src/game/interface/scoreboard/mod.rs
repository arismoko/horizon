#[test]
fn hidden_tab_skips_players_except_for_visibility_narration() {
    use crate::game::client_side::scoreboard_client::tab_players_needed;
    use sunrise_client::tab_list;
    for count in [1, 150, 1000] {
        let shown = tab_list::visible(true, false, false, count, false);
        assert!(!tab_players_needed(shown, false));
        assert!(
            tab_players_needed(shown, true),
            "closing narration needs names"
        );
        let shown = tab_list::visible(true, true, false, count, false);
        assert!(tab_players_needed(shown, false));
    }
}
