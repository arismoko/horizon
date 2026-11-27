#[cfg(test)]
#[path = "interaction_event_tests_server.rs"]
pub(crate) mod interaction_event_tests;

#[cfg(test)]
#[path = "passage_cross_tests_server.rs"]
pub(crate) mod passage_cross_tests;

#[cfg(test)]
#[path = "riptide_tests_server.rs"]
pub(crate) mod riptide_tests;

#[cfg(all(test, feature = "client"))]
pub(crate) mod tests {
    pub(crate) use crate::game::remote::tests::connect_menu_test;
    use sunrise_items::items::ItemStack;

    pub(crate) mod click_ending_tests {
        include!("click_ending_tests_server.rs");
    }

    pub(crate) mod dig_abort_tests {
        include!("dig_abort_tests_server.rs");
    }
}

#[cfg(test)]
#[path = "movement_input_tests_server.rs"]
pub(crate) mod movement_input_tests;

#[cfg(test)]
#[path = "placement_prediction_tests_server.rs"]
pub(crate) mod placement_prediction_tests;
#[path = "recipe_properties_server.rs"]
pub(crate) mod recipe_properties;

#[path = "general_server.rs"]
pub(crate) mod general;
