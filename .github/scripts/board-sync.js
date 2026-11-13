// Keeps the "Horizon roadmap" board (user project 1) in step with arismoko/horizon.
// Every open issue and PR is on the board; Status follows GitHub state:
//   merged or closed            -> Done
//   open PR (draft)             -> In progress
//   open PR                     -> In review
//   open issue with an open PR that closes it -> In review
//   open issue with an assignee -> In progress
//   other open issue            -> Todo
// An open issue labelled `claimed` also counts as In progress (a claim carried over before the
// claimer could be assigned). Owner is filled from the assignees, a "Claimed by @x" line, or the
// PR author, when empty. Phase is filled from a "**Phase.** <name>" line in the body when empty.
// Pinned issues stay off the board.
module.exports = async ({ github, context, core }) => {
  const FULL = context.eventName === 'schedule' || context.eventName === 'workflow_dispatch';
  const OWNER = 'arismoko', REPO = 'horizon', NUMBER = 1;
  // GitHub answers 502 on heavy pages now and then; retry those a few times.
  const q = async (query, vars = {}) => {
    for (let i = 0; ; i++) {
      try { return await github.graphql(query, vars); }
      // GraphQL also answers "Something went wrong while executing your query" (no status) on heavy pages.
      catch (e) { if (i >= 4 || !/50[234]|timeout|something went wrong/i.test(`${e.status} ${e.message}`)) throw e; await new Promise(r => setTimeout(r, 5000 * (i + 1))); }
    }
  };

  const proj = (await q(`query($o:String!,$n:Int!){user(login:$o){projectV2(number:$n){id
    fields(first:30){nodes{... on ProjectV2Field{id name} ... on ProjectV2SingleSelectField{id name options{id name}}}}}}}`,
    { o: OWNER, n: NUMBER })).user.projectV2;
  const field = name => proj.fields.nodes.find(f => f && f.name === name);
  const status = field('Status'), owner = field('Owner'), phase = field('Phase');
  const option = Object.fromEntries(status.options.map(o => [o.name, o.id]));
  const phaseOption = Object.fromEntries((phase?.options || []).map(o => [o.name.toLowerCase(), o.id]));

  // Board items, keyed by content node id, with the field values the sync compares or fills.
  const VALUES = `s:fieldValueByName(name:"Status"){... on ProjectV2ItemFieldSingleSelectValue{name}}
        o:fieldValueByName(name:"Owner"){... on ProjectV2ItemFieldTextValue{text}}
        p:fieldValueByName(name:"Phase"){... on ProjectV2ItemFieldSingleSelectValue{name}}`;
  const known = n => ({ item: n.id, status: n.s?.name, owner: n.o?.text, phase: n.p?.name });
  const items = new Map();
  for (let after = null; ;) {
    const page = (await q(`query($id:ID!,$a:String){node(id:$id){... on ProjectV2{items(first:50,after:$a){
      pageInfo{hasNextPage endCursor}
      nodes{id content{... on Issue{id} ... on PullRequest{id}} ${VALUES}}}}}}`, { id: proj.id, a: after })).node.items;
    for (const n of page.nodes) if (n.content) items.set(n.content.id, known(n));
    if (!page.pageInfo.hasNextPage) break;
    after = page.pageInfo.endCursor;
  }

  // Repository issues and PRs: every open one, plus closed ones already on the board.
  const want = new Map(); // content id -> {status, owner, number}
  const linked = new Set(); // issue ids an open PR closes
  const pinned = new Set();
  for (const kind of ['issues', 'pullRequests']) {
    for (const state of ['OPEN', 'CLOSED', 'MERGED']) {
      if (kind === 'issues' && state === 'MERGED') continue;
      for (let after = null; ;) {
        const extra = kind === 'issues' ? 'isPinned body labels(first:10){nodes{name}} assignees(first:5){nodes{login}}' :
          'isDraft body author{login} assignees(first:5){nodes{login}} closingIssuesReferences(first:10){nodes{id}}';
        const page = (await q(`query($o:String!,$r:String!,$a:String){repository(owner:$o,name:$r){${kind}(first:40,after:$a,states:[${state}],orderBy:{field:UPDATED_AT,direction:DESC}){
          pageInfo{hasNextPage endCursor} nodes{id number ${extra}}}}}`, { o: OWNER, r: REPO, a: after })).repository[kind];
        for (const n of page.nodes) {
          if (state !== 'OPEN' && !items.has(n.id)) continue;
          if (n.isPinned) { pinned.add(n.id); continue; }
          const who = n.assignees.nodes.map(a => a.login);
          const claimedBy = /Claimed by @([A-Za-z0-9-]+)/.exec(n.body || '')?.[1];
          const claimed = who.length > 0 || (n.labels?.nodes || []).some(l => l.name === 'claimed');
          const phaseName = /\*\*Phase\.\*\*\s*([^\n]+)/.exec(n.body || '')?.[1].trim().toLowerCase();
          if (kind === 'pullRequests' && state === 'OPEN') n.closingIssuesReferences.nodes.forEach(i => linked.add(i.id));
          let s;
          if (state !== 'OPEN') s = 'Done';
          else if (kind === 'pullRequests') s = n.isDraft ? 'In progress' : 'In review';
          else s = claimed ? 'In progress' : 'Todo';
          want.set(n.id, { status: s, owner: who.length ? who.join(', ') : (claimedBy || n.author?.login || ''), phase: phaseOption[phaseName], number: n.number, issue: kind === 'issues', open: state === 'OPEN' });
        }
        // Closed items only matter while they are on the board. Event runs read the 80 most recently
        // updated closed items; the scheduled run reads them all, so a merge an event run missed
        // (a burst of merges, a skipped pending run) still reaches Done within 15 minutes.
        if (!page.pageInfo.hasNextPage || (state !== 'OPEN' && !FULL && after !== null)) break;
        after = page.pageInfo.endCursor;
      }
    }
  }
  for (const [id, w] of want) if (w.issue && w.open && linked.has(id)) w.status = 'In review';

  // Add what is missing, then set what differs, in batches.
  // An add can answer "Content already exists in this project" when the item is on the board but was not
  // listed above (an archived item, or one another run added meanwhile). That is the state we want: the
  // other adds in the batch still succeed, and the existing item is looked up from the content's side.
  // The content's own board items, page by page, with their field values, so a recovered item keeps
  // the Owner and Phase set on it by hand exactly as a listed one does.
  const existingItem = async contentId => {
    const items = `projectItems(first:50,after:$a){pageInfo{hasNextPage endCursor} nodes{id project{id} ${VALUES}}}`;
    for (let after = null; ;) {
      const page = (await q(`query($id:ID!,$a:String){node(id:$id){... on Issue{${items}} ... on PullRequest{${items}}}}`,
        { id: contentId, a: after })).node?.projectItems;
      const found = page?.nodes.find(n => n.project.id === proj.id);
      if (found) return known(found);
      if (!page?.pageInfo.hasNextPage) break;
      after = page.pageInfo.endCursor;
    }
    throw new Error(`board add refused ${contentId} as already present, but no board item holds it`);
  };
  const missing = [...want.keys()].filter(id => !items.has(id));
  for (let i = 0; i < missing.length; i += 25) {
    const chunk = missing.slice(i, i + 25);
    let r;
    try {
      r = await q(`mutation{${chunk.map((c, j) => `a${j}:addProjectV2ItemById(input:{projectId:"${proj.id}",contentId:"${c}"}){item{id}}`).join(' ')}}`);
    } catch (e) {
      if (!e.data || !e.errors?.length || !e.errors.every(x => /content already exists in this project/i.test(x.message))) throw e;
      r = e.data;
    }
    for (const [j, c] of chunk.entries()) {
      const added = r[`a${j}`]?.item?.id;
      items.set(c, added ? { item: added } : await existingItem(c));
    }
  }
  const edits = [];
  for (const [id, w] of want) {
    const it = items.get(id);
    if (it.status !== w.status) edits.push(`updateProjectV2ItemFieldValue(input:{projectId:"${proj.id}",itemId:"${it.item}",fieldId:"${status.id}",value:{singleSelectOptionId:"${option[w.status]}"}}){clientMutationId}`);
    if (phase && !it.phase && w.phase) edits.push(`updateProjectV2ItemFieldValue(input:{projectId:"${proj.id}",itemId:"${it.item}",fieldId:"${phase.id}",value:{singleSelectOptionId:"${w.phase}"}}){clientMutationId}`);
    if (owner && !it.owner && w.owner) edits.push(`updateProjectV2ItemFieldValue(input:{projectId:"${proj.id}",itemId:"${it.item}",fieldId:"${owner.id}",value:{text:${JSON.stringify(w.owner)}}}){clientMutationId}`);
  }
  for (let i = 0; i < edits.length; i += 40) {
    await q(`mutation{${edits.slice(i, i + 40).map((e, j) => `e${j}:${e}`).join(' ')}}`);
  }
  const gone = [...pinned].filter(id => items.has(id));
  if (gone.length) await q(`mutation{${gone.map((c, j) => `d${j}:deleteProjectV2Item(input:{projectId:"${proj.id}",itemId:"${items.get(c).item}"}){deletedItemId}`).join(' ')}}`);
  core.summary.addRaw(`Board sync: ${missing.length} added, ${gone.length} pinned removed, ${edits.length} field updates, ${want.size} items checked.`).write();
};
