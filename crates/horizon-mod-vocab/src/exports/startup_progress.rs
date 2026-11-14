pub use world::startup_progress::{StartupProgress, StartupScreen};
mod __private {
    pub mod game {
        pub use crate::world::startup_progress::install_startup_screen;
    }
}
