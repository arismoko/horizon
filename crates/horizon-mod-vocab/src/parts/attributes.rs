use super::*;
use crate::Sentiment;

/// An attribute type a mod declares, as the game reads it when it grows the
/// attribute catalog (`Catalog::with_attributes`, ).
#[derive(Clone, Debug)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct AttributeDeclaration {
    /// The declaring mod's id: the first key of the catalog's sort order.
    pub mod_id: String,
    /// The attribute's namespaced id (`mymod:mana`).
    pub id: String,
    /// The attribute's default value, inside `[min, max]`.
    pub default: f64,
    /// The attribute's lower bound.
    pub min: f64,
    /// The attribute's upper bound.
    pub max: f64,
    /// Whether the attribute joins the sync set.
    pub syncable: bool,
    /// The tooltip sentiment of a rising or falling value.
    pub sentiment: Sentiment,
}

/// The attribute types mods declared.
pub fn attribute_declarations() -> &'static [AttributeDeclaration] {
    installed().map_or(&[], |installed| &installed.attributes)
}

/// Registration of new attribute types ([`Registrar::declare_attribute`](AttributesRegistrar::declare_attribute)).
pub trait AttributesRegistrar {
    /// Declare an attribute type of the mod's own (vanilla's
    /// `RangedAttribute` in `BuiltInRegistries.ATTRIBUTE`, NeoForge's
    /// `RegisterEvent`): `path` in the mod's namespace (`mana` of `mymod` is
    /// `mymod:mana`), its `default` and `[min, max]` bounds, whether it
    /// joins the attribute sync (`Attribute.setSyncable`), and the sentiment
    /// that colors its tooltip lines (`Attribute.setSentiment`, POSITIVE by
    /// default; the translation key is `attribute.name.<ns>.<path>`).
    ///
    /// Only both-side packages may declare attributes; other sides refuse because
    /// declarations assign network registry IDs.
    /// The declarations join the attribute catalog once, at load, appended
    /// after every vanilla attribute **sorted by declaring mod id, then by
    /// attribute id** — the order is the network id's, so every side running
    /// the same mods builds the same numbers and vanilla ids never shift —
    /// before entity attribute rows resolve, so a mod entity's declared set
    /// may name the mod's own new attribute. Sanitizing a value (NaN → min,
    /// else clamp) is the definition's own `sanitize`.
    ///
    /// An id outside the mod's manifest-owned namespaces (a vanilla attribute's included),
    /// a malformed id, a duplicate — of this mod's or another's, caught
    /// where the declarations join the catalog — a non-finite default or
    /// bound, or bounds the default is not inside (`min > max` included)
    /// refuses, naming the mod.
    fn declare_attribute(
        &mut self,
        path: &str,
        default: f64,
        min: f64,
        max: f64,
        syncable: bool,
        sentiment: Sentiment,
    ) -> Result<(), crate::AttributeRegistrationError>;
}
