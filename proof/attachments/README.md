# Attachment proof

Bundle `proof/attachments` is API-only. Build it through the launcher against its
pinned Horizon revision, with no injections. Run in a fresh world on the normal
server thread, then repeat on a dedicated server. Capture the server stderr lines
starting `ATTACH_PROOF` and the world NBT before and after reopening.

1. Place a chest in a loaded chunk and right-click it. The hook cancels the chest
   interaction and logs the *previous* counters. Expect level/chunk/block/memory
   and both player counters `Ok(0)`.
2. Summon a persistent cow; right-click it once. Expect `entity=... kind=minecraft:cow
   saved=Ok(0)`. Do not kill or replace it. Record chest coordinates and cow.
3. Save, quit completely, relaunch the same world and repeat the chest click and
   cow interaction. Expect level/chunk/block/entity `Ok(1)` and memory `Ok(0)`.
   Player keep/drop should both be `Ok(1)` if the same player identity rejoins.
4. Kill the player and respawn, then click the same chest. Expect `player_keep`
   `Ok(2)` but `player_drop` `Ok(0)`; the other saved holder counters retain their
   previous values. Test this with the same player on the dedicated server too.
5. With the attachment mod absent for one save/reopen, reinstall it and verify
   counters survive. In addition run the separate horizon-machine-sample mixin
   bundle and verify four cobblestone become four stone.

The block hook cancels chest UI, so the chest stays in place. Use a controlled
flat world if possible; keep the cow persistent and avoid chunk unload until the
save. For the no-mod test the launcher must relink without the bundle and then
relink with it again. Preserve the exact logs and NBT reads for review.
