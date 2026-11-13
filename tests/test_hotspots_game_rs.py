import contextlib
import io
import unittest

from hotspots_repo import Repo
from tools.hotspots import game_rs


class GameRsTest(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.close)
        self.base = self.repo.commit('main', {game_rs.GAME: '// empty\n'})
        self.table = self.repo.path / 'classifications.txt'
        self.table.write_text('Dim key Other crates/horizon-client/src/game/moved.rs unrelated key\n')

    def run_report(self, fork_owner, upstream_owner, name='key'):
        base = self.repo.commit('base', {
            'crates/horizon-client/src/game/moved.rs':
                f'impl {upstream_owner} {{\n    fn {name}() {{}}\n}}\n',
        }, base=self.base)
        fork = self.repo.commit('fork', {game_rs.GAME:
            f'impl {fork_owner} {{\n    fn {name}() {{}}\n}}\n'}, base=self.base)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = game_rs.main([fork, base, '--repo', str(self.repo.path),
                                   '--classifications', str(self.table)])
        return status, output.getvalue()

    def test_classified(self):
        status, output = self.run_report('Dim', 'Other')
        self.assertEqual(status, 0)
        self.assertIn('classified name-only matches: 1', output)
        self.assertIn('unrelated key', output)

    def test_same_name_game_move(self):
        status, output = self.run_report('Game', 'Game')
        self.assertEqual(status, 1)
        self.assertIn('genuine remnants: 1', output)

    def test_same_owner_overrides_classification(self):
        self.table.write_text('Game key Game crates/horizon-client/src/game/moved.rs not an exemption\n')
        status, output = self.run_report('Game', 'Game')
        self.assertEqual(status, 1)
        self.assertIn('genuine remnants: 1', output)
        self.assertIn('stale classification:', output)

    def test_classified_collision_cannot_hide_game_move(self):
        self.table.write_text('Game key Other crates/horizon-client/src/game/moved.rs unrelated\n')
        base = self.repo.commit('base', {
            'crates/horizon-client/src/game/moved.rs':
                'impl Other {\n fn key() {}\n}\nimpl Game {\n fn key() {}\n}\n',
        }, base=self.base)
        fork = self.repo.commit('fork', {game_rs.GAME: 'impl Game {\n fn key() {}\n}\n'}, base=self.base)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = game_rs.report(self.repo.path, fork, base, self.table)
        self.assertEqual(status, 1)
        self.assertIn('genuine remnants: 1', output.getvalue())
        self.assertIn('classified name-only matches: 0', output.getvalue())

    def test_stale(self):
        status, output = self.run_report('Game', 'Other', 'different')
        self.assertEqual(status, 1)
        self.assertIn('delete it', output)

    def test_unclassified(self):
        self.table.write_text('')
        status, output = self.run_report('Game', 'Game')
        self.assertEqual(status, 1)
        self.assertIn('genuine remnants: 1', output)

    def test_trait_owner_and_free_function(self):
        functions = game_rs.items('impl Trait for Dim {\n fn key() {}\n}\nfn key() {}\n')
        self.assertEqual([f.key for f in functions], [('Dim', 'key'), ('free', 'key')])
