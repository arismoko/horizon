//! Prepare client listeners against the new pack selection before publishing
//! any assets, and apply them on the thread that owns the game. The initial
//! load runs the same three steps: the loader thread registers and stages,
//! and the game applies once it exists (`start_game`).
mod texture_atlases;
use crate::game::ClientGame;
use mod_api::{
    __private::game::{ClientReloadListenersOrder, ReloadContextNew},
    __private::reload::{PreparedStep, ReloadStageError, prepare_listeners},
    ClientReloadContext, ClientReloadHost, ClientReloadListener, ClientReloadListenerContext,
    HookId,
};
use sunrise_client::settings::Settings;
use sunrise_gameplay_common::game::packs_common::reload_files;
use sunrise_level::datapack::DataPack;
pub(in crate::game) use texture_atlases::{
    collect_emissive_sprites, collect_texture_atlases, stage_texture_atlases,
};

/// One client reload step: a mod's apply action or model publication.
pub type PreparedClientReload = PreparedStep<()>;

/// The registration context of the first load: no game or published assets
/// exist yet, so [`published`](ClientReloadListenerContext::published) is
/// `None` while process, mod-roster, settings, math-profile, and local text
/// queries return real values. Later reloads pass `&Game` directly.
pub(in crate::game) struct StartupReloadContext<'a> {
    settings: Option<&'a Settings>,
    captured: &'a sunrise_world::native_load_context::SettingsCapture,
}

impl ClientReloadListenerContext for StartupReloadContext<'_> {
    fn dist(&self) -> mod_api::Dist {
        sunrise_level::runtime_env::runtime_environment::try_get()
            .map_or(mod_api::Dist::Client, |env| env.dist())
    }
    fn game_version(&self) -> &'static str {
        sunrise_level::runtime_env::runtime_environment::try_get()
            .map_or(sunrise_core::VANILLA_VERSION, |env| env.game_version())
    }
    fn server_data_version(&self) -> i32 {
        sunrise_core::version::DATA_VERSION
    }
    fn sunrise_version(&self) -> &'static str {
        sunrise_level::runtime_env::runtime_environment::try_get()
            .map_or("", |env| env.sunrise_version())
    }
    fn game_tests_enabled(&self) -> bool {
        sunrise_level::runtime_env::runtime_environment::try_get()
            .is_some_and(|env| env.game_tests_enabled())
    }
    fn on(&self, mod_id: &str) -> bool {
        self.settings
            .is_none_or(|settings| !settings.mods.disabled.contains(mod_id))
    }
    fn is_loaded(&self, id: &str) -> bool {
        mod_api::__private::loaded_mods().get(id).is_some()
    }
    fn setting(
        &self,
        key: mod_api::SettingKey,
    ) -> Result<mod_api::SettingValue, mod_api::SettingAccessError> {
        let (owner, spec) = mod_api::__private::setting_entry(key)
            .ok_or(mod_api::SettingAccessError::UnknownKey)?;
        Ok(spec.read(self.captured.setting(owner, &spec.key)))
    }
    fn math_profile(&self) -> mod_api::MathProfile {
        sunrise_gameplay_common::game::startup_math_profile()
    }
    fn text_string(&self, text: &mod_api::Text) -> String {
        sunrise_game_data::lang::scoped(sunrise_game_data::lang::Language::default(), || {
            sunrise_sim::api_text::to_engine_inprocess(text).plain()
        })
        .0
    }
    fn published(&self) -> Option<&dyn ClientReloadHost> {
        None
    }
}

struct CapturedReloadContext<'a, 'g> {
    game: &'a ClientGame<'g>,
    captured: &'a sunrise_world::native_load_context::SettingsCapture,
}

impl ClientReloadListenerContext for CapturedReloadContext<'_, '_> {
    fn dist(&self) -> mod_api::Dist {
        self.game.dist()
    }
    fn game_version(&self) -> &'static str {
        self.game.game_version()
    }
    fn server_data_version(&self) -> i32 {
        self.game.server_data_version()
    }
    fn sunrise_version(&self) -> &'static str {
        self.game.sunrise_version()
    }
    fn game_tests_enabled(&self) -> bool {
        self.game.game_tests_enabled()
    }
    fn on(&self, mod_id: &str) -> bool {
        self.game.on(mod_id)
    }
    fn is_loaded(&self, id: &str) -> bool {
        self.game.is_loaded(id)
    }
    fn setting(
        &self,
        key: mod_api::SettingKey,
    ) -> Result<mod_api::SettingValue, mod_api::SettingAccessError> {
        let (owner, spec) = mod_api::__private::setting_entry(key)
            .ok_or(mod_api::SettingAccessError::UnknownKey)?;
        Ok(spec.read(self.captured.setting(owner, &spec.key)))
    }
    fn math_profile(&self) -> mod_api::MathProfile {
        self.game.math_profile()
    }
    fn text_string(&self, text: &mod_api::Text) -> String {
        self.game.text_string(text)
    }
    fn published(&self) -> Option<&dyn ClientReloadHost> {
        self.game.published()
    }
}

impl ClientGame<'_> {
    /// Registers a later reload's client listeners through the game
    /// (`native_add_client_reload_listeners`), gated on the hook being
    /// listened to.
    pub fn register_client_reload_listeners(
        &self,
        captured: &sunrise_world::native_load_context::SettingsCapture,
    ) -> mod_api::ClientListenerRegistry {
        let mut listeners = <mod_api::ClientListenerRegistry as ClientReloadListenersOrder>::new();
        if mod_api::__private::listened(HookId::AddClientReloadListeners) {
            let context = CapturedReloadContext {
                game: self,
                captured,
            };
            crate::game::native_hooks::add_client_reload_listeners::native_add_client_reload_listeners(
                &context,
                |listen| listeners.register_listener(listen),
            );
        }
        listeners
    }
}

/// Registers the first load's client listeners: no game exists yet, so
/// [`StartupReloadContext`] answers the process/mod/settings facts known on
/// the loader thread and leaves `published()` as `None`.
pub fn register_startup_client_reload_listeners(
    settings: Option<&Settings>,
    captured: &sunrise_world::native_load_context::SettingsCapture,
) -> mod_api::ClientListenerRegistry {
    let mut listeners = <mod_api::ClientListenerRegistry as ClientReloadListenersOrder>::new();
    if mod_api::__private::listened(HookId::AddClientReloadListeners) {
        let context = StartupReloadContext { settings, captured };
        mod_api::dispatch_add_client_reload_listeners(&context, |listen| {
            listeners.register_listener(listen)
        });
    }
    listeners
}

/// Prepare listeners against the merged asset view, running the model build
/// at the model manager's position in their order. The models travel separately
/// so renderer preflight can use them without publishing them.
pub fn stage_client_reload<V, E: From<ReloadStageError>>(
    root: &sunrise_assets::resource_source::ResourceSource,
    listeners: mod_api::ClientListenerRegistry,
    build_models: impl FnOnce() -> Result<V, E>,
) -> Result<(V, Vec<PreparedClientReload>), E> {
    let pack = DataPack::from_source("vanilla".into(), root.clone());
    let ordered = listeners.finish().map_err(ReloadStageError::Order)?;
    let mut build_models = Some(build_models);
    let mut models = None;
    let steps = prepare_listeners(
        ordered,
        HookId::AddClientReloadListeners,
        "client_reload_prepare",
        || -> Result<(), E> {
            models = Some(build_models.take().expect("one model manager")()?);
            Ok(())
        },
        |id, converter| {
            <ClientReloadContext as ReloadContextNew>::new(reload_files(
                "assets",
                &[&pack],
                id,
                converter,
            ))
        },
        ClientReloadListener::prepare,
    )?;
    Ok((models.expect("model manager is always registered"), steps))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static APPLY_GENERATIONS: std::cell::RefCell<Option<Vec<u64>>> = const { std::cell::RefCell::new(None) };
}

impl ClientGame<'_> {
    /// Publish the models in order with the mod listener actions on the game-owning
    /// thread, before the resources-reloaded notice.
    pub(in crate::game::client_side) fn apply_client_reload(
        &mut self,
        prepared: Vec<PreparedClientReload>,
        publish_models: impl FnOnce(&mut Self),
    ) {
        let mut publish_models = Some(publish_models);
        for step in prepared {
            #[cfg(any(test, feature = "test-support"))]
            if matches!(step, PreparedStep::Mod { .. }) {
                APPLY_GENERATIONS.with_borrow_mut(|trace| {
                    if let Some(trace) = trace {
                        trace.push(self.client.client_pack.render.generation);
                    }
                });
            }
            let _ = step
                .apply(
                    HookId::AddClientReloadListeners,
                    "client_reload_apply",
                    |()| {
                        publish_models.take().expect("one model manager")(self);
                        Ok::<_, std::convert::Infallible>(())
                    },
                )
                .unwrap();
        }
    }
}
