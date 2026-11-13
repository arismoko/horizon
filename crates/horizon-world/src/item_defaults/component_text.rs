//! Transient admission at the typed text, style and dialog codec boundaries.
//! Literal strings, custom types and arbitrary NBT payloads are not decoded here.
use horizon_core::nbt::Tag;
use serde_json::Value;

/// `ComponentSerialization.CODEC`: only typed child components and style recurse.
pub fn validate_text(value: &Value) -> Result<(), String> {
    if !matches!(value, Value::Object(_) | Value::Array(_)) {
        return Ok(());
    }
    if let Some(values) = value.as_array() {
        for value in values {
            validate_text(value)?;
        }
        return Ok(());
    }
    validate_style(value)?;
    for value in json_list(&value["extra"]) {
        validate_text(value)?;
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            for (field, kind) in [
                ("text", "text"),
                ("translate", "translatable"),
                ("keybind", "keybind"),
                ("score", "score"),
                ("selector", "selector"),
                ("nbt", "nbt"),
                ("object", "object"),
            ] {
                if value.get(field).is_some() {
                    return kind;
                }
            }
            ""
        });
    match kind {
        "translatable" => {
            for argument in json_list(&value["with"]) {
                validate_text(argument)?;
            }
        }
        "selector" | "nbt" => validate_text(&value["separator"])?,
        _ => {}
    }
    Ok(())
}

/// `Style.Serializer`: hover item data is an `ItemStackTemplate`, not opaque NBT.
pub fn validate_style(value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Ok(());
    }
    let hover = &value["hover_event"];
    match hover["action"].as_str() {
        Some("show_item") => super::validate_stack(hover)?,
        Some("show_text") => validate_text(&hover["value"])?,
        Some("show_entity") => validate_text(&hover["name"])?,
        _ => {}
    }
    let click = &value["click_event"];
    if click["action"] == "show_dialog" {
        validate_dialog(&click["dialog"])?;
    }
    Ok(())
}

fn json_list(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn json_kind(value: &Value) -> &str {
    let name = value["type"].as_str().unwrap_or("");
    name.strip_prefix("minecraft:").unwrap_or(name)
}

fn plain_message(value: &Value) -> Result<(), String> {
    validate_text(value.get("contents").unwrap_or(value))
}

fn dialog_body(value: &Value) -> Result<(), String> {
    if let Some(values) = value.as_array() {
        for value in values {
            dialog_body(value)?;
        }
    } else {
        match json_kind(value) {
            "item" => {
                super::validate_stack(&value["item"])?;
                plain_message(&value["description"])?;
            }
            "plain_message" => validate_text(&value["contents"])?,
            _ => {}
        }
    }
    Ok(())
}

fn dialog_button(value: &Value) -> Result<(), String> {
    validate_text(&value["label"])?;
    validate_text(&value["tooltip"])?;
    let action = &value["action"];
    if json_kind(action) == "show_dialog" {
        validate_dialog(&action["dialog"])?;
    }
    Ok(())
}

/// `Dialog.DIRECT_CODEC`: native dialog payloads and custom actions stay opaque.
pub fn validate_dialog(value: &Value) -> Result<(), String> {
    if !matches!(value, Value::Object(_) | Value::Array(_)) {
        return Ok(());
    }
    if let Some(values) = value.as_array() {
        for value in values {
            validate_dialog(value)?;
        }
        return Ok(());
    }
    let kind = json_kind(value);
    if !matches!(
        kind,
        "notice" | "confirmation" | "multi_action" | "dialog_list" | "server_links"
    ) {
        return Ok(());
    }
    validate_text(&value["title"])?;
    validate_text(&value["external_title"])?;
    dialog_body(&value["body"])?;
    for input in json_list(&value["inputs"]) {
        match json_kind(input) {
            "boolean" | "number_range" | "single_option" | "text" => {
                validate_text(&input["label"])?;
                if json_kind(input) == "single_option" {
                    for option in json_list(&input["options"]) {
                        validate_text(&option["display"])?;
                    }
                }
            }
            _ => {}
        }
    }
    match kind {
        "confirmation" => {
            dialog_button(&value["yes"])?;
            dialog_button(&value["no"])?;
        }
        "notice" => dialog_button(&value["action"])?,
        "multi_action" => {
            for action in json_list(&value["actions"]) {
                dialog_button(action)?;
            }
        }
        "dialog_list" => validate_dialog(&value["dialogs"])?,
        _ => {}
    }
    if matches!(kind, "multi_action" | "dialog_list" | "server_links") {
        dialog_button(&value["exit_action"])?;
    }
    Ok(())
}

/// The persistent component codecs which contain typed text values.
pub fn validate_component_text(id: &str, value: &Value) -> Result<(), String> {
    match id.strip_prefix("minecraft:").unwrap_or(id) {
        "custom_name" | "item_name" => validate_text(value)?,
        "lore" => {
            for line in json_list(value) {
                validate_text(line)?;
            }
        }
        "written_book_content" => {
            for page in json_list(&value["pages"]) {
                if let Some(raw) = page.get("raw") {
                    validate_text(raw)?;
                    validate_text(&page["filtered"])?;
                } else {
                    validate_text(page)?;
                }
            }
        }
        "instrument" | "provides_trim_material" => validate_text(&value["description"])?,
        "trim" => {
            validate_text(&value["material"]["description"])?;
            validate_text(&value["pattern"]["description"])?;
        }
        "sign_text_front" | "sign_text_back" => {
            for field in ["messages", "filtered_messages"] {
                for message in json_list(&value[field]) {
                    validate_text(message)?;
                }
            }
        }
        "attribute_modifiers" => {
            for entry in json_list(value) {
                let display = &entry["display"];
                if display["type"] == "override" {
                    validate_text(&display["value"])?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

static EMPTY: Tag = Tag::End;
fn field<'a>(value: &'a Tag, key: &str) -> &'a Tag {
    value.get(key).unwrap_or(&EMPTY)
}
fn tag_list(value: &Tag) -> &[Tag] {
    match value {
        Tag::List(values) => values,
        _ => &[],
    }
}
fn string_is(value: &Tag, expected: &str) -> bool {
    matches!(value, Tag::String(name) if name.0.iter().copied().eq(expected.encode_utf16()))
}
fn kind_is(value: &Tag, expected: &str) -> bool {
    let name = field(value, "type");
    string_is(name, expected)
        || matches!(name, Tag::String(name) if name.0.iter().copied().eq("minecraft:".encode_utf16().chain(expected.encode_utf16())))
}

/// Borrowed NBT counterpart of `validate_text`; strings are not parsed as JSON.
pub fn validate_text_tag(value: &Tag) -> Result<(), String> {
    if !matches!(value, Tag::Compound(_) | Tag::List(_)) {
        return Ok(());
    }
    if let Tag::List(values) = value {
        for value in values {
            validate_text_tag(value)?;
        }
        return Ok(());
    }
    validate_style_tag(value)?;
    for value in tag_list(field(value, "extra")) {
        validate_text_tag(value)?;
    }
    let explicit = value.get("type");
    let inferred = |key: &str| explicit.is_none() && value.get(key).is_some();
    if string_is(field(value, "type"), "translatable")
        || (inferred("translate") && value.get("text").is_none())
    {
        for argument in tag_list(field(value, "with")) {
            validate_text_tag(argument)?;
        }
    }
    let inferred_separator = explicit.is_none()
        && ["text", "translate", "keybind", "score"]
            .iter()
            .all(|key| value.get(key).is_none())
        && (value.get("selector").is_some() || value.get("nbt").is_some());
    if string_is(field(value, "type"), "selector")
        || string_is(field(value, "type"), "nbt")
        || inferred_separator
    {
        validate_text_tag(field(value, "separator"))?;
    }
    Ok(())
}

/// Borrowed NBT style validation shared by text and `StyleArgument`.
pub fn validate_style_tag(value: &Tag) -> Result<(), String> {
    if !matches!(value, Tag::Compound(_)) {
        return Ok(());
    }
    let hover = field(value, "hover_event");
    let action = field(hover, "action");
    if string_is(action, "show_item") {
        super::validate_stack_tag(hover)?;
    } else if string_is(action, "show_text") {
        validate_text_tag(field(hover, "value"))?;
    } else if string_is(action, "show_entity") {
        validate_text_tag(field(hover, "name"))?;
    }
    let click = field(value, "click_event");
    if string_is(field(click, "action"), "show_dialog") {
        validate_dialog_tag(field(click, "dialog"))?;
    }
    Ok(())
}

fn plain_message_tag(value: &Tag) -> Result<(), String> {
    validate_text_tag(value.get("contents").unwrap_or(value))
}
fn dialog_body_tag(value: &Tag) -> Result<(), String> {
    if let Tag::List(values) = value {
        for value in values {
            dialog_body_tag(value)?;
        }
    } else if kind_is(value, "item") {
        super::validate_stack_tag(field(value, "item"))?;
        plain_message_tag(field(value, "description"))?;
    } else if kind_is(value, "plain_message") {
        validate_text_tag(field(value, "contents"))?;
    }
    Ok(())
}
fn dialog_button_tag(value: &Tag) -> Result<(), String> {
    validate_text_tag(field(value, "label"))?;
    validate_text_tag(field(value, "tooltip"))?;
    let action = field(value, "action");
    if kind_is(action, "show_dialog") {
        validate_dialog_tag(field(action, "dialog"))?;
    }
    Ok(())
}

/// Borrowed NBT counterpart of the stock dialog codec traversal.
pub fn validate_dialog_tag(value: &Tag) -> Result<(), String> {
    if !matches!(value, Tag::Compound(_) | Tag::List(_)) {
        return Ok(());
    }
    if let Tag::List(values) = value {
        for value in values {
            validate_dialog_tag(value)?;
        }
        return Ok(());
    }
    if ![
        "notice",
        "confirmation",
        "multi_action",
        "dialog_list",
        "server_links",
    ]
    .iter()
    .any(|kind| kind_is(value, kind))
    {
        return Ok(());
    }
    validate_text_tag(field(value, "title"))?;
    validate_text_tag(field(value, "external_title"))?;
    dialog_body_tag(field(value, "body"))?;
    for input in tag_list(field(value, "inputs")) {
        if ["boolean", "number_range", "single_option", "text"]
            .iter()
            .any(|kind| kind_is(input, kind))
        {
            validate_text_tag(field(input, "label"))?;
            if kind_is(input, "single_option") {
                for option in tag_list(field(input, "options")) {
                    validate_text_tag(field(option, "display"))?;
                }
            }
        }
    }
    if kind_is(value, "confirmation") {
        dialog_button_tag(field(value, "yes"))?;
        dialog_button_tag(field(value, "no"))?;
    } else if kind_is(value, "notice") {
        dialog_button_tag(field(value, "action"))?;
    } else if kind_is(value, "multi_action") {
        for action in tag_list(field(value, "actions")) {
            dialog_button_tag(action)?;
        }
    } else if kind_is(value, "dialog_list") {
        validate_dialog_tag(field(value, "dialogs"))?;
    }
    if ["multi_action", "dialog_list", "server_links"]
        .iter()
        .any(|kind| kind_is(value, kind))
    {
        dialog_button_tag(field(value, "exit_action"))?;
    }
    Ok(())
}

/// Borrowed NBT text-bearing component traversal, without payload conversion.
pub fn validate_component_text_tag(id: &str, value: &Tag) -> Result<(), String> {
    match id.strip_prefix("minecraft:").unwrap_or(id) {
        "custom_name" | "item_name" => validate_text_tag(value)?,
        "lore" => {
            for line in tag_list(value) {
                validate_text_tag(line)?;
            }
        }
        "written_book_content" => {
            for page in tag_list(field(value, "pages")) {
                if let Some(raw) = page.get("raw") {
                    validate_text_tag(raw)?;
                    validate_text_tag(field(page, "filtered"))?;
                } else {
                    validate_text_tag(page)?;
                }
            }
        }
        "instrument" | "provides_trim_material" => validate_text_tag(field(value, "description"))?,
        "trim" => {
            validate_text_tag(field(field(value, "material"), "description"))?;
            validate_text_tag(field(field(value, "pattern"), "description"))?;
        }
        "sign_text_front" | "sign_text_back" => {
            for key in ["messages", "filtered_messages"] {
                for message in tag_list(field(value, key)) {
                    validate_text_tag(message)?;
                }
            }
        }
        "attribute_modifiers" => {
            for entry in tag_list(value) {
                let display = field(entry, "display");
                if string_is(field(display, "type"), "override") {
                    validate_text_tag(field(display, "value"))?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn tag_list_mut(value: &mut Tag) -> &mut [Tag] {
    match value {
        Tag::List(values) => values,
        _ => &mut [],
    }
}

fn transient_name(name: &str) -> bool {
    let name = name.strip_prefix('!').unwrap_or(name);
    if name.contains(':') {
        return super::is_transient_component(name);
    }
    // Component names reach this helper through ascii_name's bounded buffer.
    let mut normalized = [0u8; 138];
    normalized[..10].copy_from_slice(b"minecraft:");
    normalized[10..10 + name.len()].copy_from_slice(name.as_bytes());
    super::is_transient_component(
        std::str::from_utf8(&normalized[..10 + name.len()]).expect("ASCII component name"),
    )
}

/// Persistent saves silently omit transient SET and REMOVE entries, not their
/// containing text, stack or native payload. Never use this on the network path.
pub fn strip_transient_component_map_tag(value: &mut Tag) {
    if let Tag::Compound(map) = value {
        let mut scratch = [0u8; 128];
        map.retain(|key, payload| {
            let Some(name) = super::ascii_name(key, &mut scratch) else {
                return true;
            };
            if transient_name(name) {
                return false;
            }
            strip_transient_component_tag(name, payload);
            true
        });
    }
}

fn strip_stack_tag(value: &mut Tag) {
    if let Some(components) = value.get_mut("components") {
        strip_transient_component_map_tag(components);
    }
}

fn strip_item_predicate_tag(value: &mut Tag) {
    strip_stack_tag(value);
    if let Some(Tag::Compound(predicates)) = value.get_mut("predicates") {
        let mut scratch = [0u8; 128];
        for (name, predicate) in predicates {
            let Some(name) = super::ascii_name(name, &mut scratch) else {
                continue;
            };
            if !matches!(
                name.strip_prefix("minecraft:").unwrap_or(name),
                "container" | "bundle_contents" | "charged_projectiles"
            ) {
                continue;
            }
            if let Some(items) = predicate.get_mut("items") {
                if let Some(contains) = items.get_mut("contains") {
                    for item in tag_list_mut(contains) {
                        strip_item_predicate_tag(item);
                    }
                }
                if let Some(counts) = items.get_mut("count") {
                    for count in tag_list_mut(counts) {
                        if let Some(test) = count.get_mut("test") {
                            strip_item_predicate_tag(test);
                        }
                    }
                }
            }
        }
    }
}

fn strip_text_tag(value: &mut Tag) {
    if !matches!(value, Tag::Compound(_) | Tag::List(_)) {
        return;
    }
    if let Tag::List(values) = value {
        for value in values {
            strip_text_tag(value);
        }
        return;
    }
    let explicit = value.get("type");
    let with = string_is(field(value, "type"), "translatable")
        || (explicit.is_none() && value.get("translate").is_some() && value.get("text").is_none());
    let separator = string_is(field(value, "type"), "selector")
        || string_is(field(value, "type"), "nbt")
        || (explicit.is_none()
            && ["text", "translate", "keybind", "score"]
                .iter()
                .all(|key| value.get(key).is_none())
            && (value.get("selector").is_some() || value.get("nbt").is_some()));
    strip_style_tag(value);
    if let Some(extra) = value.get_mut("extra") {
        for child in tag_list_mut(extra) {
            strip_text_tag(child);
        }
    }
    if with && let Some(arguments) = value.get_mut("with") {
        for argument in tag_list_mut(arguments) {
            strip_text_tag(argument);
        }
    }
    if separator && let Some(separator) = value.get_mut("separator") {
        strip_text_tag(separator);
    }
}

fn strip_style_tag(value: &mut Tag) {
    if let Some(hover) = value.get_mut("hover_event") {
        if string_is(field(hover, "action"), "show_item") {
            strip_stack_tag(hover);
        } else if string_is(field(hover, "action"), "show_text") {
            if let Some(text) = hover.get_mut("value") {
                strip_text_tag(text);
            }
        } else if string_is(field(hover, "action"), "show_entity")
            && let Some(name) = hover.get_mut("name")
        {
            strip_text_tag(name);
        }
    }
    if let Some(click) = value.get_mut("click_event")
        && string_is(field(click, "action"), "show_dialog")
        && let Some(dialog) = click.get_mut("dialog")
    {
        strip_dialog_tag(dialog);
    }
}

fn strip_plain_message_tag(value: &mut Tag) {
    if let Some(contents) = value.get_mut("contents") {
        strip_text_tag(contents);
    } else {
        strip_text_tag(value);
    }
}

fn strip_dialog_body_tag(value: &mut Tag) {
    if let Tag::List(values) = value {
        for value in values {
            strip_dialog_body_tag(value);
        }
    } else if kind_is(value, "item") {
        if let Some(item) = value.get_mut("item") {
            strip_stack_tag(item);
        }
        if let Some(description) = value.get_mut("description") {
            strip_plain_message_tag(description);
        }
    } else if kind_is(value, "plain_message")
        && let Some(contents) = value.get_mut("contents")
    {
        strip_text_tag(contents);
    }
}

fn strip_dialog_button_tag(value: &mut Tag) {
    for key in ["label", "tooltip"] {
        if let Some(text) = value.get_mut(key) {
            strip_text_tag(text);
        }
    }
    if let Some(action) = value.get_mut("action")
        && kind_is(action, "show_dialog")
        && let Some(dialog) = action.get_mut("dialog")
    {
        strip_dialog_tag(dialog);
    }
}

fn strip_dialog_tag(value: &mut Tag) {
    if !matches!(value, Tag::Compound(_) | Tag::List(_)) {
        return;
    }
    if let Tag::List(values) = value {
        for value in values {
            strip_dialog_tag(value);
        }
        return;
    }
    if ![
        "notice",
        "confirmation",
        "multi_action",
        "dialog_list",
        "server_links",
    ]
    .iter()
    .any(|kind| kind_is(value, kind))
    {
        return;
    }
    for key in ["title", "external_title"] {
        if let Some(text) = value.get_mut(key) {
            strip_text_tag(text);
        }
    }
    if let Some(body) = value.get_mut("body") {
        strip_dialog_body_tag(body);
    }
    if let Some(inputs) = value.get_mut("inputs") {
        for input in tag_list_mut(inputs) {
            if ["boolean", "number_range", "single_option", "text"]
                .iter()
                .any(|kind| kind_is(input, kind))
            {
                if let Some(label) = input.get_mut("label") {
                    strip_text_tag(label);
                }
                if kind_is(input, "single_option")
                    && let Some(options) = input.get_mut("options")
                {
                    for option in tag_list_mut(options) {
                        if let Some(display) = option.get_mut("display") {
                            strip_text_tag(display);
                        }
                    }
                }
            }
        }
    }
    if kind_is(value, "confirmation") {
        for key in ["yes", "no"] {
            if let Some(button) = value.get_mut(key) {
                strip_dialog_button_tag(button);
            }
        }
    } else if kind_is(value, "notice") {
        if let Some(button) = value.get_mut("action") {
            strip_dialog_button_tag(button);
        }
    } else if kind_is(value, "multi_action") {
        if let Some(actions) = value.get_mut("actions") {
            for action in tag_list_mut(actions) {
                strip_dialog_button_tag(action);
            }
        }
    } else if kind_is(value, "dialog_list")
        && let Some(dialogs) = value.get_mut("dialogs")
    {
        strip_dialog_tag(dialogs);
    }
    if ["multi_action", "dialog_list", "server_links"]
        .iter()
        .any(|kind| kind_is(value, kind))
        && let Some(button) = value.get_mut("exit_action")
    {
        strip_dialog_button_tag(button);
    }
}

/// Prune only the component codec's typed nested values, preserving opaque data.
pub fn strip_transient_component_tag(id: &str, value: &mut Tag) {
    match id.strip_prefix("minecraft:").unwrap_or(id) {
        "container" => {
            for slot in tag_list_mut(value) {
                if let Some(item) = slot.get_mut("item") {
                    strip_stack_tag(item);
                }
            }
        }
        "bundle_contents" | "charged_projectiles" => {
            for stack in tag_list_mut(value) {
                strip_stack_tag(stack);
            }
        }
        "use_remainder" | "sulfur_cube_content" => strip_stack_tag(value),
        "lock" => strip_item_predicate_tag(value),
        "can_break" | "can_place_on" => {
            if let Tag::List(predicates) = value {
                for predicate in predicates {
                    strip_item_predicate_tag(predicate);
                }
            } else {
                strip_item_predicate_tag(value);
            }
        }
        "custom_name" | "item_name" => strip_text_tag(value),
        "lore" => {
            for line in tag_list_mut(value) {
                strip_text_tag(line);
            }
        }
        "written_book_content" => {
            if let Some(pages) = value.get_mut("pages") {
                for page in tag_list_mut(pages) {
                    if page.get("raw").is_some() {
                        for key in ["raw", "filtered"] {
                            if let Some(text) = page.get_mut(key) {
                                strip_text_tag(text);
                            }
                        }
                    } else {
                        strip_text_tag(page);
                    }
                }
            }
        }
        "instrument" | "provides_trim_material" => {
            if let Some(description) = value.get_mut("description") {
                strip_text_tag(description);
            }
        }
        "trim" => {
            for key in ["material", "pattern"] {
                if let Some(holder) = value.get_mut(key)
                    && let Some(description) = holder.get_mut("description")
                {
                    strip_text_tag(description);
                }
            }
        }
        "sign_text_front" | "sign_text_back" => {
            for key in ["messages", "filtered_messages"] {
                if let Some(messages) = value.get_mut(key) {
                    for text in tag_list_mut(messages) {
                        strip_text_tag(text);
                    }
                }
            }
        }
        "attribute_modifiers" => {
            for entry in tag_list_mut(value) {
                if let Some(display) = entry.get_mut("display")
                    && string_is(field(display, "type"), "override")
                    && let Some(text) = display.get_mut("value")
                {
                    strip_text_tag(text);
                }
            }
        }
        _ => {}
    }
}
