#!/usr/bin/env python3
"""Item stacks on the wire: every data component, as vanilla 26.3 encodes it.

VanillaItemWireOracle encodes stacks with ItemStack.OPTIONAL_STREAM_CODEC
(what a server sends in inventories, recipe books and advancements): each
default component value of every vanilla item, the SNBT fixtures below
(direct holders, tags and lists, optional fields, every consume effect,
block predicates with component matchers, the mob variants) and patches
built in Java (components without a persistent codec, removals). The Rust
probe decodes each with sunrise_protocol::item::Slot and must consume every
byte and find the same component ids in the same order; a stack whose
components Sunrise only reads past must also encode back to the same bytes.
Every id in DataComponents must be covered."""
import argparse
import json
import os
import platform
import subprocess
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

COMPONENT_TYPES = 122

FIXTURES = [
    ('custom_data', '{custom_data:{a:1,b:"x",c:[1,2]}}'),
    ('unbreakable', '{unbreakable:{}}'),
    ('custom_name', '{custom_name:{text:"Hi",color:"red",bold:1b}}'),
    ('lore', '{lore:["a",{text:"b",italic:0b}]}'),
    ('enchantments', '{enchantments:{"minecraft:sharpness":5,"minecraft:unbreaking":3}}'),
    ('can_break tag', '{can_break:{blocks:"#minecraft:logs"}}'),
    ('can_break list state nbt', '{can_break:[{blocks:["minecraft:stone","minecraft:dirt"],state:{snowy:"true",age:{min:"1",max:"3"},level:{min:"2"}},nbt:"{a:1b}"}]}'),
    ('can_place_on components', '{can_place_on:{blocks:"minecraft:chest",components:{"minecraft:custom_name":"x","minecraft:damage":3},predicates:{"minecraft:custom_data":{a:1},"minecraft:damage":{durability:{min:1}}}}}'),
    ('can_place_on empty', '{can_place_on:[{}]}'),
    ('custom_model_data', '{custom_model_data:{floats:[1.0f,2.5f],flags:[1b,0b],strings:["a"],colors:[16711680]}}'),
    ('tooltip_display', '{tooltip_display:{hide_tooltip:1b,hidden_components:["minecraft:enchantments","minecraft:damage"]}}'),
    ('repair_cost', '{repair_cost:7}'),
    ('intangible_projectile', '{intangible_projectile:{}}'),
    ('tooltip_style', '{tooltip_style:"minecraft:fancy"}'),
    ('tooltip_style custom', '{tooltip_style:"lobby:gold"}'),
    ('tooltip_style nested', '{tooltip_style:"lobby:themes/royal"}'),
    ('interact_animation', '{interact_animation:{type:"stab",duration:9}}'),
    ('dyed_color', '{dyed_color:16711935}'),
    ('map_id', '{map_id:42}'),
    ('writable_book_content', '{writable_book_content:{pages:["one",{raw:"two",filtered:"t*o"}]}}'),
    ('written_book_content', '{written_book_content:{title:{raw:"T",filtered:"F"},author:"me",generation:1,pages:[{raw:"p1",filtered:"q1"},"p2"],resolved:1b}}'),
    ('trim registry', '{trim:{material:"minecraft:gold",pattern:"minecraft:coast"}}'),
    ('trim direct', '{trim:{material:{palette_id:"minecraft:x",description:"X"},pattern:{asset_id:"minecraft:y",description:"Y",decal:1b}}}'),
    ('block_entity_data', '{block_entity_data:{id:"minecraft:chest",Items:[]}}'),
    ('lodestone_tracker target', '{lodestone_tracker:{target:{dimension:"minecraft:the_nether",pos:[1,-64,3]},tracked:0b}}'),
    ('lodestone_tracker empty', '{lodestone_tracker:{}}'),
    ('firework_explosion', '{firework_explosion:{shape:"star",colors:[1,2],fade_colors:[3],has_trail:1b,has_twinkle:0b}}'),
    ('profile name', '{profile:{name:"Notch"}}'),
    ('profile full', '{profile:{name:"jeb_",id:[I;1,2,3,4],properties:[{name:"textures",value:"abc",signature:"sig"}]}}'),
    ('note_block_sound', '{note_block_sound:"minecraft:block.note_block.bell"}'),
    ('base_color', '{base_color:"lime"}'),
    ('sulfur_cube_content', '{sulfur_cube_content:{id:"minecraft:stone",count:1}}'),
    ('lock', '{lock:{items:"minecraft:diamond"}}'),
    ('container_loot', '{container_loot:{loot_table:"minecraft:chests/simple_dungeon",seed:5L}}'),
    ('waxed', '{waxed:{}}'),
    ('banner_patterns', '{banner_patterns:[{pattern:"minecraft:stripe_bottom",color:"red"},{pattern:{asset_id:"minecraft:x",translation_key:"y"},color:"blue"}]}'),
    ('pot_decorations', '{pot_decorations:{back:{id:"minecraft:brick"},left:{id:"minecraft:angler_pottery_sherd"},front:{id:"minecraft:skull_pottery_sherd",count:1}}}'),
    ('suspicious_stew_effects', '{suspicious_stew_effects:[{id:"minecraft:night_vision",duration:100},{id:"minecraft:poison"}]}'),
    ('potion_contents full', '{potion_contents:{potion:"minecraft:swiftness",custom_color:123,custom_effects:[{id:"minecraft:speed",amplifier:1b,duration:200,ambient:1b,show_particles:0b,show_icon:1b,hidden_effect:{id:"minecraft:speed",duration:100,hidden_effect:{id:"minecraft:speed",duration:50}}}],custom_name:"x"}}'),
    ('potion_contents bare', '{potion_contents:{}}'),
    ('container', '{container:[{slot:0,item:{id:"minecraft:stone",count:3}},{slot:5,item:{id:"minecraft:diamond_sword",components:{"minecraft:damage":5,"minecraft:trim":{material:"minecraft:gold",pattern:"minecraft:coast"}}}}]}'),
    ('bundle_contents', '{bundle_contents:[{id:"minecraft:stone",count:2},{id:"minecraft:potion",components:{"minecraft:potion_contents":{potion:"minecraft:water"}}}]}'),
    ('charged_projectiles', '{charged_projectiles:[{id:"minecraft:arrow"},{id:"minecraft:firework_rocket",components:{"minecraft:fireworks":{flight_duration:2b,explosions:[{shape:"burst",colors:[5]}]}}}]}'),
    ('block_state', '{block_state:{facing:"north",half:"top"}}'),
    ('bees', '{bees:[{entity_data:{id:"minecraft:bee",Health:5f},ticks_in_hive:5,min_ticks_in_hive:10}]}'),
    ('entity_data', '{entity_data:{id:"minecraft:pig",NoAI:1b}}'),
    ('bucket_entity_data', '{bucket_entity_data:{Health:5f}}'),
    ('instrument registry', '{instrument:"minecraft:sing_goat_horn"}'),
    ('instrument direct', '{instrument:{sound_event:{sound_id:"minecraft:x",range:8f},use_duration:3f,range:64f,description:"Horn"}}'),
    ('painting variant', '{"minecraft:painting/variant":"minecraft:kebab"}'),
    ('sign_text_front', '{sign_text_front:{messages:["a","b","c","d"],filtered_messages:["e","f","g","h"],color:"red",has_glowing_text:1b}}'),
    ('sign_text_back', '{sign_text_back:{messages:["a","","",""]}}'),
    ('map_decorations', '{map_decorations:{a:{type:"minecraft:player",x:1.0d,z:2.0d,rotation:0f}}}'),
    ('debug_stick_state', '{debug_stick_state:{"minecraft:oak_stairs":"facing"}}'),
    ('recipes', '{recipes:["minecraft:stone_bricks"]}'),
    ('attribute_modifiers', '{attribute_modifiers:[{type:"minecraft:attack_damage",id:"minecraft:x",amount:1.5d,operation:"add_value",slot:"mainhand",display:{type:"override",value:"hi"}},{type:"minecraft:armor",id:"minecraft:y",amount:2d,operation:"add_multiplied_total",display:{type:"hidden"}}]}'),
    ('consumable all effects', '{consumable:{consume_seconds:2f,animation:"drink",sound:{sound_id:"minecraft:x",range:4f},has_consume_particles:0b,on_consume_effects:[{type:"apply_effects",effects:[{id:"minecraft:speed",duration:20}],probability:0.5f},{type:"remove_effects",effects:"minecraft:poison"},{type:"remove_effects",effects:["minecraft:poison","minecraft:wither"]},{type:"clear_all_effects"},{type:"teleport_randomly",diameter:8f},{type:"play_sound",sound:"minecraft:entity.generic.eat"}]}}'),
    ('death_protection', '{death_protection:{death_effects:[{type:"clear_all_effects"}]}}'),
    ('equippable full', '{equippable:{slot:"head",equip_sound:"minecraft:item.armor.equip_gold",asset_id:"minecraft:gold",camera_overlay:"minecraft:misc/pumpkinblur",allowed_entities:"#minecraft:skeletons",dispensable:0b,swappable:0b,damage_on_hurt:0b,equip_on_interact:1b,can_be_sheared:1b,shearing_sound:{sound_id:"minecraft:y"}}}'),
    ('equippable bare', '{equippable:{slot:"chest"}}'),
    ('tool rules', '{tool:{rules:[{blocks:"#minecraft:mineable/pickaxe",speed:4f,correct_for_drops:1b},{blocks:["minecraft:stone"]}],default_mining_speed:2f,damage_per_block:2,can_destroy_blocks_in_creative:0b}}'),
    ('blocks_attacks full', '{blocks_attacks:{block_delay_seconds:0.5f,disable_cooldown_scale:2f,damage_reductions:[{horizontal_blocking_angle:45f,type:"#minecraft:is_fire",base:1f,factor:0.5f},{base:0f,factor:1f}],item_damage:{threshold:3f,base:1f,factor:2f},bypassed_by:"#minecraft:bypasses_shield",block_sound:"minecraft:item.shield.block",disable_sound:{sound_id:"minecraft:z"}}}'),
    ('damage_resistant', '{damage_resistant:{types:"#minecraft:is_fire"}}'),
    ('repairable list', '{repairable:{items:["minecraft:diamond","minecraft:emerald"]}}'),
    ('use_cooldown group', '{use_cooldown:{seconds:1.5f,cooldown_group:"minecraft:x"}}'),
    ('use_remainder', '{use_remainder:{id:"minecraft:bowl",count:2}}'),
    ('fireworks', '{fireworks:{flight_duration:3b,explosions:[{shape:"creeper",colors:[1],fade_colors:[2,3],has_trail:1b},{shape:"small_ball"}]}}'),
    ('provides_banner_patterns', '{provides_banner_patterns:"#minecraft:pattern_item/globe"}'),
    ('wolf variant', '{"minecraft:wolf/variant":"minecraft:ashen","minecraft:wolf/sound_variant":"minecraft:angry","minecraft:wolf/collar":"red"}'),
    ('fox salmon parrot', '{"minecraft:fox/variant":"snow","minecraft:salmon/size":"large","minecraft:parrot/variant":"blue"}'),
    ('tropical fish', '{"minecraft:tropical_fish/pattern":"flopper","minecraft:tropical_fish/base_color":"white","minecraft:tropical_fish/pattern_color":"black"}'),
    ('mooshroom rabbit', '{"minecraft:mooshroom/variant":"brown","minecraft:rabbit/variant":"gold"}'),
    ('pig cow chicken', '{"minecraft:pig/variant":"minecraft:cold","minecraft:pig/sound_variant":"minecraft:big","minecraft:cow/variant":"minecraft:warm","minecraft:cow/sound_variant":"minecraft:classic","minecraft:chicken/variant":"minecraft:cold","minecraft:chicken/sound_variant":"minecraft:classic"}'),
    ('nautilus frog horse', '{"minecraft:zombie_nautilus/variant":"minecraft:warm","minecraft:frog/variant":"minecraft:cold","minecraft:horse/variant":"chestnut"}'),
    ('llama axolotl cat', '{"minecraft:llama/variant":"brown","minecraft:axolotl/variant":"blue","minecraft:cat/variant":"minecraft:tabby","minecraft:cat/sound_variant":"minecraft:classic","minecraft:cat/collar":"blue"}'),
    ('sheep shulker villager', '{"minecraft:sheep/color":"pink","minecraft:shulker/color":"purple","minecraft:villager/variant":"minecraft:taiga"}'),
    ('trim material direct provider', '{provides_trim_material:{palette_id:"minecraft:x",description:"X"}}'),
    ('everything at once', '{damage:3,unbreakable:{},custom_name:"n",enchantments:{"minecraft:mending":1},trim:{material:"minecraft:gold",pattern:"minecraft:coast"},banner_patterns:[{pattern:"minecraft:globe",color:"red"}],pot_decorations:{right:{id:"minecraft:brick"}},lore:["x"],rarity:"epic"}'),
]
# Fixtures on another item than a stick (the components need it, or read better there).
ITEMS = {'profile name': 'player_head', 'profile full': 'player_head', 'trim registry': 'diamond_chestplate',
         'trim direct': 'diamond_chestplate', 'banner_patterns': 'white_banner', 'pot_decorations': 'decorated_pot',
         'everything at once': 'diamond_chestplate', 'instrument registry': 'goat_horn',
         'writable_book_content': 'writable_book', 'tooltip_style custom': 'stone', 'tooltip_style nested': 'stone'}


def compare(vanilla, rust):
    """Failures, stacks checked, and the component ids covered."""
    results = {r['label']: r for r in rust}
    failures, covered = [], set()
    for v in vanilla:
        if 'error' in v:
            failures.append(f"oracle {v['label']}: {v['error']}")
            continue
        covered.update(v['added'] + v['removed'])
        r = results.get(v['label'])
        if r is None or 'error' in r:
            failures.append(f"{v['label']}: {r and r['error']}")
        elif r['left'] or r['added'] != v['added'] or r['removed'] != v['removed']:
            failures.append(f"{v['label']}: {r['left']} bytes left, ids {r['added']}/{r['removed']}, vanilla {v['added']}/{v['removed']}")
        elif not r['same_bytes'] and not r['typed']:
            failures.append(f"{v['label']}: read past, but encoded back differently")
    missing = sorted(set(range(COMPONENT_TYPES)) - covered)
    if missing:
        failures.append(f'component ids no stack covers: {missing}')
    return failures, len(vanilla), covered


TOOLTIP_STYLE_WIRE_TEST = 'item_sync::tests::a_tooltip_style_crosses_the_wire'


def java_version_string():
    result = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def write_tooltip_style_witness(lock, jars, oracle, fixtures_path, vanilla_rows, output_path=None):
    """Write tooltip_style wire witness JSON and meta.json from VanillaItemWireOracle rows."""
    version = lock['minecraft']['version']
    wanted = [label for label, _ in FIXTURES if label.startswith('tooltip_style')]
    by_label = {row['label']: row for row in vanilla_rows}
    cases = []
    for label in wanted:
        row = by_label.get(label)
        if row is None or 'error' in row:
            raise AssertionError(f'Missing or failed oracle row for {label}: {row}')
        if len(row['added']) != 1 or row['removed']:
            raise AssertionError(f'Unexpected component patch for {label}: {row}')
        cases.append({
            'label': label,
            'item': row['item'],
            'item_id': row['item_id'],
            'component_id': row['added'][0],
            'tooltip_style': row['tooltip_style'],
            'hex': row['hex'],
        })
    dest = Path(output_path) if output_path is not None else (
        lab.LOCAL / 'witness' / version / 'item_wire' / 'tooltip_style_wire.json'
    )
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(json.dumps({
        'minecraft_version': version,
        'oracle': 'VanillaItemWireOracle',
        'cases': cases,
    }, indent=2) + '\n', encoding='utf-8')
    oracle_rel = oracle.relative_to(lab.ROOT).as_posix()
    lab.write_json(dest.parent / 'meta.json', {
        'game_version': version,
        'java_major': lock['minecraft']['java_major'],
        'java_version': java_version_string(),
        'harness': 'tools/test_item_wire_parity.py',
        'command': 'python3 tools/test_item_wire_parity.py',
        'oracle': oracle_rel,
        'witness_env': 'SUNRISE_TOOLTIP_STYLE_WIRE_WITNESS',
        'input_hashes': {
            'research-lock.json': lab.digest(lab.LOCK, 'sha256'),
            'server.jar': lab.digest(jars['server'], 'sha256'),
            oracle_rel: lab.digest(oracle, 'sha256'),
            'fixtures.tsv': lab.digest(fixtures_path, 'sha256'),
        },
        'outputs': {
            dest.name: {'sha256': lab.digest(dest, 'sha256'), 'bytes': dest.stat().st_size},
        },
    })
    return dest, len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--record-only', action='store_true',
                        help='Record 26.3 tooltip_style wire witness into .local/witness/<version>/item_wire/ using Java only')
    parser.add_argument('--record-tooltip-style', nargs='?', const='default', default=None,
                        help='Record 26.3 tooltip_style wire witness JSON using Java only (defaults to .local/witness/<version>/item_wire/tooltip_style_wire.json)')
    args = parser.parse_args()
    record_only = args.record_only or args.record_tooltip_style is not None
    build = lab.LOCAL / 'item-wire-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    if not record_only:
        report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaItemWireOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    dependency_path = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath',
         lab.ROOT / 'mods/sunrise/src/test/java', '-cp', str(jars['server']) + os.pathsep + dependency_path,
         '-d', classes, oracle], 'compile-java')
    fixtures = build / 'fixtures.tsv'
    fixtures.write_text(''.join(f'{label}\t{{id:"minecraft:{ITEMS.get(label, "stick")}",count:1,components:{snbt}}}\n'
                                for label, snbt in FIXTURES), encoding='utf-8')
    vanilla_path, rust_path = build / 'vanilla.jsonl', build / 'rust.jsonl'
    run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars['server']), dependency_path]),
         'dev.sunriselab.parity.VanillaItemWireOracle', fixtures, vanilla_path], 'oracle')
    read = lambda path: [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines() if line.strip()]
    vanilla_rows = read(vanilla_path)
    if record_only:
        dest = None if args.record_tooltip_style in (None, 'default') else Path(args.record_tooltip_style)
        witness, count = write_tooltip_style_witness(lock, jars, oracle, fixtures, vanilla_rows, dest)
        print(f'Recorded {count} tooltip_style wire cases for {lock["minecraft"]["version"]}: {witness}')
        return
    witness, _ = write_tooltip_style_witness(lock, jars, oracle, fixtures, vanilla_rows)
    run(['cargo', 'build', '--release', '-p', 'sunrise-protocol', '--example', 'item_wire_probe'], 'compile-rust')
    binary = lab.cargo_target() / 'release/examples' / ('item_wire_probe.exe' if os.name == 'nt' else 'item_wire_probe')
    run([binary, vanilla_path, rust_path], 'rust')
    failures, stacks, covered = compare(vanilla_rows, read(rust_path))
    for failure in failures[:40]:
        print(failure)
    if failures:
        raise SystemExit(f'{len(failures)} item wire mismatches (of {stacks} stacks)')
    wire_log = build / 'tooltip-style-wire-test.log'
    with wire_log.open('w', encoding='utf-8') as log:
        result = subprocess.run(
            ['cargo', 'test', '--locked', '-p', 'sunrise-sim', '--lib', '--', '--ignored', '--exact',
             TOOLTIP_STYLE_WIRE_TEST],
            cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT,
            env={**os.environ, 'SUNRISE_TOOLTIP_STYLE_WIRE_WITNESS': str(witness.resolve())},
        )
    if result.returncode != 0 or '1 passed; 0 failed' not in wire_log.read_text(encoding='utf-8'):
        tail = wire_log.read_text(encoding='utf-8', errors='replace').splitlines()[-40:]
        print('\n'.join(tail), file=sys.stderr, flush=True)
        raise SystemExit(f'{TOOLTIP_STYLE_WIRE_TEST} failed against {witness}; see {wire_log}')
    lab.write_json(report_path, {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(),
        'stacks': stacks, 'component_types_covered': len(covered),
        'oracle_sha256': lab.digest(vanilla_path, 'sha256'), 'rust_sha256': lab.digest(rust_path, 'sha256')})
    print(f'Item wire parity passed: {stacks} stacks, all {len(covered)} component types, tooltip_style witness ({witness}): {report_path}')


if __name__ == '__main__':
    main()
