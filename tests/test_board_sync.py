"""The board sync keeps going when GitHub says an item it adds is already on the board."""
import json
from pathlib import Path
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]

# Runs board-sync.js against a fake GraphQL client described by a JSON scenario on stdin and prints what it did.
HARNESS = r"""
const scenario = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const sync = require(process.argv[1]);
const edits = [];
const pr = (id, number, extra) => ({ id, number, isDraft: false, body: '', author: { login: 'author' }, assignees: { nodes: [] },
  closingIssuesReferences: { nodes: [] }, ...extra });
const issue = (id, number, extra) => ({ id, number, body: '', labels: { nodes: [] }, assignees: { nodes: [] }, ...extra });
const queries = [];
// GitHub refuses a document whose braces or parentheses do not close, or with an empty selection set.
const wellFormed = query => {
  const stack = [];
  for (const char of query.replace(/"(?:[^"\\]|\\.)*"/g, '""')) {
    if (char === '{' || char === '(') stack.push(char);
    else if (char === '}' || char === ')') { if (stack.pop() !== (char === '}' ? '{' : '(')) return false; }
  }
  return stack.length === 0 && !/\{\s*\}/.test(query);
};
const github = { graphql: async (query, vars) => {
  if (!wellFormed(query)) throw new Error(`malformed query: ${query.slice(0, 80)}`);
  queries.push(query.split(/[({]/, 1)[0].trim());
  if (query.includes('projectV2(number')) return { user: { projectV2: { id: 'P', fields: { nodes: [
    { id: 'S', name: 'Status', options: ['Todo', 'In progress', 'In review', 'Done'].map(name => ({ id: `status:${name}`, name })) },
    { id: 'O', name: 'Owner' },
    { id: 'H', name: 'Phase', options: ['Infra', 'Design'].map(name => ({ id: `phase:${name}`, name })) }] } } } };
  if (query.includes('items(first:50')) return { node: { items: { pageInfo: { hasNextPage: false }, nodes: [] } } };
  if (query.includes('issues(first:40')) return { repository: { issues: { pageInfo: { hasNextPage: false },
    nodes: query.includes('states:[OPEN]') ? scenario.issues.map(([id, number, extra]) => issue(id, number, extra)) : [] } } };
  if (query.includes('pullRequests(first:40')) return { repository: { pullRequests: { pageInfo: { hasNextPage: false },
    nodes: query.includes('states:[OPEN]') ? (scenario.prs || []).map(([id, number, extra]) => pr(id, number, extra)) : [] } } };
  if (query.includes('addProjectV2ItemById')) {
    const ids = [...query.matchAll(/(a\d+):addProjectV2ItemById\(input:\{projectId:"P",contentId:"(\w+)"/g)];
    const data = Object.fromEntries(ids.map(([, alias, id]) => [alias, scenario.refused.includes(id) ? null : { item: { id: `new:${id}` } }]));
    const errors = ids.filter(([, , id]) => scenario.refused.includes(id)).map(([, alias]) => ({ path: [alias], message: 'Content already exists in this project.' }));
    errors.push(...(scenario.otherErrors || []).map(message => ({ path: ['a0'], message })));
    if (!errors.length) return data;
    throw Object.assign(new Error(errors.map(e => e.message).join('; ')), { data, errors });
  }
  if (query.includes('projectItems')) {
    // Only the fragment for the content's own type reads its project items, as on GitHub.
    const type = vars.id.startsWith('PR') ? 'PullRequest' : 'Issue';
    return { node: query.includes(`... on ${type}{projectItems`) ? { projectItems: scenario.existing[vars.id][vars.a || 'first'] } : {} };
  }
  if (query.startsWith('mutation{e0')) { edits.push(...query.match(/updateProjectV2ItemFieldValue\(.*?\)\{clientMutationId\}/g)); return {}; }
  throw new Error(`unexpected query ${query.slice(0, 60)}`);
} };
sync({ github, context: { eventName: 'schedule' }, core: { summary: { addRaw: () => ({ write() {} }) } } })
  .then(() => console.log(JSON.stringify({ edits, queries })), e => console.log(JSON.stringify({ error: e.message, edits })));
"""


def board_item(item, status, owner, phase, project="P"):
    return {"id": item, "project": {"id": project}, "s": {"name": status}, "o": {"text": owner}, "p": {"name": phase}}


@unittest.skipUnless(shutil.which("node"), "the board sync is JavaScript")
class BoardSyncTests(unittest.TestCase):
    def run_sync(self, scenario):
        result = subprocess.run(["node", "-e", HARNESS, str(ROOT / ".github/scripts/board-sync.js")],
                                input=json.dumps(scenario), text=True, capture_output=True, check=True)
        return json.loads(result.stdout)

    def update(self, item, field, value):
        return f'updateProjectV2ItemFieldValue(input:{{projectId:"P",itemId:"{item}",fieldId:"{field}",value:{value}}})'

    def test_an_item_already_on_the_board_keeps_its_hand_set_owner_and_phase(self):
        assigned = {"assignees": {"nodes": [{"login": "alice"}]}, "body": "**Phase.** Infra"}
        result = self.run_sync({
            "issues": [["I1", 1, assigned], ["I2", 2, assigned]],
            "refused": ["I1"],
            # The board item sits on the content's second page of project items.
            "existing": {"I1": {
                "first": {"pageInfo": {"hasNextPage": True, "endCursor": "c1"},
                          "nodes": [board_item("elsewhere", "Done", "", "", project="X")]},
                "c1": {"pageInfo": {"hasNextPage": False}, "nodes": [board_item("E1", "Todo", "bob", "Design")]},
            }},
        })
        self.assertNotIn("error", result)
        # Every query the sync sent was well formed (the fake refuses one that is not), and all of them ran:
        # project, board items, open and closed issues and PRs, the add, two pages of the lookup, the edits.
        self.assertEqual(result["queries"], ["query", "query", *["query"] * 5, "mutation", "query", "query", "mutation"])
        self.assertEqual(sorted(edit.split("{clientMutationId}")[0] for edit in result["edits"]), sorted([
            self.update("E1", "S", '{singleSelectOptionId:"status:In progress"}'),
            self.update("new:I2", "S", '{singleSelectOptionId:"status:In progress"}'),
            self.update("new:I2", "H", '{singleSelectOptionId:"phase:Infra"}'),
            self.update("new:I2", "O", '{text:"alice"}'),
        ]))

    def test_a_pull_request_already_on_the_board_keeps_its_hand_set_owner_and_phase(self):
        result = self.run_sync({
            "issues": [],
            "prs": [["PR1", 7, {"author": {"login": "carol"}, "body": "**Phase.** Infra"}]],
            "refused": ["PR1"],
            "existing": {"PR1": {"first": {"pageInfo": {"hasNextPage": False},
                                           "nodes": [board_item("E7", "Todo", "bob", "Design")]}}},
        })
        self.assertNotIn("error", result)
        self.assertEqual(result["edits"], [
            self.update("E7", "S", '{singleSelectOptionId:"status:In review"}') + "{clientMutationId}"])

    def test_a_batch_with_any_other_error_still_fails(self):
        result = self.run_sync({"issues": [["I1", 1, {}]], "refused": ["I1"], "otherErrors": ["Resource not accessible"],
                                "existing": {}})
        self.assertEqual(result, {"error": "Content already exists in this project.; Resource not accessible", "edits": []})

    def test_a_refused_add_with_no_board_item_fails_naming_the_content(self):
        result = self.run_sync({"issues": [["I1", 1, {}]], "refused": ["I1"],
                                "existing": {"I1": {"first": {"pageInfo": {"hasNextPage": False}, "nodes": []}}}})
        self.assertEqual(result, {"error": "board add refused I1 as already present, but no board item holds it", "edits": []})


if __name__ == "__main__":
    unittest.main()
