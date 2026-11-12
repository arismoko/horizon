//! Cold registration construction, with native representation kept private.
/// Construct or inspect the owned registration value.
pub fn game_rule_declaration(name: String, category: String, default: bool) -> crate::GameRuleDecl {
    crate::GameRuleDecl {
        name,
        category,
        default,
    }
}
