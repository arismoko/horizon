//! The platform's public API: what the Sunrise layer, mods and ports may use of the engine
//! (docs/layers.md#platform). The Sunrise layer depends on the platform only, never on an engine
//! crate, so it can build from another repository; when it needs something the engine has, it
//! comes through here.
//!
//! Today this re-exports engine modules whole. Each one gets its own stable types as the split
//! goes on (docs/layers.md#plan, M6). The parts that pull in large engine crates sit behind
//! features (`world`, `server-files`, `mod-build`), so a crate builds only what it uses.

/// The machine the game runs on (`sunrise-system`): OS, CPU, memory, drive, power, GPU maker and
/// memory.
pub use sunrise_system as system;

/// The benchmark report format and its hardware probes (`sunrise-bench`).
pub use sunrise_bench as bench;

/// The `.hzperf` capture contract and its files (`sunrise-perf-format`).
pub use sunrise_perf_format as perf_format;

/// The pinned Minecraft version: its id, protocol and data version.
pub use sunrise_core::version;

/// The player's "no AI" switch, which the launcher, the game and their agent tools share.
pub use sunrise_core::no_ai;

/// The prepared folder's file of files (`prepared_files::FILE_NAME`).
pub use sunrise_core::prepared_files;

/// Vanilla's world files, as tools outside the game read and write them.
#[cfg(feature = "world")]
pub mod world {
    pub use sunrise_core::nbt;
    pub use sunrise_core::position::ChunkPos;
    pub use sunrise_world::{compression, region};
}

/// A server's own files: `server.properties`, the op, white and ban lists, the user cache.
#[cfg(feature = "server-files")]
pub use sunrise_server_files as server_files;

/// Online account lookups and Minecraft Services public-key fetches over HTTPS (`sunrise-server-profiles-http`).
#[cfg(feature = "server-files")]
pub use sunrise_server_profiles_http as server_profiles_http;

/// `java.util.Properties` and the JDK's date text, for the files a server keeps in that format.
#[cfg(feature = "server-files")]
pub use sunrise_java_properties as java_properties;

/// Plugging into the game (`sunrise-hooks`): implement [`hooks::Hooks`] and
/// [`hooks::register`] it before the game starts (docs/layers.md, "How a Sunrise crate plugs
/// in"). The engine's dispatch side is not part of the API.
pub mod hooks {
    pub use sunrise_hooks::{
        Activity, Client, ClientIdentity, Dimension, GameMode, GetTheGame, Hooks, Play, Started,
        TooLate, connection, perf, register, seat, tunnel,
    };
}

/// The settings store (`sunrise_hooks::settings`): a crate's own `key:value` lines in the game
/// folder, loaded and saved with the player's settings (docs/layers.md, "Platform registries").
pub mod settings {
    pub use sunrise_hooks::settings::{Store, register};
}

/// Settings pages a crate adds to the options screens (docs/layers.md, "Platform registries").
pub mod pages {
    pub use sunrise_hooks::pages::{Act, Label, Page, Place, Row, connect, register};
}

/// The volume of the audio mods make themselves (`sunrise_hooks::sound`): a crate's setting can turn
/// it down or off.
pub mod sound {
    pub use sunrise_hooks::sound::set_mod_stream_gain;
}

/// The lang files with a crate's text, loaded with the game's own (docs/layers.md, "Platform
/// registries").
pub mod lang {
    pub use sunrise_hooks::lang::{Files, register};
}

/// Rows a crate adds to the Mods screen (docs/layers.md, "Platform registries").
pub mod mod_rows {
    pub use sunrise_hooks::mod_rows::{Row, register};
}

/// Vanilla's own item catalogs, as the engine loaded them (docs/layers.md, "Item catalog").
pub mod items {
    pub use sunrise_hooks::items::creative_order;
}

/// Proxy selection and connection pooling for HTTP requests to caller-supplied URLs.
#[cfg(feature = "http-proxy")]
pub use sunrise_proxy as proxy;

/// The game's own release: [`release::VERSION`] is what its Mods screen row shows.
pub use sunrise_core::release;

/// Input and drawing on an open container screen, for the extras that act on one (docs/layers.md,
/// "Container screens"); offered through [`hooks::Hooks::containers`].
pub mod containers {
    pub use sunrise_hooks::containers::{
        Button, Click, Container, ContainerHooks, Panel, Role, Slot, SlotId, Stack,
    };

    // Compiled with the tests: a crate on the platform alone implements every hook.
    #[cfg(test)]
    mod consumer {
        use super::{Button, Click, Container, ContainerHooks, SlotId};

        struct PickOnRelease;
        impl ContainerHooks for PickOnRelease {
            fn press(&self, _: &mut dyn Container, _: Option<SlotId>, b: Button) -> bool {
                b == Button::Middle
            }
            fn release(&self, c: &mut dyn Container, slot: Option<SlotId>, _: Button) -> bool {
                slot.is_some_and(|s| c.click(s, Click::QuickMove))
            }
            fn drag(&self, _: &mut dyn Container, _: Option<SlotId>, b: Button) -> bool {
                matches!(b, Button::Other(_))
            }
        }

        const _: &dyn ContainerHooks = &PickOnRelease;
    }
}

/// The buttons a crate adds to the title screen (docs/layers.md, "Platform registries").
pub mod title {
    pub use sunrise_hooks::title::{Button, Opens, Refused, Title, register};
}

/// Drawing on the HUD (docs/layers.md, "Platform registries").
pub mod hud {
    pub use sunrise_hooks::hud::{Canvas, Corner, Layer, Stage, register};
}

/// What the engine draws differently, per player; not [`hooks::Look`] (players::Look).
pub mod look {
    pub use sunrise_hooks::look::{Chat, Look, update};
}

/// Build-time mod source generation.
#[cfg(feature = "mod-build")]
pub mod mod_build {
    /// The shared mod API and source generator.
    pub use sunrise_mod_api_gen as api_gen;
}
