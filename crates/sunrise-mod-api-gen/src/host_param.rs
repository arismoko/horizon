use super::{Hook, HostAccess, PhaseContext, Side};

/// The hook's host parameter type and the expression passed by its native method.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct HostParam {
    pub ty: String,
    pub argument: HostArgument,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum HostArgument {
    Context,
    Receiver,
    SharedReborrow,
    SharedGame,
    GameSide,
    LevelSide,
    LevelRandomSide,
    LevelTicks,
}

impl HostArgument {
    pub(super) fn render_for(
        &self,
        receiver: &str,
        split: bool,
        scoped_read: bool,
        without_container_loot: bool,
        mutable: Option<bool>,
    ) -> String {
        if scoped_read {
            return format!("{receiver}.host");
        }
        let argument = match self {
            Self::Context => "context".into(),
            Self::Receiver => receiver.into(),
            Self::SharedReborrow if !without_container_loot && mutable == Some(false) => {
                receiver.into()
            }
            Self::SharedReborrow => format!("&*{receiver}"),
            Self::SharedGame if split => format!("{receiver}.as_host()"),
            Self::GameSide if split => format!("{receiver}.host_side()"),
            Self::SharedGame => format!("crate::game::Side::as_host({receiver})"),
            Self::GameSide => format!("super::execution_role::host_side({receiver})"),
            Self::LevelSide => format!("super::mod_api_host::level_side({receiver})"),
            Self::LevelRandomSide => format!("{receiver}.level_random_side()"),
            Self::LevelTicks => {
                format!("&mut super::mod_api_host::LevelTicks::new({receiver}, ticks)")
            }
        };
        if without_container_loot {
            format!("{argument}.without_container_loot()")
        } else {
            argument
        }
    }
}

/// Whether the hook's host is a level host: a `GameLevel` method, or a hook marked
/// `"host": "level"` or `"host": "hit"` whose receiver is another level (a hit's `HitLevel`).
pub(super) fn is_level(hook: &Hook) -> bool {
    matches!(
        hook.host,
        Some(HostAccess::Level | HostAccess::LevelRandomSide | HostAccess::Hit)
    ) || game_level(hook)
}

fn game_level(hook: &Hook) -> bool {
    hook.gameplay_binding()
        .is_ok_and(|binding| binding.is_some_and(|binding| binding.kind.level()))
        || hook.target.contains("::GameLevel<")
        || hook.target.contains("::LevelView<")
}

/// Validate catalog access restrictions against the hook's receiver.
pub(super) fn validate(
    hook: &Hook,
    name: &str,
    is_level: bool,
    mutable: Option<bool>,
    ticks: bool,
) -> Result<(), String> {
    if hook.context.is_some() {
        let owns_host = mutable.is_some() || is_level || ticks;
        if owns_host || hook.host.is_some() {
            return Err(format!(
                "{name}: a phase context cannot be combined with a game/level host"
            ));
        }
    }
    if let Some(marker) = hook.host {
        match marker {
            HostAccess::Read if !is_level && hook.side != Side::Client => {}
            // A server hit's level writes through its random host; a both-side one reads, as
            // a both-side `GameLevel` does, and is told its side by `LevelSide`.
            HostAccess::Level
                if !game_level(hook)
                    && !ticks
                    && (hook.side == Side::Server && mutable.is_some()
                        || hook.side == Side::Both && mutable == Some(false)) => {}
            HostAccess::LevelRandomSide
                if !game_level(hook)
                    && !ticks
                    && hook.side == Side::Both
                    && mutable == Some(true) => {}
            // A hit's capability is the server hit's: a both-side hit's host only reads.
            HostAccess::Hit
                if !game_level(hook)
                    && !ticks
                    && hook.side == Side::Server
                    && mutable == Some(true) => {}
            HostAccess::Read
            | HostAccess::Level
            | HostAccess::LevelRandomSide
            | HostAccess::Hit => {
                return Err(format!("{name}: this entry takes no host marker"));
            }
        }
    }
    if !is_level
        && hook.side == Side::Both
        && mutable == Some(false)
        && hook.host != Some(HostAccess::Read)
    {
        return Err(format!(
            "{name}: a both-side hook with a &self target takes host \"read\""
        ));
    }
    if is_level
        && hook.side == Side::Both
        && (mutable == Some(true) || ticks)
        && hook.host != Some(HostAccess::LevelRandomSide)
    {
        return Err(format!(
            "{name}: a both-side level hook takes the read kind until a client level seam exists"
        ));
    }
    Ok(())
}

/// The phase input is declared by `context`, separate from the event fields.
pub(super) fn context_param(hook: &Hook) -> Option<HostParam> {
    Some(HostParam {
        ty: match hook.context? {
            PhaseContext::Registration => "&RegistrationContext",
            PhaseContext::DataLoad => "&dyn DataLoadContext",
            PhaseContext::Loop => "&dyn LoopContext",
            PhaseContext::Worldgen => "&dyn WorldgenHost",
            PhaseContext::ChunkDisk => "&dyn ChunkDiskHost",
            PhaseContext::Config => "&dyn ConfigContext",
            PhaseContext::Reload => "&dyn ReloadContext",
            PhaseContext::ClientReload => "&dyn ClientReloadHost",
            PhaseContext::ClientReloadListener => "&dyn ClientReloadListenerContext",
            PhaseContext::ClientSetup => "&mut dyn ClientSetupContext",
            PhaseContext::ClientIdentity => "&dyn ClientIdentityHost",
            PhaseContext::PackDiscovery => "&mut dyn PackDiscoveryContext",
            PhaseContext::ItemAbility => "&ItemAbilityContext<'_>",
        }
        .into(),
        argument: HostArgument::Context,
    })
}

pub(super) fn phase_engine_type(phase: PhaseContext) -> &'static str {
    match phase {
        PhaseContext::Registration => "&mod_api::RegistrationContext",
        PhaseContext::DataLoad => "&sunrise_world::native_load_context::LoadContext<'_>",
        PhaseContext::Loop => "&sunrise_world::native_load_context::LoopContext",
        PhaseContext::Worldgen => "&mod_api::OwningWorldgen<'_>",
        PhaseContext::ChunkDisk => "&mod_api::OwningChunkDisk<'_>",
        PhaseContext::Config => "&dyn mod_api::ConfigContext",
        PhaseContext::Reload => "&dyn mod_api::ReloadContext",
        PhaseContext::ClientReload => "&dyn mod_api::ClientReloadHost",
        PhaseContext::ClientReloadListener => "&dyn mod_api::ClientReloadListenerContext",
        PhaseContext::ClientSetup => "&mut dyn mod_api::ClientSetupContext",
        PhaseContext::ClientIdentity => "&dyn mod_api::ClientIdentityHost",
        PhaseContext::PackDiscovery => "&mut dyn mod_api::PackDiscoveryContext",
        PhaseContext::ItemAbility => "&mod_api::ItemAbilityContext<'_>",
    }
}

/// Select the host once for both generated signatures and the native call.
pub(super) fn select(hook: &Hook, mutable: Option<bool>) -> Option<HostParam> {
    context_param(hook).or_else(|| host_param(hook, mutable, hook.engine_ticks))
}

pub(super) fn host_param(hook: &Hook, mutable: Option<bool>, ticks: bool) -> Option<HostParam> {
    use HostArgument::*;
    let is_level = is_level(hook);
    mutable.map(|mutable| {
        if !is_level {
            let (ty, argument) = match hook.side {
                Side::Server if hook.host == Some(HostAccess::Read) => {
                    ("&dyn Host", SharedReborrow)
                }
                Side::Both if hook.host == Some(HostAccess::Read) => ("&dyn Host", SharedGame),
                Side::Server => ("&mut dyn ServerHost", Receiver),
                Side::Client if mutable => ("&mut dyn ClientHost", Receiver),
                Side::Client => ("&dyn ClientHost", SharedReborrow),
                Side::Both => ("HostSide", GameSide),
            };
            return HostParam {
                ty: ty.into(),
                argument,
            };
        }
        if hook.host == Some(HostAccess::LevelRandomSide) {
            return HostParam {
                ty: "LevelRandomSide".into(),
                argument: LevelRandomSide,
            };
        }
        if hook.side == Side::Both {
            return HostParam {
                ty: "LevelSide".into(),
                argument: LevelSide,
            };
        }
        if hook.host == Some(HostAccess::Hit) {
            return HostParam {
                ty: "&mut dyn HitHost".into(),
                argument: Receiver,
            };
        }
        let kind = if is_level {
            match (mutable, ticks) {
                (true, true) => "LevelWriteHost",
                (false, true) => "LevelTicksHost",
                (true, false) => "LevelRandomHost",
                (false, false) => "LevelHost",
            }
        } else {
            "Host"
        };
        let mutable = if mutable || (is_level && ticks) {
            "mut "
        } else {
            ""
        };
        HostParam {
            ty: format!("&{mutable}dyn {kind}"),
            argument: if ticks { LevelTicks } else { Receiver },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_context_is_not_an_event_field_or_game_host() {
        for (kind, public) in [
            (PhaseContext::Registration, "&RegistrationContext"),
            (PhaseContext::DataLoad, "&dyn DataLoadContext"),
            (PhaseContext::Loop, "&dyn LoopContext"),
            (PhaseContext::Worldgen, "&dyn WorldgenHost"),
            (PhaseContext::ChunkDisk, "&dyn ChunkDiskHost"),
            (PhaseContext::Config, "&dyn ConfigContext"),
            (PhaseContext::Reload, "&dyn ReloadContext"),
            (PhaseContext::ClientReload, "&dyn ClientReloadHost"),
            (
                PhaseContext::ClientReloadListener,
                "&dyn ClientReloadListenerContext",
            ),
            (PhaseContext::ClientSetup, "&mut dyn ClientSetupContext"),
            (PhaseContext::PackDiscovery, "&mut dyn PackDiscoveryContext"),
            (PhaseContext::ItemAbility, "&ItemAbilityContext<'_>"),
        ] {
            let hook = Hook {
                context: Some(kind),
                ..Hook::sample("sunrise_client::game::Game<'_>::native_phase", Side::Both)
            };
            assert_eq!(context_param(&hook).unwrap().ty, public);
            assert!(validate(&hook, "phase", false, None, false).is_ok());
            assert!(validate(&hook, "phase", false, Some(true), false).is_err());
        }
    }
}
