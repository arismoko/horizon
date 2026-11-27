//! A player's mod-owned advancement triggers and their scoped predicate context.

use crate::{ItemStack, NoBackend};

/// One open criterion being tested during [`PlayerRef::trigger_criterion`](crate::PlayerRef::trigger_criterion).
///
/// The game lends this resource only for [`CriterionTest::test`]. Its private world context is
/// the target player's own level; a mod cannot retain the resource after the test returns.
/// The same native implementation reads conditions and evaluates item predicates for both
/// build targets.
pub trait CriterionContext {
    /// Borrow the criterion's parsed `conditions` (`Value::Null` when absent). The game's test
    /// of its `player` predicate follows the mod's test, as in vanilla's `SimpleInstance.player`.
    fn conditions(&self) -> &serde_json::Value;

    /// Whether `stack` passes the item predicate under `key` in the conditions, as the game's
    /// own triggers read an `ItemPredicate`: `items` (an id, a list or a `#tag`), `count`, the
    /// `potion_contents` and `damage` components, and the `enchantments` and `damage`
    /// predicates; other components and predicates aren't checked. A missing key passes, as
    /// an absent `Optional<ItemPredicate>` does. Tag lookups use the target player's world.
    /// An invalid stack is a test error, not a predicate mismatch.
    fn item(&self, key: &str, stack: &ItemStack) -> Result<bool, CriterionTestError>;
}

/// A predicate borrowed by [`PlayerRef::trigger_criterion`](crate::PlayerRef::trigger_criterion).
///
/// The game calls it synchronously for each open criterion, before testing the criterion's
/// `player` predicate. It keeps neither the test nor its context after the trigger returns.
/// The predicate may borrow data from the triggering event and need not be thread-safe.
pub trait CriterionTest {
    /// Whether the mod's part of this criterion passes. A callback that cannot run refuses
    /// with a typed error, rather than silently answering that the criterion did not match.
    fn test(&mut self, criterion: &dyn CriterionContext) -> Result<bool, CriterionTestError>;
}

/// Why the mod's criterion test could not answer.
// no wire: returned directly from the synchronous native predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CriterionTestError {
    /// The test was unable to evaluate the criterion; includes the mod's diagnostic.
    Refused(String),
    /// An item predicate was given an invalid API stack.
    Stack(crate::StackWriteError),
}

/// Why a player's advancement trigger could not complete.
// no wire: returned directly to the native caller on the server thread.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TriggerCriterionError {
    /// The trigger identifier has an empty namespace or path.
    InvalidTrigger,
    /// This exact trigger belongs to the game and uses its own event-specific matcher.
    BuiltinTrigger,
    /// This game is a remote client's copy, not the authoritative server.
    NotServer,
    /// The player handle is stale, offline, displaced by a nested turn, or their level could not be entered.
    PlayerUnavailable,
    /// The target level could not provide the event's block state for a block-use trigger.
    BlockUnavailable,
    /// The generated item boundary refused a supplied stack before firing a vanilla trigger.
    Stack(crate::StackWriteError),
    /// A criterion test could not answer. Earlier matching criteria may already be granted.
    Test(CriterionTestError),
    /// The borrowed test panicked. The game logged it and queued an error toast. No further
    /// tests run in this invocation; subsequent invocations may supply another test.
    TestPanicked,
    /// The host has no game backend (for example, a mod's test double).
    NoBackend,
}

impl From<crate::StackWriteError> for TriggerCriterionError {
    fn from(error: crate::StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl NoBackend for TriggerCriterionError {
    fn no_backend() -> Self {
        Self::NoBackend
    }

    fn wrong_side() -> Self {
        Self::NotServer
    }
}

/// Run one borrowed criterion predicate with the current mod's panic attribution.
// hidden: engine-side attributed invocation helper for criterion predicates.
#[doc(hidden)]
pub fn run_criterion_test(
    trigger: &str,
    test: &mut dyn CriterionTest,
    context: &dyn CriterionContext,
) -> Result<bool, TriggerCriterionError> {
    const OPERATION: &str = "trigger_criterion";
    let owner = crate::content::crash::current_native_mod().unwrap_or("unknown");
    let _guard = crate::mark_mod(owner, OPERATION);
    match crate::runtime::contain_callback(
        || test.test(context),
        |payload| {
            crate::runtime::report_borrowed_callback_panic(OPERATION, owner, trigger, payload)
        },
    ) {
        Some(Ok(matched)) => Ok(matched),
        Some(Err(error)) => {
            log::warn!("mod {owner}: criterion {trigger} test refused: {error:?}");
            Err(TriggerCriterionError::Test(error))
        }
        None => Err(TriggerCriterionError::TestPanicked),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityRef, Host, Identifier, PlayerRef, ServerHost, WorldEpoch};

    #[test]
    fn a_panicking_matcher_does_not_disable_the_next_matcher_for_the_same_trigger() {
        struct Context;
        impl CriterionContext for Context {
            fn conditions(&self) -> &serde_json::Value {
                &serde_json::Value::Null
            }
            fn item(&self, _: &str, _: &ItemStack) -> Result<bool, CriterionTestError> {
                unreachable!("this predicate tests error propagation, not items")
            }
        }
        struct Test<'a>(&'a mut usize);
        impl CriterionTest for Test<'_> {
            fn test(&mut self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                *self.0 += 1;
                match *self.0 {
                    1 => Err(CriterionTestError::Refused("missing input".into())),
                    2 => panic!("test predicate failed"),
                    _ => Ok(true),
                }
            }
        }

        crate::testing::with_dispatch(Vec::new().into(), || {
            let mut calls = 0;
            let mut test = Test(&mut calls);
            let _guard = crate::mark_mod("p3-81-criteria", "testing");
            assert_eq!(
                run_criterion_test("test:first", &mut test, &Context),
                Err(TriggerCriterionError::Test(CriterionTestError::Refused(
                    "missing input".into()
                )))
            );
            assert_eq!(
                run_criterion_test("test:first", &mut test, &Context),
                Err(TriggerCriterionError::TestPanicked)
            );
            assert_eq!(calls, 2);
            assert_eq!(
                crate::runtime::take_hook_panics(),
                [crate::__private::HookPanicReport {
                    mod_id: "p3-81-criteria",
                    hook: "trigger_criterion",
                }]
            );
            struct Healthy;
            impl CriterionTest for Healthy {
                fn test(&mut self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                    Ok(true)
                }
            }
            assert_eq!(
                run_criterion_test("test:first", &mut Healthy, &Context),
                Ok(true)
            );
        });
    }

    #[test]
    fn a_trigger_without_a_game_refuses_without_testing() {
        struct TestHost;
        impl Host for TestHost {}
        mod_api::impl_host_areas!(TestHost);
        impl ServerHost for TestHost {}
        mod_api::impl_server_host_areas!(TestHost);
        struct Never;
        impl CriterionTest for Never {
            fn test(&mut self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                panic!("a refused trigger must not call the mod's predicate")
            }
        }
        let player = PlayerRef {
            entity: EntityRef::new(WorldEpoch::new(1), 1),
            uuid: 1,
        };
        assert_eq!(
            player.trigger_criterion(
                &mut TestHost,
                &Identifier::parse("test:probe").unwrap(),
                &mut Never,
            ),
            Err(TriggerCriterionError::NoBackend)
        );
    }
}
