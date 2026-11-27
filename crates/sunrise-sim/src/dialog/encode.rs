//! The NbtOps projection of a decoded datapack dialog. Components have
//! already been encoded by the component reader, and inline holders use
//! this same projection.
use super::{Action, AfterAction, Body, Button, Click, Control, Dialog, Input, Kind};
use sunrise_core::nbt::{Compound, Tag};

fn string(s: impl AsRef<str>) -> Tag {
    Tag::String(s.as_ref().into())
}

fn flag(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

fn float(value: f32) -> Tag {
    Tag::Float(value.to_bits())
}

impl Dialog<Tag> {
    pub(super) fn encode(self) -> Tag {
        let mut fields = Compound::from([
            ("title".into(), self.title),
            ("pause".into(), flag(self.pause)),
            (
                "can_close_with_escape".into(),
                flag(self.can_close_with_escape),
            ),
            (
                "after_action".into(),
                string(match self.after_action {
                    AfterAction::Close => "close",
                    AfterAction::None => "none",
                    AfterAction::WaitForResponse => "wait_for_response",
                }),
            ),
        ]);
        fields.extend(
            self.external_title
                .map(|title| ("external_title".into(), title)),
        );
        if !self.body.is_empty() {
            fields.insert(
                "body".into(),
                Tag::List(self.body.into_iter().map(Body::encode).collect()),
            );
        }
        if !self.inputs.is_empty() {
            fields.insert(
                "inputs".into(),
                Tag::List(self.inputs.into_iter().map(Input::encode).collect()),
            );
        }
        let (kind, exit, columns, width) = match self.kind {
            Kind::Notice(action) => {
                fields.insert("action".into(), action.encode());
                ("notice", None, None, None)
            }
            Kind::Confirmation { yes, no } => {
                fields.insert("yes".into(), yes.encode());
                fields.insert("no".into(), no.encode());
                ("confirmation", None, None, None)
            }
            Kind::MultiAction {
                actions,
                exit,
                columns,
            } => {
                fields.insert(
                    "actions".into(),
                    Tag::List(actions.into_iter().map(Button::encode).collect()),
                );
                ("multi_action", exit, Some(columns), None)
            }
            Kind::ServerLinks {
                exit,
                columns,
                button_width,
            } => ("server_links", exit, Some(columns), Some(button_width)),
            Kind::DialogList {
                dialogs,
                exit,
                columns,
                button_width,
                ..
            } => {
                fields.insert("dialogs".into(), dialogs);
                ("dialog_list", exit, Some(columns), Some(button_width))
            }
        };
        fields.insert("type".into(), string(format!("minecraft:{kind}")));
        fields.extend(exit.map(|exit| ("exit_action".into(), exit.encode())));
        fields.extend(columns.map(|columns| ("columns".into(), Tag::Int(columns.get() as i32))));
        fields.extend(width.map(|width| ("button_width".into(), Tag::Int(width))));
        Tag::Compound(fields)
    }
}

impl Button<Tag> {
    fn encode(self) -> Tag {
        Tag::compound(
            [("label", self.label), ("width", Tag::Int(self.width))]
                .into_iter()
                .chain(self.tooltip.map(|t| ("tooltip", t)))
                .chain(self.action.map(|a| ("action", a.encode()))),
        )
    }
}

impl Action {
    fn encode(self) -> Tag {
        match self {
            Self::RunCommand(template) => Tag::compound([
                ("type", string("dynamic/run_command")),
                ("template", string(template)),
            ]),
            Self::Custom { id, additions } => Tag::compound(
                [("type", string("dynamic/custom")), ("id", string(id))]
                    .into_iter()
                    .chain(additions.map(|v| ("additions", v))),
            ),
            Self::Static(click) => click.encode("type"),
        }
    }
}

impl Click {
    pub(super) fn encode(self, dispatch: &'static str) -> Tag {
        let mut payload = None;
        let (kind, field, value) = match self {
            Self::OpenUrl(url) => ("open_url", "url", string(url)),
            Self::RunCommand(command) => ("run_command", "command", string(command)),
            Self::SuggestCommand(command) => ("suggest_command", "command", string(command)),
            Self::ShowDialog(dialog) => ("show_dialog", "dialog", dialog),
            Self::ChangePage(page) => ("change_page", "page", Tag::Int(page)),
            Self::CopyToClipboard(value) => ("copy_to_clipboard", "value", string(value)),
            Self::Custom {
                id,
                payload: custom_payload,
            } => {
                payload = custom_payload;
                ("custom", "id", string(id))
            }
        };
        Tag::compound(
            [(dispatch, string(kind)), (field, value)]
                .into_iter()
                .chain(payload.map(|v| ("payload", v))),
        )
    }
}

fn message(contents: Tag, width: i32) -> [(&'static str, Tag); 2] {
    [("contents", contents), ("width", Tag::Int(width))]
}

impl Body<Tag> {
    fn encode(self) -> Tag {
        match self {
            Self::Message { contents, width } => Tag::compound(
                [("type", string("minecraft:plain_message"))]
                    .into_iter()
                    .chain(message(contents, width)),
            ),
            Self::Item {
                item,
                description,
                show_decorations,
                show_tooltip,
                width,
                height,
            } => Tag::compound(
                [
                    ("type", string("minecraft:item")),
                    ("item", item),
                    ("show_decorations", flag(show_decorations)),
                    ("show_tooltip", flag(show_tooltip)),
                    ("width", Tag::Int(width)),
                    ("height", Tag::Int(height)),
                ]
                .into_iter()
                .chain(description.map(|(contents, width)| {
                    ("description", Tag::compound(message(contents, width)))
                })),
            ),
        }
    }
}

impl Input<Tag> {
    fn encode(self) -> Tag {
        let mut fields = Compound::from([("key".into(), string(self.key))]);
        let kind = match self.control {
            Control::Text {
                width,
                label,
                label_visible,
                initial,
                max_length,
                multiline,
            } => {
                fields.extend([
                    ("width".into(), Tag::Int(width)),
                    ("label".into(), label),
                    ("label_visible".into(), flag(label_visible)),
                    ("initial".into(), string(initial)),
                    ("max_length".into(), Tag::Int(max_length)),
                ]);
                if let Some((max_lines, height)) = multiline {
                    fields.insert(
                        "multiline".into(),
                        Tag::compound(
                            [("max_lines", max_lines), ("height", height)]
                                .into_iter()
                                .filter_map(|(k, v)| Some((k, Tag::Int(v?)))),
                        ),
                    );
                }
                "text"
            }
            Control::Boolean {
                label,
                initial,
                on_true,
                on_false,
            } => {
                fields.extend([
                    ("label".into(), label),
                    ("initial".into(), flag(initial)),
                    ("on_true".into(), string(on_true)),
                    ("on_false".into(), string(on_false)),
                ]);
                "boolean"
            }
            Control::SingleOption {
                width,
                options,
                label,
                label_visible,
            } => {
                let options = options
                    .into_iter()
                    .map(|(id, display, initial)| {
                        Tag::compound(
                            [("id", string(id)), ("initial", flag(initial))]
                                .into_iter()
                                .chain(display.map(|v| ("display", v))),
                        )
                    })
                    .collect();
                fields.extend([
                    ("width".into(), Tag::Int(width)),
                    ("options".into(), Tag::List(options)),
                    ("label".into(), label),
                    ("label_visible".into(), flag(label_visible)),
                ]);
                "single_option"
            }
            Control::NumberRange {
                width,
                label,
                label_format,
                start,
                end,
                initial,
                step,
            } => {
                fields.extend([
                    ("width".into(), Tag::Int(width)),
                    ("label".into(), label),
                    ("label_format".into(), string(label_format)),
                    ("start".into(), float(start)),
                    ("end".into(), float(end)),
                ]);
                fields.extend(initial.map(|v| ("initial".into(), float(v))));
                fields.extend(step.map(|v| ("step".into(), float(v))));
                "number_range"
            }
        };
        fields.insert("type".into(), string(format!("minecraft:{kind}")));
        Tag::Compound(fields)
    }
}
