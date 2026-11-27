#!/usr/bin/env python3
"""Compare the camera with the pinned client: Camera.tick (eye height and
field-of-view easing), alignWithEntity (view rotation quaternion and basis,
interpolated eye, third person back and front, camera distance and scale
attributes with mounts, getMaxZoom clipping against visual shapes, sleeping),
calculateFov/calculateHudFov (option, modifier, death and fluid effects,
panoramic mode) with the world projection and the first-person hand's
(GameRenderer.render3dHud at hudFov, whatever the FOV option),
AbstractClientPlayer.getFieldOfViewModifier (flight, speed attribute against
walking speed, bow draw, spyglass), the walk and bob
history (ClientAvatarState, updateBob) and GameRenderer.bobHurt/bobView,
driven tick by tick and frame by frame with identical scripted states.

Also records the 26.3 camera-entity witnesses into .local/witness/<version>/camera/
(VanillaCameraParityOracle, VanillaCameraOracle, VanillaHudOracle,
VanillaSetCameraOracle, VanillaCameraDrawOracle, VanillaTeamRulesOracle) and runs
the sunrise-client-render and sunrise-client camera parity tests against them."""
import argparse
import json
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking


def f32(v):
    return struct.unpack('>f', struct.pack('>f', v))[0]


WALK = [0.1, 0.0, 0.05, 0.2, 1e-3, -0.1, 0.15]
SPEED = [f32(0.1), f32(0.1) * 1.3, 0.0, 0.03, 0.5, 1024.0, 0.13, f32(0.1) * 0.85, f32(0.1) * 1.4]
TICKS = [0, 1, 5, 10, 19, 20, 21, 72000, -5, 3]


def fov_fields(rng):
    use = rng.choice([None, None, ['bow', rng.choice(TICKS)], ['spyglass', rng.choice(TICKS)], ['other', rng.choice(TICKS)]])
    return dict(flying=rng.random() < 0.3, ws=lab.fbits(rng.choice(WALK + [rng.uniform(-0.3, 0.3)])),
                speed=lab.dbits(rng.choice(SPEED + [rng.uniform(0, 2)])), use=use)


def fov_cases(rng):
    for flying in (False, True):
        for ws in WALK:
            for speed in SPEED:
                for use in [None, ['bow', 0], ['bow', 7], ['bow', 20], ['bow', 40], ['spyglass', 2], ['other', 5]]:
                    for fp in (False, True):
                        for scale in (0.0, 0.5, 1.0):
                            yield dict(t='fov', flying=flying, ws=lab.fbits(ws), speed=lab.dbits(speed), use=use, fp=fp, scale=lab.fbits(scale))
    for _ in range(3000):
        yield dict(t='fov', **fov_fields(rng), fp=rng.random() < 0.6, scale=lab.fbits(rng.choice([0.0, 1.0, rng.random()])))


FULL = [[0, 0, 0, 1, 1, 1]]
SHAPES = [FULL, FULL, [[0, 0, 0, 1, 0.5, 1]], [[0, 0.5, 0, 1, 1, 1]], [[0, 0, 0, 1, 1, 0.125]], [[0.375, 0, 0.375, 0.625, 1, 0.625]],
          [[0, 0, 0, 0.5, 1, 1], [0.5, 0, 0.5, 1, 1, 1]], []]


def world(rng, center):
    cells = {}
    cx, cy, cz = center
    for _ in range(rng.choice([0, 3, 10, 40, 120])):
        p = (cx + rng.randint(-6, 6), cy + rng.randint(-3, 5), cz + rng.randint(-6, 6))
        cells[f'{p[0]} {p[1]} {p[2]}'] = [[lab.dbits(v) for v in box] for box in rng.choice(SHAPES)]
    return cells


ANGLES = [0.0, 90.0, -90.0, 180.0, -180.0, 45.0, 360.0, 720.5, -33.3, 1e-3, 89.9]


def angle(rng):
    return rng.choice(ANGLES + [rng.uniform(-90, 90), rng.uniform(-400, 400)])


def entity_state(rng, center):
    pos = [center[i] + rng.uniform(0, 1) for i in range(3)]
    prev = [p - rng.choice([0, 0, 0.1, 0.3, 1.0, rng.uniform(-1, 1)]) for p in pos]
    kind = 'player' if rng.random() < 0.85 else 'marker'
    xr, yr = angle(rng), angle(rng)
    s = dict(kind=kind, pos=[lab.dbits(v) for v in pos], prev=[lab.dbits(v) for v in prev], xr=lab.fbits(xr), yr=lab.fbits(yr),
             xro=lab.fbits(rng.choice([xr, xr - 5, angle(rng)])), yro=lab.fbits(rng.choice([yr, yr + 10, angle(rng)])),
             eye=lab.fbits(rng.choice([1.62, 1.27, 0.4, 0.2, 0.81, 3.24, rng.uniform(0, 4)])))
    s.update(fov_fields(rng))
    s.update(scale_attr=lab.dbits(rng.choice([1.0, 1.0, 0.5, 2.0, 0.0625, 16.0, rng.uniform(0.0625, 3)])),
             dist=lab.dbits(rng.choice([4.0, 4.0, 0.0, 1.0, 10.0, 32.0, rng.uniform(0, 32)])),
             sleeping=rng.choice([None] * 6 + [[None], [0], [1], [2], [3]]),
             dead=rng.choice([None] * 6 + [0, 1, 5, 19, 20, 25]),
             mount=rng.choice([None] * 4 + [[lab.dbits(rng.choice([1.0, 2.0, 0.5])), lab.dbits(rng.choice([4.0, 8.0, 1.0]))]]))
    return s


def window(rng):
    """Window size and Camera.depthFar for the world and hand projections."""
    w, h = rng.choice([(1600, 1000), (854, 480), (1920, 1080), (800, 1200), (1, 1), (rng.randint(1, 4000), rng.randint(1, 3000))])
    return [lab.fbits(w), lab.fbits(h), lab.fbits(rng.choice([768.0, 128.0, 2048.0, 4096.0, rng.uniform(32, 4096)]))]


def camera_cases(rng):
    for n in range(2500):
        center = rng.choice([(0, 64, 0), (-10, -60, 7), (1000, 120, -3000), (0, 0, 0)])
        ops = []
        for _ in range(rng.randrange(1, 40)):
            kind = rng.choice(['entity', 'tick', 'tick', 'tick', 'frame', 'frame', 'frame', 'type'])
            if not ops:
                kind = 'entity'
            if kind == 'entity':
                ops.append(['entity', entity_state(rng, center)])
            elif kind == 'tick':
                ops.append(['tick'])
            elif kind == 'frame':
                pt = rng.choice([0.0, 1.0, 0.5, 0.25, 0.999, rng.random()])
                ops.append(['frame', lab.fbits(pt), rng.choice([0, 0, 0, 1, 2, 3]), rng.random() < 0.05] + window(rng))
            else:
                ops.append(['type', rng.randrange(3)])
        yield dict(t='camera', fov=rng.choice([70, 30, 110, 90, rng.randint(30, 110)]),
                   effect=lab.dbits(rng.choice([1.0, 0.0, 0.5, rng.random()])), type=rng.choice([0, 1, 1, 2]),
                   cells=world(rng, center), ops=ops)
    # The hand's projection at FOV 30, 70 and 110 while walking, sprinting, under
    # speed, drawing a bow, dying and in fluids: GameRenderer.render3dHud uses
    # hudFov, never the option or the modifier.
    for fov in (30, 70, 110):
        for speed in (f32(0.1), f32(0.1) * 1.3, 0.2, 0.05):
            for use in (None, ['bow', 10]):
                for dead in (None, 5):
                    for fluid in (0, 1, 2):
                        s = dict(kind='player', pos=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)], prev=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)],
                                 xr=lab.fbits(0.0), yr=lab.fbits(0.0), xro=lab.fbits(0.0), yro=lab.fbits(0.0), eye=lab.fbits(1.62), flying=False,
                                 ws=lab.fbits(0.1), speed=lab.dbits(speed), use=use, scale_attr=lab.dbits(1.0), dist=lab.dbits(4.0),
                                 sleeping=None, dead=dead, mount=None)
                        ops = [['entity', s]] + [['tick']] * 8 + [['frame', lab.fbits(pt), fluid, False, lab.fbits(1600.0), lab.fbits(1000.0), lab.fbits(768.0)]
                                                                 for pt in (0.0, 0.5, 1.0)]
                        yield dict(t='camera', fov=fov, effect=lab.dbits(1.0), type=0, cells={}, ops=ops)
    # A walled-in third-person camera stepping through every distance bound.
    for dist in [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 32.0]:
        for wall in range(0, 6):
            for yaw in [0.0, 90.0, 180.0, 270.0, 45.0]:
                cells = {}
                for dx in range(-6, 7):
                    for dy in range(62, 68):
                        cells[f'{dx} {dy} {-wall - 1}'] = [[lab.dbits(v) for v in box] for box in FULL]
                        cells[f'{dx} {dy} {wall + 1}'] = [[lab.dbits(v) for v in box] for box in FULL]
                s = dict(kind='player', pos=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)], prev=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)],
                         xr=lab.fbits(15.0), yr=lab.fbits(yaw), xro=lab.fbits(15.0), yro=lab.fbits(yaw), eye=lab.fbits(1.62), flying=False,
                         ws=lab.fbits(0.1), speed=lab.dbits(f32(0.1)), use=None, scale_attr=lab.dbits(1.0), dist=lab.dbits(dist),
                         sleeping=None, dead=None, mount=None)
                ops = [['entity', s]] + [['tick']] * 6 + [['frame', lab.fbits(pt), 0, False] + window(rng) for pt in (0.0, 0.5, 1.0)]
                yield dict(t='camera', fov=70, effect=lab.dbits(1.0), type=1 + (wall % 2), cells=cells, ops=ops)


def bob_cases(rng):
    for n in range(6000):
        living = None
        if rng.random() < 0.8:
            living = dict(hurt=lab.fbits(rng.choice([-1.0, -0.5, 0.0, 10.0, 9.5, 5.25, 1.0, rng.uniform(-2, 11)])),
                          duration=rng.choice([10, 10, 0, 5, 20]), dir=lab.fbits(rng.choice([0.0, 90.0, -45.0, 180.0, rng.uniform(-360, 360)])),
                          death=rng.choice([None, None, None, lab.fbits(rng.choice([0.0, 0.5, 19.5, 20.0, 25.0, rng.uniform(0, 30)]))]))
        player = None
        if rng.random() < 0.8:
            player = [lab.fbits(rng.choice([0.0, -0.3, -1.7, -100.25, rng.uniform(-1000, 0)])),
                      lab.fbits(rng.choice([0.0, 0.1, 0.05, 0.0999, rng.uniform(0, 0.1)]))]
        yield dict(t='bob', living=living, player=player, tilt=lab.dbits(rng.choice([1.0, 0.0, 0.5, rng.random()])),
                   view=rng.random() < 0.8)


def avatar_cases(rng):
    for n in range(1500):
        ops = []
        for _ in range(rng.randrange(1, 60)):
            kind = rng.choice(['tick', 'move', 'move', 'bob', 'bob', 'frame', 'frame', 'reset'])
            if kind == 'tick':
                ops.append(['tick'])
            elif kind == 'move':
                ops.append(['move', lab.fbits(rng.choice([0.0, 0.2158, -0.1, rng.uniform(-1, 1)])), lab.fbits(rng.choice([0.0, 0.2806, rng.uniform(-1, 1)]))])
            elif kind == 'bob':
                v = [rng.choice([0.0, 0.05, -0.2, rng.uniform(-0.5, 0.5)]), rng.uniform(-1, 1), rng.choice([0.0, 0.08, rng.uniform(-0.5, 0.5)])]
                ops.append(['bob', rng.random() < 0.7, rng.random() < 0.1, rng.random() < 0.1, [lab.dbits(x) for x in v]])
            elif kind == 'frame':
                ops.append(['frame', lab.fbits(rng.choice([0.0, 1.0, 0.5, rng.random()]))])
            else:
                ops.append(['reset'])
        yield dict(t='avatar', ops=ops)


def boat_cases(rng):
    """AbstractBoat.clampRotation: a rider's yaw against the boat's, near the 105-degree limit, around the wrap
    at 180, at large turns and at float steps where adding and subtracting round differently."""
    # The float immediately below +180 wraps to the opposite sign if 180 is added first.
    # The second angle is in range but changes when vanilla adds the target before subtracting the delta.
    yield dict(t='boat', views=[[lab.fbits(rider), lab.fbits(boat)]
                                for rider, boat in [(179.99998, 0.0), (-179.99998, -75.12346)]])

    def angle():
        r = rng.random()
        if r < 0.3:
            return rng.uniform(-720, 720)
        if r < 0.5:
            return rng.choice([-1, 1]) * rng.uniform(1e3, 1e6)
        if r < 0.8:
            return rng.choice([-180.0, 180.0, -105.0, 105.0, 0.0, 360.0]) + rng.uniform(-1e-3, 1e-3)
        return rng.choice([-180.0, 180.0, 105.0, -105.0, 75.0, 255.0, 285.0, 540.0])
    for _ in range(40):
        views = []
        for _ in range(500):
            boat = angle()
            rider = boat + angle() if rng.random() < 0.6 else angle()
            views.append([lab.fbits(rider), lab.fbits(boat)])
        yield dict(t='boat', views=views)


def make_cases(build):
    rng = random.Random(0x43414d45)
    path = build / 'cases.jsonl'
    cases = build / 'cases'
    cases.mkdir(exist_ok=True)
    count = 0
    with path.open('w') as out:
        for generator in (fov_cases, camera_cases, bob_cases, avatar_cases, boat_cases):
            for case in generator(rng):
                text = json.dumps(case, separators=(',', ':'))
                out.write(text + '\n')
                if count < 20000:
                    (cases / str(count)).write_text(text)
                count += 1
    return path, count


# --- Camera-entity witnesses recorded into .local/witness/<version>/camera/ ---

SET_CAMERA_KINDS = [
    ('minecraft:zombie', 'net.minecraft.world.entity.monster.zombie.Zombie'),
    ('minecraft:armor_stand', 'net.minecraft.world.entity.decoration.ArmorStand'),
    ('minecraft:oak_boat', 'net.minecraft.world.entity.vehicle.boat.Boat'),
    ('minecraft:item_frame', 'net.minecraft.world.entity.decoration.ItemFrame'),
    ('minecraft:tnt', 'net.minecraft.world.entity.item.PrimedTnt'),
    ('minecraft:minecart', 'net.minecraft.world.entity.vehicle.minecart.Minecart'),
    ('minecraft:arrow', 'net.minecraft.world.entity.projectile.arrow.Arrow'),
    ('minecraft:end_crystal', 'net.minecraft.world.entity.boss.enderdragon.EndCrystal'),
    ('minecraft:item', 'net.minecraft.world.entity.item.ItemEntity'),
    ('minecraft:experience_orb', 'net.minecraft.world.entity.ExperienceOrb'),
    ('minecraft:falling_block', 'net.minecraft.world.entity.item.FallingBlockEntity'),
    ('minecraft:painting', 'net.minecraft.world.entity.decoration.painting.Painting'),
    ('minecraft:interaction', 'net.minecraft.world.entity.Interaction'),
    ('minecraft:block_display', 'net.minecraft.world.entity.Display$BlockDisplay'),
]

MOUNT_VEHICLES = [
    None,
    ('happy_ghast', 1.0, 8.0), ('happy_ghast', 0.5, 8.0), ('happy_ghast', 2.0, 8.0), ('happy_ghast', 0.0625, 8.0),
    ('happy_ghast', float('nan'), 8.0), ('happy_ghast', 1.0, 16.0), ('happy_ghast', 1.0, 32.0), ('happy_ghast', 1.0, 100.0),
    ('happy_ghast', 1.0, 0.0), ('happy_ghast', 1.0, -0.0), ('happy_ghast', 1.0, 4.0),
    ('ghast', 1.0, 8.0), ('ghast', 2.0, 8.0), ('zombie', 1.0, 4.0), ('zombie', 4.0, 8.0),
    ('armor_stand', 1.0, 4.0), ('boat', 1.0, 4.0),
]

SLEEPING_VIEWS = [
    (0.0, 0.0, 0.0, 0.0, 1.0),
    (33.5, 12.25, 30.0, 10.0, 0.5),
    (-120.0, -45.0, -100.0, -40.0, 0.25),
    (200.0, 80.0, 190.0, 75.0, 0.75),
    (90.0, 0.0, 90.0, 0.0, 0.0),
]

DEATH_TIMES = [None, 0, 5, 19, 20]
DEATH_PARTIALS = [0.0, 0.5, 1.0]


def hud_camera_cases():
    common = dict(
        width=427, height=240, crosshair=False, attackIndicator='OFF', attackScale=0.0, attackDelay=10.0,
        selected=2, rightHanded=False, slots=[0, None, 3, None, None, 5, None, None, 0], offhand=None,
        partial=0.0, health=14.0, maxHealth=20.0, absorption=0.0, armor=7, food=11, saturation=5.0,
        effects=[], frozen=False, hardcore=False, damageCooldown=0, air=150, maxAir=300, underWater=False,
        tick=0, lastHealth=0, displayHealth=0, lastHealthAgo=0, healthBlinkTime=0, xpProgress=0.0, xpLevel=0,
        experienceBar=True, kind='decorations',
    )
    watched_base = dict(
        health=9.0, maxHealth=20.0, absorption=4.0, armor=3, air=210, underWater=False,
        frozen=False, rightHanded=True, mainHand=True, offhand=True,
    )
    watched_mounted = dict(watched_base, vehicleMaxHealth=24.0, vehicleHealth=11.0)
    return [
        dict(common, gameMode='survival', cameraKind='self'),
        dict(common, gameMode='survival', cameraKind='self', vehicleMaxHealth=30.0, vehicleHealth=17.0),
        dict(common, gameMode='survival', cameraKind='mob', vehicleMaxHealth=30.0, vehicleHealth=17.0),
        dict(common, gameMode='adventure', cameraKind='mob'),
        dict(common, gameMode='creative', cameraKind='mob'),
        dict(common, gameMode='survival', cameraKind='otherPlayer', watched=watched_base),
        dict(common, gameMode='survival', attackIndicator='HOTBAR', cameraKind='otherPlayer', watched=watched_base),
        dict(common, gameMode='survival', cameraKind='otherPlayer', watched=watched_mounted),
        dict(common, gameMode='creative', cameraKind='otherPlayer', watched=watched_mounted),
        dict(common, gameMode='spectator', cameraKind='otherPlayer', watched=watched_mounted),
        dict(common, gameMode='adventure', cameraKind='otherPlayer', watched=dict(watched_base, air=300)),
    ]


def mount_cases():
    zombie_eye, boat_eye = 1.74, 0.5625
    for vehicle in MOUNT_VEHICLES:
        mount = None if vehicle is None else [lab.dbits(vehicle[1]), lab.dbits(vehicle[2]), vehicle[0]]
        for rider in ('zombie', 'boat'):
            riders = [(s, d) for s in (1.0, 2.0) for d in (4.0, 10.0)] if rider == 'zombie' else [(1.0, 4.0)]
            for scale, dist in riders:
                eye = zombie_eye * scale if rider == 'zombie' else boat_eye
                common = dict(
                    pos=[lab.dbits(10.25), lab.dbits(64.0), lab.dbits(-3.75)],
                    prev=[lab.dbits(10.25), lab.dbits(64.0), lab.dbits(-3.75)],
                    xr=lab.fbits(22.5), yr=lab.fbits(45.0), xro=lab.fbits(22.5), yro=lab.fbits(45.0),
                    eye=lab.fbits(eye), mount=mount,
                )
                ent = dict(kind='marker', **common) if rider == 'boat' else dict(
                    kind='player', flying=False, ws=lab.fbits(0.1), speed=lab.dbits(0.1), use=None,
                    scale_attr=lab.dbits(scale), dist=lab.dbits(dist), sleeping=None, dead=None, **common,
                )
                for camera_type in (0, 1, 2):
                    for pt in ((0.0, 0.5, 1.0) if camera_type else (0.0,)):
                        ops = [['entity', ent], ['tick'],
                               ['frame', lab.fbits(pt), 0, False, lab.fbits(1600.0), lab.fbits(1000.0), lab.fbits(768.0)]]
                        yield dict(
                            rider=rider, rider_scale=lab.dbits(scale), rider_distance=lab.dbits(dist),
                            vehicle=None if vehicle is None else [vehicle[0], lab.dbits(vehicle[1]), lab.dbits(vehicle[2])],
                            eye=lab.fbits(eye), camera_type=camera_type, partial=pt,
                            case=dict(t='camera', fov=70, effect=lab.dbits(1.0), type=camera_type, cells={},
                                      report_distance=True, ops=ops),
                        )


def sleeping_cases():
    rng = random.Random(0x534c4550)
    for kind, facing in [(k, f) for k in ('villager', 'remote_player') for f in range(4)]:
        for yaw, pitch, yaw_o, pitch_o, pt in SLEEPING_VIEWS:
            s = entity_state(rng, (0, 64, 0))
            pos = [10.5, 64.0625, -3.5]
            s.update(
                kind=kind, sleeping_pos=[10, 64, -4],
                pos=[lab.dbits(v) for v in pos], prev=[lab.dbits(v) for v in [10.25, 64.0625, -3.75]],
                xr=lab.fbits(pitch), yr=lab.fbits(yaw), xro=lab.fbits(pitch_o), yro=lab.fbits(yaw_o),
                eye=lab.fbits(0.2), scale_attr=lab.dbits(1.0), dist=lab.dbits(4.0),
                sleeping=[facing], dead=None, mount=None,
            )
            yield dict(t='camera', bed=[10, 64, -4, facing], fov=70, effect=lab.dbits(1.0), type=0, cells={},
                       ops=[['entity', s], ['tick'], ['frame', lab.fbits(pt), 0, False] + window(rng)])


RENDER_WITNESS_TESTS = [
    'gfx::view::tests::cull_frustum_matches_26_3_witness_with_zoom_widen_and_bob',
]

CLIENT_WITNESS_TESTS = [
    'game::client_side::interactions::tests::interaction_negative_height_bounds_invert_downward_26_3',
    'game::client_side::interactions::tests::interaction_unsynced_default_sync_and_pose_sync_seat_height_26_3',
    'game::remote::tests::camera_death_tests::a_spectated_dying_living_entity_squeezes_the_fov_26_3',
    'game::remote::tests::camera_draw_tests::local_player_and_camera_entity_draw_like_the_real_client',
    'game::remote::tests::camera_parity_red_tests::any_tracked_entity_resolves_as_camera_26_3',
    'game::remote::tests::camera_parity_red_tests::block_outline_requires_player_camera_26_3',
    'game::remote::tests::camera_parity_red_tests::camera_consumers_bob_fog_dark_disc_and_vignette_26_3',
    'game::remote::tests::camera_parity_red_tests::camera_consumers_streaming_particles_debug_and_spectator_crosshair_26_3',
    'game::remote::tests::camera_parity_red_tests::camera_entity_excluded_from_pick_for_all_pickable_kinds_26_3',
    'game::remote::tests::camera_parity_red_tests::camera_entity_hidden_in_first_person_for_all_kinds_and_local_player_hidden_26_3',
    'game::remote::tests::camera_parity_red_tests::camera_player_fov_modifier_and_death_isolation_26_3',
    'game::remote::tests::camera_parity_red_tests::culling_near_plane_uses_max_of_fov_and_option_fov_26_3',
    'game::remote::tests::camera_parity_red_tests::frozen_camera_partial_tick_matches_26_3_witness',
    'game::remote::tests::camera_parity_red_tests::interaction_camera_eye_rotation_move_and_teleport_26_3',
    'game::remote::tests::camera_parity_red_tests::particle_camera_before_first_frame_and_after_reset_26_3',
    'game::remote::tests::camera_parity_red_tests::removing_camera_entity_keeps_detached_camera_26_3',
    'game::remote::tests::camera_parity_red_tests::review_followup_camera_parity_26_3',
    'game::remote::tests::camera_parity_red_tests::sleeping_spectated_living_entity_bed_view_and_visibility_26_3',
    'game::remote::tests::camera_parity_red_tests::spectated_rider_mount_attributes_26_3',
    'game::client_side::remote_client::client_camera_tests::a_named_mob_the_server_camera_is_attached_to_shows_no_name_tag',
    'game::remote::tests::client_hud_camera_tests::hud_bars_follow_the_camera_player_like_26_3',
    'game::remote::tests::set_camera_kinds_tests::set_camera_follows_the_real_26_3_clients_camera_for_every_entity_kind',
    'game::client_side::remote_camera::camera_mount_tests::a_spectated_rider_backs_off_by_its_living_vehicles_camera_distance',
    'game::client_side::remote_camera::camera_tests::a_spectated_sleeping_villager_shows_the_bed_view',
]


def record_camera_witnesses(lock, jars, libraries, build):
    version = lock['minecraft']['version']
    java_major = lock['minecraft']['java_major']
    witness_dir = lab.LOCAL / 'witness' / version / 'camera'
    witness_dir.mkdir(parents=True, exist_ok=True)
    java_ver = subprocess.check_output(['java', '--version'], text=True).splitlines()[0].strip()
    client_sha = lab.digest(jars['client'], 'sha256')
    oracles = {}

    def compile_one(name):
        sub = build / f'oracle-{name}'
        sub.mkdir(parents=True, exist_ok=True)
        classes, deps, path = harness.compile_oracle(sub, name, jars, libraries, java_major)
        oracles[name] = lab.digest(path, 'sha256')
        return sub, classes, deps

    # 1. VanillaCameraParityOracle -> camera_parity.json
    sub, classes, deps = compile_one('VanillaCameraParityOracle')
    parity_raw = sub / 'raw.json'
    harness.java_run(sub, classes, deps, 'VanillaCameraParityOracle', [parity_raw], 'client')
    parity_path = witness_dir / 'camera_parity.json'
    parity_data = json.loads(parity_raw.read_text(encoding='utf-8'))
    parity_path.write_text(json.dumps(dict(
        meta=dict(
            minecraft=version,
            client_jar_sha256=client_sha,
            java_version=java_ver,
            command='python3 tools/test_camera_parity.py',
            oracle='mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCameraParityOracle.java',
        ),
        **parity_data,
    ), indent=2) + '\n', encoding='utf-8')

    # 2. VanillaCameraOracle -> mount, sleeping, and death witnesses
    cam_sub, cam_classes, cam_deps = compile_one('VanillaCameraOracle')

    all_mount = list(mount_cases())
    mount_in = cam_sub / 'mount-cases.jsonl'
    mount_in.write_text(''.join(json.dumps(c['case'], separators=(',', ':')) + '\n' for c in all_mount), encoding='utf-8')
    mount_raw = cam_sub / 'mount-client.txt'
    harness.java_run(cam_sub, cam_classes, cam_deps, 'VanillaCameraOracle', [mount_in, mount_raw], 'mount')
    sections, cur = {}, None
    for line in mount_raw.read_text(encoding='utf-8').splitlines():
        if line.startswith('== '):
            cur = int(line[3:])
            sections[cur] = []
        else:
            sections[cur].append(line)
    mount_records = []
    for n, c in enumerate(all_mount):
        frame = next(l for l in sections[n] if l.startswith('frame')).split(' ')
        distance = next(l for l in sections[n] if l.startswith('dist ')).split(' ')[1]
        mount_records.append(dict(
            rider=c['rider'], rider_scale=c['rider_scale'], rider_distance=c['rider_distance'],
            vehicle=c['vehicle'], eye=c['eye'], camera_type=c['camera_type'], partial=c['partial'],
            distance=distance, expected=frame[1:20],
        ))
    mount_path = witness_dir / 'camera_mount_witness.json'
    mount_path.write_text(json.dumps(dict(
        minecraft=version, client_jar_sha256=client_sha, records=mount_records,
    ), indent=1) + '\n', encoding='utf-8')

    sleeping_dir = witness_dir / 'remote_camera_sleeping'
    sleeping_dir.mkdir(parents=True, exist_ok=True)
    sleeping_in = sleeping_dir / 'cases.jsonl'
    sleeping_in.write_text(''.join(json.dumps(c, separators=(',', ':')) + '\n' for c in sleeping_cases()), encoding='utf-8')
    sleeping_out = sleeping_dir / 'vanilla.txt'
    harness.java_run(cam_sub, cam_classes, cam_deps, 'VanillaCameraOracle', [sleeping_in, sleeping_out], 'sleeping')

    death_cases = []
    for dead in DEATH_TIMES:
        s = dict(
            kind='creeper',
            pos=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)],
            prev=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)],
            xr=lab.fbits(0.0), yr=lab.fbits(0.0), xro=lab.fbits(0.0), yro=lab.fbits(0.0),
            eye=lab.fbits(1.62), flying=False, ws=lab.fbits(0.0), speed=lab.dbits(f32(0.1)), use=None,
            scale_attr=lab.dbits(1.0), dist=lab.dbits(4.0), sleeping=None, dead=dead, mount=None,
        )
        ops = [['entity', s]] + [['tick']] * 8 + [
            ['frame', lab.fbits(pt), 0, False, lab.fbits(1600.0), lab.fbits(1000.0), lab.fbits(768.0)]
            for pt in DEATH_PARTIALS
        ]
        death_cases.append(dict(t='camera', fov=70, effect=lab.dbits(1.0), type=0, cells={}, ops=ops))
    death_in = cam_sub / 'death-cases.jsonl'
    death_in.write_text(''.join(json.dumps(c, separators=(',', ':')) + '\n' for c in death_cases), encoding='utf-8')
    death_raw = cam_sub / 'death-client.txt'
    harness.java_run(cam_sub, cam_classes, cam_deps, 'VanillaCameraOracle', [death_in, death_raw], 'death')
    death_frames = [l.split() for l in death_raw.read_text(encoding='utf-8').splitlines() if l.startswith('frame ')]
    death_rows = []
    for idx, dead in enumerate(DEATH_TIMES):
        chunk = death_frames[idx * len(DEATH_PARTIALS):(idx + 1) * len(DEATH_PARTIALS)]
        for pt, frame in zip(DEATH_PARTIALS, chunk):
            death_rows.append(f'{"none" if dead is None else dead}\t{lab.fbits(pt)}\t{frame[-2]}\t{frame[-1]}')
    death_path = witness_dir / 'camera_death_witness.tsv'
    death_path.write_text(
        f'# Vanilla {version} Camera.calculateFov/calculateHudFov deathTime witness (tools/test_camera_parity.py)\n'
        + '\n'.join(death_rows) + '\n',
        encoding='utf-8',
    )

    # 3. VanillaHudOracle -> hud_camera/vanilla.jsonl
    hud_sub, hud_classes, hud_deps = compile_one('VanillaHudOracle')
    hud_dir = witness_dir / 'hud_camera'
    hud_dir.mkdir(parents=True, exist_ok=True)
    hud_in = hud_dir / 'cases.jsonl'
    hud_in.write_text(''.join(json.dumps(c) + '\n' for c in hud_camera_cases()), encoding='utf-8')
    hud_out = hud_dir / 'vanilla.jsonl'
    harness.java_run(hud_sub, hud_classes, hud_deps, 'VanillaHudOracle', [hud_in, hud_out], 'hud')

    # 4. VanillaSetCameraOracle -> set_camera_witness.tsv
    set_sub, set_classes, set_deps = compile_one('VanillaSetCameraOracle')
    kinds_in = set_sub / 'kinds.tsv'
    kinds_in.write_text(''.join(f'{label}\t{cls}\n' for label, cls in SET_CAMERA_KINDS), encoding='utf-8')
    set_raw = set_sub / 'client.txt'
    harness.java_run(set_sub, set_classes, set_deps, 'VanillaSetCameraOracle', [set_raw, kinds_in], 'set-camera')
    set_path = witness_dir / 'set_camera_witness.tsv'
    set_path.write_text(
        f'# Vanilla {version} ClientPacketListener.handleSetCamera witness (tools/test_camera_parity.py)\n'
        + set_raw.read_text(encoding='utf-8'),
        encoding='utf-8',
    )

    # 5. VanillaCameraDrawOracle -> camera_draw_witness.txt
    draw_sub, draw_classes, draw_deps = compile_one('VanillaCameraDrawOracle')
    draw_path = witness_dir / 'camera_draw_witness.txt'
    harness.java_run(draw_sub, draw_classes, draw_deps, 'VanillaCameraDrawOracle', [draw_path], 'camera-draw')

    # 6. VanillaTeamRulesOracle -> name_tag_camera.json
    team_sub, team_classes, team_deps = compile_one('VanillaTeamRulesOracle')
    team_opt = {'ff': False, 'si': False, 'collision': 'always', 'nametag': 'always'}
    name_cases = [
        {
            'case': idx, 'A': team_opt, 'B': team_opt, 'own': None, 'other': None, 'viewer': None,
            'pushable': True, 'spectator': False, 'invisible': False, 'discrete': False, 'vehicle': False,
            'viewer_spectator': False, 'camera_is_entity': is_cam, 'hud_hidden': False, 'distance': 5.0,
        }
        for idx, is_cam in enumerate((False, True))
    ]
    team_in = team_sub / 'name-tag-cases.jsonl'
    team_in.write_text(''.join(json.dumps(c) + '\n' for c in name_cases), encoding='utf-8')
    team_raw = team_sub / 'name-tag-vanilla.jsonl'
    harness.java_run(team_sub, team_classes, team_deps, 'VanillaTeamRulesOracle', [team_in, team_raw], 'name-tag')
    team_rows = [json.loads(l) for l in team_raw.read_text(encoding='utf-8').splitlines() if l.strip()]
    name_tag_path = witness_dir / 'name_tag_camera.json'
    name_tag_path.write_text(json.dumps(dict(
        minecraft=version,
        cases=[
            dict(camera_is_entity=bool(c['camera_is_entity']), name=bool(r['name']))
            for c, r in zip(name_cases, team_rows)
        ],
    ), indent=2) + '\n', encoding='utf-8')

    lab.write_json(witness_dir / 'meta.json', dict(
        minecraft=version,
        client_jar_sha256=client_sha,
        java_version=java_ver,
        command='python3 tools/test_camera_parity.py',
        oracles=oracles,
        inputs_sha256={
            'mount_cases': lab.digest(mount_in, 'sha256'),
            'sleeping_cases': lab.digest(sleeping_in, 'sha256'),
            'death_cases': lab.digest(death_in, 'sha256'),
            'hud_cases': lab.digest(hud_in, 'sha256'),
            'set_camera_kinds': lab.digest(kinds_in, 'sha256'),
            'name_tag_cases': lab.digest(team_in, 'sha256'),
        },
    ))

    return {
        'SUNRISE_CAMERA_PARITY_WITNESS': str(parity_path.resolve()),
        'SUNRISE_CAMERA_MOUNT_WITNESS': str(mount_path.resolve()),
        'SUNRISE_HUD_CAMERA_WITNESS': str(hud_out.resolve()),
        'SUNRISE_SET_CAMERA_WITNESS': str(set_path.resolve()),
        'SUNRISE_REMOTE_CAMERA_SLEEPING_DIR': str(sleeping_dir.resolve()),
        'SUNRISE_CAMERA_DRAW_WITNESS': str(draw_path.resolve()),
        'SUNRISE_CAMERA_DEATH_WITNESS': str(death_path.resolve()),
        'SUNRISE_NAME_TAG_CAMERA_WITNESS': str(name_tag_path.resolve()),
    }


def run_logged(command, build, name, cwd=lab.ROOT, env=None):
    path = build / f'{name}.log'
    with path.open('w', encoding='utf-8') as log:
        result = subprocess.run([str(c) for c in command], cwd=cwd, env=env, timeout=3600,
                                stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = path.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        sys.exit(f'{name} failed: see {path}')
    return path


def build_test_binary(build, package, target_name, env):
    compiled = run_logged(
        ['cargo', 'test', '--locked', '-p', package, '--lib', '--no-run', '--message-format=json'],
        build, f'compile-{package}', env=env,
    )
    binaries = []
    for line in compiled.read_text(encoding='utf-8').splitlines():
        if not line.startswith('{'):
            continue
        row = json.loads(line)
        if (row.get('reason') == 'compiler-artifact' and row.get('executable')
                and row.get('target', {}).get('name') == target_name and row.get('profile', {}).get('test')):
            binaries.append(Path(row['executable']))
    assert len(binaries) == 1, f'expected one {target_name} test binary, found {binaries}'
    return binaries[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--record-only', action='store_true')
    args = parser.parse_args()
    scope = ('Real Camera.tick, tickFov, alignWithEntity (setRotation quaternion and forwards/up/left basis, move, '
             'interpolated eye with eye-height easing, first person, third person back and mirrored front, scale and '
             'camera_distance attributes including a living mount, getMaxZoom over the actual Level.clip with '
             'ClipContext.Block.VISUAL against controlled visual shapes, sleeping bed orientation), calculateFov and '
             'calculateHudFov (option, eased modifier, death, water/lava with the effect scale, panoramic), the world projection '
             '(Camera.setupPerspective) and the hand projection GameRenderer.render3dHud sets up at hudFov, as JOML setPerspective '
             'matrices, at FOV options 30, 70 and 110 among others, '
             'AbstractClientPlayer.getFieldOfViewModifier with real Abilities, attribute maps and item stacks, '
             'ClientAvatarState walk/bob history with AbstractClientPlayer.updateBob, and GameRenderer.bobHurt/bobView '
             'as the full JOML pose matrix with its property flags. The camera entity is a shell with controlled '
             'queries; getFluidInCamera is supplied (the fog harnesses cover fluid detection) and the new-minecart '
             'interpolation branch is not exercised. AbstractBoat.clampRotation runs on a boat shell for a player shell.')
    if not args.record_only:
        harness.differential(
            'rust-camera-parity', 'VanillaCameraOracle', 'camera_probe', make_cases, scope,
            sources=[lab.ROOT / 'crates/sunrise-client-render/src/camera.rs'])

    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'camera-entity-parity'
    build.mkdir(parents=True, exist_ok=True)
    witness_env = record_camera_witnesses(lock, jars, libraries, build)
    if args.record_only:
        print(f'wrote camera witnesses under {lab.LOCAL / "witness" / lock["minecraft"]["version"] / "camera"}')
        return

    with tempfile.TemporaryDirectory(prefix='sunrise-camera-parity-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216', **witness_env)
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())

        render_bin = build_test_binary(build, 'sunrise-client-render', 'sunrise_client_render', env)
        render_log = run_logged(
            [render_bin, '--ignored', '--exact', *RENDER_WITNESS_TESTS], build, 'render-tests', cwd=scratch, env=env,
        )
        expected_render = f'{len(RENDER_WITNESS_TESTS)} passed; 0 failed'
        assert expected_render in render_log.read_text(encoding='utf-8'), f'expected "{expected_render}" in {render_log}'

        client_bin = lab.parity_fixtures()
        # Game fixtures register process-global keys and providers. Don't let another test replace them.
        for index, test in enumerate(CLIENT_WITNESS_TESTS):
            client_root = scratch / f'client-{index}'
            client_root.mkdir()
            client_env = dict(env, **{key: str(client_root / key) for key in homes})
            client_log = run_logged(
                [client_bin, '--ignored', '--exact', test], build, f'client-test-{index}',
                cwd=client_root, env=client_env,
            )
            assert '1 passed; 0 failed' in client_log.read_text(encoding='utf-8'), f'{test} did not pass: {client_log}'

    total = len(RENDER_WITNESS_TESTS) + len(CLIENT_WITNESS_TESTS)
    print(f'Camera entity parity passed ({total} witness tests against .local/witness/{lock["minecraft"]["version"]}/camera)')


if __name__ == '__main__':
    main()
