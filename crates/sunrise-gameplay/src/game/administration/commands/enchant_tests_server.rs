use super::Text;
use sunrise_game_data::scoreboard::comp_text;

const RIPTIDE: &str = "minecraft:riptide";

pub fn enchant_outcome(
    game: &mut crate::game::ServerGame<'_>,
    target: &str,
    level: u32,
    success: bool,
    count: i32,
    key: &str,
    first_arg: Option<&str>,
) {
    let command = format!("enchant {target} {RIPTIDE} {level}");
    game.server.chat_capture = Some(Vec::new());
    let (ok, outcome) =
        game.recording_command_outcome(|g| g.as_console(|g| g.run_command(&command)));
    let feedback = game.server.chat_capture.take().unwrap();
    assert_eq!(
        (ok, outcome),
        (success, Some((success, count))),
        "{command}: {feedback:?}"
    );
    let key = format!("commands.enchant.{key}");
    let component = feedback
        .iter()
        .find_map(|line: &Text| {
            let component = line.component();
            if component["translate"] == key {
                return Some(component);
            }
            component["extra"]
                .as_array()?
                .iter()
                .find(|part| part["translate"] == key)
                .cloned()
        })
        .unwrap_or_else(|| panic!("{command}: expected {key}, got {feedback:?}"));
    if let Some(first_arg) = first_arg {
        assert_eq!(comp_text(&component["with"][0]).plain(), first_arg);
    }
}
