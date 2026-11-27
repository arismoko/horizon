"""lab.parity_fixtures() builds the parity fixtures' binary and runs only it, never another Cargo artifact."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import lab


def artifact(name, executable, kind='bin'):
    return json.dumps(dict(reason='compiler-artifact', executable=str(executable), target=dict(name=name, kind=[kind])))


class ParityFixturesBuildTests(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.dict(lab._PARITY_FIXTURES, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.target = lab.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.target)
        env = mock.patch.dict(os.environ, CARGO_TARGET_DIR=str(self.target))
        env.start()
        self.addCleanup(env.stop)
        self.uplifted = self.target / 'debug' / 'sunrise-parity-fixtures'
        self.uplifted.parent.mkdir()

    def cargo(self, *others):
        """A fake `cargo build` that, as Cargo does, writes the variant it was asked for to the one uplifted path."""
        def run(command, **kwargs):
            features = command[command.index('--features') + 1]
            self.uplifted.unlink(missing_ok=True)
            self.uplifted.write_text(features)
            rows = [*others, artifact('sunrise-parity-fixtures', self.uplifted),
                    json.dumps(dict(reason='build-finished', success=True))]
            return subprocess.CompletedProcess(command, 0, stdout='\n'.join(rows))
        return run

    def test_only_the_fixtures_executable_is_taken(self):
        others = ['', artifact('sunrise-client', self.target / 'sunrise-client'),
                  artifact('build-script-build', self.target / 'build-script'),
                  json.dumps(dict(reason='compiler-artifact', executable=None, target=dict(name='sunrise_client')))]
        with mock.patch.object(lab.subprocess, 'run', side_effect=self.cargo(*others)):
            self.assertEqual(lab.parity_fixtures().read_text(), 'fixtures,client-defaults')

    def test_each_variant_builds_once_and_keeps_its_binary(self):
        with mock.patch.object(lab.subprocess, 'run', side_effect=self.cargo()) as cargo:
            for _ in range(2):
                default = lab.parity_fixtures()
                vanilla = lab.parity_fixtures(default_features=False)
                # Building the other variant rewrote Cargo's path; each variant's binary stayed its own.
                self.assertEqual(default.read_text(), 'fixtures,client-defaults')
                self.assertEqual(vanilla.read_text(), 'fixtures')
        self.assertEqual(cargo.call_count, 2)
        for call in cargo.call_args_list:
            command = call.args[0]
            self.assertEqual(command[command.index('-p') + 1], 'sunrise-parity-fixtures')
            self.assertEqual(command[command.index('--bin') + 1], 'sunrise-parity-fixtures')
            self.assertNotIn('test', command[:2])

    def test_no_executable_or_several_is_refused(self):
        for rows in ([artifact('sunrise-client', '/t/sunrise-client')],
                     [artifact('sunrise-parity-fixtures', '/t/a'), artifact('sunrise-parity-fixtures', '/t/b')]):
            built = subprocess.CompletedProcess([], 0, stdout='\n'.join(rows))
            with self.subTest(rows=rows), mock.patch.object(lab.subprocess, 'run', return_value=built):
                with self.assertRaisesRegex(RuntimeError, 'sunrise-parity-fixtures executables'):
                    lab.parity_fixtures()
            self.assertEqual(lab._PARITY_FIXTURES, {})


class ParityFixturesManifestTests(unittest.TestCase):
    def test_the_package_follows_the_clients_manifest(self):
        # Its dependencies and features are sunrise-client's: a client change that misses it breaks only the daily.
        from tools.ci import parity_fixtures_manifest
        self.assertEqual(parity_fixtures_manifest.main(['--check']), 0)

    def test_the_package_has_every_client_dependency_and_feature_behind_fixtures(self):
        import tomllib
        from tools.ci import parity_fixtures_manifest as manifest
        client = tomllib.loads(manifest.CLIENT.read_text(encoding='utf-8'))
        fixtures = tomllib.loads(manifest.render())
        expected_deps = {'sunrise-client', 'sunrise-client-gameplay', 'sunrise-gameplay', 'sunrise-gameplay-common'}
        self.assertEqual(set(fixtures['dependencies']), expected_deps)
        self.assertTrue(all(entry['optional'] and not entry['default-features']
                            for entry in fixtures['dependencies'].values()))
        self.assertEqual(
            set(fixtures['features']['fixtures']),
            {'dep:sunrise-gameplay-common', 'dep:sunrise-gameplay',
             'sunrise-gameplay-common/parity-fixtures', 'sunrise-gameplay/parity-fixtures'},
        )
        client_defaults = set(fixtures['features']['client-defaults'])
        self.assertTrue({'fixtures', 'dep:sunrise-client-gameplay', 'dep:sunrise-client',
                         'sunrise-client-gameplay/parity-fixtures', 'sunrise-client/parity-fixtures'} <= client_defaults)
        self.assertTrue({f'sunrise-client/{name}' for name in client['features']['default']} <= client_defaults)
        self.assertNotIn('default', fixtures['features'])


if __name__ == '__main__':
    unittest.main()
