//! Owned component trees. No language is resolved until the game displays a component.
use serde::{Deserialize, Serialize};

/// A component tree, retaining translation keys, styles and ordered children.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[horizon_mod_api_macros::wasm(via = TextForm)]
#[non_exhaustive]
pub struct Text {
    /// The component's contents before its children.
    pub content: TextContent,
    /// Fields not set here inherit from the enclosing component.
    pub style: TextStyle,
    /// Components appended in order.
    pub children: Vec<Text>,
    /// Unrecognised component metadata survives an engine round trip.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

/// What supplies the characters or inline object of a component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
pub enum TextContent {
    /// Literal characters, without interpreting legacy formatting codes.
    Literal(String),
    /// A translation remains a key and components, not a localized string.
    Translatable {
        /// Translation identifier.
        key: String,
        /// Template used when the key is not present in the language.
        fallback: Option<String>,
        /// Ordered `%s` arguments, retaining their own styles.
        arguments: Vec<Text>,
    },
    /// A key binding's localized display name.
    Keybind(String),
    /// A selector resolved by the game's component context.
    Selector {
        /// Selector expression.
        pattern: String,
        /// Component between the selected names.
        separator: Option<Box<Text>>,
    },
    /// A scoreboard value resolved by the game's component context.
    Score {
        /// Score holder or selector.
        name: String,
        /// Scoreboard objective.
        objective: String,
    },
    /// NBT resolved by the game's component context.
    Nbt {
        /// NBT path expression.
        path: String,
        /// Whether the selected strings contain components.
        interpret: bool,
        /// Where to obtain the NBT.
        source: TextNbtSource,
        /// Component between selected values.
        separator: Option<Box<Text>>,
    },
    /// A non-character inline object.
    Object(TextObject),
}

/// The source of an NBT component, not the NBT itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
pub enum TextNbtSource {
    /// Block position expression.
    Block(String),
    /// Entity selector expression.
    Entity(String),
    /// Command storage identifier.
    Storage(String),
}

/// An inline object embedded in text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
pub enum TextObject {
    /// A player head, retaining the profile properties carried by the component.
    Player {
        /// Profile data, including signed skin properties when present.
        profile: serde_json::Value,
        /// Whether to draw the hat layer.
        hat: bool,
        /// Optional component for consumers unable to display the object; None uses the default.
        fallback: Option<Box<Text>>,
    },
    /// A sprite in a named atlas.
    Atlas {
        /// Atlas identifier.
        atlas: String,
        /// Sprite identifier.
        sprite: String,
        /// Optional component for consumers unable to display the object; None uses the default.
        fallback: Option<Box<Text>>,
    },
}

/// Optional style fields inherit; explicit `false` overrides an inherited decoration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TextStyle {
    /// Named colour or `#RRGGBB` component colour.
    pub color: Option<String>,
    /// Packed ARGB shadow colour; zero suppresses shadows.
    pub shadow_color: Option<u32>,
    /// Bold decoration.
    pub bold: Option<bool>,
    /// Italic decoration.
    pub italic: Option<bool>,
    /// Underline decoration.
    pub underlined: Option<bool>,
    /// Strikethrough decoration.
    pub strikethrough: Option<bool>,
    /// Obfuscated decoration.
    pub obfuscated: Option<bool>,
    /// Action invoked by clicking the component.
    pub click: Option<TextClick>,
    /// Information displayed on hover.
    pub hover: Option<TextHover>,
    /// Text inserted into the chat input on Shift-click.
    pub insertion: Option<String>,
    /// Font identifier.
    pub font: Option<String>,
    /// Unknown style metadata remains owned data, without engine types.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

/// A text component's click action.
///
/// File-opening actions are intentionally absent: mods can neither produce nor
/// round-trip `OpenFile`. Capture drops untrusted file-opening action metadata.
/// Part of the [per-node origin tracking follow-up](https://github.com/arismoko/horizon/issues/1738).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
pub enum TextClick {
    /// Open an HTTP or HTTPS URL.
    OpenUrl(String),
    /// Switch to the numbered page of a book.
    ChangePage(i32),
    /// Put a command in the chat input.
    SuggestCommand(String),
    /// Run a command.
    RunCommand(String),
    /// Copy text to the clipboard.
    CopyToClipboard(String),
    /// Show a dialog identifier or inline declaration.
    ShowDialog(serde_json::Value),
    /// A server-defined action with optional payload.
    Custom {
        /// Action identifier.
        id: String,
        /// Payload retained without interpretation.
        payload: Option<serde_json::Value>,
    },
}

/// A text component's hover action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
pub enum TextHover {
    /// Show another styled component tree.
    Text(Box<Text>),
    /// Show an item with its component patch.
    Item {
        /// Item identifier, not a process-local registry slot.
        id: String,
        /// Display count.
        count: u32,
        /// Component patch carried by the component, if any.
        components: Option<serde_json::Value>,
    },
    /// Show an entity's identity and optional name.
    Entity {
        /// Entity type identifier.
        kind: String,
        /// UUID as the high and low halves.
        id: [u64; 2],
        /// Optional styled entity name.
        name: Option<Box<Text>>,
    },
}

impl Default for Text {
    fn default() -> Self {
        Self::literal("")
    }
}
impl Text {
    /// A component with the given contents, with no style overrides or children.
    pub fn new(content: TextContent) -> Self {
        Self {
            content,
            style: TextStyle::default(),
            children: Vec::new(),
            metadata: Default::default(),
        }
    }
    /// A literal component, with no inherited style overrides or children.
    pub fn literal(value: impl Into<String>) -> Self {
        Self::new(TextContent::Literal(value.into()))
    }
    /// A translatable component. The key and arguments remain unresolved data.
    pub fn translate(key: impl Into<String>, arguments: Vec<Text>) -> Self {
        Self::new(TextContent::Translatable {
            key: key.into(),
            fallback: None,
            arguments,
        })
    }
    /// Append a child without flattening its style or translation.
    pub fn append(mut self, child: Text) -> Self {
        self.children.push(child);
        self
    }
    /// Set this component's style; children continue to inherit unset fields.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }
}

/// A finite, serializable representation of the recursive tree.
/// Native boundary conversion and generated wasm bindings share the same serde model.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextForm {
    /// Serialized owned tree, including component metadata and unresolved keys.
    pub tree: String,
}
impl From<&Text> for TextForm {
    fn from(value: &Text) -> Self {
        Self {
            tree: serde_json::to_string(value).expect("owned text contains serializable data"),
        }
    }
}
impl TryFrom<TextForm> for Text {
    type Error = serde_json::Error;
    fn try_from(value: TextForm) -> Result<Self, Self::Error> {
        serde_json::from_str(&value.tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_retains_translation_children_and_explicit_style_overrides() {
        let text = Text::translate(
            "chat.type.text",
            vec![Text::literal("Alex"), Text::literal("Hi")],
        )
        .with_style(TextStyle {
            bold: Some(true),
            ..Default::default()
        })
        .append(Text::literal("!").with_style(TextStyle {
            bold: Some(false),
            ..Default::default()
        }));
        let decoded = Text::try_from(TextForm::from(&text)).unwrap();
        assert_eq!(decoded, text);
        assert!(
            matches!(&decoded.content, TextContent::Translatable { key, .. } if key == "chat.type.text")
        );
        assert_eq!(decoded.children[0].style.bold, Some(false));
    }
    #[test]
    fn object_fallback_form_distinguishes_omitted_styled_and_translatable() {
        use serde_json::json;
        let styled = json!({"content":{"Literal":"styled"}, "style":{"bold":false}, "children":[]});
        let translated = json!({"content":{"Translatable":{"key":"test.head", "arguments":[]}}, "style":{}, "children":[]});
        for object in [
            json!({"Player":{"profile":{"name":"Test"}, "hat":true}}),
            json!({"Atlas":{"atlas":"minecraft:blocks", "sprite":"test:icon"}}),
        ] {
            for fallback in [None, Some(styled.clone()), Some(translated.clone())] {
                let mut object = object.clone();
                if let Some(fallback) = &fallback {
                    object.as_object_mut().unwrap().values_mut().next().unwrap()["fallback"] =
                        fallback.clone();
                }
                let tree =
                    json!({"content":{"Object":object}, "style":{}, "children":[]}).to_string();
                let text = Text::try_from(TextForm { tree }).unwrap();
                let TextContent::Object(object) = &text.content else {
                    panic!("object contents")
                };
                let captured = match object {
                    TextObject::Player { fallback, .. } | TextObject::Atlas { fallback, .. } => {
                        fallback
                    }
                };
                match &fallback {
                    None => assert!(captured.is_none()),
                    Some(value) if value == &styled => {
                        assert_eq!(captured.as_ref().unwrap().style.bold, Some(false))
                    }
                    Some(_) => assert!(
                        matches!(&captured.as_ref().unwrap().content, TextContent::Translatable { key, .. } if key == "test.head")
                    ),
                }
                assert_eq!(Text::try_from(TextForm::from(&text)).unwrap(), text);
            }
        }
    }
    #[test]
    fn form_keeps_unknown_metadata_and_refuses_malformed_trees() {
        let mut value = serde_json::to_value(Text::literal("value")).unwrap();
        value["metadata"]["future_component"] = serde_json::json!({"version": 7});
        value["style"]["metadata"]["future_style"] = serde_json::json!([1, 2]);
        let text: Text = serde_json::from_value(value.clone()).unwrap();
        let decoded = Text::try_from(TextForm::from(&text)).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        assert!(
            Text::try_from(TextForm {
                tree: "{\"content\": 42}".into()
            })
            .is_err()
        );
    }
}
