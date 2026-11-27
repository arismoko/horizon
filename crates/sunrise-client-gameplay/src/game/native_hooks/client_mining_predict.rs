use mod_api::ClientMiningPrediction;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn native_client_mining_predict(
        &self,
        prediction: ClientMiningPrediction,
        canceled: &mut bool,
    ) {
        self.fire_client_mining_predict(prediction, canceled);
    }
}
