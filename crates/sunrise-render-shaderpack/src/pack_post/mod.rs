//! A shader pack's post-processing over Sunrise's own world (spec 2.1 rows 4, 6, 8, 9; 6.1; 6.2;
//! 6.4): the world renders into its own target as with a resolution scale, its colour becomes
//! colortex0 and its depth `depthtex0`, then the pack's `prepare`, `deferred`, `composite` and
//! `final` programs run in order, and `final` draws onto the screen under the hand and HUD.
//!
//! Every colortex buffer a program reads or writes has two textures, A and B. Programs read
//! "main" and write "alt", and each buffer a program wrote flips afterwards (6.1.6). Every
//! frame starts with A as "main".
//!
//! The crate split by what each part does:
//
// - `types`: the pack's entries and exits (what it is handed, what it makes, what it refuses)
//   and the puzzle of passes, buffers and bindings they are built from.
// - `construction`: `PackPost` itself, `PackPost::new` and the pieces it builds them with.
// - `execution`: the frame's work on a `PackPost` (its passes, flips, resize, shadow pass and
//   `run`), as inherent methods taking the decoupled `UniformFrameView`.

pub mod construction;
pub mod execution;
mod image_formats;
mod mod_link;
pub mod mod_pack;
mod packed;
pub mod types;

pub use image_formats::{image_binding, image_format, rebinds, sampled_image, storage_formats_of};

pub use construction::*;
pub use execution::*;
pub use mod_pack::*;
pub use types::*;

#[cfg(test)]
mod tests;
