//! Parallel tick mode setting (`graphics::ParallelTickMode`), shared by server and client.

/// Parallel Tick's setting, for the worlds this game runs itself (single player, open to LAN):
/// whether the server tick's phases that give the same result in any order share their work
/// among helper threads (`parallel`). Auto shares it only while the tick is heavy and sharing
/// makes it faster; a quiet world stays on one thread and starts no helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParallelTickMode {
    #[default]
    Off = 0,
    Auto = 1,
    On = 2,
}
impl ParallelTickMode {
    pub const ALL: [ParallelTickMode; 3] = [
        ParallelTickMode::Off,
        ParallelTickMode::Auto,
        ParallelTickMode::On,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::On => "on",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Auto => "Auto",
            Self::On => "On",
        }
    }
    pub fn parse(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|m| m.name() == s)
            .unwrap_or_default()
    }
}
