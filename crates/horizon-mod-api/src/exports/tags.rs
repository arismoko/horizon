mod tags;
pub use tags::{TagKey, Tags};
mod __private {
    pub use crate::tags::{tag_key, with_tag_name};
}
