#!/usr/bin/env python3
"""Compare the furnace block entity's tick with vanilla's AbstractFurnaceBlockEntity.

The Java oracle loads the vanilla data pack's real RecipeManager, puts a
furnace, blast furnace or smoker in a block-update level and calls the real
AbstractFurnaceBlockEntity.serverTick. The Rust probe runs the same furnace
plan and apply steps used by the game. Cases cover the burn table (every cooking_fuel item),
stack component additions, overrides and removals, including provider references,
the three kinds' 200/100/100 cook times, fuel stacks and the LIT state, the
RecipesUsed experience, full output slots, timer decay, and an input changed
mid-cook. `--no-change` leaves out the mid-cook family, reproducing the corpus
before it.
"""
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness

KINDS = ['furnace', 'blast_furnace', 'smoker']
KIND_RECIPE = {'furnace': 'minecraft:smelting', 'blast_furnace': 'minecraft:blasting',
               'smoker': 'minecraft:smoking'}


def stack(item, count=1, components=None):
    patch = ',components:' + components if components else ''
    return '{id:"%s",count:%d%s}' % (item, count, patch)


def fuel_stacks():
    return {
        'added': stack('minecraft:diamond', components='{"minecraft:cooking_fuel":{burn_time:407,speed_multiplier:2.5f}}'),
        'removed': stack('minecraft:coal', components='{"!minecraft:cooking_fuel":{}}'),
        'overridden': stack('minecraft:coal', components='{"minecraft:cooking_fuel":{burn_time:323,speed_multiplier:1.25f}}'),
        'zero_time': stack('minecraft:diamond', components='{"minecraft:cooking_fuel":{burn_time:0,speed_multiplier:2.5f}}'),
        'negative_time': stack('minecraft:diamond', components='{"minecraft:cooking_fuel":{burn_time:-5,speed_multiplier:1.25f}}'),
        'both_set_wins': stack('minecraft:diamond', components='{"minecraft:cooking_fuel":{burn_time:31,speed_multiplier:1.25f},"!minecraft:cooking_fuel":{}}'),
        'both_removal_wins': stack('minecraft:coal', components='{":cooking_fuel":{burn_time:31,speed_multiplier:1.25f},"!:cooking_fuel":{}}'),
        'default_fuel': stack('minecraft:coal'),
        'default_nonfuel': stack('minecraft:diamond'),
    }


def fcase(kind, ticks, inv, fuel, out='-', timer=0, total=0, lit=0, littotal=0,
          speed=1.0, change_at=-1, new='-'):
    return 'furnace %s %d %s %s %s %d %d %d %d %s %d %s' % (
        kind, ticks, inv, fuel, out, timer, total, lit, littotal, speed, change_at, new)


def recipes(resources):
    out = {}
    for kind, recipe_type in KIND_RECIPE.items():
        found = []
        for path in sorted((resources / 'data/minecraft/recipe').rglob('*.json')):
            data = json.loads(path.read_text())
            if data.get('type') != recipe_type:
                continue
            ing = data.get('ingredient')
            if isinstance(ing, dict):
                ing = ing.get('item') or ing.get('id')
            result = data.get('result')
            rid = result.get('id') if isinstance(result, dict) else result
            if not ing or not rid:
                continue
            found.append((ing, rid, int(data.get('cookingtime', 200)),
                          float(data.get('experience', 0.0))))
        out[kind] = found
    return out


def make_cases(build):
    resources = lab.RESOURCES
    catalog = json.loads((resources / 'items.json').read_text())
    all_recipes = recipes(resources)
    fuels = sorted(item for item, data in catalog.items()
                   if 'minecraft:cooking_fuel' in data)
    coal = stack('minecraft:coal')
    # Patched fuels all start with a matching recipe. Observe ignition, speed,
    # time remaining and recipe completion, without deriving any expected values here.
    iron = stack('minecraft:iron_ore')
    cases = [fcase('furnace', ticks, iron, fuel, total=200)
             for fuel in fuel_stacks().values() for ticks in (1, 200)]
    # Reuse a real registry reference on a different item, in both furnace contexts.
    named = json.dumps({'minecraft:cooking_fuel': catalog['minecraft:coal']['minecraft:cooking_fuel']},
                       separators=(',', ':'))
    for kind in KINDS:
        ing, _, cook, _ = all_recipes[kind][0]
        cases.append(fcase(kind, 1, stack(ing), stack('minecraft:diamond', components=named), total=cook))
    fixture = build / 'fixture-pack/data/test/context_float_provider'
    fixture.mkdir(parents=True, exist_ok=True)
    definitions = {
        'fraction': {'type': 'minecraft:div', 'left': 3, 'right': 2},
        'conditional': {'type': 'minecraft:conditional',
                        'condition': {'type': 'minecraft:match_block', 'blocks': 'minecraft:furnace',
                                      'state': {'lit': 'false'}, 'nbt': '{lit_time_remaining:0}'},
                        'on_true': 'test:fraction', 'on_false': 2},
    }
    for name, definition in definitions.items():
        (fixture / f'{name}.json').write_text(json.dumps(definition))
    for kind in KINDS:
        ing, _, cook, _ = all_recipes[kind][0]
        for name in definitions:
            patch = '{"minecraft:cooking_fuel":{burn_time:31,speed_multiplier:"test:%s"}}' % name
            cases.append(fcase(kind, 1, stack(ing), stack('minecraft:diamond', components=patch), total=cook)
                         + ' fixture')
    patch = '{"minecraft:cooking_fuel":{burn_time:31,speed_multiplier:"test:conditional"}}'
    cases.append(fcase('furnace', 1, iron, stack('minecraft:diamond', components=patch),
                       timer=20, total=200, lit=1, littotal=80) + ' fixture')
    int_fixture = fixture.with_name('context_int_provider')
    int_fixture.mkdir(exist_ok=True)
    for n, (left, right) in enumerate([(-(2**31), -1), (-5, 3), (5, -3), (-5, -3)]):
        (int_fixture / f'floor_mod_{n}.json').write_text(json.dumps(
            {'type': 'minecraft:floor_mod', 'left': left, 'right': right}))
        patch = '{"minecraft:cooking_fuel":{burn_time:"test:floor_mod_%d",speed_multiplier:1.25f}}' % n
        cases.append(fcase('furnace', 1, iron, stack('minecraft:diamond', components=patch), total=200)
                     + ' fixture')

    # Each file is a valid direct provider definition, referring to the next holder.
    for folder, terminal in [(int_fixture, 29), (fixture, 1.75)]:
        deep = folder / 'deep'
        deep.mkdir(exist_ok=True)
        for n in range(80):
            value = terminal if n == 79 else f'test:deep/{n + 1}'
            (deep / f'{n}.json').write_text(json.dumps({'type': 'minecraft:add', 'inputs': [value]}))
    patch = '{"minecraft:cooking_fuel":{burn_time:"test:deep/0",speed_multiplier:"test:deep/0"}}'
    cases.append(fcase('furnace', 1, iron, stack('minecraft:diamond', components=patch), total=200)
                 + ' fixture')

    # For seed zero, the first uniform probability is smaller than the next
    # draw. Swapping their order changes the selected branch; omitting either
    # draw changes the subsequent RNG sample. A condition within the chance
    # holder also needs the same furnace block context.
    (fixture / 'chance.json').write_text(json.dumps({
        'type': 'minecraft:conditional',
        'condition': {'type': 'minecraft:match_block', 'blocks': 'minecraft:furnace'},
        'on_true': {'type': 'minecraft:uniform', 'min': 0, 'max': 1},
        'on_false': 1,
    }))
    (fixture / 'rng_order.json').write_text(json.dumps({
        'type': 'minecraft:conditional',
        'condition': {'type': 'minecraft:random_chance', 'chance': 'test:chance'},
        'on_true': 1.25, 'on_false': 2.5,
    }))
    patch = '{"minecraft:cooking_fuel":{burn_time:31,speed_multiplier:"test:rng_order"}}'
    cases.append(fcase('furnace', 1, iron, stack('minecraft:diamond', components=patch), total=200)
                 + ' fixture rng')
    missing = '{"minecraft:cooking_fuel":{burn_time:31,speed_multiplier:"test:absent"}}'
    cases.append(fcase('furnace', 1, iron, stack('minecraft:diamond', components=missing), total=200))

    # One recipe per kind, at its cook-time boundaries.
    for kind in KINDS:
        ing, result, cook, _ = all_recipes[kind][0]
        for ticks in (0, 1, cook - 1, cook, cook + 1, 2 * cook):
            cases.append(fcase(kind, ticks, stack(ing), coal, total=cook))
        # XP accumulates per recipe and per item.
        cases.append(fcase(kind, 3 * cook + 1, stack(ing, 3), coal, total=cook))
        cases.append(fcase(kind, 3 * cook + 2, stack(ing, 3), coal, total=cook))

    # Every cooking_fuel item: burn time and speed multiplier after one/two ticks.
    for fuel in fuels:
        cases.append(fcase('furnace', 1, stack('minecraft:iron_ore'), stack(fuel), total=200))
        cases.append(fcase('furnace', 2, stack('minecraft:iron_ore'), stack(fuel), total=200))
    cases.append(fcase('furnace', 1, stack('minecraft:iron_ore'),
                       '{id:"minecraft:coal",count:2}', total=200))
    cases.append(fcase('furnace', 2, stack('minecraft:iron_ore'),
                       '{id:"minecraft:coal",count:2}', total=200))

    # LIT changes at the end of the burn, and fuel is consumed once.
    for ticks in (1599, 1600, 1601, 1602):
        cases.append(fcase('furnace', ticks, stack('minecraft:iron_ore'), coal, total=200))

    # A full output stack cannot take another result; the timer decays.
    cases.append(fcase('furnace', 1, stack('minecraft:iron_ore'),
                       stack('minecraft:coal'), stack('minecraft:iron_ingot', 63), total=200))
    cases.append(fcase('furnace', 200, stack('minecraft:iron_ore'),
                       stack('minecraft:coal'), stack('minecraft:iron_ingot', 63), total=200))
    for ticks in (1, 5, 10):
        cases.append(fcase('furnace', ticks, '-', '-', '-', timer=10, total=200))

    # The mid-cook family: a new input resets total and timer, the same input
    # does not, and an empty input stops the recipe.
    if '--no-change' not in sys.argv:
        for kind in KINDS:
            ing, result, cook, _ = all_recipes[kind][0]
            other = next((r for r in all_recipes[kind] if r[0] != ing), None)
            if not other:
                continue
            new = stack(other[0])
            for ticks in (50, 51, cook, cook + 1):
                cases.append(fcase(kind, ticks, stack(ing), coal, timer=50, total=cook,
                                   change_at=50, new=new))
            for ticks in (1, cook, cook + 1):
                cases.append(fcase(kind, ticks, stack(ing), coal, change_at=0, new=new))
            cases.append(fcase(kind, 51, stack(ing), coal, timer=50, total=cook,
                               change_at=50, new=stack(ing)))
            cases.append(fcase(kind, 51, stack(ing), coal, timer=50, total=cook,
                               change_at=50, new='-'))
        # A change exactly at the end of the run.
        ing, result, cook, _ = all_recipes['furnace'][0]
        cases.append(fcase('furnace', 1, stack(ing), coal, total=cook, change_at=1,
                           new=stack('minecraft:gold_ore')))

    path = build / 'cases.txt'
    hopper = lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/blocks/machines/furnace_hopper.tsv'
    path.write_text(f'#resources {resources}\n#hopper {hopper}\n' + ''.join(c + '\n' for c in cases))
    return path, len(cases)


if __name__ == '__main__':
    harness.differential(
        'furnace-parity', 'VanillaFurnaceOracle', 'furnace_fixture', make_cases,
        'Furnace block entity tick: default and patched cooking_fuel, 200/100/100 '
        'cook times, fuel stack and LIT state, RecipesUsed experience, full output and '
        'timer decay, and input changed mid-cook. --no-change leaves out the change family.',
        sources=[lab.ROOT / 'crates/sunrise-level/src/chunk/block_entity.rs',
                 lab.ROOT / 'crates/sunrise-level/src/providers.rs',
                 lab.ROOT / 'crates/sunrise-level/src/providers/evaluation.rs',
                 lab.ROOT / 'crates/sunrise-client/src/cut_layout/game/blocks/machines/furnace_hopper.tsv',
                 lab.ROOT / 'crates/sunrise-items/src/item_data/cooking_fuel.rs',
                 lab.ROOT / 'crates/sunrise-items/src/properties/resolvable.rs',
                 lab.ROOT / 'crates/sunrise-sim/src/furnace.rs',
                 lab.ROOT / 'crates/sunrise-sim/src/crafting.rs'])
