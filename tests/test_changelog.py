import copy
from datetime import datetime, timezone
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from tools.changelog import collect, escaped, get, members, merged_prs, render
from urllib.error import HTTPError


class ChangelogTests(unittest.TestCase):
    def setUp(self):
        self.data = json.loads((Path(__file__).parent / 'fixtures/changelog.json').read_text())
        self.now = datetime(2026, 11, 13, 12, tzinfo=timezone.utc)

    def test_rollup_expands_deduplicates_and_groups_at_landing(self):
        self.assertEqual(members(self.data['prs'][1]), [1])
        output = render(self.data, self.now)
        self.assertEqual(output.count('(#1)'), 1)
        self.assertNotIn('(#2)', output)
        self.assertIn('## 2026-11-13', output)
        self.assertNotIn('## 2026-11-12', output)
        self.assertIn('### Fixed bugs', output)
        self.assertIn('&#60;script&#62;', output)
        self.assertNotIn('@all', output)

    def test_window_and_cap(self):
        self.data['prs'] = []
        issue = self.data['issues'][0]
        self.data['issues'] += [dict(copy.deepcopy(issue), number=4)]
        output = render(self.data, self.now, cap=1)
        self.assertIn('and 1 more', output)
        self.now = datetime(2026, 12, 14, tzinfo=timezone.utc)
        self.assertIn('No matching activity.', render(self.data, self.now))

    @patch('tools.changelog.time.sleep')
    @patch('tools.changelog.get')
    def test_pull_collection_is_batched(self, get, sleep):
        node = {'number': 1, 'title': 'Change', 'body': '',
                'mergedAt': '2026-11-13T00:00:00Z', 'headRefName': 'topic',
                'headRefOid': 'a' * 40, 'author': {'login': 'author', '__typename': 'User'},
                'labels': {'nodes': []}}
        get.side_effect = [
            {'data': {'search': {'issueCount': 101, 'nodes': [dict(node, number=n) for n in range(100)],
                                'pageInfo': {'hasNextPage': True, 'endCursor': 'next'}}}},
            {'data': {'search': {'issueCount': 101, 'nodes': [dict(node, number=100)],
                                'pageInfo': {'hasNextPage': False, 'endCursor': 'end'}}}}]
        self.assertEqual(len(merged_prs('2026-11-13')), 101)
        self.assertEqual(get.call_count, 2)
        self.assertEqual(get.call_args.args[1]['variables']['after'], 'next')

    @patch('tools.changelog.search', return_value=[])
    @patch('tools.changelog.merged_prs')
    @patch('tools.changelog.get', return_value=[])
    def test_collect_uses_no_per_pull_rest_requests(self, get, pulls, search):
        pulls.return_value = self.data['prs']
        result = collect(self.now, 1)
        self.assertEqual(len(result['prs']), 2)
        self.assertEqual(get.call_count, 1)  # Releases only; member already batched.
        self.assertIn('/releases?', get.call_args.args[0])

    @patch('tools.changelog.search', return_value=[])
    @patch('tools.changelog.merged_prs', return_value=[])
    @patch('tools.changelog.get')
    def test_release_paging_is_bounded(self, get, pulls, search):
        get.return_value = [{'published_at': '2026-11-13T00:00:00Z'}] * 100
        with self.assertRaisesRegex(RuntimeError, 'five pages'):
            collect(self.now, 1)
        self.assertEqual(get.call_count, 5)

    @patch('tools.changelog.time.sleep')
    @patch('tools.changelog.get')
    def test_daily_search_limit_fails_clearly(self, get, sleep):
        get.return_value = {'data': {'search': {'issueCount': 1001}}}
        with self.assertRaisesRegex(RuntimeError, '1,000 results'):
            merged_prs('2026-11-13')

    @patch('tools.changelog.search', return_value=[])
    @patch('tools.changelog.merged_prs', return_value=[])
    @patch('tools.changelog.get')
    def test_old_release_does_not_stop_pagination(self, api, pulls, search):
        old = {'published_at': '2020-01-01T00:00:00Z'}
        recent = {'published_at': '2026-11-13T00:00:00Z'}
        api.side_effect = [[old] * 100, [recent]]
        self.assertEqual(collect(self.now, 1)['releases'][-1], recent)
        self.assertEqual(api.call_count, 2)

    @patch('tools.changelog.DEADLINE', None)
    @patch('tools.changelog.REQUESTS', 99)
    @patch('tools.changelog.time.sleep')
    @patch('tools.changelog.urlopen')
    def test_retry_counts_against_limit(self, open_url, sleep):
        open_url.side_effect = HTTPError('url', 429, 'limited', {}, None)
        with self.assertRaisesRegex(RuntimeError, 'budget exceeded before retry'):
            get('https://api.github.com/test')
        self.assertEqual(open_url.call_count, 1)

    @patch('tools.changelog.DEADLINE', 10)
    @patch('tools.changelog.REQUESTS', 0)
    @patch('tools.changelog.time.monotonic', side_effect=[0, 0, 0, 11])
    @patch('tools.changelog.time.sleep')
    @patch('tools.changelog.urlopen')
    def test_retry_rechecks_deadline(self, open_url, sleep, clock):
        open_url.side_effect = HTTPError('url', 429, 'limited', {}, None)
        with self.assertRaisesRegex(RuntimeError, 'budget exceeded before retry'):
            get('https://api.github.com/test')
        self.assertEqual(open_url.call_count, 1)

    def test_tool_prerelease_is_labelled(self):
        self.data['releases'] = [{'id': 1, 'draft': False, 'prerelease': True,
            'tag_name': 'tool-v1', 'name': 'Tool', 'published_at': '2026-11-13T00:00:00Z'}]
        self.assertIn('(tool pre-release)', render(self.data, self.now))

    def test_readable_hostile_title(self):
        title = "# Hello_foo: 'x' (#12), a-b [link](bad) <img> `code` |\n> @all"
        self.assertEqual(escaped(title),
            "&#35; Hello_foo: 'x' (#12), a-b &#91;link&#93;(bad) &#60;img&#62; &#96;code&#96; &#124; &#62; &#64;all")

    def test_contributors_are_uncapped_and_maintenance_is_separate(self):
        base = self.data['prs'][0]
        self.data['prs'] = [dict(copy.deepcopy(base), number=n) for n in range(20)]
        output = render(self.data, self.now)
        self.assertIn('#### &#64;author (20)', output)
        self.assertEqual(output.count('/pull/'), 20)
        for pr in self.data['prs']:
            pr['user']['login'] = 'arismoko'
        rollup = dict(copy.deepcopy(base), number=1000,
            head={'ref': 'fork-review-rollup-1', 'sha': 'b' * 40},
            merged_at='2026-11-13T01:00:00Z',
            body='\n'.join(f'| #{n} | author |' for n in range(20)))
        self.data['prs'].append(rollup)
        output = render(self.data, self.now)
        self.assertIn('### Maintenance', output)
        self.assertEqual(output.count('/pull/'), 20)
        self.assertEqual(output.split('<details>')[0].count('/pull/'), 5)
        self.assertIn('15 more maintenance', output)
        for number in range(15):
            self.assertIn(f'[#{number}](https://github.com/arismoko/horizon/pull/{number})', output)
        self.assertNotIn('merged%3A', output)

    def test_bot_filter_and_label_override(self):
        self.data['prs'] = [self.data['prs'][0]]
        pr = self.data['prs'][0]
        pr['user']['type'] = 'Bot'
        self.assertNotIn('(#1)', render(self.data, self.now))
        pr['labels'] = [{'name': 'bug'}]
        self.assertIn('(#1)', render(self.data, self.now))
        pr['labels'] = [{'name': 'skip-release'}]
        self.assertNotIn('(#1)', render(self.data, self.now))


if __name__ == '__main__':
    unittest.main()
