# Upstream sync 29 decisions

Upstream pin: `c94cdb0c76d2e06e13240eee7d9e1f807977318f`. Initial fork main: `4a9dce6c2dbf0b532266fe850db334d098951c1f`. Common ancestor: sync 28's upstream `d9b9996a4dfc61d0316daf599d7f937ccb8b95a8`.

## Boundaries

Preserve typed mod hooks and resident/JoinList state rather than adding direct upstream lookup paths. Incoming vanilla behavior is ported to its compiled typed owner; retained tests are compared with that owner before the first push. Main's `.github/` and `tools/ci/` remain authoritative and unchanged by this sync.

Diverges from upstream: parallel mob dispatch stays disabled through `parallel.rs::parallel_tick` and `FORK_PARALLEL_TICK_ENABLED = false`. Upstream mob_group, mob_islands and prep stay byte-identical under the nondefault `upstream-parallel-mobs` source-retention feature; no all-features checks. The parallel integration follow-up owns changing that decision.

## Registry documentation

Keep the fork's modding-book relocation and the API-page link rather than reviving the old full page with obsolete native-mod formats. Carry upstream's new optional `updated` publishing metadata and example into the canonical book reference. Existing package format and locally built native-mod contracts remain authoritative.

## Verification

Pending area resolution, full one-side-conflict/test/omitted-file semantic sweep, fresh main integration, rustfmt and zero-warning compile-only workspace/all-target check. No local tests or clippy. HDR's ignored near-black dither failure remains unresolved from sync 28 until the next slow-shard measured evidence is available; no shader, tolerance or gate change is made speculatively.

## Typed behavior integrations

- Freeze immunity uses the active new `mobs/ai/freezing.rs` extraction with typed equipment access; skeleton and body-carrying sulfur-cube behavior is retained.
- Keep the inline typed effect-instance owner; the incoming uncompiled `game/creatures/effect_instance.rs` is behavior-equivalent and removed after comparison, not merely because it lacked a declaration.
- Fox consumption uses shared typed effect application and the existing host teleport transaction, with emission before effects and eat/drink events after effects at the current position. StoredEffect/effect-chain handling and typed damage remain authoritative.
- Sulfur portal-proof blast state is passed through the source-aware explosion argument; it does not become ambient state for later unrelated blasts.
- Books/sign/wind tests are relocated to compiled owners under `game/interface/tests/`; obsolete root consume/item_use/passengers/use_hand files stay deleted after behavioral comparison with active typed owners.
- Preserve original trap-horse LegacyRandom scatter draws after successful rider creation/mounting. No world stream replaces them.
- Retain vanilla byte narrowing before shulker AttachFace remainder.
- Use the shared animation decoder and typed consumable owner; direct SoundEvent `{sound_id}` form is accepted by typed parsing, avoiding a new raw-helper/typed-owner disagreement.
- Loot set-count saturation and minimum-one uniform bonus bound are taken. Deliberately retain the fork's Java wrapping for enchanted count growth and typed attacker-level lookup instead of upstream contextless services. Both conditional-load and incoming enchantment regression coverage remain.

## Historical boundaries

Unknown Allay player identity intentionally does not match a verified liked-player UUID. Do not restore upstream's contextless legacy-attribution test/API; the typed refusal/matching coverage remains. Historical horse armor tests split equipment/drop and effective live-attribute coverage rather than invoking the removed worn_armor helper. Two old mount_rolls helper tests remain absent because the fork uses typed SpawnSite/finalization; that broad historical natural/non-natural mount coverage is not claimed newly proven by this sync.

## Integration review corrections

An independent static review found fox admission still using prototype-only food/consumable state and PlaySound effects being dropped by the shared death-protection effect helper. Both standalone fox and the Game transaction now use the effective component owner and preserve ordered explicit level-style sound effects. Missing/removed consumables cannot decrement the held item. Food listener emission is effective-food gated. Focused existing-module regressions cover removal/addition admission and ordered block-centered sounds for a silent fox; not run locally.

The new raw/generic consumable extraction helpers were removed instead of adding a second decoding implementation. Their incoming test names and field/default/removal assertions now exercise ItemStack::consumable directly, with added replacement-effects coverage. Setting an explicit removal marker can legitimately coexist with the cached positive value through public StackData::set_other; effective selection therefore checks removal first. Port ledger entries point at the actual typed field decoders.

Supplemental historical-test sweeps restored two orb fixtures' actual NBT-loading setup and lightning conversion persistence assertions. These repairs retain the fork's stronger Air round-trip checks; they do not replace load coverage with literal construction.

Completed compile-only `cargo check --workspace --all-targets` with zero warnings before the first push. Touched Rust was formatted; no tests, clippy, harness or game runs were performed. Initial errors were private animation paths, the incoming warden RNG context and stale typed fixture fields/arguments; the complete all-target pass now compiles them. Independent static consumption review has no remaining blocker after three rounds.

## CI repair round

The complete first-head verify log had one failing test: `game::lightning::tests::a_baby_pig_hit_by_lightning_becomes_a_baby_zombified_piglin`, at its restored persistence assertion. The adaptation called generic `conversion::single`, which intentionally only copies persistence, omitting the species-specific `convert_to` completion that already makes lightning-converted pigs persistent. Share that existing weapon/persistence completion with the fixture as `conversion::pig_thunder_hit`; retain every age, yaw and persistence assertion and preserve production RNG order. This is a fixture adaptation correction, not a newly introduced runtime lightning defect.

Also repair the fox needless borrow and two projectile comment/documentation errors. The precision-sensitive HDR endpoint dither test now returns early for `DeviceType::Cpu` with an explicit skip message; neither shader nor tolerance changes. No local compile, clippy or tests for this CI repair; touched Rust formatted.

## Rollup 48 main refresh

Merged main `a6fd1291e15b7cf889eb310deb79492ded2ad267` once after the first CI repair. The merge was clean; the prior head gate was still running, not all green. Rustfmt on the two auto-merged Rust files made no changes. Compile-only `cargo check --workspace --all-targets` passed with zero warnings on the integrated tree. Main CI files remain unchanged; the size baseline was refreshed against the new main. No tests or clippy ran locally.
