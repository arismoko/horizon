//! Canonical command syntax shared by commands and persistent Java-string codecs.
use crate::suggestion::Suggestion;
use crate::text::Text;
use horizon_core::nbt::{JavaString, Tag};

pub mod selector;

/// An owned syntax source. Its cursor is UTF-8 bytes or Java UTF-16 units.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CommandInput {
    #[default]
    None,
    Utf8(Box<str>),
    Utf16(Box<[u16]>),
}
impl CommandInput {
    pub fn utf8(&self) -> Option<&str> {
        match self {
            Self::Utf8(value) => Some(value),
            _ => None,
        }
    }
    pub fn utf16(&self) -> Option<&[u16]> {
        match self {
            Self::Utf16(value) => Some(value),
            _ => None,
        }
    }
}

/// A command error (CommandSyntaxException): the message and, for parse
/// errors, the input and cursor it occurred at.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandError {
    pub message: Text,
    pub input: CommandInput,
    pub cursor: usize,
    /// BuiltInExceptions.literalIncorrect (summarised specially by the chat UI).
    pub literal_incorrect: bool,
    /// The translation key of the exception type, when known (the
    /// exception "type" vanilla code compares against).
    pub key: String,
    /// Translation arguments retained for lower syntax adapters.
    pub translation_args: Box<[String]>,
}
impl CommandError {
    pub fn new(message: Text) -> Self {
        Self {
            message,
            input: CommandInput::None,
            cursor: 0,
            literal_incorrect: false,
            key: String::new(),
            translation_args: Box::default(),
        }
    }
    pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> Self {
        let mut e = Self::new(Text::tr(key, args));
        e.key = key.to_owned();
        e.translation_args = args.iter().map(ToString::to_string).collect();
        e
    }
    pub fn at<S: CommandString + ?Sized>(message: Text, reader: &Reader<S>) -> Self {
        Self {
            message,
            input: if let Some(input) = reader.string.utf8() {
                CommandInput::Utf8(input.into())
            } else {
                CommandInput::Utf16(
                    reader
                        .string
                        .utf16()
                        .expect("sealed source encoding")
                        .into(),
                )
            },
            cursor: reader.cursor,
            literal_incorrect: false,
            key: String::new(),
            translation_args: Box::default(),
        }
    }
    pub fn tr_at<S: CommandString + ?Sized>(
        key: &str,
        args: &[&dyn std::fmt::Display],
        reader: &Reader<S>,
    ) -> Self {
        let mut e = Self::at(Text::tr(key, args), reader);
        e.key = key.to_owned();
        e.translation_args = args.iter().map(ToString::to_string).collect();
        e
    }
    /// CommandSyntaxException.getContext: up to ten characters before the
    /// cursor followed by "<--[HERE]".
    pub fn context(&self) -> Option<String> {
        if let Some(input) = self.input.utf16() {
            let cursor = self.cursor.min(input.len());
            let mut out = if cursor > 10 {
                "...".to_owned()
            } else {
                String::new()
            };
            out.push_str(&display_java(&input[cursor.saturating_sub(10)..cursor]));
            out.push_str("<--[HERE]");
            return Some(out);
        }
        let input = self.input.utf8()?;
        let cursor = self.cursor.min(input.len());
        let start = (0..=cursor.saturating_sub(10))
            .rev()
            .find(|&i| input.is_char_boundary(i))
            .unwrap_or(0);
        let mut out = String::new();
        if cursor > 10 {
            out.push_str("...");
        }
        out.push_str(&input[start..cursor]);
        out.push_str("<--[HERE]");
        Some(out)
    }
    /// The message as shown by command suggestions ("... at position N: ...").
    pub fn with_context(&self) -> Text {
        match self.context() {
            Some(context) => Text::translate(
                "command.context.parse_error",
                &[
                    self.message.clone(),
                    Text::literal(self.cursor.to_string()),
                    Text::literal(context),
                ],
            ),
            None => self.message.clone(),
        }
    }
}
pub type CmdResult<T> = Result<T, CommandError>;

/// The two borrowed command encodings. UTF-8 cursors remain byte offsets; Java strings use units.
/// Sealed so the parser's scalar/unit and slice invariants cannot be broken by callers.
pub trait CommandString: sealed::Sealed {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn codepoint(&self, cursor: usize) -> Option<u32>;
    fn width(&self, codepoint: u32) -> usize;
    fn utf8(&self) -> Option<&str>;
    fn utf16(&self) -> Option<&[u16]>;
    fn java_slice(&self, start: usize, end: usize) -> JavaString;
    fn java_len(&self, start: usize, end: usize) -> usize;
    fn ascii_slice(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str>;
}
mod sealed {
    pub trait Sealed {}
    impl Sealed for str {}
    impl Sealed for [u16] {}
}
impl CommandString for str {
    fn len(&self) -> usize {
        str::len(self)
    }
    fn codepoint(&self, cursor: usize) -> Option<u32> {
        self.get(cursor..)?.chars().next().map(u32::from)
    }
    fn width(&self, codepoint: u32) -> usize {
        char::from_u32(codepoint).expect("UTF-8 scalar").len_utf8()
    }
    fn utf8(&self) -> Option<&str> {
        Some(self)
    }
    fn utf16(&self) -> Option<&[u16]> {
        None
    }
    fn java_slice(&self, start: usize, end: usize) -> JavaString {
        JavaString::from(&self[start..end])
    }
    fn java_len(&self, start: usize, end: usize) -> usize {
        self[start..end].encode_utf16().count()
    }
    fn ascii_slice(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        debug_assert!(self[start..end].is_ascii());
        (&self[start..end]).into()
    }
}
impl CommandString for [u16] {
    fn len(&self) -> usize {
        <[u16]>::len(self)
    }
    fn codepoint(&self, cursor: usize) -> Option<u32> {
        self.get(cursor).copied().map(u32::from)
    }
    fn width(&self, _: u32) -> usize {
        1
    }
    fn utf8(&self) -> Option<&str> {
        None
    }
    fn utf16(&self) -> Option<&[u16]> {
        Some(self)
    }
    fn java_slice(&self, start: usize, end: usize) -> JavaString {
        JavaString(self[start..end].to_vec())
    }
    fn java_len(&self, start: usize, end: usize) -> usize {
        end - start
    }
    fn ascii_slice(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        self[start..end]
            .iter()
            .map(|&u| char::from(u8::try_from(u).expect("ASCII token")))
            .collect::<String>()
            .into()
    }
}

/// Brigadier StringReader; the normal command path borrows UTF-8 without transcoding.
#[derive(Debug)]
pub struct Reader<'a, S: CommandString + ?Sized = str> {
    pub string: &'a S,
    pub cursor: usize,
}
impl<S: CommandString + ?Sized> Clone for Reader<'_, S> {
    fn clone(&self) -> Self {
        Self {
            string: self.string,
            cursor: self.cursor,
        }
    }
}
impl<'a> Reader<'a> {
    pub fn new(string: &'a str) -> Self {
        Self { string, cursor: 0 }
    }
    pub fn peek(&self) -> char {
        self.peek_scalar().unwrap_or('\0')
    }
    pub fn peek_at(&self, offset: usize) -> char {
        self.peek_scalar_at(offset).unwrap_or('\0')
    }
    pub fn read(&mut self) -> char {
        let value = self.peek();
        self.skip();
        value
    }
    pub fn is_java_whitespace(c: char) -> bool {
        horizon_core::nbt::snbt_whitespace(u32::from(c))
    }
    pub fn is_allowed_number(c: char) -> bool {
        c.is_ascii_digit() || c == '.' || c == '-'
    }
    pub fn is_allowed_in_unquoted(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
    }
    pub fn remaining(&self) -> &'a str {
        &self.string[self.cursor..]
    }
    pub fn read_unquoted_string(&mut self) -> &'a str {
        let start = self.cursor;
        self.scan_unquoted();
        &self.string[start..self.cursor]
    }
    pub fn read_string_until(&mut self, terminator: char) -> CmdResult<String> {
        self.quoted::<String>(terminator)
    }
    pub fn read_string(&mut self) -> CmdResult<String> {
        self.string_value::<String>()
    }
}
impl<'a> Reader<'a, [u16]> {
    pub fn from_utf16(string: &'a [u16]) -> Self {
        Self { string, cursor: 0 }
    }
}
impl<'a, S: CommandString + ?Sized> Reader<'a, S> {
    pub fn can_read(&self) -> bool {
        self.cursor < self.string.len()
    }
    pub fn can_read_n(&self, n: usize) -> bool {
        let mut cursor = self.cursor;
        for _ in 0..n {
            let Some(value) = self.string.codepoint(cursor) else {
                return false;
            };
            cursor += self.string.width(value);
        }
        true
    }
    pub fn peek_codepoint(&self) -> Option<u32> {
        self.string.codepoint(self.cursor)
    }
    /// None for an unpaired surrogate, which is not a Unicode scalar.
    pub fn peek_scalar(&self) -> Option<char> {
        self.peek_codepoint().and_then(char::from_u32)
    }
    pub fn peek_scalar_at(&self, offset: usize) -> Option<char> {
        let mut cursor = self.cursor;
        for _ in 0..offset {
            let value = self.string.codepoint(cursor)?;
            cursor += self.string.width(value);
        }
        self.string.codepoint(cursor).and_then(char::from_u32)
    }
    pub fn skip(&mut self) {
        if let Some(value) = self.peek_codepoint() {
            self.cursor += self.string.width(value);
        } else {
            self.cursor += 1;
        }
    }
    pub fn read_codepoint(&mut self) -> Option<u32> {
        let value = self.peek_codepoint()?;
        self.cursor += self.string.width(value);
        Some(value)
    }
    pub fn java_slice(&self, start: usize, end: usize) -> JavaString {
        self.string.java_slice(start, end)
    }
    pub fn java_len(&self, start: usize, end: usize) -> usize {
        self.string.java_len(start, end)
    }
    pub fn ascii_slice(&self, start: usize, end: usize) -> std::borrow::Cow<'a, str> {
        self.string.ascii_slice(start, end)
    }
    pub fn skip_whitespace(&mut self) {
        while self
            .peek_codepoint()
            .is_some_and(horizon_core::nbt::snbt_whitespace)
        {
            self.skip();
        }
    }
    fn number<T: std::str::FromStr>(&mut self, kind: &str) -> CmdResult<T> {
        let start = self.cursor;
        while self.peek_scalar().is_some_and(Reader::is_allowed_number) {
            self.skip();
        }
        let text = self.string.ascii_slice(start, self.cursor);
        if text.is_empty() {
            return Err(CommandError::tr_at(
                &format!("parsing.{kind}.expected"),
                &[],
                self,
            ));
        }
        match text.parse::<T>() {
            Ok(value) if !(kind == "int" || kind == "long") || !text.contains('.') => Ok(value),
            _ => {
                self.cursor = start;
                Err(CommandError::tr_at(
                    &format!("parsing.{kind}.invalid"),
                    &[&text],
                    self,
                ))
            }
        }
    }
    pub fn read_int(&mut self) -> CmdResult<i32> {
        self.number("int")
    }
    pub fn read_double(&mut self) -> CmdResult<f64> {
        self.number("double")
    }
    pub fn read_float(&mut self) -> CmdResult<f32> {
        self.number("float")
    }
    fn scan_unquoted(&mut self) {
        while self
            .peek_scalar()
            .is_some_and(Reader::is_allowed_in_unquoted)
        {
            self.skip();
        }
    }
    pub fn read_ascii_word(&mut self) -> std::borrow::Cow<'a, str> {
        let start = self.cursor;
        self.scan_unquoted();
        self.ascii_slice(start, self.cursor)
    }
    fn quoted<B: StringBuffer>(&mut self, terminator: char) -> CmdResult<B> {
        let mut out = B::default();
        let mut escaped = false;
        while let Some(value) = self.peek_codepoint() {
            let start = self.cursor;
            self.skip();
            if escaped {
                if value == u32::from(terminator) || value == u32::from('\\') {
                    out.push_codepoint(value);
                    escaped = false;
                } else {
                    self.cursor = start;
                    return Err(CommandError::tr_at(
                        "parsing.quote.escape",
                        &[&display_codepoint(value)],
                        self,
                    ));
                }
            } else if value == u32::from('\\') {
                escaped = true;
            } else if value == u32::from(terminator) {
                return Ok(out);
            } else {
                out.push_codepoint(value);
            }
        }
        Err(CommandError::tr_at("parsing.quote.expected.end", &[], self))
    }
    fn string_value<B: StringBuffer>(&mut self) -> CmdResult<B> {
        if let Some(quote @ ('"' | '\'')) = self.peek_scalar() {
            self.skip();
            return self.quoted(quote);
        }
        let mut out = B::default();
        while self
            .peek_scalar()
            .is_some_and(Reader::is_allowed_in_unquoted)
        {
            out.push_codepoint(self.read_codepoint().expect("unquoted character"));
        }
        Ok(out)
    }
    pub fn read_java_string(&mut self) -> CmdResult<JavaString> {
        self.string_value::<JavaString>()
    }
    pub fn read_boolean(&mut self) -> CmdResult<bool> {
        let start = self.cursor;
        let value = self.read_java_string()?;
        if value.0.is_empty() {
            return Err(CommandError::tr_at("parsing.bool.expected", &[], self));
        }
        if java_equals(&value, "true") {
            Ok(true)
        } else if java_equals(&value, "false") {
            Ok(false)
        } else {
            self.cursor = start;
            Err(CommandError::tr_at(
                "parsing.bool.invalid",
                &[&display_java(&value.0)],
                self,
            ))
        }
    }
    pub fn expect(&mut self, c: char) -> CmdResult<()> {
        if self.peek_scalar() != Some(c) {
            return Err(CommandError::tr_at("parsing.expected", &[&c], self));
        }
        self.skip();
        Ok(())
    }
    /// `TagParser.parseAsArgument`, using the canonical full SNBT grammar.
    pub fn read_tag(&mut self) -> CmdResult<Tag> {
        let start = self.cursor;
        let result = if let Some(text) = self.string.utf8() {
            horizon_core::nbt::parse_snbt_argument(&text[start..])
        } else {
            horizon_core::nbt::parse_snbt_utf16(
                &self.string.utf16().expect("sealed Java input")[start..],
            )
        };
        match result {
            Ok((tag, used)) => {
                self.cursor += used;
                Ok(tag)
            }
            Err(error) => {
                self.cursor = start + error.cursor;
                let args: Vec<&dyn std::fmt::Display> = error
                    .args
                    .iter()
                    .map(|s| s as &dyn std::fmt::Display)
                    .collect();
                Err(CommandError::tr_at(error.key, &args, self))
            }
        }
    }
    pub fn read_compound(&mut self) -> CmdResult<horizon_core::nbt::Compound> {
        match self.read_tag()? {
            Tag::Compound(value) => Ok(value),
            _ => Err(CommandError::tr_at(
                "argument.nbt.expected.compound",
                &[],
                self,
            )),
        }
    }
}
trait StringBuffer: Default {
    fn push_codepoint(&mut self, value: u32);
}
impl StringBuffer for String {
    fn push_codepoint(&mut self, value: u32) {
        self.push(char::from_u32(value).expect("UTF-8 scalar"));
    }
}
impl StringBuffer for JavaString {
    fn push_codepoint(&mut self, value: u32) {
        if value <= 0xffff {
            self.0.push(value as u16);
        } else {
            self.0.extend_from_slice(
                char::from_u32(value)
                    .expect("UTF-8 scalar")
                    .encode_utf16(&mut [0; 2]),
            );
        }
    }
}
/// Exact comparison without allocating a Java key for each candidate.
pub fn java_equals(value: &JavaString, text: &str) -> bool {
    value.0.iter().copied().eq(text.encode_utf16())
}
fn display_codepoint(value: u32) -> String {
    match char::from_u32(value) {
        Some(c) => c.to_string(),
        None => format!("\\u{value:04X}"),
    }
}
pub(crate) fn display_java(units: &[u16]) -> String {
    let mut out = String::new();
    for value in char::decode_utf16(units.iter().copied()) {
        match value {
            Ok(c) => out.push(c),
            Err(e) => out.push_str(&format!("\\u{:04X}", e.unpaired_surrogate())),
        }
    }
    out
}

/// SuggestionsBuilder: suggestions for the text from `start` to the cursor.
pub struct SuggestionsBuilder<'a> {
    pub input: &'a str,
    pub start: usize,
    pub out: Vec<Suggestion>,
}
impl<'a> SuggestionsBuilder<'a> {
    pub fn new(input: &'a str, start: usize) -> Self {
        Self {
            input,
            start: start.min(input.len()),
            out: Vec::new(),
        }
    }
    pub fn remaining(&self) -> &'a str {
        &self.input[self.start..]
    }
    pub fn remaining_lower(&self) -> String {
        self.remaining().to_lowercase()
    }
    pub fn suggest(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text != self.remaining() {
            self.out.push(Suggestion {
                range: (self.start, self.input.len()),
                text,
                tooltip: None,
                int_value: None,
            });
        }
    }
    /// SuggestionsBuilder.suggest(int).
    pub fn suggest_int(&mut self, value: i32) {
        self.out.push(Suggestion {
            range: (self.start, self.input.len()),
            text: value.to_string(),
            tooltip: None,
            int_value: Some(value),
        });
    }
    pub fn suggest_tooltip(&mut self, text: impl Into<String>, tooltip: impl Into<String>) {
        let text = text.into();
        if text != self.remaining() {
            self.out.push(Suggestion {
                range: (self.start, self.input.len()),
                text,
                tooltip: Some(tooltip.into()),
                int_value: None,
            });
        }
    }
    pub fn offset(&self, start: usize) -> SuggestionsBuilder<'a> {
        SuggestionsBuilder::new(self.input, start)
    }
    /// SuggestionsBuilder.add: keep another builder's suggestions (with their own ranges).
    pub fn add(&mut self, other: SuggestionsBuilder) {
        self.out.extend(other.out);
    }
    /// SharedSuggestionProvider.suggest: candidates matching the remaining
    /// text at the start or after a `.`, `_` or `/`.
    pub fn suggest_matching<'b>(&mut self, candidates: impl IntoIterator<Item = &'b str>) {
        let lower = self.remaining_lower();
        for c in candidates {
            if matches_sub_str(&lower, &c.to_lowercase()) {
                self.suggest(c);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coord {
    pub relative: bool,
    pub value: f64,
}
impl Coord {
    pub fn get(self, base: f64) -> f64 {
        if self.relative {
            base + self.value
        } else {
            self.value
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Coordinates {
    World([Coord; 3]),
    Local([f64; 3]),
}
impl Coordinates {
    /// WorldCoordinates/LocalCoordinates.getPosition from the source position
    /// (feet) and rotation (yaw, pitch in degrees).
    pub fn position(&self, pos: [f64; 3], rot: [f32; 2]) -> [f64; 3] {
        match self {
            Coordinates::World(c) => [c[0].get(pos[0]), c[1].get(pos[1]), c[2].get(pos[2])],
            Coordinates::Local([left, up, forwards]) => {
                use horizon_core::geometry::trig;
                // `LocalCoordinates.apply`: widen the f32 angle for Mth's table,
                // keep basis products f32, then sum the f64 offset before the origin.
                let d = trig::DEG_TO_RAD;
                let (yaw, pitch) = (rot[0], rot[1]);
                let cos_y = trig::cos(f64::from((yaw + 90.0) * d));
                let sin_y = trig::sin(f64::from((yaw + 90.0) * d));
                let cos_p = trig::cos(f64::from(-pitch * d));
                let sin_p = trig::sin(f64::from(-pitch * d));
                let cos_p2 = trig::cos(f64::from((-pitch + 90.0) * d));
                let sin_p2 = trig::sin(f64::from((-pitch + 90.0) * d));
                let f = [
                    f64::from(cos_y * cos_p),
                    f64::from(sin_p),
                    f64::from(sin_y * cos_p),
                ];
                let u = [
                    f64::from(cos_y * cos_p2),
                    f64::from(sin_p2),
                    f64::from(sin_y * cos_p2),
                ];
                // left = -(forward x up)
                let l = [
                    -(f[1] * u[2] - f[2] * u[1]),
                    -(f[2] * u[0] - f[0] * u[2]),
                    -(f[0] * u[1] - f[1] * u[0]),
                ];
                [
                    pos[0] + (f[0] * forwards + u[0] * up + l[0] * left),
                    pos[1] + (f[1] * forwards + u[1] * up + l[1] * left),
                    pos[2] + (f[2] * forwards + u[2] * up + l[2] * left),
                ]
            }
        }
    }
    pub fn block(&self, pos: [f64; 3], rot: [f32; 2]) -> [i32; 3] {
        let p = self.position(pos, rot);
        [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ]
    }
    /// RotationArgument: (yaw, pitch) from the source rotation.
    pub fn rotation(&self, rot: [f32; 2]) -> [f32; 2] {
        match self {
            Coordinates::World(c) => [
                c[0].get(f64::from(rot[0])) as f32,
                c[1].get(f64::from(rot[1])) as f32,
            ],
            Coordinates::Local(_) => rot,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Arbitrary,
    Nearest,
    Furthest,
    Random,
}
pub type DoubleRange = (Option<f64>, Option<f64>);
pub type IntRange = (Option<i32>, Option<i32>);
/// A parsed entity selector (EntitySelector) or player name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selector {
    pub player_name: Option<JavaString>,
    /// The entity UUID when the argument was one.
    pub uuid: Option<u128>,
    /// 'p', 'a', 'r', 's', 'e', 'n'.
    pub kind: char,
    pub limit: usize,
    pub sort: Sort,
    pub include_entities: bool,
    pub types: Vec<(String, bool)>,
    pub names: Vec<(JavaString, bool)>,
    pub tags: Vec<(String, bool)>,
    pub gamemodes: Vec<(String, bool)>,
    pub distance: Option<DoubleRange>,
    pub level: Option<IntRange>,
    pub x_rotation: Option<DoubleRange>,
    pub y_rotation: Option<DoubleRange>,
    pub pos: [Option<f64>; 3],
    pub delta: [Option<f64>; 3],
    /// `scores={objective=range,...}`.
    pub scores: Vec<(String, IntRange)>,
    /// `team=` (an empty name matches entities without a team).
    pub teams: Vec<(String, bool)>,
    /// Parsed `nbt=` compound patterns, reused without reparsing per candidate.
    pub nbt: Vec<(Tag, bool)>,
    /// `advancements={id=true|{criterion=true}}`.
    pub advancements: Vec<(String, AdvCondition)>,
    /// `predicate=` ids.
    pub predicates: Vec<(String, bool)>,
}
/// An `advancements=` entry: all done or not, or per-criterion states.
#[derive(Clone, Debug, PartialEq)]
pub enum AdvCondition {
    Done(bool),
    Criteria(Vec<(String, bool)>),
}
impl Selector {
    pub fn is_self(&self) -> bool {
        self.kind == 's'
    }
    pub fn single(&self) -> bool {
        self.limit <= 1
    }
    /// `EntitySelectorParser.setWorldLimited`: a selector with a spatial
    /// constraint (`distance` or a position) reaches the level the command
    /// runs in only; an unbounded one reaches every loaded level.
    pub fn world_limited(&self) -> bool {
        self.distance.is_some()
            || self.pos.iter().any(Option::is_some)
            || self.delta.iter().any(Option::is_some)
    }
}

pub fn is_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ':' | '/')
}
/// Identifier.read.
pub fn read_identifier<S: CommandString + ?Sized>(reader: &mut Reader<S>) -> CmdResult<String> {
    let start = reader.cursor;
    while reader.can_read() && reader.peek_scalar().is_some_and(is_id_char) {
        reader.skip();
    }
    let raw = reader.ascii_slice(start, reader.cursor);
    match parse_identifier(&raw) {
        Some(id) => Ok(id),
        None => {
            reader.cursor = start;
            Err(CommandError::tr_at("argument.id.invalid", &[], reader))
        }
    }
}
use crate::identifier::parse_identifier;
/// SharedSuggestionProvider.matchesSubStr: `input` is a prefix of
/// `candidate` or of a part after a `.`, `_` or `/`.
pub fn matches_sub_str(input: &str, candidate: &str) -> bool {
    let mut index = 0;
    loop {
        if candidate[index..].starts_with(input) {
            return true;
        }
        match candidate[index..].find(['.', '_', '/']) {
            Some(i) => index += i + 1,
            None => return false,
        }
    }
}
/// SharedSuggestionProvider.filterResources (no prefix).
pub fn resource_matches(contents: &str, id: &str) -> bool {
    if contents.contains(':') {
        matches_sub_str(contents, id)
    } else {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        matches_sub_str(contents, ns) || matches_sub_str(contents, path)
    }
}
/// SharedSuggestionProvider.suggestResource (with an optional prefix such as `#`).
pub fn suggest_resources<'a>(
    builder: &mut SuggestionsBuilder,
    ids: impl IntoIterator<Item = &'a String>,
    prefix: &str,
) {
    let contents = builder.remaining_lower();
    if prefix.is_empty() {
        for id in ids {
            if resource_matches(&contents, id) {
                builder.suggest(id.clone());
            }
        }
        return;
    }
    if contents.is_empty() {
        for id in ids {
            builder.suggest(format!("{prefix}{id}"));
        }
        return;
    }
    let common = contents
        .chars()
        .zip(prefix.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    if common == 0 {
        return;
    }
    let stripped = &contents[common..];
    for id in ids {
        if resource_matches(stripped, id) {
            builder.suggest(format!("{prefix}{id}"));
        }
    }
}
/// MinMaxBounds.Bounds.fromReader: `a`, `a..`, `..b` or `a..b`, each part
/// converted by `convert` (errors rewound to the start).
fn read_bounds<T: Copy, S: CommandString + ?Sized>(
    reader: &mut Reader<S>,
    convert: impl Fn(&str) -> Option<T>,
    invalid: &str,
) -> CmdResult<(Option<T>, Option<T>)> {
    if !reader.can_read() {
        return Err(CommandError::tr_at("argument.range.empty", &[], reader));
    }
    let start = reader.cursor;
    let number = |reader: &mut Reader<S>| -> CmdResult<Option<T>> {
        let s = reader.cursor;
        while reader.can_read() {
            let c = reader.peek_scalar();
            let allowed = c.is_some_and(|c| c.is_ascii_digit())
                || c == Some('-')
                || (c == Some('.')
                    && !(reader.can_read_n(2) && reader.peek_scalar_at(1) == Some('.')));
            if !allowed {
                break;
            }
            reader.skip();
        }
        let text = reader.ascii_slice(s, reader.cursor);
        if text.is_empty() {
            return Ok(None);
        }
        match convert(&text) {
            Some(v) => Ok(Some(v)),
            None => Err(CommandError::tr_at(invalid, &[&text], reader)),
        }
    };
    let result = (|| {
        let min = number(reader)?;
        let max = if reader.can_read_n(2)
            && reader.peek_scalar() == Some('.')
            && reader.peek_scalar_at(1) == Some('.')
        {
            reader.skip();
            reader.skip();
            number(reader)?
        } else {
            min
        };
        if min.is_none() && max.is_none() {
            return Err(CommandError::tr_at("argument.range.empty", &[], reader));
        }
        Ok((min, max))
    })();
    result.map_err(|mut e| {
        reader.cursor = start;
        e.cursor = start;
        e
    })
}
/// MinMaxBounds.Ints.fromReader.
pub fn read_int_bounds<S: CommandString + ?Sized>(reader: &mut Reader<S>) -> CmdResult<IntRange> {
    let start = reader.cursor;
    let b = read_bounds(reader, java_parse_int, "parsing.int.invalid")?;
    if let (Some(l), Some(h)) = b
        && l > h
    {
        reader.cursor = start;
        return Err(CommandError::tr_at("argument.range.swapped", &[], reader));
    }
    Ok(b)
}
/// MinMaxBounds.Doubles.fromReader.
pub fn read_double_bounds<S: CommandString + ?Sized>(
    reader: &mut Reader<S>,
) -> CmdResult<DoubleRange> {
    let start = reader.cursor;
    let b = read_bounds(reader, java_parse_double, "parsing.double.invalid")?;
    if let (Some(l), Some(h)) = b
        && l > h
    {
        reader.cursor = start;
        return Err(CommandError::tr_at("argument.range.swapped", &[], reader));
    }
    Ok(b)
}
/// MinMaxBounds.FloatDegrees.fromReader (no swap check).
pub fn read_float_bounds<S: CommandString + ?Sized>(
    reader: &mut Reader<S>,
) -> CmdResult<(Option<f32>, Option<f32>)> {
    read_bounds(
        reader,
        |s| java_parse_double(s).map(|v| v as f32),
        "parsing.float.invalid",
    )
}
/// Integer.parseInt over number characters.
pub fn java_parse_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}
/// Double.parseDouble over the characters StringReader reads as a number.
pub fn java_parse_double(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let body = t.strip_prefix(['-', '+']).unwrap_or(t);
    if body.is_empty() || body == "." {
        return None;
    }
    let body = body.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(body);
    if body == "NaN" || body == "Infinity" {
        return t.parse::<f64>().ok().or(Some(if t.starts_with('-') {
            f64::NEG_INFINITY
        } else if body == "NaN" {
            f64::NAN
        } else {
            f64::INFINITY
        }));
    }
    let mut chars = body.chars().peekable();
    let mut digits = 0;
    while chars.peek().is_some_and(char::is_ascii_digit) {
        chars.next();
        digits += 1;
    }
    if chars.peek() == Some(&'.') {
        chars.next();
        while chars.peek().is_some_and(char::is_ascii_digit) {
            chars.next();
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(chars.peek(), Some('e' | 'E')) {
        chars.next();
        if matches!(chars.peek(), Some('+' | '-')) {
            chars.next();
        }
        let mut exp = 0;
        while chars.peek().is_some_and(char::is_ascii_digit) {
            chars.next();
            exp += 1;
        }
        if exp == 0 {
            return None;
        }
    }
    if chars.next().is_some() {
        return None;
    }
    let clean: String = t
        .trim_end_matches(['d', 'D', 'f', 'F'])
        .trim_start_matches('+')
        .to_owned();
    clean.parse::<f64>().ok()
}
pub fn parse_coord_int<S: CommandString + ?Sized>(reader: &mut Reader<S>) -> CmdResult<Coord> {
    if reader.can_read() && reader.peek_scalar() == Some('^') {
        return Err(CommandError::tr_at("argument.pos.mixed", &[], reader));
    }
    if !reader.can_read() {
        return Err(CommandError::tr_at("argument.pos.missing.int", &[], reader));
    }
    let relative = reader.peek_scalar() == Some('~');
    if relative {
        reader.skip();
    }
    let value = if reader.can_read() && reader.peek_scalar() != Some(' ') {
        if relative {
            reader.read_double()?
        } else {
            f64::from(reader.read_int()?)
        }
    } else {
        0.0
    };
    Ok(Coord { relative, value })
}
pub fn parse_coord_double<S: CommandString + ?Sized>(
    reader: &mut Reader<S>,
    center: bool,
) -> CmdResult<Coord> {
    if reader.can_read() && reader.peek_scalar() == Some('^') {
        return Err(CommandError::tr_at("argument.pos.mixed", &[], reader));
    }
    if !reader.can_read() {
        return Err(CommandError::tr_at(
            "argument.pos.missing.double",
            &[],
            reader,
        ));
    }
    let relative = reader.peek_scalar() == Some('~');
    if relative {
        reader.skip();
    }
    let start = reader.cursor;
    let mut value = if reader.can_read() && reader.peek_scalar() != Some(' ') {
        reader.read_double()?
    } else {
        0.0
    };
    let number = reader.ascii_slice(start, reader.cursor);
    if relative && number.is_empty() {
        return Ok(Coord {
            relative,
            value: 0.0,
        });
    }
    if !number.contains('.') && !relative && center {
        value += 0.5;
    }
    Ok(Coord { relative, value })
}
/// LocalCoordinates.parse / readDouble.
pub fn parse_local<S: CommandString + ?Sized>(reader: &mut Reader<S>) -> CmdResult<Coordinates> {
    let start = reader.cursor;
    let mut v = [0.0; 3];
    for (i, slot) in v.iter_mut().enumerate() {
        if i > 0 {
            if !reader.can_read() || reader.peek_scalar() != Some(' ') {
                reader.cursor = start;
                return Err(CommandError::tr_at(
                    "argument.pos3d.incomplete",
                    &[],
                    reader,
                ));
            }
            reader.skip();
        }
        if !reader.can_read() {
            return Err(CommandError::tr_at(
                "argument.pos.missing.double",
                &[],
                reader,
            ));
        }
        if reader.peek_scalar() != Some('^') {
            reader.cursor = start;
            return Err(CommandError::tr_at("argument.pos.mixed", &[], reader));
        }
        reader.skip();
        *slot = if reader.can_read() && reader.peek_scalar() != Some(' ') {
            reader.read_double()?
        } else {
            0.0
        };
    }
    Ok(Coordinates::Local(v))
}
pub fn parse_world<S: CommandString + ?Sized>(
    reader: &mut Reader<S>,
    block: bool,
    center: bool,
) -> CmdResult<Coordinates> {
    let start = reader.cursor;
    let mut c = [Coord {
        relative: false,
        value: 0.0,
    }; 3];
    for (i, slot) in c.iter_mut().enumerate() {
        if i > 0 {
            if !reader.can_read() || reader.peek_scalar() != Some(' ') {
                reader.cursor = start;
                return Err(CommandError::tr_at(
                    "argument.pos3d.incomplete",
                    &[],
                    reader,
                ));
            }
            reader.skip();
        }
        *slot = if block {
            parse_coord_int(reader)?
        } else {
            parse_coord_double(reader, center && i != 1)?
        };
    }
    Ok(Coordinates::World(c))
}

/// `BlockPosArgument.parse`, including local/world coordinate mixing and numeric errors.
pub fn parse_block_pos<S: CommandString + ?Sized>(
    reader: &mut Reader<S>,
) -> CmdResult<Coordinates> {
    if reader.peek_scalar() == Some('^') {
        parse_local(reader)
    } else {
        parse_world(reader, true, false)
    }
}
/// `CompilableString.parse`: compile the complete Java block-position source.
pub fn parse_block_pos_utf16(text: &[u16]) -> CmdResult<Coordinates> {
    let mut reader = Reader::from_utf16(text);
    let position = parse_block_pos(&mut reader)?;
    ensure_fully_read(&reader)?;
    Ok(position)
}

/// `CompilableString.CommandParserHelper`: trailing input is not whitespace-trimmed.
pub(crate) fn ensure_fully_read<S: CommandString + ?Sized>(reader: &Reader<S>) -> CmdResult<()> {
    if reader.can_read() {
        let parsed = reader.java_slice(0, reader.cursor);
        let rest = reader.java_slice(reader.cursor, reader.string.len());
        return Err(CommandError::tr(
            "command.trailing_data",
            &[&format!(
                "{}[{}]",
                display_java(&parsed.0),
                display_java(&rest.0)
            )],
        ));
    }
    Ok(())
}

#[cfg(test)]
mod lossless_source_tests {
    use super::*;
    use crate::registries::Registries;

    fn registries() -> Registries {
        Registries {
            permission: 4,
            entity_types: vec!["minecraft:pig".into(), "minecraft:player".into()],
            ..Registries::default()
        }
    }
    #[test]
    fn quoted_surrogates_keep_selector_identity_and_java_name_limits() {
        let reg = registries();
        let source = [64, 101, 91, 110, 97, 109, 101, 61, 34, 0xd800, 34, 93];
        let selector = selector::parse_utf16(&source, &reg).unwrap();
        assert_eq!(selector.names, [(JavaString(vec![0xd800]), false)]);
        assert_eq!(selector.kind, 'e');
        for unit in [0xd800, 0xdfff] {
            let mut name = vec![34];
            name.extend(std::iter::repeat_n(unit, 16));
            name.push(34);
            assert_eq!(
                selector::parse_utf16(&name, &reg).unwrap().player_name,
                Some(JavaString(vec![unit; 16]))
            );
            name.insert(1, unit);
            assert_eq!(
                selector::parse_utf16(&name, &reg).unwrap_err().key,
                "argument.entity.invalid"
            );
        }
        let mut supplementary = vec![34];
        for _ in 0..8 {
            supplementary.extend([0xd83d, 0xde00]);
        }
        supplementary.push(34);
        assert_eq!(
            selector::parse_utf16(&supplementary, &reg)
                .unwrap()
                .player_name
                .unwrap()
                .0
                .len(),
            16
        );
        supplementary.splice(1..1, [0xd83d, 0xde00]);
        assert_eq!(
            selector::parse_utf16(&supplementary, &reg).unwrap_err().key,
            "argument.entity.invalid"
        );
        let mut option: Vec<u16> = "@e[name=\"".encode_utf16().collect();
        option.extend([0xd800; 17]);
        option.extend([34, 93]);
        assert_eq!(
            selector::parse_utf16(&option, &reg).unwrap().names,
            [(JavaString(vec![0xd800; 17]), false)]
        );
    }
    #[test]
    fn selector_nbt_is_compiled_once_without_changing_surrogate_keys() {
        let source = [
            64, 101, 91, 110, 98, 116, 61, 123, 34, 0xd800, 34, 58, 34, 0xdfff, 34, 125, 93,
        ];
        let selector = selector::parse_utf16(&source, &registries()).unwrap();
        assert_eq!(selector.nbt.len(), 1);
        let Tag::Compound(pattern) = &selector.nbt[0].0 else {
            panic!("compound pattern")
        };
        assert_eq!(
            pattern.get(&JavaString(vec![0xd800])),
            Some(&Tag::String(JavaString(vec![0xdfff])))
        );
        assert!(!selector.nbt[0].1);
    }
    #[test]
    fn complete_selector_grammar_and_registry_snapshot_reject_invalid_sources() {
        let reg = registries();
        for source in [
            "@e[type=pig,limit=2,sort=nearest,distance=0..10,level=..3,x=1,y=2,z=3,dx=1,dy=2,dz=3,x_rotation=..90,y_rotation=-180..180,gamemode=!creative,team=red,tag=a,nbt={a:[I;1,2]},scores={s=1..},advancements={a={c=true}},predicate=p]",
            "@e[name=!\"a\\\"b\",name=!other]",
            "@s",
            "Steve",
            "00000000-0000-0000-0000-000000000001",
        ] {
            assert!(
                selector::parse_utf16(&source.encode_utf16().collect::<Vec<_>>(), &reg).is_ok(),
                "{source}"
            );
            let mut reader = Reader::new(source);
            assert!(selector::parse(&mut reader, &reg, true).is_ok(), "{source}");
            assert!(!reader.can_read());
        }
        for source in [
            "@e[name=\"a\\q\"]",
            "@e[name=\"x]",
            "@e[name]",
            "@e[unknown=x]",
            "@e[distance=2..1]",
            "@e[level=-1]",
            "@e[limit=0]",
            "@e[sort=near]",
            "@e[gamemode=builder]",
            "@e[type=unknown]",
            "@e[nbt={a:}]",
            "@e[scores={a=..}]",
            "@e[advancements={a={c=maybe}}]",
            "@e[name=a,name=b]",
            "@e[",
            "@q",
        ] {
            assert!(
                selector::parse_utf16(&source.encode_utf16().collect::<Vec<_>>(), &reg).is_err(),
                "{source}"
            );
        }
        for source in ["@e ", "@e[]x", "Steve trailing"] {
            assert_eq!(
                selector::parse_utf16(&source.encode_utf16().collect::<Vec<_>>(), &reg)
                    .unwrap_err()
                    .key,
                "command.trailing_data",
                "{source}"
            );
        }
    }
    #[test]
    fn block_positions_compile_the_complete_real_coordinate_grammar() {
        for (source, expected) in [
            (
                "~ ~ ~",
                Coordinates::World(
                    [Coord {
                        relative: true,
                        value: 0.0,
                    }; 3],
                ),
            ),
            ("^1 ^2 ^-3", Coordinates::Local([1.0, 2.0, -3.0])),
            (
                "-2147483648 0 2147483647",
                Coordinates::World([
                    Coord {
                        relative: false,
                        value: -2147483648.0,
                    },
                    Coord {
                        relative: false,
                        value: 0.0,
                    },
                    Coord {
                        relative: false,
                        value: 2147483647.0,
                    },
                ]),
            ),
        ] {
            assert_eq!(
                parse_block_pos_utf16(&source.encode_utf16().collect::<Vec<_>>()).unwrap(),
                expected
            );
            assert_eq!(parse_block_pos(&mut Reader::new(source)).unwrap(), expected);
        }
        for source in [
            "0 0",
            "0 0 0 ",
            "0 0 0x",
            "1.5 0 0",
            "2147483648 0 0",
            "^ ~ ^",
            "~ ^ ~",
        ] {
            assert!(
                parse_block_pos_utf16(&source.encode_utf16().collect::<Vec<_>>()).is_err(),
                "{source}"
            );
        }
        assert_eq!(
            parse_block_pos_utf16(&[48, 32, 48, 32, 0xd800])
                .unwrap_err()
                .key,
            "parsing.int.expected"
        );
    }
}
