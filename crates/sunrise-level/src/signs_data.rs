//! Sign block entities (`SignBlockEntity`, `SignText`): two four-line text
//! sides with a dye colour and glow, saved in vanilla's `front_text` /
//! `back_text` NBT layout.
pub use crate::dyes::DYE_TEXT;
use crate::{scoreboard, text::Text};
use sunrise_core::nbt::Tag;

/// One sign side (`SignText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignText {
    pub lines: [String; 4],
    /// `DyeColor` id.
    pub color: u8,
    pub glowing: bool,
    /// The `click_event` in each line's component style, as saved.
    pub click: [Option<Tag>; 4],
    /// Each line's saved component when it is more than plain text (a colour,
    /// siblings, a list), so its style survives loading, saving and the
    /// network. See `SignText::message` for a line edited since.
    pub saved: [Option<SavedLine>; 4],
}
/// A sign line's saved component and the text it showed when it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedLine {
    pub text: String,
    pub component: Tag,
}
/// `SignBlockEntity` / `HangingSignBlockEntity` data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignData {
    pub hanging: bool,
    pub front: SignText,
    pub back: SignText,
    pub waxed: bool,
    /// `allow_op_features`: the sign's click events may run commands.
    pub allow_op_features: bool,
    /// `playerWhoMayEdit`: the one player whose editor is on this sign, as a
    /// UUID. Every other player's click, write and applicator is refused
    /// while it is set. Not saved (vanilla keeps it out of the NBT).
    pub allowed_editor: Option<u128>,
}

/// `getBooleanOr(key, false)` and `Codec.BOOL`: any numeric tag's nonzero byte.
fn nbt_flag(tag: &Tag, key: &str) -> bool {
    tag.get(key)
        .and_then(sunrise_game_data::nbt_path::int_value)
        .is_some_and(|value| value as i8 != 0)
}

const BLACK: u8 = 15;

impl Default for SignText {
    fn default() -> Self {
        Self {
            lines: Default::default(),
            color: BLACK,
            glowing: false,
            click: Default::default(),
            saved: Default::default(),
        }
    }
}
/// The click events `SignBlockEntity.executeClickCommandsIfPresent` acts on.
#[derive(Debug, PartialEq)]
pub enum ClickAction<'a> {
    /// `ClickEvent.RunCommand`.
    RunCommand(String),
    /// `ClickEvent.ShowDialog`: a registry id or the dialog itself.
    ShowDialog(&'a Tag),
    /// `ClickEvent.Custom`.
    Custom {
        id: String,
        payload: Option<&'a Tag>,
    },
}
/// A saved `click_event` as its action: the three a sign acts on, read the
/// way `ClickEvent.CODEC` does; any other action or a malformed one is None.
pub fn click_action(event: &Tag) -> Option<ClickAction<'_>> {
    let Some(Tag::String(action)) = event.get("action") else {
        return None;
    };
    match action.to_string_lossy().as_str() {
        "run_command" => match event.get("command") {
            Some(Tag::String(command)) => Some(ClickAction::RunCommand(command.to_string_lossy())),
            _ => None,
        },
        "show_dialog" => event.get("dialog").map(ClickAction::ShowDialog),
        "custom" => match event.get("id") {
            Some(Tag::String(id)) => Some(ClickAction::Custom {
                id: id.to_string_lossy(),
                payload: event.get("payload"),
            }),
            _ => None,
        },
        _ => None,
    }
}
impl SignText {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
    }
    /// `SignText.hasEditableText`: every line's contents is plain text, so the
    /// editor can rewrite it without losing anything. A style does not count
    /// (a red literal is still plain), a `translatable`, `score` or other
    /// contents does. (Sunrise keeps no filtered text, so
    /// `Player.isTextFilteringEnabled` never selects that copy.)
    pub fn has_editable_text(&self) -> bool {
        (0..self.lines.len()).all(|i| plain_contents(&self.message(i)))
    }
    /// `DyeColor.getTextColor` of this side.
    pub fn text_color(&self) -> u32 {
        crate::dyes::text(usize::from(self.color.min(15)))
    }
    /// `SignText.hasAnyClickCommands`: a line whose style has a `run_command`
    /// click event.
    pub fn has_any_click_commands(&self) -> bool {
        self.click
            .iter()
            .flatten()
            .any(|event| matches!(click_action(event), Some(ClickAction::RunCommand(_))))
    }
    /// `AbstractSignRenderer.getDarkColor`.
    pub fn dark_color(&self) -> u32 {
        let c = self.text_color();
        if c == 0 && self.glowing {
            0xF0EBCC
        } else {
            let s = |shift: u32| ((((c >> shift) & 255) as f32 * 0.4) as u32) << shift;
            s(16) | s(8) | s(0)
        }
    }
    /// The component a line is saved as (`SignText.CODEC`): the one it was
    /// read as while its text is unchanged. A line edited since is a literal in
    /// the old line's root style, as `SignBlockEntity.setMessages` makes it
    /// (the style keeps its colour and click event, the siblings go).
    pub fn message(&self, i: usize) -> Tag {
        let line = &self.lines[i];
        let saved = self.saved[i].as_ref();
        if let Some(saved) = saved.filter(|s| s.text == *line) {
            return saved.component.clone();
        }
        let mut style = match saved {
            Some(saved) => root_style(&saved.component),
            None => self.click[i]
                .iter()
                .map(|event| ("click_event", event.clone()))
                .collect(),
        };
        if style.is_empty() {
            return Tag::String(line.as_str().into());
        }
        style.insert(0, ("text", Tag::String(line.as_str().into())));
        Tag::compound(style)
    }
    /// A line as the font draws it: its styled runs, `§` codes not yet applied.
    pub fn line_text(&self, i: usize) -> Text {
        // Signs are drawn every frame: a line read as plain text (most of them) skips the
        // NBT -> JSON -> component round trip. Its click event does not change the drawing.
        if self.saved[i].is_none() {
            return Text::untrusted(self.lines[i].as_str());
        }
        scoreboard::comp_text(&scoreboard::tag_to_json(&self.message(i)))
    }
}

impl SignData {
    pub fn id(&self) -> &'static str {
        if self.hanging {
            "minecraft:hanging_sign"
        } else {
            "minecraft:sign"
        }
    }
    /// `SignBlockEntity.getMaxTextLineWidth`.
    pub fn max_line_width(&self) -> f32 {
        if self.hanging { 60.0 } else { 90.0 }
    }
    /// `SignBlockEntity.getTextLineHeight`.
    pub fn line_height(&self) -> f32 {
        if self.hanging { 9.0 } else { 10.0 }
    }
    /// `SignBlockEntity.canExecuteClickCommands`: a waxed sign whose side
    /// has a `run_command` click event. (Sunrise keeps no filtered text, so
    /// `Player.isTextFilteringEnabled` never selects it.)
    pub fn can_execute_click_commands(&self, front: bool) -> bool {
        let side = if front { &self.front } else { &self.back };
        self.waxed && side.has_any_click_commands()
    }
    /// NBT entries after the common block-entity header.
    pub fn encode(&self) -> Vec<(&'static str, Tag)> {
        let side = |t: &SignText| {
            Tag::compound([
                (
                    "messages",
                    Tag::List((0..t.lines.len()).map(|i| t.message(i)).collect()),
                ),
                (
                    "color",
                    Tag::String(DYE_TEXT[usize::from(t.color.min(15))].0.into()),
                ),
                ("has_glowing_text", Tag::Byte(i8::from(t.glowing))),
            ])
        };
        let mut out = Vec::new();
        if self.allow_op_features {
            out.push(("allow_op_features", Tag::Byte(1)));
        }
        out.extend([
            ("front_text", side(&self.front)),
            ("back_text", side(&self.back)),
            ("is_waxed", Tag::Byte(i8::from(self.waxed))),
        ]);
        out
    }
    pub fn decode(&mut self, tag: &Tag) {
        let side = |t: Option<&Tag>| -> SignText {
            let mut out = SignText::default();
            let Some(t) = t else { return out };
            if let Some(messages) = t.get("messages") {
                // `SignText.CODEC` reads a side all or nothing: a messages
                // field that is not a list leaves the side empty.
                let Tag::List(messages) = messages else {
                    return SignText::default();
                };
                for (i, m) in messages.iter().take(4).enumerate() {
                    out.lines[i] = component_text(m);
                    out.click[i] = click_event(m);
                    out.saved[i] = is_styled(m).then(|| SavedLine {
                        text: out.lines[i].clone(),
                        component: m.clone(),
                    });
                }
            }
            if let Some(Tag::String(c)) = t.get("color") {
                let name = c.to_string_lossy();
                if let Some(i) = DYE_TEXT.iter().position(|(n, _)| *n == name) {
                    out.color = i as u8;
                }
            }
            out.glowing = nbt_flag(t, "has_glowing_text");
            out
        };
        self.front = side(tag.get("front_text"));
        self.back = side(tag.get("back_text"));
        self.waxed = nbt_flag(tag, "is_waxed");
        self.allow_op_features =
            matches!(tag.get("allow_op_features"), Some(Tag::Byte(b)) if *b != 0);
    }
}
/// The `click_event` in the style of a stored line: the one of a component
/// object, or of the first component of a list (`createFromList` copies it
/// and appends the rest as siblings).
fn click_event(tag: &Tag) -> Option<Tag> {
    let root = match tag {
        Tag::List(list) => list.first()?,
        other => other,
    };
    match root.get("click_event") {
        Some(event @ Tag::Compound(_)) => Some(event.clone()),
        _ => None,
    }
}
/// Whether a saved message is more than its text: a style, siblings, another
/// kind of content, or a list. A string and a lone `{text: ...}` are not.
fn is_styled(tag: &Tag) -> bool {
    match tag {
        Tag::List(_) => true,
        Tag::Compound(c) => !(c.len() == 1 && matches!(tag.get("text"), Some(Tag::String(_)))),
        _ => false,
    }
}
/// Whether a message's contents is `PlainTextContents` (`Component.getContents`):
/// a string, a `{text: ...}` object, or a list of them (`createFromList` keeps
/// the first element's contents and appends the rest as siblings).
fn plain_contents(tag: &Tag) -> bool {
    match tag {
        Tag::String(_) => true,
        Tag::List(list) => list.first().is_some_and(plain_contents),
        Tag::Compound(_) => tag.get("text").is_some(),
        _ => false,
    }
}
/// The fields of a component's own style (`Style.CODEC`), the ones a literal
/// made from it keeps. A list's style is its first element's.
const STYLE_KEYS: [&str; 11] = [
    "color",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "shadow_color",
    "font",
    "click_event",
    "hover_event",
    "insertion",
];
fn root_style(component: &Tag) -> Vec<(&'static str, Tag)> {
    let root = match component {
        Tag::List(list) => list.first(),
        other => Some(other),
    };
    STYLE_KEYS
        .iter()
        .filter_map(|key| Some((*key, root?.get(key)?.clone())))
        .collect()
}
/// `ComponentSerialization.CODEC`: modern NBT strings are literal text, not JSON.
fn component_text(tag: &Tag) -> String {
    match tag {
        Tag::String(s) => s.to_string_lossy(),
        // Component.getString: translations and key binds resolved.
        // The editor sends these lines back to the server: a key only Sunrise's own files give
        // stays the key, as in vanilla's client, so a server's sign can't tell this client by it.
        Tag::Compound(_) | Tag::List(_) => sunrise_game_data::lang::returned_to_server(|| {
            scoreboard::comp_text(&scoreboard::tag_to_json(tag)).plain()
        }),
        _ => String::new(),
    }
}
/// Whether a vanilla block's name is a sign (standing, wall, hanging or wall
/// hanging). A mod's sign goes by its template's class, whatever its name; a
/// mod's `road_sign` of stone is no sign.
pub fn sign_kind(name: &str) -> Option<bool> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if name.contains(':') {
        None
    } else if name == "hanging_sign" || name.ends_with("_hanging_sign") {
        Some(true)
    } else if name == "sign" || name.ends_with("_sign") {
        Some(false)
    } else {
        None
    }
}
/// The sprite of a wood type's editor background, `folder` being `signs` or
/// `hanging_signs`: `textures/gui/<folder>/<wood>.png` (`SignEditScreen`), in
/// a mod's namespace when the wood names one.
pub fn editor_sprite(folder: &str, wood: &str) -> String {
    match wood.split_once(':') {
        Some((namespace, path)) if namespace != "minecraft" => {
            format!("{namespace}:gui/{folder}/{path}")
        }
        Some((_, path)) => format!("gui/{folder}/{path}"),
        None => format!("gui/{folder}/{wood}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A sign's lines are what the editor sends back: a translation key only Sunrise's own files
    /// give stays the key, as vanilla's client leaves it.
    #[test]
    fn a_sign_resolves_no_key_only_sunrise_gives() {
        use sunrise_game_data::lang;
        let key = "sunrise.osk.close";
        let side = Tag::compound([(
            "messages",
            Tag::List(vec![
                Tag::compound([("translate", Tag::String(key.into()))]),
                Tag::String("plain".into()),
                Tag::String("".into()),
            ]),
        )]);
        let tag = Tag::compound([("front_text", side)]);
        let english = || lang::Language::load(&[], "en_us");
        let screens = lang::scoped(english(), || lang::get(key)).0;
        assert_ne!(screens, key, "the screens do know the key");
        let (lines, _) = lang::scoped(english(), || {
            let mut decoded = SignData::default();
            decoded.decode(&tag);
            decoded.front.lines
        });
        assert_eq!(lines[0], key);
        assert_eq!(lines[1], "plain");
    }
    /// `SignText.CODEC` reads messages as a list: a string field fails the
    /// whole side, taking its colour and glow with it.
    #[test]
    fn messages_that_are_not_a_list_load_an_empty_side() {
        let tag = Tag::compound([(
            "front_text",
            Tag::compound([
                ("messages", Tag::String("not a list".into())),
                ("color", Tag::String("red".into())),
                ("has_glowing_text", Tag::Byte(1)),
            ]),
        )]);
        let mut decoded = SignData::default();
        decoded.decode(&tag);
        assert_eq!(
            decoded.front,
            SignText::default(),
            "a failed message list takes the side's colour and glow with it"
        );
    }

    #[test]
    fn waxed_and_glowing_text_take_any_number() {
        let tag = Tag::compound([
            ("is_waxed", Tag::Int(1)),
            (
                "front_text",
                Tag::compound([("has_glowing_text", Tag::Short(1))]),
            ),
        ]);
        let mut decoded = SignData::default();
        decoded.decode(&tag);
        assert!(decoded.waxed && decoded.front.glowing);
    }
    #[test]
    fn round_trips_nbt() {
        let mut data = SignData {
            hanging: false,
            ..SignData::default()
        };
        data.front.lines[0] = "Hello".into();
        data.front.color = 14;
        data.back.glowing = true;
        let tag = Tag::compound(data.encode());
        let mut decoded = SignData::default();
        decoded.decode(&tag);
        assert_eq!(decoded, data);
    }
    #[test]
    fn modern_string_messages_keep_literal_quotes_and_json_text() {
        let lines = [
            "plain text",
            "\"quoted text\"",
            r#"{"text":"literal object","bold":true}"#,
            "",
        ];
        let mut saved = styled_sign();
        let messages = Tag::List(lines.map(|line| Tag::String(line.into())).to_vec());
        if let Tag::Compound(front) = saved.get_mut("front_text").unwrap() {
            front.insert("messages".into(), messages.clone());
        }
        let mut data = SignData::default();
        data.decode(&saved);
        assert_eq!(data.front.lines, lines.map(str::to_owned));
        let encoded = Tag::compound(data.encode());
        assert_eq!(
            encoded.get("front_text").unwrap().get("messages"),
            Some(&messages)
        );
        let mut copied = SignData::default();
        copied.decode(&encoded);
        assert_eq!(copied.front.lines, data.front.lines);
        assert_eq!(
            front_messages(&Tag::compound(copied.encode())),
            front_messages(&encoded)
        );
    }
    /// A sign as a server sends it: a green "Online" and a plain line, the
    /// way `ComponentSerialization.CODEC` writes them to NBT.
    fn styled_sign() -> Tag {
        let green = Tag::compound([
            ("text", Tag::String("Online".into())),
            ("color", Tag::String("green".into())),
        ]);
        let front = Tag::compound([(
            "messages",
            Tag::List(vec![
                Tag::String("[KnockIT-1]".into()),
                green,
                Tag::String("".into()),
                Tag::String("0 / 20".into()),
            ]),
        )]);
        Tag::compound([("front_text", front)])
    }
    fn front_messages(tag: &Tag) -> &Vec<Tag> {
        match tag.get("front_text").and_then(|t| t.get("messages")) {
            Some(Tag::List(list)) => list,
            other => panic!("no messages: {other:?}"),
        }
    }
    fn run_colors(text: &SignText, line: usize) -> Vec<(String, Option<u32>)> {
        text.line_text(line)
            .spans
            .iter()
            .map(|s| (s.text.clone(), s.style.color))
            .collect()
    }
    #[test]
    fn a_lines_color_is_read_and_written_back() {
        let saved = styled_sign();
        let mut data = SignData::default();
        data.decode(&saved);
        assert_eq!(data.front.lines[1], "Online");
        // Green is `ChatFormatting.GREEN`; the dye colour of the side is untouched.
        assert_eq!(
            run_colors(&data.front, 1),
            [("Online".to_owned(), Some(0x55FF55))]
        );
        assert_eq!(
            run_colors(&data.front, 0),
            [("[KnockIT-1]".to_owned(), None)]
        );
        assert!(run_colors(&data.front, 2).is_empty());
        // Saved and sent as it came: the coloured line keeps its component and
        // the plain ones stay plain strings.
        let encoded = Tag::compound(data.encode());
        assert_eq!(front_messages(&encoded), front_messages(&saved));
    }
    #[test]
    fn a_list_component_shows_all_its_parts_in_their_own_styles() {
        let tag = Tag::compound([(
            "front_text",
            Tag::compound([(
                "messages",
                Tag::List(vec![Tag::List(vec![
                    Tag::compound([
                        ("text", Tag::String("Hi ".into())),
                        ("color", Tag::String("red".into())),
                    ]),
                    Tag::compound([
                        ("text", Tag::String("there".into())),
                        ("color", Tag::String("#00FF00".into())),
                    ]),
                ])]),
            )]),
        )]);
        let mut data = SignData::default();
        data.decode(&tag);
        assert_eq!(data.front.lines[0], "Hi there");
        // The list's later parts take the first one's style unless they set their own.
        assert_eq!(
            run_colors(&data.front, 0),
            [
                ("Hi ".to_owned(), Some(0xFF5555)),
                ("there".to_owned(), Some(0x00FF00))
            ]
        );
    }
    #[test]
    fn an_edited_line_keeps_only_the_old_lines_style() {
        let mut data = SignData::default();
        data.decode(&styled_sign());
        // `SignBlockEntity.setMessages`: a new literal with the old line's style.
        data.front.lines[1] = "Full".into();
        let encoded = Tag::compound(data.encode());
        assert_eq!(
            front_messages(&encoded),
            &vec![
                Tag::String("[KnockIT-1]".into()),
                Tag::compound([
                    ("text", Tag::String("Full".into())),
                    ("color", Tag::String("green".into())),
                ]),
                Tag::String("".into()),
                Tag::String("0 / 20".into()),
            ]
        );
        assert_eq!(
            run_colors(&data.front, 1),
            [("Full".to_owned(), Some(0x55FF55))]
        );
        // A line that had no style stays plain.
        data.front.lines[0] = "[KnockIT-2]".into();
        assert!(matches!(data.front.message(0), Tag::String(_)));
    }
    #[test]
    fn a_translated_line_is_not_flattened_by_a_round_trip() {
        let tag = Tag::compound([(
            "front_text",
            Tag::compound([(
                "messages",
                Tag::List(vec![Tag::compound([(
                    "translate",
                    Tag::String("block.minecraft.stone".into()),
                )])]),
            )]),
        )]);
        let mut data = SignData::default();
        data.decode(&tag);
        let encoded = Tag::compound(data.encode());
        assert_eq!(front_messages(&encoded)[0], front_messages(&tag)[0]);
    }
    #[test]
    fn a_mods_sign_like_name_does_not_make_it_a_sign() {
        assert_eq!(sign_kind("ruby:road_sign"), None);
    }
    #[test]
    fn names_and_colors() {
        assert_eq!(sign_kind("minecraft:oak_wall_hanging_sign"), Some(true));
        assert_eq!(sign_kind("minecraft:sign"), Some(false));
        assert_eq!(sign_kind("minecraft:spruce_wall_sign"), Some(false));
        assert_eq!(sign_kind("minecraft:stone"), None);
        assert_eq!(editor_sprite("signs", "dark_oak"), "gui/signs/dark_oak");
        assert_eq!(
            editor_sprite("signs", "minecraft:cherry"),
            "gui/signs/cherry"
        );
        assert_eq!(
            editor_sprite("hanging_signs", "ruby:elm"),
            "ruby:gui/hanging_signs/elm"
        );
        assert_eq!(
            editor_sprite("signs", "farmersdelight:canvas"),
            "farmersdelight:gui/signs/canvas"
        );
        assert_eq!(
            editor_sprite("hanging_signs", "farmersdelight:canvas"),
            "farmersdelight:gui/hanging_signs/canvas"
        );
        assert_eq!(SignText::default().dark_color(), 0);
        let red = SignText {
            color: 14,
            ..SignText::default()
        };
        assert_eq!(red.dark_color(), 0x660000);
    }
}
