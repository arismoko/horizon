import unittest

from tools.ci import check_selector_map


class SelectorMapTests(unittest.TestCase):
    def test_old_bindings_and_ambiguous_or_wrong_owner_tests_fail(self):
        server_slot = "fixture_server::game::ServerGame::save"
        client_hook = {"id": "hook:paint", "target": "fixture_client::game::paint"}
        common_test = {"package": "fixture-common", "target": "lib", "name": "save::tests::witness"}
        server_test = dict(common_test, package="fixture-server")
        functions = {"functions": {server_slot: {}}, "refused": {"fixture_common::game::helper": {}}}
        surface = {"hooks": [{"id": client_hook["id"], "engine": {"target": client_hook["target"]}}]}
        tests = [common_test, server_test]
        selections = {"replaceable": [server_slot], "hooks": [client_hook], "ignored_tests": [common_test]}
        self.assertEqual(check_selector_map.problems(selections, functions, surface, tests), [])

        refused = "fixture_common::game::helper"
        stale = dict(client_hook, target="fixture_server::game::paint")
        selections = {"replaceable": [refused], "hooks": [stale],
                      "ignored_tests": [dict(common_test, target="bin:dispatch"), server_test]}
        self.assertEqual(check_selector_map.problems(selections, functions, surface, tests + [server_test]), [
            f"{refused}: selector has no replaceable function in the staged index",
            "hook:paint -> fixture_server::game::paint: expected one engine binding, found 0",
            "fixture-common bin:dispatch save::tests::witness: expected one test, found 0",
            "fixture-server lib save::tests::witness: expected one test, found 2",
        ])
