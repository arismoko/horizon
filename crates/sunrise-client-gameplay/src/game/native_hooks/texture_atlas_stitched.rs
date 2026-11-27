#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    #[cfg(test)]
    pub(in crate::game) fn texture_atlases_stitched(&mut self) {
        for atlas in [mod_api::TextureAtlas::Blocks, mod_api::TextureAtlas::Gui] {
            self.native_texture_atlas_stitched(atlas);
        }
    }

    /// Atlas `atlas` was stitched and published: its sprites can be looked up.
    pub(in crate::game) fn native_texture_atlas_stitched(&mut self, atlas: mod_api::TextureAtlas) {
        mod_api::dispatch_texture_atlas_stitched(
            |call| {
                self.storage_scope(|game, _| {
                    let commit = call(game);
                    ((), commit)
                });
            },
            atlas,
        );
    }
}
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
