#!/usr/bin/env python3
"""Option C, engine side, mechanical part: delete what exists only for internals points (listed
call sites, slot lists, their fixtures and CI tools) and take the function-replacement staging
files from the `no-points` branch as they are. Hand fixes follow in separate commits.

    python3 tools/refactors/api_points_core.py [NO_POINTS_REF]
"""
import subprocess
import sys

REF = sys.argv[1] if len(sys.argv) > 1 else "origin/no-points"

DELETE = [
    "crates/horizon-mod-api-gen/slots",
    "crates/horizon-mod-api-gen/src/sites",
    "crates/horizon-mod-api-gen/src/sites.rs",
    "crates/horizon-mod-api-gen/src/internals_table.rs",
    "crates/horizon-mod-api-gen/tests/knockback_entries.rs",
    "crates/horizon-mod-api-gen/tests/knockback_fixture",
    "crates/horizon-mod-api-gen/tests/return_authority.rs",
    "crates/horizon-mod-api-macros/tests/site.rs",
    "crates/horizon-mod-points/tests/replace_internals.rs",
    "crates/horizon-mod-points/tests/sites.rs",
    ".github/workflows/mixins-check.yml",
    "tests/test_ci_stage_slots.py",
    "tools/ci/listed_calls.py",
    "tools/ci/mixins-check",
    "tools/ci/mixins_area_execution.py",
    "tools/ci/mixins_check.sh",
    "tools/ci/slot_inputs.py",
    "tools/ci/stage_slots.py",
    "tools/ci/test_mixins_area_execution.py",
    "tools/fixtures/native-slot-order",
    "tools/ci/fn-replace-check/lower/src/mixins/advance.rs",
]

# Function replacement as `no-points` stages and checks it.
TAKE = [
    "crates/horizon-mod-api-gen/replaceable.txt",
    "crates/horizon-mod-api-gen/src/replace.rs",
    "crates/horizon-mod-api-gen/src/source.rs",
    "crates/horizon-mod-api-gen/src/patch.rs",
    "tools/ci/fn-replace-check",
    "tools/ci/fn_replace_check.sh",
    "tools/ci/stage_replacements.py",
    "tools/fn_replace/bench.rs",
    "crates/horizon-client-render/fixtures/standard_pack_gpu_entities.rs",
    "crates/horizon-client-render/fixtures/world_views.rs",
    "tools/run_standard_pack_gpu_entities.py",
    "tools/run_world_views.py",
]

subprocess.run(["git", "rm", "-rq", "--", *DELETE], check=True)
subprocess.run(["git", "checkout", REF, "--", *TAKE], check=True)
