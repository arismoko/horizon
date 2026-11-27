# Implementation decisions

Behaviours the spec ([shader-packs-spec.md](shader-packs-spec.md), revision 1) leaves open, each with the decision,
its reason and the tests that hold it ([README](README.md), "Decisions on record"). An entry is never taken from another
implementation, its code, its issue tracker or its changelog. Decisions taken before this file existed (on the spec's
open questions, OQ-25, OQ-26 and others) are in the implementation plan, which was not published, so this file is not
the full list.

## D-1: which bolt `lightningBoltPosition` gives

- **Open point:** spec rev 1, 5.2, line 692 gives "player-space position of one rendered lightning bolt, w = 1; zero
  if none", without saying which bolt when several are drawn. It is not one of section 9's open questions.
- **Decision:** the last bolt in Sunrise's draw list: the frame's bolts (`bolt_instances`), which are the ones within
  vanilla's draw distance (`LightningBolt.shouldRenderAtSqrDistance`; `LightningBoltRenderer.affectedByCulling` is
  false), in the order the game keeps them (a bolt added later goes last). Its offset from the frame's `cameraPosition`
  (player space, spec line 607), subtracted in double precision by the same function that places the bolt's geometry
  (`gfx::lightning::offset`); zeros when the list is empty.
- **Reason:** the spec asks for one rendered bolt, and the draw list is what Sunrise renders, so the uniform and the
  drawn geometry agree. The order bolts are submitted in doesn't decide whose pixels end up on screen last, so this is
  not a claim about which flash the player sees last; the uniform only follows the existing list. Which bolts are
  visible and how they sort is separate rendering work. A bolt never moves, so its render position is where it
  struck.
- **Tests:** `gfx::pack_uniforms::lightning_bolt_test::the_last_bolt_in_the_draw_list_reaches_the_pack_in_player_space`
  (`sunrise-client-render`) and the daily `game::lightning_bolt_uniform_tests::a_summoned_bolt_reaches_the_pack_in_player_space`
  (`sunrise-client`).

## D-2: which player booleans programs read, from which vanilla method, and their type

- **Open point:** spec rev 1, 3.4, line 552 lists fourteen player booleans (`is_alive` … `is_wet`) for expressions,
  "state of the player (the camera entity)", and adds "`[IR]` also exposes six of them as uniforms". The uniform
  table of 5.2 (line 664) lists only those six (`is_burning is_hurt is_invisible is_on_ground is_sneaking
  is_sprinting`). The bullet at line 558 ("the player booleans as normal uniforms too") can be read as all fourteen.
  The spec names no vanilla method for any of them, and no type beyond "bool". None of this is one of section 9's
  open questions.
- **Decision 1, expression-only:** custom uniforms (`uniform.<type>.<name> = <expression>`) read all fourteen. A
  program that declares one of the eight others (`is_alive is_child is_glowing is_in_lava is_in_water is_ridden
  is_riding is_wet`) gets no built-in value: it reads 0, unless the pack defines a custom uniform of that name, which
  binds instead. Compute programs and quad programs follow the same rule. The basis is line 552's "six of them" and
  the six-name table at line 664, which agree; line 558 is the looser of the three and is read through them. The 3.4
  heading ("Parameters only for expressions (Tier A) and as uniforms (Tier E)") is no basis: `biome`, `temperature`
  and the rest sit under it and are program uniforms.
- **Decision 2, the vanilla method behind each name** (Minecraft 26.3, the window's player as a client answers;
  `LocalPlayer` and `AbstractClientPlayer` override none of them):
  - `is_alive`: `LivingEntity.isAlive` (`!isRemoved() && getHealth() > 0`; the window's player is never removed
    while it is the camera);
  - `is_child`: `LivingEntity.isBaby`, which `Player` keeps, so always 0;
  - `is_glowing`: `Entity.isCurrentlyGlowing` on a client, which skips the effect check and reads shared flag 6: the
    flag a server sends, or in single player the Glowing effect the flag is set from (a local `Glowing` tag is not
    modelled). A server's flag resets with a new `LocalPlayer` (a login, a respawn without `KEEP_ENTITY_DATA`), as
    the player's other shared flags do;
  - `is_in_lava`: `Entity.isInLava` (`!firstTick && isInFluid(LAVA)`);
  - `is_in_water`: `Entity.isInWater` (`wasTouchingWater`), not `isUnderWater`;
  - `is_ridden`: `Entity.isVehicle` (`!passengers.isEmpty()`), so a dying passenger counts until it is removed;
  - `is_riding`: `Entity.isPassenger` (`getVehicle() != null`);
  - `is_wet`: `Entity.isInWaterOrRain` (`isInWater() || isInRain()`, rain at `blockPosition` or at the box's top);
  - the six of the table keep the methods they already had.
- **Decision 3, bool only:** the eight are `bool` built-ins, like the six, so `uniform.float.x = is_wet` reads 0 as
  `uniform.float.x = is_sneaking` does. Only the six were probed; the eight are bool by inference from the spec's
  "bool" and from the six, not by measurement.
- **Not covered:** the spec's "the camera entity". While spectating another entity all fourteen read the window's
  player, as the six did before this entry.
- **Tests:** `gfx::pack_uniforms::initial_history_tests::custom_uniforms_read_every_player_boolean`,
  `gfx::pack_uniforms::initial_history_tests::programs_read_only_six_player_booleans` and
  `gfx::pack_post::player_boolean_tests::compute_and_quad_programs_read_only_the_listed_player_booleans`
  (`sunrise-client-render`); the daily `game::server_side::shader_packs_server::player_status_tests::pack_status_follows_the_player`,
  `game::server_side::shader_packs_server::player_status_tests::pack_status_leaves_queued_effects_alone`,
  `game::client_side::shader_packs::player_status_tests::pack_status_on_a_server_follows_what_it_sent` and
  `game::client_side::shader_packs::player_status_tests::pack_status_starts_over_with_a_new_local_player` (`sunrise-client`).

## D-3: an image whose format qualifier differs from its buffer's format

- **Open point:** spec rev 1, 5.2, lines 783-784 declare images as `layout(<format>) uniform image2D <name>;` over the
  buffer they name, and 6.7, lines 984-996 list the formats a buffer, raw texture or image may have. The spec doesn't
  say what happens when the qualifier names another format than the buffer's (`layout(rgba16)` on a buffer whose
  `colortex8Format` is `RGBA16F`), and it is not one of section 9's open questions. wgpu binds a storage image only
  in its texture's exact format, and the shader must declare that same format, so a differing qualifier is either
  replaced by the texture's format or its program is refused (#548).
- **Decision:** a 16-bit normalized qualifier (`rgba16`, `rg16`, `r16` and their `_snorm` forms) on the 16-bit float
  format of the same channels (`RGBA16F`, `RG16F`, `R16F`), which is the same size per texel, binds the texture's
  float format, and the program gets a note naming it, the image, the qualifier and the format bound. The same holds
  for the float format Sunrise makes of a 16-bit unorm or snorm buffer on a device without 16-bit normalized
  formats. Every other differing pair is refused, with the note naming the program, the image, the qualifier and
  the texture's format: another size per texel (`rgba8` on `RGBA16F`), other channels, an integer qualifier on a
  float or normalized texture or the reverse, unorm against snorm, and a float qualifier on a normalized texture.
  The `r11f_g11f_b10f` qualifier on Sunrise's `RGBA16F` stand-in for an `R11F_G11F_B10F` buffer or image keeps its
  earlier rule, which is the same in kind, and now gets the same note.
- **Semantics:** a rebound image is accessed as the texture's values, with the conversion of the texture's format,
  not the qualifier's. A store of 2.0 through `layout(rgba16)` stays 2.0 in an `RGBA16F` buffer, and a store of 0.5
  reads back as 0.5. So the same program can store differently by device: over an `RGBA16` buffer that the device
  holds as `RGBA16` (with `TEXTURE_FORMAT_16BIT_NORM`), the qualifier matches, and the store of 2.0 is clamped to 1.0;
  over the `RGBA16F` that Sunrise makes of that buffer without the feature, it stays 2.0. The note says when this
  happens. Sunrise does not reinterpret the texture's bits as the qualifier's format; doing so would leave the
  unorm16 bits of 0.5 (0x8000), which a half float reads as -0.0.
- **Reason:** the spec leaves the pair open, and wgpu binds a storage texture only in its own format, with the shader
  declaring that same format; so the program can keep its image only in the texture's format. The buffer's format is
  what the pack chose for its data. The shader reads and writes floating-point vectors through unorm, snorm and float
  formats alike, so taking the texture's format changes no expression of the program, only the conversion at the
  texture. The pairs are limited to equal texel sizes so that a qualifier is never taken over a buffer of another
  precision or channel count.
- **Tests:** `pack_post::tests::only_16_bit_normalized_qualifiers_bind_their_float_texture`,
  `pack_post::tests::a_rebound_image_declares_its_textures_format_alone` and
  `pack_post::tests::a_rebound_image_is_noted_and_an_exact_one_is_not` (`sunrise-render-shaderpack`); the daily
  `gfx::pack_post::normalized_image_tests::an_rgba16_image_of_a_float_buffer_keeps_the_floats_stored`,
  `gfx::pack_post::normalized_image_tests::an_rgba16_snorm_image_of_a_float_buffer_keeps_the_floats_a_quad_program_stores`,
  `gfx::pack_post::normalized_image_tests::an_rgba16f_image_of_a_float_buffer_binds_without_a_note` and
  `gfx::pack_post::normalized_image_tests::an_rgba8_image_of_a_float_buffer_still_refuses_its_program`
  (`sunrise-client-render`).

## D-4: native terrain quads of block entities and entities under a pack

- **Open point:** spec rev 1, 2.3 (line 294) gives `gbuffers_block` "block entities, sign text" and `gbuffers_entities`
  "entities", and 6.1.4 sends a program's outputs to the buffers its `RENDERTARGETS`/`DRAWBUFFERS` list names.
  Sunrise's mods can draw, inside a block entity's or an entity's run, terrain-sheet geometry that Sunrise draws
  natively from its sprite arrays: still fluids and terrain models (`FLUID_OVERLAY`, `TERRAIN_MODEL_*_OVERLAY`; a
  block entity's renderer, a mod's model copies, and a mod's entity renderer or layer all draw through the same
  context). The spec has no such geometry, so it does not say which program draws it, which texture it reads, which
  maps its `normals` and `specular` are, or in what order its translucent quads draw.
- **Decision:** the program of its run draws it, as the rest of that block entity or entity: `gbuffers_block` or
  `gbuffers_entities`, and for the translucent kinds (a still fluid, a translucent terrain model) that program's
  translucent one (`gbuffers_block_translucent`, `gbuffers_entities_translucent`) when the pack has it, with the run's
  `blockEntityId` or `entityId`. It samples the pack's block atlas, the one the pack's terrain samples (`pack_atlas`):
  each quad's UVs are placed in its sprite's cell of that atlas, clamped to the sprite per vertex as the pack's
  terrain clamps them. Its `normals` and `specular` read the stand-ins a program reads with no map bound: these
  programs read those maps as the shared atlas's paged maps, whose pages say nothing of the block atlas. A run's
  translucent quads, wherever they lie in it, are one draw after its other quads, far to near by the distance and tie
  order Sunrise's own draw sorts them with (`QuadSort`); separate runs stay separate draws, in the order they came.
  The pack's shadow program draws what the run's program draws and its shadow pass casts (an entity's translucent
  parts cast nothing, as its other translucent parts). A quad whose sprite the block atlas lacks (a layer past the
  atlas's table, or a fluid layer below 0, NaN or infinite), a pack without the run's program (even one with a shadow
  program: nothing casts the quad then, as before), or one without a block atlas leaves the quad to Sunrise.
- **Reason:** 2.3's programs for block entities and entities are the only ones the spec ties to this geometry, and
  6.1.4 then puts it in the buffers the pack's `final` reads; drawn natively, it reached only `colortex0`. The block
  atlas already holds every terrain sprite, as the pack's terrain reads them, so no texture is copied; the renderer
  has no table from a terrain sprite layer to its place in the shared atlas. Paged-map reads of the block atlas's UVs
  would return another texture's material. Sunrise's own draw sorts these translucent kinds on upload, across every
  run; the pack's draws sort each run, since one draw carries one `blockEntityId` or `entityId`.
- **Tests:** `pack_entities::material_route_tests::native_terrain_quads_take_the_runs_program_where_the_block_atlas_holds_them`
  and `pack_entities::material_route_tests::the_block_atlas_places_native_terrain_quads` (`sunrise-render-shaderpack`);
  the daily `pack_entities::gpu_span_tests::native_terrain_quads_draw_in_the_pack_programs_from_the_block_atlas`,
  `pack_entities::gpu_span_tests::a_runs_translucent_native_quads_draw_once_far_to_near` and
  `pack_entities::gpu_span_tests::an_entitys_native_terrain_quads_draw_in_the_entity_programs`
  (`sunrise-render-shaderpack`), and `pack_block_entity_runs` and
  `gfx::pack_post::block_atlas_group_tests::a_resize_drops_the_portal_and_block_atlas_groups` (`sunrise-client-render`).

## D-5: which faces a `layer.*` rule moves, and what wins

- **Open point:** spec rev 1, 4.2, lines 590-593 say `layer.solid`, `layer.cutout`, E `layer.cutout_mipped` and
  `layer.translucent` in `block.properties` "move blocks to another terrain layer", and that full opaque cubes cannot
  move `[OF-SH]` (followed as written: such a state keeps its layer, with a load note naming the rule and the states;
  the issue's own control moves one, so that conflict is asked in `questions/Q-001.md`). 4.1, lines 575-579 give the
  names, state filters and E `%<tag>` entries, and say that "individual entries win over tags when both match". The
  spec doesn't say which rule wins when several match one state, what `cutout_mipped` is where the terrain has one
  alpha-tested pass, whether a rule also moves the fluid a block holds, or whether it wins over the options that choose
  a block's layer (`cutoutLeaves`, `opaqueIce`). None of these is one of section 9's open questions.
- **Decision:**
  - A rule naming the block wins over a rule naming its tag; then the rule with the most state filters that all match;
    then the first in the file. This is the order 4.1 gives entries, and the one `block.<id>` lines already use.
  - `layer.cutout_mipped` is `layer.cutout`: the terrain has one alpha-tested pass, the mesher's solid quads without
    `OPAQUE_LAYER` (`Mesher::quad_as`).
  - An unknown `layer.<name>` and a word that is no block name are skipped with a load note naming them.
  - A rule moves the state's own model faces, with their connected-texture tiles and overlays, and wins over the
    face's own sprite and over `cutoutLeaves` and `opaqueIce`. Fluid faces (a liquid block, the water of a waterlogged
    block), the models a mod attaches to a block and a mod's section geometry keep their own layer.
  - Solid draws with no alpha test and no blending; cutout keeps the alpha test even where every texel is opaque;
    translucent blends and is drawn by `gbuffers_water` (and in the shadow pass by `shadow_water`).
  - Without a pack, with a pack without `layer.*` rules, and with rules that move no state of the game, every state
    keeps its own layer and the terrain meshes as without them.
- **Reason:** the precedence is the one 4.1 states and the id rules use, so one file reads one way. The options and
  the face's sprite are the game's own choices of a layer, which the pack's rule replaces. The fluid has a layer of its
  own, the fluid's, which no rule names.
- **Tests:** `block_ids::tests::layer_rules_move_named_states_and_tags` and `block_ids::tests::layer_rule_precedence`
  (`sunrise-shaders`); the daily `mesher::layer_override_tests::translucent_moves_only_the_named_state`,
  `mesher::layer_override_tests::cutout_clears_the_opaque_bit_of_the_named_state_only`,
  `mesher::layer_override_tests::rules_win_over_the_sprite_and_the_options` and
  `mesher::layer_override_tests::no_moved_state_meshes_to_the_byte`, and
  `mesher::quarter_tests::a_tile_over_a_moved_face_takes_its_layer` (`sunrise-client-render`);
  `game::pack_layers::tests::states_get_their_rules_layer_by_id`,
  `game::pack_layers::tests::full_opaque_cubes_keep_their_layer_with_a_note` and the daily
  `game::pack_layer_frame_tests::layer_rules_move_states_between_the_packs_terrain_programs` and
  `game::pack_layer_frame_tests::layer_rules_move_states_between_the_packs_shadow_programs` (`sunrise-client`).

## D-6: a sampler whose class differs from its image's format

- **Open point:** spec rev 1, 5.2, line 786 samples an image through its `image.*` sampler as `sampler2D` for float
  and normalized formats and `isampler2D`/`usampler2D` for integer formats. It doesn't say what happens when the
  sampler's class differs from the image's format (`usampler2D` over an image whose format is `R32F`), and it is not
  one of section 9's open questions. wgpu makes a pipeline only when a sampled texture's class matches the
  shader's, so such a program failed at pipeline creation with wgpu's error (#5223).
- **Decision:** the program is refused by Sunrise's own check before its pipeline is made, with a note naming the
  program, the sampler's type and name, the image and the texture's format. This holds for every pair of classes
  that differ: an integer sampler over a float or normalized image, a float sampler over an integer image, and a
  signed sampler over an unsigned image or the reverse. The rest of the pack keeps loading.
- **Reason:** D-3 refuses an integer storage qualifier on a float texture for the same cause. The two other ways to
  keep the program were rejected: a second texture in the sampler's class, kept in step with the image, needs a copy
  after each writer and a rule for which copy a store reaches (Vulkan has no view between `R32_SFLOAT` and
  `R32_UINT`); a stand-in of the sampler's class would let the program run while it reads nothing the pack wrote,
  hiding the fault instead of naming it. The refusal skips no program that ran before: such a program never got a
  pipeline.
- **Tests:** `pack_post::tests::a_sampler_of_another_class_than_its_image_refuses_the_program`
  (`sunrise-render-shaderpack`).
