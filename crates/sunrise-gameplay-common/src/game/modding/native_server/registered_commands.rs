//! The mods' commands as the session holds them (`register_commands`). As on the reference
//! loader, the mods are asked whenever the session's command tree is built: when a world opens,
//! before its functions load, and on each `/reload`. The tree lives on `Game`, so a function
//! line naming a mod command parses with no hub; a hub, once there is one, takes that same tree.

use super::{CommandArguments, CommandExecutorSlots};
use mod_api::__private::{CommandExecutor, RetainedCallback};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub struct RegisteredExecutor {
    pub callback: RetainedCallback,
    pub call: Arc<CommandExecutor>,
}

/// The commands the mods registered, as the session's tree holds them.
pub struct NativeCommands {
    /// Each root's traversal level.
    pub levels: HashMap<String, u8>,
    /// Accepted display data, before filtering roots of a replacement tree.
    pub tree: Value,
    pub arguments: CommandArguments,
    /// Executable path to registration slot; never sent to the hub.
    pub slots: CommandExecutorSlots,
    /// Process-local execution table indexed by the current dispatcher's parsed keys.
    pub executors: Vec<RegisteredExecutor>,
}

/// The session's state of the mods' commands.
#[derive(Default)]
pub struct NativeCommandState {
    /// A mod asked for its commands again (`refresh_commands`): done at the next server tick.
    pub refresh: bool,
    pub registered: Option<NativeCommands>,
    /// The display report of the currently installed server command tree.
    pub report: Value,
    /// A hub holds the report revision it was given, not the dispatcher incarnation.
    pub display_revision: u64,
}

pub fn without_reserved_roots(report: &Value, reserved: impl Fn(&str) -> bool) -> Value {
    let mut report = report.clone();
    if let Some(roots) = report.get_mut("children").and_then(Value::as_object_mut) {
        roots.retain(|name, _| !reserved(name));
        if roots.is_empty() {
            return Value::Null;
        }
    }
    report
}

impl NativeCommandState {
    /// The traversal level of a mod's root (`line`, without its slash), when registered.
    /// The parsed executable node enforces its own usage level separately.
    pub fn level(&self, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        self.report["children"].get(name)?;
        self.registered.as_ref()?.levels.get(name).copied()
    }
}
