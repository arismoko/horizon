/// `Util.parseAndValidateUntrustedUri`: an absolute `http`/`https` URI
/// (java.net.URI's scheme then the allowlist; whitespace-free).
pub fn valid_untrusted_uri(uri: &str) -> bool {
    let Some(colon) = uri.find(':') else {
        return false;
    };
    let scheme = &uri[..colon];
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    !uri.chars().any(|c| c.is_control() || c == ' ')
}
