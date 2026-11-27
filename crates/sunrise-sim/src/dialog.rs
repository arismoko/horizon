//! Server dialogs (`net.minecraft.server.dialog`): what `Dialog.DIRECT_CODEC`
//! decodes from a `show_dialog` packet, the values its inputs hold, and the
//! click event an action makes of them (`Action.createAction`).
mod encode;

use crate::{command::Registries, text::Text};
use serde_json::{Value, json};
use std::{cell::RefCell, num::NonZeroUsize};
use sunrise_core::{identifier::Identifier, nbt::Tag};

/// `TextInputHandler.addControl`: the 20 high `EditBox`, or the
/// `MultiLineEditBox` of the options' height, by default as tall as their most
/// lines (4 without a limit) need, at most 512.
pub fn text_height(multiline: Option<(Option<i32>, Option<i32>)>) -> f32 {
    match multiline {
        None => 20.0,
        Some((max_lines, height)) => height.map_or_else(
            || (9 * i64::from(max_lines.unwrap_or(4)) + 8).min(512) as f32,
            |height| height as f32,
        ),
    }
}

/// `CommonDialogData` and the type's own fields.
#[derive(Clone, Debug)]
pub struct Dialog<T = Text> {
    pub kind: Kind<T>,
    pub title: T,
    pub external_title: Option<T>,
    pub can_close_with_escape: bool,
    pub pause: bool,
    pub after_action: AfterAction,
    pub body: Vec<Body<T>>,
    pub inputs: Vec<Input<T>>,
}

/// The dialog types (`DialogTypes`).
#[derive(Clone, Debug)]
pub enum Kind<T = Text> {
    /// `NoticeDialog`: one button, "OK" when the server names none.
    Notice(Button<T>),
    Confirmation {
        yes: Button<T>,
        no: Button<T>,
    },
    MultiAction {
        actions: Vec<Button<T>>,
        exit: Option<Button<T>>,
        columns: NonZeroUsize,
    },
    ServerLinks {
        exit: Option<Button<T>>,
        columns: NonZeroUsize,
        button_width: i32,
    },
    /// `DialogListDialog`: its dialogs as sent (ids, a tag or inline dialogs),
    /// and one button per dialog they resolve to (`set_list`).
    DialogList {
        dialogs: Tag,
        entries: Vec<Button<T>>,
        exit: Option<Button<T>>,
        columns: NonZeroUsize,
        button_width: i32,
    },
}

/// `DialogAction`: the screen after an action runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterAction {
    Close,
    None,
    WaitForResponse,
}

/// `ActionButton`: `CommonButtonData` and what it does.
#[derive(Clone, Debug)]
pub struct Button<T = Text> {
    pub label: T,
    pub tooltip: Option<T>,
    pub width: i32,
    pub action: Option<Action>,
}

/// `Action`: a fixed click event, or one made from the inputs' values.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// `StaticAction`: a click event as it is.
    Static(Click),
    /// `CommandTemplate`: a command with `$(key)` replaced by input values.
    RunCommand(String),
    /// `CustomAll`: a custom click event whose payload is `additions` plus
    /// every input's value under its key.
    Custom { id: String, additions: Option<Tag> },
}

/// `ClickEvent`, the kinds a server may send (`isAllowedFromServer`).
#[derive(Clone, Debug, PartialEq)]
pub enum Click {
    OpenUrl(String),
    RunCommand(String),
    SuggestCommand(String),
    /// The dialog: an id of the `minecraft:dialog` registry or the dialog itself.
    ShowDialog(Tag),
    ChangePage(i32),
    CopyToClipboard(String),
    Custom {
        id: String,
        payload: Option<Tag>,
    },
}

/// `DialogBody`.
#[derive(Clone, Debug)]
pub enum Body<T = Text> {
    /// `PlainMessage`: centred text wrapped to `width`.
    Message { contents: T, width: i32 },
    /// `ItemBody`: the item as `ItemStackTemplate` sent it.
    Item {
        item: Tag,
        description: Option<(T, i32)>,
        show_decorations: bool,
        show_tooltip: bool,
        width: i32,
        height: i32,
    },
}

/// `Input`: the key its value goes by, and its control.
#[derive(Clone, Debug)]
pub struct Input<T = Text> {
    pub key: String,
    pub control: Control<T>,
}

/// `InputControl`.
#[derive(Clone, Debug)]
pub enum Control<T = Text> {
    Text {
        width: i32,
        label: T,
        label_visible: bool,
        initial: String,
        max_length: i32,
        /// `MultilineOptions`: the most lines and the height, when set.
        multiline: Option<(Option<i32>, Option<i32>)>,
    },
    Boolean {
        label: T,
        initial: bool,
        on_true: String,
        on_false: String,
    },
    SingleOption {
        width: i32,
        /// Each option's id, its label and whether it is the initial one.
        options: Vec<(String, Option<T>, bool)>,
        label: T,
        label_visible: bool,
    },
    NumberRange {
        width: i32,
        label: T,
        label_format: String,
        start: f32,
        end: f32,
        initial: Option<f32>,
        step: Option<f32>,
    },
}

/// What an input holds while the dialog is open.
#[derive(Clone, Debug, PartialEq)]
pub enum InputValue {
    Text(String),
    Bool(bool),
    /// The chosen option's index.
    Option(usize),
    /// The slider's position, 0 to 1.
    Slider(f32),
}

fn number(t: Option<&Tag>) -> Option<f64> {
    Some(match t? {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => f64::from(f32::from_bits(*v)),
        Tag::Double(v) => f64::from_bits(*v),
        _ => return None,
    })
}
/// A registry name without its default namespace.
fn path(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

impl Action {
    /// `createAction`: the click event, given each input's key, control and value.
    pub fn click(&self, inputs: &[Input], values: &[InputValue]) -> Option<Click> {
        let values = || inputs.iter().zip(values);
        Some(match self {
            Self::Static(click) => click.clone(),
            Self::RunCommand(template) => Click::RunCommand(substitute(template, &|key| {
                values()
                    .find(|(i, _)| i.key == key)
                    .map(|(i, v)| i.control.substitution(v))
            })),
            Self::Custom { id, additions } => {
                let mut tag = match additions {
                    Some(t @ Tag::Compound(_)) => t.clone(),
                    _ => Tag::Compound(Default::default()),
                };
                for (input, value) in values() {
                    if let Tag::Compound(map) = &mut tag {
                        map.insert(input.key.as_str().into(), input.control.tag(value));
                    }
                }
                Click::Custom {
                    id: id.clone(),
                    payload: Some(tag),
                }
            }
        })
    }
}

/// `ParsedTemplate.instantiate`: each `$(key)` replaced by its value, or by
/// nothing for a key no input has.
fn substitute(template: &str, value: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(at) = rest.find("$(") {
        let Some(end) = rest[at..].find(')') else {
            break;
        };
        out.push_str(&rest[..at]);
        out.push_str(&value(&rest[at + 2..at + end]).unwrap_or_default());
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
}

/// `StringTag.escapeWithoutQuotes`.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\'' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if c < ' ' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

impl Control {
    /// The value it starts with.
    pub fn initial(&self) -> InputValue {
        match self {
            Self::Text { initial, .. } => InputValue::Text(initial.clone()),
            Self::Boolean { initial, .. } => InputValue::Bool(*initial),
            Self::SingleOption { options, .. } => {
                InputValue::Option(options.iter().position(|o| o.2).unwrap_or(0))
            }
            Self::NumberRange {
                start,
                end,
                initial,
                ..
            } => InputValue::Slider(to_slider(
                initial.unwrap_or((start + end) / 2.0),
                *start,
                *end,
            )),
        }
    }
    /// The text on a cycle button or slider holding `value`
    /// (`CycleButton.createFullName`, `NumberRangeInput.computeLabel`).
    pub fn message(&self, value: &InputValue) -> Option<Text> {
        match (self, value) {
            (
                Self::SingleOption {
                    options,
                    label,
                    label_visible,
                    ..
                },
                InputValue::Option(i),
            ) => {
                let (id, display, _) = options.get(*i)?;
                let shown = display.clone().unwrap_or_else(|| Text::literal(id));
                Some(if *label_visible {
                    Text::translate("options.generic_value", &[label.clone(), shown])
                } else {
                    shown
                })
            }
            (
                Self::NumberRange {
                    label,
                    label_format,
                    ..
                },
                InputValue::Slider(_),
            ) => {
                let shown = number_string(self.scaled(value)?);
                Some(Text::translate(
                    label_format,
                    &[label.clone(), Text::literal(shown)],
                ))
            }
            _ => None,
        }
    }
    /// `ValueGetter.asTemplateSubstitution`.
    pub fn substitution(&self, value: &InputValue) -> String {
        match (self, value) {
            (Self::Text { .. }, InputValue::Text(s)) => escape(s),
            (
                Self::Boolean {
                    on_true, on_false, ..
                },
                InputValue::Bool(b),
            ) => if *b { on_true } else { on_false }.clone(),
            (Self::SingleOption { options, .. }, InputValue::Option(i)) => {
                options.get(*i).map(|o| o.0.clone()).unwrap_or_default()
            }
            (Self::NumberRange { .. }, InputValue::Slider(_)) => {
                number_string(self.scaled(value).unwrap_or(0.0))
            }
            _ => String::new(),
        }
    }
    /// `ValueGetter.asTag`.
    pub fn tag(&self, value: &InputValue) -> Tag {
        match (self, value) {
            (Self::Boolean { .. }, InputValue::Bool(b)) => Tag::Byte(i8::from(*b)),
            (Self::NumberRange { .. }, _) => {
                Tag::Float(self.scaled(value).unwrap_or(0.0).to_bits())
            }
            (Self::Text { .. }, InputValue::Text(s)) => Tag::String(s.as_str().into()),
            _ => Tag::String(self.substitution(value).as_str().into()),
        }
    }
    /// `RangeInfo.computeScaledValue`: a number range's value at its slider position.
    pub fn scaled(&self, value: &InputValue) -> Option<f32> {
        let (
            Self::NumberRange {
                start,
                end,
                initial,
                step,
                ..
            },
            InputValue::Slider(slider),
        ) = (self, value)
        else {
            return None;
        };
        let (start, end) = (*start, *end);
        let in_range = start + slider * (end - start);
        let Some(step) = step else {
            return Some(in_range);
        };
        let first = initial.unwrap_or((start + end) / 2.0);
        // Math.round: half up.
        let steps = ((in_range - first) / step + 0.5).floor() as i32;
        let result = first + steps as f32 * step;
        let position = to_slider(result, start, end);
        if (0.0..=1.0).contains(&position) {
            return Some(result);
        }
        Some(first + (steps - steps.signum()) as f32 * step)
    }
}

/// `RangeInfo.scaledValueToSlider`.
fn to_slider(value: f32, start: f32, end: f32) -> f32 {
    if start == end {
        0.5
    } else {
        (value - start) / (end - start)
    }
}

/// `SliderImpl.valueToString`: whole numbers without a fraction, the rest as
/// Java's `Float.toString`.
pub fn number_string(v: f32) -> String {
    if v == (v as i32) as f32 {
        (v as i32).to_string()
    } else {
        sunrise_core::java_number::float_to_string(v)
    }
}

/// Where a dialog is read from: a data pack's JSON at world open, or the NBT a
/// server sends. Each reads a value as the matching `DynamicOps` does
/// (`JsonOps`, `NbtOps`), so one decoder serves both.
trait Source: Sized {
    const NBT: bool;
    fn component_json(&self) -> Option<std::borrow::Cow<'_, Value>>;
    /// An object's field. A JSON `null` is an absent field (`JsonOps`' `MapLike`).
    fn get(&self, key: &str) -> Option<&Self>;
    fn is_map(&self) -> bool;
    fn list(&self) -> Option<&[Self]>;
    fn string(&self) -> Option<String>;
    fn boolean(&self) -> Option<bool>;
    /// `Number.intValue`.
    fn int(&self) -> Option<i32>;
    /// `Number.floatValue`.
    fn float(&self) -> Option<f32>;
    fn tag(&self) -> Tag;
    fn nbt(&self) -> Option<&Tag> {
        None
    }
}

impl Source for Tag {
    const NBT: bool = true;

    fn component_json(&self) -> Option<std::borrow::Cow<'_, Value>> {
        sunrise_core::json_nbt::tag_to_json(self)
            .ok()
            .map(std::borrow::Cow::Owned)
    }

    fn get(&self, key: &str) -> Option<&Self> {
        Tag::get(self, key)
    }
    fn is_map(&self) -> bool {
        matches!(self, Tag::Compound(_))
    }
    fn list(&self) -> Option<&[Self]> {
        match self {
            Tag::List(items) => Some(items),
            _ => None,
        }
    }
    fn string(&self) -> Option<String> {
        match self {
            Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        }
    }
    /// `NbtOps.getBooleanValue`: any number but zero.
    fn boolean(&self) -> Option<bool> {
        number(Some(self)).map(|n| n != 0.0)
    }
    fn int(&self) -> Option<i32> {
        Some(match self {
            Tag::Byte(v) => i32::from(*v),
            Tag::Short(v) => i32::from(*v),
            Tag::Int(v) => *v,
            Tag::Long(v) => *v as i32,
            // `Mth.floor`.
            Tag::Float(v) => f32::from_bits(*v).floor() as i32,
            Tag::Double(v) => f64::from_bits(*v).floor() as i32,
            _ => return None,
        })
    }
    fn float(&self) -> Option<f32> {
        number(Some(self)).map(|v| v as f32)
    }
    fn tag(&self) -> Tag {
        self.clone()
    }
    fn nbt(&self) -> Option<&Tag> {
        Some(self)
    }
}

impl Source for Value {
    const NBT: bool = false;

    fn component_json(&self) -> Option<std::borrow::Cow<'_, Value>> {
        Some(std::borrow::Cow::Borrowed(self))
    }

    fn get(&self, key: &str) -> Option<&Self> {
        Value::get(self, key).filter(|v| !v.is_null())
    }
    fn is_map(&self) -> bool {
        self.is_object()
    }
    fn list(&self) -> Option<&[Self]> {
        self.as_array().map(Vec::as_slice)
    }
    fn string(&self) -> Option<String> {
        self.as_str().map(str::to_owned)
    }
    /// `JsonOps.getBooleanValue`: a JSON boolean, never a number.
    fn boolean(&self) -> Option<bool> {
        self.as_bool()
    }
    /// `LazilyParsedNumber.intValue`: the integer part's low 32 bits.
    fn int(&self) -> Option<i32> {
        let n = self.as_number()?;
        Some(match (n.as_i64(), n.as_u64()) {
            (Some(v), _) => v as i32,
            (_, Some(v)) => v as i32,
            _ => n.as_f64()?.trunc() as i128 as i32,
        })
    }
    fn float(&self) -> Option<f32> {
        self.as_number()?.as_f64().map(|v| v as f32)
    }
    fn tag(&self) -> Tag {
        crate::lan::data::json_to_nbt(self)
    }
}

/// A dialog another entry or the registry holds (`Dialog.CODEC`'s holder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    /// A registry id, with its namespace.
    Id(String),
    /// A `#` tag, without the `#`.
    Tag(String),
}

type PersistentText<'a> = dyn Fn(&Tag) -> Option<Tag> + 'a;

/// The registries shared by the dialog and its text components.
struct Reader<'a> {
    registries: &'a Registries,
    item: &'a dyn Fn(&str) -> bool,
    references: RefCell<Vec<Reference>>,
    persistent_text: Option<&'a PersistentText<'a>>,
}

/// The path of a failing field: its name, then the path inside it.
fn at(key: &str, inner: String) -> String {
    match inner.chars().next() {
        None => key.to_owned(),
        Some('[') => format!("{key}{inner}"),
        _ => format!("{key}.{inner}"),
    }
}
/// A field the codec defaults when absent (`optionalFieldOf`). A field that is
/// there and unreadable fails; it is never defaulted.
fn optional<S: Source, T>(
    t: &S,
    key: &str,
    read: impl FnOnce(&S) -> Result<T, String>,
) -> Result<Option<T>, String> {
    t.get(key)
        .map(read)
        .transpose()
        .map_err(|inner| at(key, inner))
}
/// A field the codec requires (`fieldOf`).
fn required<S: Source, T>(
    t: &S,
    key: &str,
    read: impl FnOnce(&S) -> Result<T, String>,
) -> Result<T, String> {
    optional(t, key, read)?.ok_or_else(|| key.to_owned())
}
/// A list's members, each read or the list refused with the member's path.
fn members<S: Source, T>(
    t: &S,
    mut read: impl FnMut(&S) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    t.list()
        .ok_or_default()?
        .iter()
        .enumerate()
        .map(|(i, member)| read(member).map_err(|inner| at(&format!("[{i}]"), inner)))
        .collect()
}
/// `ExtraCodecs.intRange`.
fn ranged<S: Source>(t: &S, low: i32, high: i32) -> Result<i32, String> {
    t.int().filter(|n| (low..=high).contains(n)).ok_or_default()
}
fn boolean<S: Source>(t: &S) -> Result<bool, String> {
    t.boolean().ok_or_default()
}
fn string<S: Source>(t: &S) -> Result<String, String> {
    t.string().ok_or_default()
}
fn float<S: Source>(t: &S) -> Result<f32, String> {
    t.float().ok_or_default()
}
/// `StringTemplate.isValidVariableName`, over UTF-16 units as Java reads them.
fn variable_name(name: &str) -> bool {
    name.encode_utf16().all(|unit| {
        unit == u16::from(b'_') || sunrise_core::java_character::is_letter_or_digit(unit)
    })
}
/// `Identifier.CODEC`.
fn identifier<S: Source>(value: &S) -> Result<String, String> {
    Identifier::parse(&string(value)?)
        .map(|id| id.to_string())
        .map_err(|_| String::new())
}
/// `ExtraCodecs.CHAT_STRING`.
fn chat_string(s: &str) -> bool {
    s.chars()
        .all(|c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
}
/// `ParsedTemplate.CODEC`: `StringTemplate.fromString` accepts a string with at
/// least one `$(name)`, each name valid and closed.
fn template(s: &str) -> bool {
    let mut rest = s;
    let mut variables = 0;
    while let Some(at) = rest.find("$(") {
        let Some(end) = rest[at + 2..].find(')') else {
            return false;
        };
        if !variable_name(&rest[at + 2..at + 2 + end]) {
            return false;
        }
        variables += 1;
        rest = &rest[at + 3 + end..];
    }
    variables > 0
}

impl Reader<'_> {
    fn dialog_nbt<S: Source>(&self, t: &S) -> Result<Tag, String> {
        let dialog = self.dialog(
            t,
            |v| {
                if let Some(component) = self.persistent_text {
                    component(v.nbt().ok_or_default()?).ok_or_default()
                } else {
                    self.text(v, |click| click.encode("action"))
                }
            },
            || Tag::compound([("translate", Tag::String("gui.ok".into()))]),
        )?;
        Ok(dialog.encode())
    }

    fn text<S: Source>(&self, t: &S, mut click: impl FnMut(Click) -> Tag) -> Result<Tag, String> {
        let json = t.component_json().ok_or_default()?;
        let built =
            crate::text_grammar::build_for_dialog(&json, self.registries, S::NBT, self.item)
                .ok_or_default()?;
        self.component_nbt(t, built, &mut click)
    }

    /// Emit the fields selected by the shared component grammar. Its child
    /// components keep the shape of the source, so the click codec still sees
    /// the original JsonOps/NbtOps values (including an inline dialog).
    fn component_nbt<S: Source>(
        &self,
        source: &S,
        built: Value,
        project_click: &mut impl FnMut(Click) -> Tag,
    ) -> Result<Tag, String> {
        match built {
            Value::Array(items) => {
                let sources = source.list().ok_or_default()?;
                Ok(Tag::List(
                    sources
                        .iter()
                        .zip(items)
                        .map(|(raw, item)| self.component_nbt(raw, item, project_click))
                        .collect::<Result<_, _>>()?,
                ))
            }
            Value::Object(fields) => {
                let mut out = sunrise_core::nbt::Compound::new();
                for (key, value) in fields {
                    let tag = match key.as_str() {
                        "extra" | "with" | "separator" => self.component_nbt(
                            source.get(&key).ok_or_default()?,
                            value,
                            project_click,
                        )?,
                        "fallback" if value.is_object() || value.is_array() => self.component_nbt(
                            source.get(&key).ok_or_default()?,
                            value,
                            project_click,
                        )?,
                        "click_event" => {
                            let click = source.get(&key).ok_or_default()?;
                            let action = required(click, "action", string)?;
                            project_click(self.click(click, &action)?)
                        }
                        "hover_event" => {
                            let raw = source.get(&key).ok_or_default()?;
                            let Value::Object(fields) = value else {
                                unreachable!("decoded hover event")
                            };
                            let child = match fields["action"].as_str() {
                                Some("show_text") => Some("value"),
                                Some("show_entity") => Some("name"),
                                _ => None,
                            };
                            Tag::compound(
                                fields
                                    .into_iter()
                                    .map(|(key, value)| {
                                        let tag = if child == Some(key.as_str()) {
                                            self.component_nbt(
                                                raw.get(&key).ok_or_default()?,
                                                value,
                                                project_click,
                                            )?
                                        } else {
                                            crate::lan::data::json_to_nbt(&value)
                                        };
                                        Ok((sunrise_core::nbt::JavaString::from(key.as_str()), tag))
                                    })
                                    .collect::<Result<Vec<_>, String>>()?,
                            )
                        }
                        _ => crate::lan::data::json_to_nbt(&value),
                    };
                    out.insert(key.as_str().into(), tag);
                }
                Ok(Tag::Compound(out))
            }
            value => Ok(crate::lan::data::json_to_nbt(&value)),
        }
    }

    /// `Dialog.DIRECT_CODEC`, dispatched on `type`.
    fn dialog<S: Source, T>(
        &self,
        t: &S,
        text: impl Fn(&S) -> Result<T, String> + Copy,
        default_label: impl FnOnce() -> T,
    ) -> Result<Dialog<T>, String> {
        if !t.is_map() {
            return Err(String::new());
        }
        let kind = required(t, "type", string)?;
        let pause = optional(t, "pause", boolean)?.unwrap_or(true);
        let after_action = optional(t, "after_action", |v| match string(v)?.as_str() {
            "close" => Ok(AfterAction::Close),
            "none" => Ok(AfterAction::None),
            "wait_for_response" => Ok(AfterAction::WaitForResponse),
            _ => Err(String::new()),
        })?
        .unwrap_or(AfterAction::Close);
        // CommonDialogData: a dialog that pauses must unpause after an action.
        if pause && after_action == AfterAction::None {
            return Err("after_action: cannot be none when pause is true".into());
        }
        let columns = || {
            let n = optional(t, "columns", |v| ranged(v, 1, i32::MAX))?.unwrap_or(2);
            NonZeroUsize::new(n as usize).ok_or_else(|| "columns".to_owned())
        };
        let width = || -> Result<i32, String> {
            Ok(optional(t, "button_width", |v| ranged(v, 1, 1024))?.unwrap_or(150))
        };
        let button = |key| optional(t, key, |v| self.button(v, text));
        let kind = match path(&kind) {
            "notice" => Kind::Notice(button("action")?.unwrap_or_else(|| Button {
                label: default_label(),
                tooltip: None,
                width: 150,
                action: None,
            })),
            "confirmation" => Kind::Confirmation {
                yes: required(t, "yes", |v| self.button(v, text))?,
                no: required(t, "no", |v| self.button(v, text))?,
            },
            "multi_action" => Kind::MultiAction {
                actions: required(t, "actions", |v| {
                    members(v, |b| self.button(b, text)).and_then(|actions| {
                        (!actions.is_empty()).then_some(actions).ok_or_default()
                    })
                })?,
                exit: button("exit_action")?,
                columns: columns()?,
            },
            "server_links" => Kind::ServerLinks {
                exit: button("exit_action")?,
                columns: columns()?,
                button_width: width()?,
            },
            "dialog_list" => Kind::DialogList {
                dialogs: required(t, "dialogs", |v| self.holders(v))?,
                entries: Vec::new(),
                exit: button("exit_action")?,
                columns: columns()?,
                button_width: width()?,
            },
            _ => return Err("type".into()),
        };
        // DialogBody.COMPACT_LIST_CODEC: one body, or a list of them.
        let body = optional(t, "body", |v| match v.list() {
            Some(_) => members(v, |b| self.body(b, text)),
            None => Ok(vec![self.body(v, text)?]),
        })?
        .unwrap_or_default();
        let inputs =
            optional(t, "inputs", |v| members(v, |i| self.input(i, text)))?.unwrap_or_default();
        Ok(Dialog {
            kind,
            title: required(t, "title", text)?,
            external_title: optional(t, "external_title", text)?,
            can_close_with_escape: optional(t, "can_close_with_escape", boolean)?.unwrap_or(true),
            pause,
            after_action,
            body,
            inputs,
        })
    }

    /// `Dialog.CODEC`: an id of the dialog registry, or the dialog itself.
    fn holder<S: Source>(&self, t: &S) -> Result<Tag, String> {
        if t.is_map() {
            return sunrise_core::nbt::with_tree_stack(|| self.dialog_nbt(t));
        }
        let id = identifier(t)?;
        self.references.borrow_mut().push(Reference::Id(id.clone()));
        Ok(Tag::String(id.as_str().into()))
    }

    /// `Dialog.LIST_CODEC`: a `#tag`, or one holder, or a list of them.
    fn holders<S: Source>(&self, t: &S) -> Result<Tag, String> {
        if let Some(id) = t.string().as_deref().and_then(|s| s.strip_prefix('#')) {
            let tag = Identifier::parse(id)
                .map_err(|_| String::new())?
                .to_string();
            self.references
                .borrow_mut()
                .push(Reference::Tag(tag.clone()));
            Ok(Tag::String(format!("#{tag}").as_str().into()))
        } else if t.list().is_some() {
            Ok(Tag::List(members(t, |holder| self.holder(holder))?))
        } else {
            self.holder(t)
        }
    }

    /// `ActionButton.CODEC`.
    fn button<S: Source, T>(
        &self,
        t: &S,
        text: impl Fn(&S) -> Result<T, String>,
    ) -> Result<Button<T>, String> {
        if !t.is_map() {
            return Err(String::new());
        }
        Ok(Button {
            label: required(t, "label", &text)?,
            tooltip: optional(t, "tooltip", &text)?,
            width: optional(t, "width", |v| ranged(v, 1, 1024))?.unwrap_or(150),
            action: optional(t, "action", |v| self.action(v))?,
        })
    }

    /// `Action.CODEC`, dispatched on `type`.
    fn action<S: Source>(&self, t: &S) -> Result<Action, String> {
        if !t.is_map() {
            return Err(String::new());
        }
        let kind = required(t, "type", string)?;
        Ok(match path(&kind) {
            "dynamic/run_command" => Action::RunCommand(required(t, "template", |v| {
                string(v).and_then(|s| template(&s).then_some(s).ok_or_default())
            })?),
            "dynamic/custom" => Action::Custom {
                id: required(t, "id", identifier)?,
                additions: optional(t, "additions", |v| {
                    v.is_map().then(|| v.tag()).ok_or_default()
                })?,
            },
            kind => Action::Static(self.click(t, kind)?),
        })
    }

    /// `ClickEvent.CODEC`, from its `action` (a click event in a component)
    /// or `type` (an action) field.
    fn click<S: Source>(&self, t: &S, kind: &str) -> Result<Click, String> {
        let command = || {
            required(t, "command", |v| {
                string(v).and_then(|s| chat_string(&s).then_some(s).ok_or_default())
            })
        };
        Ok(match kind {
            // ClickEvent.OpenUrl's url is an ExtraCodecs.UNTRUSTED_URI.
            "open_url" => Click::OpenUrl(required(t, "url", |v| {
                string(v).and_then(|url| {
                    crate::component::untrusted_http_url(&url)
                        .then_some(url)
                        .ok_or_default()
                })
            })?),
            "run_command" => Click::RunCommand(command()?),
            "suggest_command" => Click::SuggestCommand(command()?),
            "show_dialog" => Click::ShowDialog(required(t, "dialog", |v| self.holder(v))?),
            "change_page" => Click::ChangePage(required(t, "page", |v| ranged(v, 1, i32::MAX))?),
            "copy_to_clipboard" => Click::CopyToClipboard(required(t, "value", string)?),
            "custom" => Click::Custom {
                id: required(t, "id", identifier)?,
                payload: t.get("payload").map(S::tag),
            },
            _ => return Err("type".into()),
        })
    }

    /// `Input.CODEC`: its key, and `InputControl.MAP_CODEC` dispatched on `type`.
    fn input<S: Source, T>(
        &self,
        t: &S,
        text: impl Fn(&S) -> Result<T, String>,
    ) -> Result<Input<T>, String> {
        if !t.is_map() {
            return Err(String::new());
        }
        let kind = required(t, "type", string)?;
        let key = required(t, "key", |v| {
            string(v).and_then(|s| variable_name(&s).then_some(s).ok_or_default())
        })?;
        let label = || required(t, "label", &text);
        let label_visible = || -> Result<bool, String> {
            Ok(optional(t, "label_visible", boolean)?.unwrap_or(true))
        };
        let width = || -> Result<i32, String> {
            Ok(optional(t, "width", |v| ranged(v, 1, 1024))?.unwrap_or(200))
        };
        let control = match path(&kind) {
            "text" => {
                let initial = optional(t, "initial", string)?.unwrap_or_default();
                let max_length =
                    optional(t, "max_length", |v| ranged(v, 1, i32::MAX))?.unwrap_or(32);
                // TextInput: the default text may not be longer than the length.
                if initial.encode_utf16().count() > max_length as usize {
                    return Err("initial".into());
                }
                Control::Text {
                    width: width()?,
                    label: label()?,
                    label_visible: label_visible()?,
                    initial,
                    max_length,
                    multiline: optional(t, "multiline", |m| {
                        if !m.is_map() {
                            return Err(String::new());
                        }
                        Ok((
                            optional(m, "max_lines", |v| ranged(v, 1, i32::MAX))?,
                            optional(m, "height", |v| ranged(v, 1, 512))?,
                        ))
                    })?,
                }
            }
            "boolean" => Control::Boolean {
                label: label()?,
                initial: optional(t, "initial", boolean)?.unwrap_or(false),
                on_true: optional(t, "on_true", string)?.unwrap_or_else(|| "true".into()),
                on_false: optional(t, "on_false", string)?.unwrap_or_else(|| "false".into()),
            },
            "single_option" => {
                // SingleOptionInput.Entry.CODEC: a full entry or just its id.
                let options = required(t, "options", |v| {
                    members(v, |o| match o.string() {
                        Some(id) => Ok((id, None, false)),
                        None if o.is_map() => Ok((
                            required(o, "id", string)?,
                            optional(o, "display", &text)?,
                            optional(o, "initial", boolean)?.unwrap_or(false),
                        )),
                        None => Err(String::new()),
                    })
                })?;
                // A non-empty list with at most one initial entry.
                if options.is_empty() || options.iter().filter(|o| o.2).count() > 1 {
                    return Err("options".into());
                }
                Control::SingleOption {
                    width: width()?,
                    options,
                    label: label()?,
                    label_visible: label_visible()?,
                }
            }
            "number_range" => {
                let start = required(t, "start", float)?;
                let end = required(t, "end", float)?;
                let initial = optional(t, "initial", float)?;
                // `Float.MIN_VALUE` up to `Float.MAX_VALUE`.
                let step = optional(t, "step", |v| {
                    float(v).and_then(|s| {
                        (s >= f32::from_bits(1) && s <= f32::MAX)
                            .then_some(s)
                            .ok_or_default()
                    })
                })?;
                // The initial value lies between the ends, whichever is lower.
                if let Some(initial) = initial {
                    let (initial, start, end) =
                        (f64::from(initial), f64::from(start), f64::from(end));
                    // `Math.min` and `Math.max` are NaN when either end is.
                    let (low, high) = match start.is_nan() || end.is_nan() {
                        true => (f64::NAN, f64::NAN),
                        false => (start.min(end), start.max(end)),
                    };
                    if initial < low || initial > high {
                        return Err("initial".into());
                    }
                }
                Control::NumberRange {
                    width: width()?,
                    label: label()?,
                    label_format: optional(t, "label_format", string)?
                        .unwrap_or_else(|| "options.generic_value".into()),
                    start,
                    end,
                    initial,
                    step,
                }
            }
            _ => return Err("type".into()),
        };
        Ok(Input { key, control })
    }

    /// `PlainMessage.CODEC`: the full form or just its contents.
    fn message<S: Source, T>(
        &self,
        t: &S,
        text: impl Fn(&S) -> Result<T, String>,
    ) -> Result<(T, i32), String> {
        let full = || -> Result<(T, i32), String> {
            Ok((
                required(t, "contents", &text)?,
                optional(t, "width", |v| ranged(v, 1, 1024))?.unwrap_or(200),
            ))
        };
        full().or_else(|error| text(t).map(|text| (text, 200)).map_err(|_| error))
    }

    /// `DialogBody.DIALOG_BODY_CODEC`, dispatched on `type`.
    fn body<S: Source, T>(
        &self,
        t: &S,
        text: impl Fn(&S) -> Result<T, String>,
    ) -> Result<Body<T>, String> {
        if !t.is_map() {
            return Err(String::new());
        }
        let kind = required(t, "type", string)?;
        Ok(match path(&kind) {
            "plain_message" => {
                let (contents, width) = self.message(t, &text)?;
                Body::Message { contents, width }
            }
            "item" => Body::Item {
                item: required(t, "item", |v| self.stack(v))?,
                description: optional(t, "description", |v| self.message(v, &text))?,
                show_decorations: optional(t, "show_decorations", boolean)?.unwrap_or(true),
                show_tooltip: optional(t, "show_tooltip", boolean)?.unwrap_or(true),
                width: optional(t, "width", |v| ranged(v, 1, 256))?.unwrap_or(16),
                height: optional(t, "height", |v| ranged(v, 1, 256))?.unwrap_or(16),
            },
            _ => return Err("type".into()),
        })
    }

    /// `ItemStackTemplate.CODEC`: an item's id, or `id`, `count` and `components`.
    fn stack<S: Source>(&self, t: &S) -> Result<Tag, String> {
        let item = |t: &S| {
            let id = identifier(t)?;
            (id != "minecraft:air" && (self.item)(&id))
                .then_some(id)
                .ok_or_default()
        };
        let (id, count, components) = if t.is_map() {
            (
                required(t, "id", item)?,
                optional(t, "count", |v| ranged(v, 1, 99))?.unwrap_or(1),
                // The persistent component patch itself is not decoded yet.
                optional(t, "components", |v| {
                    v.is_map().then(|| v.tag()).ok_or_default()
                })?,
            )
        } else {
            (item(t)?, 1, None)
        };
        Ok(Tag::compound(
            [
                ("id", Tag::String(id.as_str().into())),
                ("count", Tag::Int(count)),
            ]
            .into_iter()
            .chain(components.map(|patch| ("components", patch))),
        ))
    }
}

/// `Option::ok_or_else(String::new)`: a value no field name can add to.
trait OkOrDefault<T> {
    fn ok_or_default(self) -> Result<T, String>;
}
impl<T> OkOrDefault<T> for Option<T> {
    fn ok_or_default(self) -> Result<T, String> {
        self.ok_or_else(String::new)
    }
}

/// Read and encode the `Dialog.CODEC` holder in a persistent text component,
/// retaining registry references by id and using the caller's NBT text codec.
pub fn persistent_holder(
    tag: &Tag,
    registries: &Registries,
    item: &dyn Fn(&str) -> bool,
    text: &dyn Fn(&Tag) -> Option<Tag>,
) -> Option<Tag> {
    let reader = Reader {
        registries,
        item,
        references: RefCell::default(),
        persistent_text: Some(text),
    };
    sunrise_core::nbt::with_tree_stack(|| reader.holder(tag).ok())
}

impl Dialog {
    /// Decode a packet dialog against the dynamic registry of the phase that sent it.
    pub fn parse_in_registry(
        t: &Tag,
        registries: &Registries,
        item: &dyn Fn(&str) -> bool,
        dynamic: Option<&sunrise_protocol::registry::Dynamic>,
    ) -> Option<Self> {
        Self::parse(t, registries, item, &|reference| {
            let Some(dynamic) = dynamic else {
                return false;
            };
            match reference {
                Reference::Id(id) => dynamic
                    .id("minecraft:dialog", id)
                    .and_then(|id| dynamic.data("minecraft:dialog", id))
                    .is_some(),
                Reference::Tag(id) => dynamic.tag("minecraft:dialog", id).is_some(),
            }
        })
    }

    /// `Dialog.DIRECT_CODEC` over the connection's item and dialog registries.
    pub fn parse(
        t: &Tag,
        registries: &Registries,
        item: &dyn Fn(&str) -> bool,
        bound: &dyn Fn(&Reference) -> bool,
    ) -> Option<Self> {
        let reader = Reader {
            registries,
            item,
            references: RefCell::default(),
            persistent_text: None,
        };
        let dialog = reader
            .dialog(
                t,
                |value| {
                    use crate::text::ClickEvent;
                    let mut clicks = Vec::new();
                    let component = reader.text(value, |click| {
                        let index = Tag::Int(clicks.len() as i32);
                        clicks.push(match click {
                            Click::OpenUrl(url) => ClickEvent::OpenUrl(url),
                            Click::RunCommand(command) => ClickEvent::RunCommand(command),
                            Click::SuggestCommand(command) => ClickEvent::SuggestCommand(command),
                            Click::ShowDialog(holder) => ClickEvent::ShowDialog(holder.into()),
                            Click::ChangePage(page) => ClickEvent::ChangePage(page),
                            Click::CopyToClipboard(text) => ClickEvent::CopyToClipboard(text),
                            Click::Custom { id, payload } => ClickEvent::Custom {
                                id,
                                payload: payload.map(Into::into),
                            },
                        });
                        index
                    })?;
                    Ok(crate::component::text_with_decoded_clicks(
                        &crate::scoreboard::tag_to_json(&component),
                        &clicks,
                    ))
                },
                || Text::translate("gui.ok", &[]),
            )
            .ok()?;
        reader
            .references
            .into_inner()
            .iter()
            .all(bound)
            .then_some(dialog)
    }

    /// `Dialog.DIRECT_CODEC` on a data pack's file at world open, with the
    /// dialogs it names: the path of the failing field, or the dialog.
    pub(crate) fn decode_pack(
        value: &Value,
        registries: &crate::command::Registries,
        item: &dyn Fn(&str) -> bool,
    ) -> Result<(Tag, Vec<Reference>), String> {
        let reader = Reader {
            registries,
            item,
            references: RefCell::default(),
            persistent_text: None,
        };
        let dialog = reader.dialog_nbt(value)?;
        Ok((dialog, reader.references.into_inner()))
    }

    /// `onCancel`: what Escape (or closing it) does.
    pub fn on_cancel(&self) -> Option<&Action> {
        match &self.kind {
            Kind::Notice(b) => b.action.as_ref(),
            Kind::Confirmation { no, .. } => no.action.as_ref(),
            Kind::MultiAction { exit, .. }
            | Kind::ServerLinks { exit, .. }
            | Kind::DialogList { exit, .. } => exit.as_ref().and_then(|b| b.action.as_ref()),
        }
    }
    /// Which of [`Self::buttons`] does what Escape does, if Escape does anything.
    pub fn escape_button(&self) -> Option<usize> {
        if !self.can_close_with_escape {
            return None;
        }
        let last = self.buttons().len().checked_sub(1);
        match &self.kind {
            Kind::Notice(_) | Kind::Confirmation { .. } => last,
            Kind::MultiAction { exit, .. }
            | Kind::ServerLinks { exit, .. }
            | Kind::DialogList { exit, .. } => exit.as_ref().and(last),
        }
    }
    /// The buttons it draws, in order: the list, then the footer's.
    pub fn buttons(&self) -> Vec<&Button> {
        match &self.kind {
            Kind::Notice(b) => vec![b],
            Kind::Confirmation { yes, no } => vec![yes, no],
            Kind::MultiAction { actions, exit, .. } => actions.iter().chain(exit).collect(),
            Kind::ServerLinks { exit, .. } => exit.iter().collect(),
            Kind::DialogList { entries, exit, .. } => entries.iter().chain(exit).collect(),
        }
    }
    /// `DialogListDialogScreen.createListActions`: a button for each of `dialogs`
    /// (what the list names), labelled by its external title, opening it.
    pub fn set_list(&mut self, dialogs: &[Tag], decode: impl Fn(&Tag) -> Option<Dialog>) {
        let Kind::DialogList {
            entries,
            button_width,
            ..
        } = &mut self.kind
        else {
            return;
        };
        *entries = dialogs
            .iter()
            .filter_map(|tag| {
                let sub = decode(tag)?;
                Some(Button {
                    label: sub.external_title.unwrap_or(sub.title),
                    tooltip: None,
                    width: *button_width,
                    action: Some(Action::Static(Click::ShowDialog(tag.clone()))),
                })
            })
            .collect();
    }
    /// The dialogs a `dialog_list`'s `dialogs` names: the entries of a `#tag`
    /// (by `tag`), registry ids (by `registry`) and dialogs written inline.
    pub fn list_entries(
        dialogs: &Tag,
        tag: &dyn Fn(&str) -> Vec<Tag>,
        registry: &dyn Fn(&str) -> Option<Tag>,
    ) -> Vec<Tag> {
        let one = |t: &Tag| match t {
            Tag::String(id) => registry(&id.to_string_lossy()),
            inline => Some(inline.clone()),
        };
        match dialogs {
            Tag::String(id) if id.to_string_lossy().starts_with('#') => {
                tag(&id.to_string_lossy()[1..])
            }
            Tag::List(items) => items.iter().filter_map(one).collect(),
            single => one(single).into_iter().collect(),
        }
    }
    /// What `status` reports of it.
    pub fn json(&self, values: &[InputValue]) -> Value {
        let kind = match &self.kind {
            Kind::Notice(_) => "notice",
            Kind::Confirmation { .. } => "confirmation",
            Kind::MultiAction { .. } => "multi_action",
            Kind::ServerLinks { .. } => "server_links",
            Kind::DialogList { .. } => "dialog_list",
        };
        let inputs: Vec<Value> = self
            .inputs
            .iter()
            .zip(values)
            .map(|(i, v)| {
                let kind = match i.control {
                    Control::Text { .. } => "text",
                    Control::Boolean { .. } => "boolean",
                    Control::SingleOption { .. } => "single_option",
                    Control::NumberRange { .. } => "number_range",
                };
                json!({"key": i.key, "type": kind, "value": i.control.substitution(v)})
            })
            .collect();
        json!({
            "type": kind,
            "title": self.title.plain(),
            "body": self.body.iter().map(|b| match b {
                Body::Message { contents, .. } => contents.plain(),
                Body::Item { item, .. } => item.get("id").and_then(Source::string).unwrap_or_default(),
            }).collect::<Vec<_>>(),
            "inputs": inputs,
            "buttons": self.buttons().iter().map(|b| b.label.plain()).collect::<Vec<_>>(),
            "can_close_with_escape": self.can_close_with_escape,
            "after_action": match self.after_action {
                AfterAction::Close => "close",
                AfterAction::None => "none",
                AfterAction::WaitForResponse => "wait_for_response",
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(tag: &Tag) -> Option<Dialog> {
        Dialog::parse(tag, &Registries::default(), &|_| false, &|_| false)
    }

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }

    /// The register dialog AuthMe 6 sends in configuration (the shape its
    /// `show_dialog` packet has on the test bed).
    fn register() -> Tag {
        Tag::compound([
            ("type", s("minecraft:multi_action")),
            ("title", s("Register")),
            ("can_close_with_escape", Tag::Byte(0)),
            ("after_action", s("wait_for_response")),
            (
                "body",
                Tag::compound([
                    ("type", s("minecraft:plain_message")),
                    ("contents", s("Choose a password")),
                ]),
            ),
            (
                "inputs",
                Tag::List(vec![Tag::compound([
                    ("type", s("minecraft:text")),
                    ("key", s("password")),
                    ("label", s("Password")),
                ])]),
            ),
            (
                "actions",
                Tag::List(vec![
                    Tag::compound([
                        ("label", s("Register")),
                        (
                            "action",
                            Tag::compound([
                                ("type", s("minecraft:dynamic/custom")),
                                ("id", s("authme:prejoin-register/submit")),
                            ]),
                        ),
                    ]),
                    Tag::compound([
                        ("label", s("Cancel")),
                        (
                            "action",
                            Tag::compound([
                                ("type", s("minecraft:dynamic/run_command")),
                                ("template", s("say $(password) $(missing)!")),
                            ]),
                        ),
                    ]),
                ]),
            ),
        ])
    }

    #[test]
    fn decodes_a_multi_action_dialog_with_defaults() {
        let d = parse(&register()).unwrap();
        assert_eq!(d.title.plain(), "Register");
        assert!(!d.can_close_with_escape);
        assert!(d.pause);
        assert_eq!(d.after_action, AfterAction::WaitForResponse);
        assert!(matches!(&d.body[..], [Body::Message { width: 200, .. }]));
        let Kind::MultiAction {
            actions,
            exit,
            columns,
        } = &d.kind
        else {
            panic!("{:?}", d.kind)
        };
        assert_eq!((actions.len(), exit.is_none(), columns.get()), (2, true, 2));
        assert_eq!(actions[0].width, 150);
        assert!(d.on_cancel().is_none());
        let [
            Input {
                key,
                control:
                    Control::Text {
                        max_length: 32,
                        width: 200,
                        ..
                    },
            },
        ] = &d.inputs[..]
        else {
            panic!("{:?}", d.inputs)
        };
        assert_eq!(key, "password");
    }

    #[test]
    fn a_packet_dialog_with_a_malformed_nested_member_is_refused() {
        let button = |extra: (&str, Tag)| Tag::compound([("label", s("L")), extra]);
        let multi = |actions: Vec<Tag>| {
            Tag::compound([
                ("type", s("minecraft:multi_action")),
                ("title", s("T")),
                ("actions", Tag::List(actions)),
            ])
        };
        // `NbtOps` reads any number as a boolean and floors a float to an int.
        let kept = multi(vec![button(("width", Tag::Double(150.9f64.to_bits())))]);
        assert_eq!(
            parse(&kept).unwrap().buttons()[0].width,
            150,
            "a double width is floored"
        );
        for wrong in [
            ("width", s("wide")),
            ("width", Tag::Int(0)),
            ("action", s("run")),
            (
                "action",
                Tag::compound([("type", s("minecraft:run_command"))]),
            ),
        ] {
            assert!(
                parse(&multi(vec![
                    button(("width", Tag::Int(20))),
                    button(wrong.clone())
                ]))
                .is_none(),
                "{wrong:?}"
            );
        }
    }

    #[test]
    fn open_url_actions_take_only_web_links() {
        let open = |url: &str| {
            let action = Tag::compound([("type", s("minecraft:open_url")), ("url", s(url))]);
            let reader = Reader {
                registries: &Registries::default(),
                item: &|_| true,
                references: RefCell::default(),
                persistent_text: None,
            };
            reader.action(&action).ok().and_then(|a| a.click(&[], &[]))
        };
        assert_eq!(
            open("https://example.com/a"),
            Some(Click::OpenUrl("https://example.com/a".into()))
        );
        assert!(open("HTTP://example.com").is_some());
        for url in [
            "file:///C:/Windows/System32/cmd.exe",
            "\\\\attacker\\share\\x.exe",
            "search-ms:query=x&crumb=location:\\\\attacker\\share",
            "javascript:alert(1)",
            "/usr/bin/xterm",
        ] {
            assert_eq!(open(url), None, "{url}");
        }
    }

    #[test]
    fn actions_carry_the_input_values() {
        let d = parse(&register()).unwrap();
        let values = [InputValue::Text("p\"w".into())];
        let Kind::MultiAction { actions, .. } = &d.kind else {
            unreachable!()
        };
        let custom = actions[0]
            .action
            .as_ref()
            .unwrap()
            .click(&d.inputs, &values);
        assert_eq!(
            custom,
            Some(Click::Custom {
                id: "authme:prejoin-register/submit".into(),
                payload: Some(Tag::compound([("password", s("p\"w"))])),
            })
        );
        // The template escapes the text; a key no input has becomes nothing.
        let command = actions[1]
            .action
            .as_ref()
            .unwrap()
            .click(&d.inputs, &values);
        assert_eq!(command, Some(Click::RunCommand("say p\\\"w !".into())));
    }

    #[test]
    fn the_button_that_does_what_escape_does() {
        let notice = |escape: i8| {
            parse(&Tag::compound([
                ("type", s("notice")),
                ("title", s("Hi")),
                ("can_close_with_escape", Tag::Byte(escape)),
            ]))
            .unwrap()
        };
        assert_eq!(notice(1).escape_button(), Some(0));
        assert_eq!(notice(0).escape_button(), None);
        // Multi-action with no exit button: Escape has no button to be.
        assert_eq!(parse(&register()).unwrap().escape_button(), None);
    }

    #[test]
    fn a_notice_without_an_action_has_ok() {
        let d = parse(&Tag::compound([("type", s("notice")), ("title", s("Hi"))])).unwrap();
        let Kind::Notice(b) = &d.kind else {
            unreachable!()
        };
        assert_eq!(b.label, Text::translate("gui.ok", &[]));
        // Without a title, or with an unknown type, there is no dialog.
        assert!(parse(&Tag::compound([("type", s("notice"))])).is_none());
        assert!(parse(&Tag::compound([("type", s("x")), ("title", s("Hi"))])).is_none());
    }

    #[test]
    fn number_ranges_snap_to_their_step() {
        let mut range = Control::NumberRange {
            width: 200,
            label: Text::literal("n"),
            label_format: "options.generic_value".into(),
            start: 0.0,
            end: 10.0,
            initial: Some(4.0),
            step: Some(3.0),
        };
        assert_eq!(range.initial(), InputValue::Slider(0.4));
        assert_eq!(range.scaled(&InputValue::Slider(0.62)), Some(7.0));
        assert_eq!(range.scaled(&InputValue::Slider(1.0)), Some(10.0));
        let Control::NumberRange { step, .. } = &mut range else {
            unreachable!()
        };
        *step = Some(4.0);
        // 12 is out of range: one step less.
        assert_eq!(range.scaled(&InputValue::Slider(1.0)), Some(8.0));
        assert_eq!(range.substitution(&InputValue::Slider(0.0)), "0");
        assert_eq!(number_string(2.5), "2.5");
    }
}
