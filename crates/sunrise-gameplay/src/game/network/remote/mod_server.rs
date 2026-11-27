#[cfg(any())]
use crate::game::client_side::remote_client::RemoteInputWorld;
#[cfg(any())]
use crate::game::client_side::remote_client::RemoteUseWorld;
use std::sync::Arc;
#[allow(unused_imports)]
use sunrise_core::geometry::Vec3;
#[allow(unused_imports)]
use sunrise_items::items::ItemStack;
#[allow(unused_imports)]
use sunrise_protocol::packets::{
    Empty,
    play::{self, Clientbound as C, Serverbound as S},
};
#[allow(unused_imports)]
use sunrise_sim::first_person_sim::Hand;
#[allow(unused_imports)]
use sunrise_sim::{
    motion::{MotionWorld, PlayerMotion},
    save::SaveDir,
    sprint::Keys,
};

#[path = "general_server.rs"]
pub(in crate::game) mod general;

#[path = "recipe_properties_server.rs"]
pub(in crate::game) mod recipe_properties;

#[path = "inventory_server.rs"]
pub(in crate::game) mod inventory;

#[path = "session_server.rs"]
pub(in crate::game) mod session;
