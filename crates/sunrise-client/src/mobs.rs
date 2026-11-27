pub use sunrise_sim::mobs::*;

// BEGIN CLIENT-ONLY MOBS BLOCK (stripped into sunrise-client/src/mobs.rs shim in M6)
pub mod cem;
pub mod cem_anim;
pub mod model;
pub mod model_layers;
pub mod random_textures;
pub mod render;
pub mod skin;
pub mod villager_hats;
pub use crate::mobs_render::{
    EntityModelDocument, load_model, render_model, render_model_filtered,
    render_single_mob as render,
};
pub use model::{ModelNormals, Part, Polygon};
// END CLIENT-ONLY MOBS BLOCK
