fn sample_mob(game: &Game<'_>, mob: &NativeMobRender<'_>, draw: &mut BlockEntityDraw, drawn: &mut bool) {
    if mob.kind != "minecraft:pig" { return; }
    if mob.hurt_time > 0 || mob.death_time > 0 || (mob.age as i32 % 40 == 0 && mob.partial < 0.1) {
        eprintln!("PIG_STATE id={} baby={} age={} yaw={} head={} pitch={} walk={} speed={} hurt={} death={}", mob.id, mob.baby, mob.age, mob.body_yaw, mob.head_yaw, mob.pitch, mob.walk_position, mob.walk_speed, mob.hurt_time, mob.death_time);
    }
    let face = Some(super::native_block_entity_draw::CubeFace { sprite: "block/white_wool", uv: Some([0.0, 0.0, 16.0, 16.0]) });
    let faces = [face; 6];
    game.native_draw_cube(draw, [-7.0, 5.0, -10.0], [7.0, 14.0, 6.0], faces, 0xff44dd);
    draw.push();
    draw.rotate_about([1.0, 0.0, 0.0], mob.pitch, [0.0, 0.7, 0.35]);
    game.native_draw_cube(draw, [-5.0, 8.0, 5.0], [5.0, 16.0, 13.0], faces, 0x44ff44);
    draw.pop();
    let swing = (mob.walk_position * 0.6662).cos() * 45.0 * mob.walk_speed;
    for (x, z, dir) in [(-5.0, -6.0, 1.0), (3.0, -6.0, -1.0), (-5.0, 4.0, -1.0), (3.0, 4.0, 1.0)] {
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], swing * dir, [x / 16.0, 5.0 / 16.0, z / 16.0]);
        game.native_draw_cube(draw, [x, 0.0, z], [x + 2.0, 6.0, z + 2.0], faces, 0x4444ff);
        draw.pop();
    }
    *drawn = true;
}
