#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay::game::mods::tests::Fixture;
    use sunrise_gameplay::game::mods::tests::smod;

    pub(crate) mod inventory {
        include!("mod_inventory_tests_server.rs");
    }
}
