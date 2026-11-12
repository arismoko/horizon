//! Self-refusal transport and the registration admission savepoint.

/// Engine admission plumbing; mods call `RegistrationContext::refuse` instead.
// hidden: the generated registrar call table transports self-refusals.
#[doc(hidden)]
pub trait RegistrationRefusalRegistrar {
    /// Deliver the current mod's refusal, without accepting an owner argument.
    fn registration_refusal(&mut self, reason: String);
}

/// Engine transport plumbing for the generated guest dispatch arm.
// hidden: drains the guest-local context after the registration callback.
#[doc(hidden)]
pub fn take_registration_refusal(cx: &crate::RegistrationContext) -> Option<String> {
    cx.self_refusal
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use crate::RegistrationContext;
    #[test]
    fn bounded_reasons_preserve_utf8_and_empty_is_a_refusal() {
        for (reason, expected) in [
            ("a".repeat(4095) + "é!", "a".repeat(4095)),
            ("é".repeat(2048), "é".repeat(2048)),
            (String::new(), String::new()),
        ] {
            let cx = RegistrationContext::new();
            cx.refuse(&reason);
            cx.refuse("not the first reason");
            assert_eq!(*cx.self_refusal.lock().unwrap(), Some(expected));
        }
    }
}
