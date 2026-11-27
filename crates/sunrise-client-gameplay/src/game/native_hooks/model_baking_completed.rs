#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ModelEvent.BakingCompleted`: the resources' models are baked and published, the
    /// standalone ones included, so `model` resolves them in the new store. Once per successful
    /// model load, after both atlases' `texture_atlas_stitched`: on the first load before
    /// `client_setup`, after a successful reload before `client_resources_reloaded`.
    pub(in crate::game) fn native_model_baking_completed(&mut self) {
        mod_api::dispatch_model_baking_completed(|call| {
            self.storage_scope(|game, _| {
                let commit = call(game);
                ((), commit)
            });
        });
    }
}
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
