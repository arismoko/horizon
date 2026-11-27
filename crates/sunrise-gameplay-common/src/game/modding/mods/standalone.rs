//! Client-only standalone model registration.
use super::{ExecutionRole, RegistrationContext};
use mod_api::{
    __private::game::{RegistrationContextNew, StandaloneModelRegistration},
    Answered,
};

pub fn collect_standalone_models(
    role: ExecutionRole,
) -> Option<Vec<Answered<StandaloneModelRegistration>>> {
    if !role.is_client() {
        return None;
    }
    let mut registrations = Vec::new();
    crate::game::native_hooks::native_standalone_models(
        &RegistrationContext::new(),
        &mut registrations,
    );
    Some(registrations)
}
