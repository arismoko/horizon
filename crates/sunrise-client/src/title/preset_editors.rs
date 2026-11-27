//! Data-pack preset selection and bound world-preset editor factories (`register_preset_editors`,
//! NeoForge's `RegisterPresetEditorsEvent` and vanilla's `PresetEditor`).
use super::*;
use crate::save::{
    valid_world_gen_dimension_stems as valid_dimension_stems,
    valid_world_gen_dimensions as valid_dimensions,
};
use mod_api::{
    __private::game::{
        CallbackOwner, CallbackValue, PresetEditorFactoryRegistration, new_preset_editor_context,
        new_preset_editor_draft,
    },
    Answered, Compound, DrawContext, Identifier, PresetEditor, PresetEditorFactoryFn,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt,
    sync::{Arc, LazyLock, Mutex, RwLock},
};
use sunrise_core::nbt::Tag;

struct BoundPresetEditorFactory {
    preset: Identifier,
    factory: CallbackValue<Arc<PresetEditorFactoryFn>>,
}

/// The registration snapshot used by a title screen.
#[derive(Default)]
pub struct PresetEditorFactories(BTreeMap<String, BoundPresetEditorFactory>);

static EDITORS: LazyLock<RwLock<Arc<PresetEditorFactories>>> = LazyLock::new(Default::default);

impl PresetEditorFactories {
    /// Bind the client registration answers (last registration per preset wins, including
    /// `minecraft:` overrides).
    pub fn bind(editors: Vec<Answered<PresetEditorFactoryRegistration>>) -> Self {
        let mut table = BTreeMap::new();
        for Answered { answer, mod_id, .. } in editors {
            let key = answer.preset.to_string();
            let Some(owner) = CallbackOwner::new(mod_id) else {
                log::error!("Preset editor {key} has no installed owner {mod_id}");
                CallbackOwner::for_mod(mod_id).dispose(
                    "preset editor",
                    "drop factory",
                    answer.factory,
                );
                continue;
            };
            let entry = BoundPresetEditorFactory {
                preset: answer.preset,
                factory: CallbackValue::new(owner, "preset editor", "drop factory", answer.factory),
            };
            if let Some(previous) = table.insert(key.clone(), entry) {
                log::debug!(
                    "PresetEditor {key} from mod {} overridden by mod {mod_id}",
                    previous.factory.owner().id()
                );
            } else if matches!(
                key.as_str(),
                "minecraft:flat" | "minecraft:single_biome_surface"
            ) {
                log::debug!("PresetEditor {key} (vanilla) overridden by mod {mod_id}");
            }
        }
        Self(table)
    }

    pub(super) fn installed() -> Arc<Self> {
        EDITORS
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn lookup(&self, preset: &str) -> Option<&BoundPresetEditorFactory> {
        self.0.get(preset)
    }

    pub(super) fn has_editor(&self, preset: &str) -> bool {
        self.lookup(preset).is_some()
    }

    /// Invoke the bound factory for `preset`; no registration is `Ok(None)`, a failed open is
    /// `Err`, and a successful open returns the session.
    pub(super) fn open_session(
        &self,
        preset: &str,
        dimensions: Compound,
        seed: &str,
    ) -> Result<Option<PresetEditorSession>, String> {
        let Some(entry) = self.lookup(preset) else {
            return Ok(None);
        };
        let owner = entry.factory.owner();
        let ctx = new_preset_editor_context(&entry.preset, &dimensions, seed);
        let created = owner.call("preset editor", "open", || {
            (entry.factory.value())(&ctx).map(|editor| {
                // The callback can return a value after a caught phase refusal. It still
                // belongs to the owner when the refusal fence discards it.
                CallbackValue::new(
                    owner.clone(),
                    "preset editor",
                    "drop editor",
                    Mutex::new(editor),
                )
            })
        })?;
        let editor = match created {
            Ok(editor) => editor,
            Err(reason) => {
                log::warn!(
                    "Mod {}: preset editor factory for {preset} refused to open: {reason}",
                    owner.id()
                );
                return Err(reason);
            }
        };
        let title = editor.owner().call("preset editor", "title", || {
            editor
                .value()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .title()
        })?;
        Ok(Some(PresetEditorSession {
            preset: preset.to_owned(),
            preset_id: entry.preset.clone(),
            dimensions: RefCell::new(dimensions),
            title,
            live: LivePresetEditor {
                editor: Arc::new(editor),
            },
        }))
    }
}

/// Install the client registration-phase table for subsequently created title screens.
pub fn register(editors: Vec<Answered<PresetEditorFactoryRegistration>>) {
    let table = Arc::new(PresetEditorFactories::bind(editors));
    let mut installed = EDITORS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let displaced = std::mem::replace(&mut *installed, table);
    drop(installed);
    drop(displaced);
}

#[derive(Clone)]
struct LivePresetEditor {
    editor: Arc<CallbackValue<Mutex<Box<dyn PresetEditor>>>>,
}

impl fmt::Debug for LivePresetEditor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LivePresetEditor")
            .field("owner", &self.editor.owner().id())
            .finish()
    }
}

/// An open world-preset editor session bound to its owning mod's [`PresetEditor`] instance.
#[derive(Clone, Debug)]
pub struct PresetEditorSession {
    /// World-preset registry identifier (`example:islands` or `minecraft:flat`).
    pub preset: String,
    preset_id: Identifier,
    dimensions: RefCell<Compound>,
    title: Option<String>,
    live: LivePresetEditor,
}

impl PresetEditorSession {
    /// Optional header title supplied by `PresetEditor::title`.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Snapshot of the working dimension stems as a `Tag::Compound`.
    pub fn dimensions(&self) -> Tag {
        Tag::Compound(self.dimensions.borrow().clone())
    }

    /// Replace the working dimension stems after structural validation.
    pub fn set_dimensions(&self, value: Tag) -> bool {
        if !valid_dimensions(&value) {
            return false;
        }
        let Tag::Compound(stems) = value else {
            return false;
        };
        *self.dimensions.borrow_mut() = stems;
        true
    }

    fn with_editor<R>(
        &self,
        operation: &'static str,
        invoke: impl FnOnce(&mut dyn PresetEditor, &mut mod_api::PresetEditorDraft<'_>) -> R,
    ) -> Option<R> {
        let mut working = self.dimensions.borrow().clone();
        let validator = valid_dimension_stems;
        let editor = &self.live.editor;
        let result = editor.owner().call("preset editor", operation, || {
            let mut guard = editor
                .value()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut draft = new_preset_editor_draft(&self.preset_id, &mut working, &validator);
            invoke(guard.as_mut(), &mut draft)
        });
        match result {
            Ok(out) => {
                *self.dimensions.borrow_mut() = working;
                Some(out)
            }
            Err(_) => None,
        }
    }

    /// Render the bound editor into `draw` inside `rect`, returning whether the call succeeded.
    pub fn render(&self, draw: &mut dyn DrawContext, rect: [f32; 4]) -> bool {
        self.with_editor("render", |editor, draft| editor.render(draw, rect, draft))
            .is_some()
    }

    /// Forward a mouse press (`button`: GLFW index) and its double-click flag to the bound editor.
    pub fn mouse_clicked(&self, x: f32, y: f32, button: i32, double_click: bool) -> bool {
        self.with_editor("mouse_clicked", |editor, draft| {
            editor.mouse_clicked(x, y, button, double_click, draft)
        })
        .unwrap_or(false)
    }

    /// Forward a key press (`key`: vanilla `InputConstants` code) to the bound editor.
    pub fn key_pressed(&self, key: u32, scancode: u32, modifiers: u32) -> bool {
        self.with_editor("key_pressed", |editor, draft| {
            editor.key_pressed(key, scancode, modifiers, draft)
        })
        .unwrap_or(false)
    }

    /// Forward a typed character (`modifiers`: GLFW bits) to the bound editor.
    pub fn char_typed(&self, ch: char, modifiers: u32) -> bool {
        self.with_editor("char_typed", |editor, draft| {
            editor.char_typed(ch, modifiers, draft)
        })
        .unwrap_or(false)
    }
}

pub(super) use sunrise_sim::world_options::preset_label;

impl Title {
    /// Use an explicit registration snapshot for this title, without changing other titles.
    pub fn set_preset_editor_factories(&mut self, factories: PresetEditorFactories) {
        self.preset_editors = Arc::new(factories);
    }

    /// The already-enabled mod data layer, below the world's selectable packs.
    pub fn set_preset_data_source(&mut self, source: crate::resource_source::ResourceSource) {
        self.preset_data = Some(std::sync::Arc::new(crate::datapack::DataPack::from_source(
            "mods".into(),
            source,
        )));
        if let Mode::Create { options, .. } = &mut self.mode {
            options.refresh_presets(&self.resources, self.preset_data.as_deref());
        }
    }
    /// The open preset editor session; ordinary title screens return none.
    pub fn preset_editor_session(&self) -> Option<&PresetEditorSession> {
        match &self.mode {
            Mode::CreatePreset { session, .. } => Some(session),
            _ => None,
        }
    }
    /// A snapshot, so screen callbacks never retain a borrow across dispatch.
    pub fn preset_editor_dimensions(&self) -> Option<Tag> {
        self.preset_editor_session()
            .map(PresetEditorSession::dimensions)
    }
    /// Replace only the editor's draft, preserving its parent until Done.
    pub fn set_preset_editor_dimensions(&self, value: Tag) -> bool {
        self.preset_editor_session()
            .is_some_and(|session| session.set_dimensions(value))
    }
    pub(super) fn close_preset_editor(&mut self, apply: bool) {
        let Mode::CreatePreset { parent, session } = &self.mode else {
            return;
        };
        let mut parent = (**parent).clone();
        if apply && let Mode::Create { options, .. } = &mut parent {
            options.preset_dimensions = Some(session.dimensions());
        }
        self.set_mode(parent);
    }
}
