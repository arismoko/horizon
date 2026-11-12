//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl AttributesRegistrar for PartRegistrar {
    fn declare_attribute(
        &mut self,
        path: &str,
        default: f64,
        min: f64,
        max: f64,
        syncable: bool,
        sentiment: Sentiment,
    ) -> Result<(), crate::AttributeRegistrationError> {
        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return Err(crate::AttributeRegistrationError::InvalidId);
            }
        };
        let result = (|| {
            if mod_vocab::__private::parts_installation::loaded_mods()
                .get(self.mod_id)
                .is_some_and(|package| package.side() != crate::Side::Both)
            {
                return Err(crate::AttributeRegistrationError::WrongSide);
            }
            if self.attributes.iter().any(|declared| declared.id == id) {
                return Err(crate::AttributeRegistrationError::Duplicate);
            }
            if ![default, min, max].into_iter().all(f64::is_finite) {
                return Err(crate::AttributeRegistrationError::NotFinite);
            }
            if min > max || default < min || default > max {
                return Err(crate::AttributeRegistrationError::InvalidBounds);
            }
            self.attributes.push(AttributeDeclaration {
                mod_id: self.mod_id.to_owned(),
                id,
                default,
                min,
                max,
                syncable,
                sentiment,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} attribute {path:?}: {error:?}", self.mod_id));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AttributeRegistrationError;

    /// `RangedAttribute`'s own checks and the registration's: an id outside
    /// the mod's namespace (a vanilla attribute's included), a duplicate
    /// across mods, non-finite values and bounds the default is not inside
    /// all refuse, naming the mod; the good declaration stands.
    #[test]
    pub(crate) fn attribute_declarations_validate() {
        let mut registrar = PartRegistrar::new("mymod");
        registrar
            .declare_attribute("mana", 10.0, 0.0, 100.0, true, Sentiment::Neutral)
            .unwrap();
        // The path stays in the declaring mod's namespace.
        assert_eq!(
            registrar.declare_attribute("minecraft:armor", 0.0, 0.0, 30.0, true, positive()),
            Err(AttributeRegistrationError::InvalidId)
        );
        assert_eq!(
            registrar.declare_attribute("foreign:mana", 0.0, 0.0, 1.0, false, positive()),
            Err(AttributeRegistrationError::InvalidId)
        );
        assert_eq!(
            registrar.declare_attribute("Not An Id", 0.0, 0.0, 1.0, false, positive()),
            Err(AttributeRegistrationError::InvalidId)
        );
        // The same attribute twice, whatever declared it, is a duplicate.
        assert_eq!(
            registrar.declare_attribute("mana", 1.0, 0.0, 1.0, false, positive()),
            Err(AttributeRegistrationError::Duplicate)
        );
        // RangedAttribute's own bounds checks.
        assert_eq!(
            registrar.declare_attribute("rage", 5.0, 10.0, 0.0, false, positive()),
            Err(AttributeRegistrationError::InvalidBounds)
        );
        assert_eq!(
            registrar.declare_attribute("rage", 0.0, 1.0, 10.0, false, positive()),
            Err(AttributeRegistrationError::InvalidBounds)
        );
        assert_eq!(
            registrar.declare_attribute("rage", 20.0, 1.0, 10.0, false, positive()),
            Err(AttributeRegistrationError::InvalidBounds)
        );
        // A non-finite default or bound.
        assert_eq!(
            registrar.declare_attribute("rage", f64::NAN, 0.0, 1.0, false, positive()),
            Err(AttributeRegistrationError::NotFinite)
        );
        assert_eq!(
            registrar.declare_attribute("rage", 0.5, f64::NEG_INFINITY, 1.0, false, positive()),
            Err(AttributeRegistrationError::NotFinite)
        );
        assert_eq!(
            registrar.declare_attribute("rage", 0.5, 0.0, f64::INFINITY, false, positive()),
            Err(AttributeRegistrationError::NotFinite)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.attributes.len(), 1);
        let declared = &registrar.attributes[0];
        assert_eq!(
            (declared.mod_id.as_str(), declared.id.as_str()),
            ("mymod", "mymod:mana")
        );
        assert_eq!(
            (declared.default, declared.min, declared.max),
            (10.0, 0.0, 100.0)
        );
        assert!(declared.syncable);
        assert_eq!(declared.sentiment, Sentiment::Neutral);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod mymod"))
        );
    }

    #[test]
    fn attribute_sentiment_wire_round_trip() {
        use crate::wire::{Reader, Wire, Writer};
        for sentiment in [Sentiment::Positive, Sentiment::Negative, Sentiment::Neutral] {
            let mut writer = Writer::new();
            sentiment.put(&mut writer);
            let bytes = writer.into_bytes();
            assert_eq!(
                Sentiment::take(&mut Reader::new(&bytes)).unwrap(),
                sentiment
            );
        }
    }

    fn positive() -> Sentiment {
        Sentiment::default()
    }
}
