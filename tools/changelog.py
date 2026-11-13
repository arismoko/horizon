#!/usr/bin/env python3
"""Generate a rolling public activity digest; no dependencies or model calls.

Rollups expand only the PR column of their membership table, never arbitrary
issue references. Member PRs also returned by search are deduplicated.
"""
import argparse
from collections import defaultdict
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import re
import time
import sys
from urllib.error import HTTPError

DEADLINE = None
REQUESTS = 0
from urllib.parse import quote
from urllib.request import Request, urlopen

REPO = "arismoko/horizon"
WEB = f"https://github.com/{REPO}"
API = f"https://api.github.com/repos/{REPO}"
SECTIONS = ("Merged", "Releases", "New issues open for contributors", "Fixed bugs")


def escaped(value):
    # Preserve ordinary punctuation for readers of the raw generated file.
    value = ' '.join(value.split())
    return ''.join(f'&#{ord(c)};' if c in '[]<>`|@\\\\' or (index == 0 and c == '#') else c
                   for index, c in enumerate(value))


def get(url, payload=None):
    headers = {"Accept": "application/vnd.github+json"}
    if os.environ.get("GH_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GH_TOKEN"]
    global REQUESTS
    REQUESTS += 1
    if REQUESTS > 100 or (DEADLINE is not None and time.monotonic() >= DEADLINE):
        raise RuntimeError("Collection budget exceeded (100 requests / 110 seconds)")
    print(f"GitHub request {REQUESTS}: {url.split('?')[0]}", file=sys.stderr, flush=True)
    body = json.dumps(payload).encode() if payload is not None else None
    timeout = min(15, DEADLINE - time.monotonic()) if DEADLINE else 15
    try:
        with urlopen(Request(url, data=body, headers=headers), timeout=timeout) as response:
            return json.load(response)
    except HTTPError as error:
        if error.code not in (403, 429):
            raise
        delay = max(1, int(error.headers.get('Retry-After', 0)),
                    int(error.headers.get('X-RateLimit-Reset', 0)) - int(time.time()))
        if delay > 10 or (DEADLINE and time.monotonic() + delay >= DEADLINE):
            raise RuntimeError(f"GitHub rate limited; retry after {delay} seconds") from error
        print(f"GitHub rate limited; backing off {delay} seconds", file=sys.stderr, flush=True)
        time.sleep(delay)
        # Count the retry as another HTTP attempt and recompute its remaining budget.
        REQUESTS += 1
        if REQUESTS > 100 or (DEADLINE is not None and time.monotonic() >= DEADLINE):
            raise RuntimeError('Collection budget exceeded before retry') from error
        timeout = min(15, DEADLINE - time.monotonic()) if DEADLINE else 15
        print(f'GitHub retry {REQUESTS}', file=sys.stderr, flush=True)
        with urlopen(Request(url, data=body, headers=headers), timeout=timeout) as response:
            return json.load(response)


def search(query):
    # Split by date at the caller to keep below GitHub's 1,000-result ceiling.
    result = []
    for page in range(1, 11):
        # GitHub permits 30 authenticated search requests per minute.
        time.sleep(2.1)
        data = get("https://api.github.com/search/issues?q=" + quote(
            f"repo:{REPO} {query}") + f"&per_page=100&page={page}")
        if data.get("incomplete_results") or data["total_count"] > 1000:
            raise RuntimeError("GitHub search is incomplete; refusing partial digest")
        result.extend(data["items"])
        if len(result) >= data["total_count"]:
            return result
    raise RuntimeError("Search pagination exhausted")


def members(pr):
    return [int(n) for n in re.findall(r"^\|\s*#(\d+)\s*\|", pr.get("body") or "", re.M)]


def merged_prs(day):
    """Batch the fields needed for expansion instead of one REST call per PR."""
    query = """query($query: String!, $after: String) {
      search(query: $query, type: ISSUE, first: 100, after: $after) {
        issueCount pageInfo { hasNextPage endCursor }
        nodes { ... on PullRequest {
          number title body mergedAt headRefName headRefOid
          author { login __typename } labels(first: 100) { nodes { name } }
        } }
      }
    }"""
    after = None
    result = []
    while True:
        time.sleep(2.1)
        response = get("https://api.github.com/graphql", {"query": query, "variables": {
            "query": f"repo:{REPO} is:pr is:merged merged:{day}", "after": after}})
        if response.get("errors"):
            raise RuntimeError("GitHub GraphQL query failed")
        batch = response["data"]["search"]
        if batch["issueCount"] > 1000:
            if '..' not in day:
                raise RuntimeError("GitHub search exceeds 1,000 results for one day")
            start, end = (datetime.fromisoformat(value) for value in day.split('..'))
            if start == end:
                raise RuntimeError('GitHub search exceeds 1,000 results for one day')
            middle = start + (end - start) // 2
            left = start.date().isoformat() + '..' + middle.date().isoformat()
            right = (middle + timedelta(days=1)).date().isoformat() + '..' + end.date().isoformat()
            return merged_prs(left) + merged_prs(right)
        for pr in batch["nodes"]:
            author = pr["author"] or {"login": "deleted", "__typename": "User"}
            result.append({"number": pr["number"], "title": pr["title"], "body": pr["body"],
                           "merged_at": pr["mergedAt"], "head": {
                               "ref": pr["headRefName"], "sha": pr["headRefOid"]},
                           "user": {"login": author["login"], "type": author["__typename"]},
                           "labels": pr["labels"]["nodes"]})
        if not batch["pageInfo"]["hasNextPage"]:
            return result
        after = batch["pageInfo"]["endCursor"]


def collect(now, days):
    prs, issues = {}, {}
    day = (now - timedelta(days=days - 1)).date().isoformat() + '..' + now.date().isoformat()
    for item in merged_prs(day):
        prs[item['number']] = item
    for query in (f'is:issue label:"help wanted" created:{day}',
                  f'is:issue label:bug closed:{day}'):
        for item in search(query):
            issues[item['number']] = item
    # Batch out-of-window members, never make a serial request for each member.
    missing = sorted({n for pr in prs.values()
                      if pr['head']['ref'].startswith('fork-review-rollup-')
                      for n in members(pr)} - prs.keys())
    for offset in range(0, len(missing), 50):
        fields = 'number title body mergedAt headRefName headRefOid author { login __typename } labels(first: 100) { nodes { name } }'
        aliases = ' '.join(f'p{n}: pullRequest(number: {n}) {{ {fields} }}' for n in missing[offset:offset + 50])
        response = get('https://api.github.com/graphql', {'query': '{ repository(owner: "arismoko", name: "horizon") { ' + aliases + ' } }'})
        if response.get('errors'):
            raise RuntimeError('GitHub member lookup failed')
        for pr in response['data']['repository'].values():
            if pr is None:
                raise RuntimeError('Rollup member not found')
            author = pr['author'] or {'login': 'deleted', '__typename': 'User'}
            prs[pr['number']] = dict(number=pr['number'], title=pr['title'], body=pr['body'],
                merged_at=pr['mergedAt'], head={'ref': pr['headRefName'], 'sha': pr['headRefOid']},
                user={'login': author['login'], 'type': author['__typename']}, labels=pr['labels']['nodes'])
    releases = []
    page = 1
    while True:
        if page > 5:
            raise RuntimeError('Release pagination exceeded five pages')
        batch = get(API + f"/releases?per_page=100&page={page}")
        releases.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return {"prs": list(prs.values()), "issues": list(issues.values()), "releases": releases}


def render(data, now, days=30, cap=12):
    first = (now - timedelta(days=days - 1)).date().isoformat()
    last = now.date().isoformat()
    groups = defaultdict(lambda: defaultdict(list))

    def add(date, section, text, number=0):
        if date and first <= date[:10] <= last:
            groups[date[:10]][section].append((number, text))

    prs = {p["number"]: p for p in data["prs"]}
    rollups = [p for p in prs.values() if p["head"]["ref"].startswith("fork-review-rollup-")]
    dates = {n: p["merged_at"] for p in rollups for n in members(p)}
    for n, pr in prs.items():
        if pr in rollups or not pr.get("merged_at"):
            continue
        labels = {label["name"] for label in pr["labels"]}
        if "skip-release" in labels:
            continue
        if not labels and (pr["user"].get("type") == "Bot" or
                           re.match(r"^(ci|chore\(ci\)|build)(:|\()", pr["title"], re.I)):
            continue
        text = f"[{escaped(pr['title'])} (#{n})]({WEB}/pull/{n}) — &#64;{escaped(pr['user']['login'])}"
        if re.search(r"upstream.*sync|sync.*upstream|merge upstream", pr["title"], re.I):
            text += f" — upstream commit `{pr['head']['sha']}`"
        add(dates.get(n, pr["merged_at"]), "Merged", text, n)
    for item in data["issues"]:
        labels = {label["name"] for label in item["labels"]}
        text = f"[{escaped(item['title'])} (#{item['number']})]({WEB}/issues/{item['number']})"
        if "help wanted" in labels and item["state"] == "open":
            add(item["created_at"], SECTIONS[2], text, item["number"])
        if "bug" in labels and item["state"] == "closed":
            add(item["closed_at"], SECTIONS[3], text, item["number"])
    for release in data["releases"]:
        if not release["draft"]:
            tag = release["tag_name"]
            suffix = ' (tool pre-release)' if release.get('prerelease') else ''
            add(release["published_at"], "Releases", f"[{escaped(release['name'] or tag)}]({WEB}/releases/tag/{quote(tag, safe='')}){suffix}", release["id"])
    lines = ["> 🤖 Generated by Aria's agent swarm, Dyna, not by Aria.", "",
             "# Changelog", "", f"Generated {now.strftime('%Y-%m-%d %H:%M UTC')}. Last {days} UTC calendar days.", "",
             "Public repository activity, not a curated release promise. CI/build and bot PRs without labels, and skip-release PRs are omitted. Rollups expand their membership tables. Issue labels and open status reflect generation time.", ""]
    for day in sorted(groups, reverse=True):
        lines += [f"## {day}", ""]
        for section in SECTIONS:
            entries = sorted(groups[day][section], reverse=True)
            if not entries:
                continue
            if section == 'Merged':
                entries.sort(key=lambda entry: (dates.get(entry[0], prs[entry[0]]['merged_at']), entry[0]), reverse=True)
                contributors = defaultdict(list)
                maintenance = []
                for number, text in entries:
                    login = prs[number]['user']['login']
                    if login == 'arismoko':
                        maintenance.append((number, text))
                    else:
                        contributors[login].append((number, text))
                if contributors:
                    lines += ['### From contributors', '']
                    for login, items in sorted(contributors.items(), key=lambda pair: max((dates.get(n, prs[n]['merged_at']), n) for n, _ in pair[1]), reverse=True):
                        lines += [f'#### &#64;{escaped(login)} ({len(items)})', '']
                        lines += ['- ' + text for _, text in items]
                        lines.append('')
                if maintenance:
                    lines += ['### Maintenance', '']
                    syncs = [(n, text) for n, text in maintenance if ' — upstream commit `' in text]
                    others = [(n, text) for n, text in maintenance if ' — upstream commit `' not in text]
                    lines += ['- ' + text for _, text in syncs + others[:5]]
                    if len(others) > 5:
                        # Exact links remain correct when the rollup landing date
                        # differs from the member's merged_at. Avoid search length
                        # and boolean-operator limits for large rollups.
                        links = ', '.join(f'[#{n}]({WEB}/pull/{n})' for n, _ in others[5:])
                        lines += ['<details>', f'<summary>and {len(others) - 5} more maintenance PRs</summary>', '', links, '', '</details>']
                    lines.append('')
                continue
            lines += [f"### {section}", ""]
            lines += ["- " + text for _, text in entries[:cap]]
            if len(entries) > cap:
                query = {"Merged": f"is:pr is:merged merged:{day}", "Releases": "", SECTIONS[2]: f'is:issue label:"help wanted" is:open created:{day}', SECTIONS[3]: f"is:issue label:bug is:closed closed:{day}"}[section]
                url = WEB + ("/releases" if section == "Releases" else "/issues?q=" + quote(query))
                lines.append(f"- [and {len(entries) - cap} more]({url})")
            lines.append("")
    if not groups:
        lines += ["No matching activity.", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--days", type=int, choices=range(1, 31), default=30)
    parser.add_argument("--output", default="CHANGELOG.md")
    args = parser.parse_args()
    now = datetime.now(timezone.utc)
    DEADLINE = time.monotonic() + 110
    try:
        output = render(collect(now, args.days), now, args.days)
    except Exception as error:
        parser.exit(1, f"Changelog generation failed: {error}\n")
    Path(args.output).write_text(output, encoding="utf-8")
    print(f"Wrote {args.output} after {REQUESTS} requests", file=sys.stderr)
