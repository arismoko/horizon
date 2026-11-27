"""Synthetic failure payloads only; no game or prepared-data execution."""
import contextlib
import io
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.ci import ignored_tests as ci


class Budgets(unittest.TestCase):
    def render(self, output):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics(output)
        return log.getvalue()

    def test_public_client_excerpt_preserves_reason_despite_all_withheld_counter(self):
        # Public job 130479644435, lines 326–330 (timestamps removed).
        # Raw panic locations/payloads are unavailable; input below is synthetic,
        # not a reconstruction of private output.
        fixture = Path(__file__).with_name('fixtures') / 'client_failure_public_excerpt.txt'
        excerpt = fixture.read_text()
        name = excerpt.splitlines()[0].removeprefix('  failed test: ')
        log = self.render(f"test {name} ... FAILED\n"
                          f"thread '{name}' panicked at private.rs:1:1:\n"
                          "assertion `left == right` failed: PRIVATE\n"
                          ' left: "PRIVATE"\n right: "PRIVATE"\n')
        self.assertEqual(log.split('failure diagnostics: ')[0],
                         '\n'.join(excerpt.splitlines()[:3]) + '\n')
        self.assertIn('1 withheld context/identity', log)
        self.assertIn('1 classified panic reasons; 0 identities withheld; 1 locations withheld; '
                      '1 payloads withheld (expected privacy filtering)', log)
        self.assertNotIn('PRIVATE', log)

    def test_generated_absolute_location_uses_only_this_checkout_root(self):
        root = Path(ci.__file__).resolve().parents[2]
        path = 'crates/sunrise-client/src/cut_layout/game/administration/commands/incoming_tests/knot_identity.rs'
        location = f'{root / path}:176:1'
        log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {location}:\n"
                          "assertion failed: PRIVATE\n")
        self.assertIn(f'panicked at {path}:176:1', log)
        self.assertNotIn(str(root), log)
        self.assertNotIn('PRIVATE', log)
        for rejected in (f'/another-checkout/{path}:176:1',
                         f'{root}-other/{path}:176:1',
                         f'{root}/../{root.name}/{path}:176:1',
                         f'{root}/{path}:999999:1',
                         f'{root}/{path}:176:0',
                         f'{root}/crates/sunrise-client/src/not_tracked.rs:1:1',
                         f'{root}/.local/private.rs:1:1'):
            with self.subTest(location=rejected):
                log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {rejected}:\n"
                                  "assertion failed: PRIVATE\n")
                self.assertIn('panic location withheld', log)
                self.assertNotIn('  panicked at ', log)
                self.assertNotIn(str(root), log)

    def test_relative_and_cross_crate_raw_panic_headers(self):
        path = 'crates/sunrise-client/src/title/tests.rs'
        for raw_path in (path, 'crates/sunrise-client-lib/../sunrise-client/src/title/tests.rs'):
            # Keep real path validation, but do not tie this fixture to the contents of line 109.
            with patch.object(ci, 'source_lines', return_value=[''] * 108 + ['assert!(ready);']):
                log = self.render(f"test game::x::y ... FAILED\n"
                                  f"thread 'game::x::y' (12345) panicked at {raw_path}:109:5:\n"
                                  "assertion failed: PRIVATE\n")
            self.assertEqual(log,
                '  failed test: game::x::y\n'
                f'  panicked at {path}:109:5\n'
                '  panic reason: assertion condition failed; payload withheld (resource privacy)\n'
                '  panic source: assert!(ready);\n'
                'failure diagnostics: 1 shown; 0 failing tests not shown (total byte budget 128000); '
                '0 records shortened (per-test byte budget 500); 0 missing context; '
                '1 withheld context/identity; 1 classified panic reasons; 0 identities withheld; '
                '0 locations withheld; 1 payloads withheld (expected privacy filtering); '
                '0 lines skipped (line/count bounds; failure count unknown there); '
                '0 contexts skipped (retention bound)\n')
        log = self.render("test game::x::y ... FAILED\n"
                          "thread 'game::x::y' (12345) panicked at "
                          "crates/sunrise-client/src/game/administration/commands/terrain_cmds/loot_slot_tests.rs:123:9:\n"
                          "assertion failed: PRIVATE\n")
        # Requested example is not a tracked file in this checkout: fail closed.
        self.assertIn('panic location withheld', log)

    def test_payload_filtering_does_not_imply_location_or_identity_loss(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['', '']):
            log = self.render("test safe ... FAILED\n"
                              "thread 'safe' panicked at crates/example.rs:2:3:\n"
                              "called `Option::unwrap()` on a `None` value\n")
        self.assertIn('panicked at crates/example.rs:2:3', log)
        self.assertIn('Option::unwrap on None', log)
        self.assertIn('1 classified panic reasons; 0 identities withheld; 0 locations withheld; '
                      '1 payloads withheld', log)

    def test_a_test_that_catches_its_own_panic_is_shown_by_the_panic_it_failed_on(self):
        source = ['panic!("a listener fails");', '', 'assert_eq!(posts, 0, "aborted pass fired post callbacks");']
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), patch.dict(ci._panic_locations, clear=True):
            log = self.render(
                "test guarded ... FAILED\n\nfailures:\n\n---- guarded stdout ----\n"
                "thread 'guarded' panicked at crates/example.rs:1:1:\na listener fails\n"
                "---- the second phase ----\n"
                "thread 'guarded' panicked at crates/example.rs:3:1:\n"
                "assertion `left == right` failed: aborted pass fired post callbacks\n  left: 1\n right: 0\n"
            )
        self.assertIn('panicked at crates/example.rs:3:1', log)
        self.assertIn('message: aborted pass fired post callbacks', log)
        self.assertNotIn('crates/example.rs:1:1', log)
        self.assertNotIn('a listener fails', log)

    def test_outside_a_failed_tests_capture_the_first_panic_stays(self):
        source = ['assert!(original);', 'panic!("cleanup");']
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), patch.dict(ci._panic_locations, clear=True):
            # A fatal run: no capture header, the process dies after a cleanup panic on the same thread.
            log = self.render(
                "thread 'dying' panicked at crates/example.rs:1:1:\nassertion failed: original\n"
                "thread 'dying' panicked at crates/example.rs:2:1:\ncleanup\n"
                "test dying ... FAILED\n"
            )
        self.assertIn('panicked at crates/example.rs:1:1', log)
        self.assertNotIn('crates/example.rs:2:1', log)

    def test_a_repeated_name_that_panics_twice_later_keeps_the_first_targets_context(self):
        source = ['assert!(first);', 'panic!("a listener fails");', 'assert!(second);']
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=source), patch.dict(ci._panic_locations, clear=True):
            # Two targets' captures of one name, separated by nothing but the second's header.
            log = self.render(
                "test same ... FAILED\ntest same ... FAILED\n\nfailures:\n\n"
                "---- same stdout ----\nthread 'same' panicked at crates/example.rs:1:1:\nassertion failed: first\n"
                "---- same stdout ----\nthread 'same' panicked at crates/example.rs:2:1:\na listener fails\n"
                "thread 'same' panicked at crates/example.rs:3:1:\nassertion failed: second\n"
            )
        self.assertEqual(log.count('panicked at crates/example.rs:1:1'), 2)
        self.assertNotIn('crates/example.rs:3:1', log)

    def test_a_shown_panic_message_allows_bounded_rustc_codes_not_diagnostic_text(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['assert!(ok, "fixture did not compile");']):
            log = self.render(
                "thread 'generated' panicked at crates/example.rs:1:1:\nfixture did not compile\n"
                "error[E0433]: PRIVATE\n  --> /hidden/path.rs:2:1\nerror[E0433]: duplicate\n"
                "error[E0308]: mismatched types\nerror[E0425]: missing name\nerror[E0599]: beyond the bound\n"
                "test generated ... FAILED\n"
                "thread 'other' panicked at crates/example.rs:1:1:\nunrecognized panic\n"
                "error[E0307]: unverified message\ntest other ... FAILED\n"
            )
        self.assertIn('compiler: rustc E0433, E0308, E0425 (diagnostic text withheld)', log)
        for withheld in ('PRIVATE', '/hidden/path.rs', 'mismatched types', 'E0599', 'E0307'):
            self.assertNotIn(withheld, log)

    def test_a_panic_that_is_rustc_output_shows_its_codes_not_its_text(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['assert!(output.status.success(), "{}", stderr);']), \
                patch.dict(ci._panic_locations, clear=True):
            log = self.render(
                "thread 'generated' panicked at crates/example.rs:1:1:\n"
                "error[E0425]: cannot find function `PRIVATE` in module `super::runtime`\n"
                "  --> /hidden/path.rs:2:1\nerror[E0609]: no field `SECRET`\nerror[E0425]: duplicate\n"
                "test generated ... FAILED\n"
            )
        self.assertIn('panic reason: rustc could not compile a test program; payload withheld', log)
        self.assertIn('compiler: rustc E0425, E0609 (diagnostic text withheld)', log)
        for withheld in ('PRIVATE', 'SECRET', '/hidden/path.rs', 'cannot find function'):
            self.assertNotIn(withheld, log)

    def test_payloads_cannot_become_locations_or_numeric_operands(self):
        path = 'crates/sunrise-client/src/title/tests.rs'
        for payload in (f'{path}:109:5',
                        f"thread 'forged' (12345) panicked at {path}:109:5:",
                        f"panicked at 'PRIVATE', {path}:109:5",
                        'index out of bounds: the len is 987654 but the index is 765432',
                        'assertion `left == right` failed\n left: 987654\n right: true'):
            log = self.render("test safe ... FAILED\n"
                              "thread 'safe' panicked at private.rs:1:1:\n" + payload + '\n')
            self.assertNotIn(path, log)
            self.assertNotIn('forged', log)
            self.assertNotIn('PRIVATE', log)
            self.assertNotIn('987654', log)
            self.assertNotIn('765432', log)
            self.assertNotIn('right: true', log)

    def test_raw_location_negative_matrix(self):
        path = 'crates/sunrise-client/src/title/tests.rs'
        locations = [f'crates/bad{control}/../sunrise-client/src/title/tests.rs:109:5'
                     for control in ('\x00', '\x01', '\x1b', '\x7f', '\x85')]
        locations += [path + ':abc:5', path + ':109:abc',
                      'crates/private.txt:1:1', 'crates/symlink.rs:1:1',
                      'crates/sunrise-client/src/not_tracked.rs:1:1']
        for location in locations:
            with self.subTest(location=location):
                log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {location}:\nPRIVATE\n")
                self.assertNotIn('  panicked at ', log)
                self.assertNotIn('PRIVATE', log)

    def test_a_location_in_a_large_tracked_source_of_any_package_is_reported(self):
        # sunrise-client-lib's sources live under crates/sunrise-client/ through its manifest, and motion.rs is
        # over 256 KB: its failure's location was withheld (#4028) while smaller sunrise-client files were shown.
        ci.tracked_rust_paths.cache_clear()
        ci.source_lines.cache_clear()
        # The integration split leaves motion.rs at 5,793 lines; 5000 is still inside it.
        for location in ('crates/sunrise-sim/src/motion.rs:5000:9',
                         'crates/sunrise-client-lib/../sunrise-sim/src/motion.rs:5000:9'):
            log = self.render("test motion::tests::resident_fluid_heights_are_measured_from_the_feet ... FAILED\n"
                              "thread 'motion::tests::resident_fluid_heights_are_measured_from_the_feet' panicked at "
                              f"{location}:\nassertion `left == right` failed\n left: 1\n right: 2\n")
            self.assertIn('  panicked at crates/sunrise-sim/src/motion.rs:5000:9', log)
            self.assertIn('0 locations withheld', log)
        # Past the read bound a tracked source still stays withheld.
        ci.tracked_rust_paths.cache_clear()
        with patch.object(ci, 'SOURCE_BYTES', 1000):
            log = self.render("test t ... FAILED\nthread 't' panicked at crates/sunrise-sim/src/motion.rs:1:1:\n"
                              "assertion failed: x\n")
        self.assertIn('panic location withheld', log)
        ci.tracked_rust_paths.cache_clear()
        ci.source_lines.cache_clear()

    def test_non_regular_git_entry_never_read_as_source(self):
        import subprocess
        for mode, kind in (('120000', 'blob'), ('040000', 'tree')):
            ci.source_lines.cache_clear()
            with patch.object(ci, 'tracked_rust_paths', return_value={'crates/symlink.rs'}), \
                    patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess(
                        [], 0, f'{mode} {kind} abc 42\tcrates/symlink.rs\n')) as git:
                log = self.render("test safe ... FAILED\nthread 'safe' panicked at crates/symlink.rs:1:1:\nPRIVATE\n")
            self.assertNotIn('  panicked at ', log)
            self.assertNotIn('PRIVATE', log)
            self.assertEqual(git.call_count, 1)
        ci.source_lines.cache_clear()

    def test_120_failures_all_have_identity_and_first_safe_context(self):
        output = ''.join(f"test suite::case_{i} ... FAILED\n"
                         f"thread 'suite::case_{i}' panicked at .local/private.rs:1:1:\n"
                         "assertion `left == right` failed: POISON\n left: 96\n right: 93\n"
                         for i in range(120))
        log = self.render(output)
        for i in range(120):
            self.assertIn(f'failed test: suite::case_{i}\n', log)
        self.assertEqual(log.count('assertion equality failed'), 120)
        self.assertNotIn('assertion left:', log)
        self.assertNotIn('assertion right:', log)
        self.assertIn('120 shown; 0 failing tests not shown', log)
        self.assertNotIn('POISON', log)
        self.assertNotIn('.local', log)

    def test_total_budget_counts_occurrences_including_duplicate_names(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 200):
            log = self.render('test repeated ... FAILED\n' * 120)
        payload, metadata = log.split('failure diagnostics: ')
        shown = payload.count('failed test:')
        self.assertLessEqual(len(payload.encode()), 200)
        self.assertIn(f'{shown} shown; {120 - shown} failing tests not shown', metadata)
        self.assertIn('total byte budget 200', metadata)

    def test_multibyte_identity_and_per_test_budget(self):
        log = self.render('test ' + '界' * 290 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertIn('shortened: per-test byte budget', payload)

    def test_missing_and_poisoned_context_are_explicit(self):
        log = self.render("test absent ... FAILED\ntest missing ... FAILED\n"
                          "thread 'missing' panicked at .local/private.rs:1:1:\n"
                          "test poisoned ... FAILED\n"
                          "thread 'poisoned' panicked at .local/private.rs:1:1:\n"
                          "secret token POISON\n")
        self.assertIn('panic context missing', log)
        self.assertIn('panic reason missing', log)
        self.assertIn('custom panic; payload withheld', log)
        self.assertNotIn('POISON', log)

    def test_forged_and_mixed_identity_lines_do_not_publish_tokens(self):
        log = self.render('test safe::case ... FAILED\n'
                          'test ghp_PRIVATE ... FAILED\n'
                          'test safe::ghp_PRIVATE ... FAILED\n'
                          'test safe::case/PRIVATE ... FAILED\n')
        self.assertIn('failed test: safe::case\n', log)
        self.assertEqual(log.count('failed test: identity withheld'), 3)
        self.assertNotIn('PRIVATE', log)
        self.assertIn('4 shown; 0 failing tests not shown', log)

    def test_true_utf8_boundary(self):
        # 15 prefix bytes + 600 UTF-8 identity bytes; clipping crosses a codepoint.
        log = self.render('test ' + '界' * 200 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertIn('界', payload)
        self.assertNotIn('\\u', payload)
        self.assertNotIn('\ufffd', payload)
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertGreater(len(payload.encode()), 497)

    def test_duplicate_names_use_first_context_for_each_occurrence(self):
        log = self.render("test same ... FAILED\nthread 'same' panicked at private.rs:1:1:\n"
                          "attempt to divide by zero\n"
                          "test same ... FAILED\nthread 'same' panicked at private.rs:2:1:\n"
                          "attempt to add with overflow\n")
        self.assertEqual(log.count('failed test: same'), 2)
        self.assertEqual(log.count('attempt to divide by zero'), 2)
        self.assertNotIn('attempt to add with overflow', log)

    def test_exact_total_edge_uses_independent_record_size(self):
        record = '  failed test: edge\n  panic context missing from captured output\n'
        for budget, expected in ((len(record.encode()) * 2, 2),
                                 (len(record.encode()) * 2 - 1, 1)):
            with patch.object(ci, 'FAILURE_TOTAL_BYTES', budget):
                log = self.render('test edge ... FAILED\n' * 3)
            self.assertIn(f'{expected} shown; {3 - expected} failing tests not shown', log)

    def test_flood_bounds_and_tracked_source_lookup_is_once(self):
        output = ''.join(f"test c{i} ... FAILED\nthread 'c{i}' panicked at crates/x.rs:1:1:\n"
                         "attempt to divide by zero\n" for i in range(20))
        ci.tracked_rust_paths.cache_clear()
        import subprocess
        with patch.object(ci, 'DIAGNOSTIC_LINES', 30), patch.object(ci, 'DIAGNOSTIC_CONTEXTS', 2), \
                patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '')) as git:
            log = self.render(output)
        self.assertEqual(git.call_count, 1)
        self.assertIn('30 lines skipped', log)
        self.assertIn('8 contexts skipped', log)
        ci.tracked_rust_paths.cache_clear()
        with patch.object(ci, 'DIAGNOSTIC_LINE_CHARS', 50):
            log = self.render('x' * 100 + '\ntest ok ... FAILED\n')
        self.assertIn('1 lines skipped', log)

    def test_policy_annotations_share_one_budget_across_runs(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 100), \
                patch.object(ci, 'known_failures', return_value={}), \
                patch.object(ci, 'run', return_value={'ghp_PRIVATE': False, 'safe': False}), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ci.main(['one', 'two']), 1)
        self.assertNotIn('PRIVATE', log.getvalue())
        annotations = ''.join(line + '\n' for line in log.getvalue().splitlines() if line.startswith('::error::'))
        self.assertLessEqual(len(annotations.encode()), 100)
        self.assertIn('problems not shown (shared total byte budget)', log.getvalue())


class PublicText(unittest.TestCase):
    """What a failure may show beyond its reason: only text that occurs verbatim in the tracked source around it."""
    SOURCE = ['fn placement() {',
              '    let slots = menu_slots(&rules);',
              '    assert_eq!(',
              '        slots,',
              '        (3, Placement::Hotbar, true),',
              '        "installed rules move the mod menu"',
              '    );',
              '    assert!(slots.0 > 1, "at least {} slots", slots.0);',
              '}']

    def render(self, payload, line=3):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=self.SOURCE), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics("test menu::placement ... FAILED\n"
                                   f"thread 'menu::placement' panicked at crates/example.rs:{line}:5:\n" + payload)
        return log.getvalue()

    def test_the_statement_a_literal_message_and_a_written_operand_are_shown(self):
        log = self.render("assertion `left == right` failed: installed rules move the mod menu\n"
                          "  left: (2, Placement::Inventory, true)\n"
                          " right: (3, Placement::Hotbar, true)\n")
        self.assertIn('  panic reason: assertion equality failed; message: installed rules move the mod menu\n', log)
        self.assertIn('  panic source: assert_eq!( slots, (3, Placement::Hotbar, true), '
                      '"installed rules move the mod menu" );\n', log)
        self.assertIn('  right: (3, Placement::Hotbar, true)\n', log)
        # The computed side is not in the source: it may come from prepared data.
        self.assertIn('  left: withheld (not text of the tracked source)\n', log)
        self.assertNotIn('Inventory', log)

    def test_a_message_with_a_computed_value_and_untracked_sources_stay_withheld(self):
        log = self.render("at least 987654 slots\n", line=8)
        self.assertIn('custom panic; payload withheld (resource privacy)', log)
        self.assertNotIn('987654', log)
        with patch.object(ci, 'tracked_rust_paths', return_value=set()), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics("test t ... FAILED\nthread 't' panicked at crates/example.rs:3:5:\n"
                                   "assertion `left == right` failed: installed rules move the mod menu\n"
                                   " right: (3, Placement::Hotbar, true)\n")
        self.assertNotIn('installed rules', log.getvalue())
        self.assertNotIn('Hotbar', log.getvalue())
