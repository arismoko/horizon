//! `text_from_json`: a text component read as `/tellraw` takes it.
use super::Game;
use horizon_client::text::Text;

impl Game<'_> {
    /// A component read as `/tellraw` takes it; None when it isn't one.
    pub(super) fn text_from_json(&self, json: &str) -> Option<Text> {
        component_from_json(json)
    }
}

/// One JSON or SNBT component (`ComponentSerialization.CODEC`'s string, list or object
/// forms), with nothing but whitespace after it.
fn component_from_json(json: &str) -> Option<Text> {
    let (value, used) = horizon_client::component::parse_snbt(json).ok()?;
    if !json.as_bytes()[used..].iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    if !(value.is_string() || value.is_array() || value.is_object()) {
        return None;
    }
    Some(Text::from(value))
}

#[cfg(test)]
mod tests {
    use super::{Text, component_from_json};
    use crate::game::api_values::TryIntoApi;
    use mod_api::text::{TextClick, TextContent};

    fn api(text: &Text) -> mod_api::Text {
        text.try_into_api().unwrap()
    }

    /// JSON and SNBT components keep their structure: styles, children, translations
    /// and click events; what is not one component is refused.
    #[test]
    fn text_from_json_reads_components_as_tellraw_does() {
        let text = api(&component_from_json(
            r#"{"text":"Hi ","color":"gold","extra":[{"text":"there","bold":true}]}"#,
        )
        .unwrap());
        assert_eq!(text.content, TextContent::Literal("Hi ".into()));
        assert_eq!(text.style.color.as_deref(), Some("gold"));
        assert_eq!(text.children.len(), 1);
        assert_eq!(text.children[0].style.bold, Some(true));
        let text = api(
            &component_from_json(r#"{"translate":"chat.type.text","with":["a","b"]}"#).unwrap(),
        );
        assert!(
            matches!(&text.content, TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" && arguments.len() == 2)
        );
        let text = api(&component_from_json(
            "{text:'run',click_event:{action:run_command,command:'/spawn'}}",
        )
        .unwrap());
        assert_eq!(
            text.style.click,
            Some(TextClick::RunCommand("/spawn".into()))
        );
        assert_eq!(
            api(&component_from_json("  \"plain\"\n").unwrap()).content,
            TextContent::Literal("plain".into())
        );
        for refused in [
            "",
            "{\"text\":\"a\"} trailing",
            "{\"text\":\"open",
            "12",
            "true",
        ] {
            assert!(component_from_json(refused).is_none(), "{refused:?}");
        }
    }
}
