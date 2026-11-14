#!/usr/bin/env python3
"""Posts a rollup or sync pull request, opened or merged, to the team's Discord channel.

The workflow `merge-updates.yml` passes the pull request through the environment; the message
is one embed with the state and title ("Merged: Rollup 62"), the link and the body's `## Summary` section.
"""
import json
import os
import re
import sys
import urllib.request

TITLE_LIMIT = 250
DESCRIPTION_LIMIT = 3500
COLOURS = {"opened": 0x3498DB, "merged": 0x2ECC71}
HEADING = re.compile(r"^##\s")
TABLE_ROW = re.compile(r"^\s*\|.*\|\s*$")
TABLE_RULE = re.compile(r"^\s*\|[\s:|-]+\|\s*$")


def summary(body):
    """The text under `## Summary`, up to the next `## ` heading, without the disclosure line."""
    lines = (body or "").replace("\r\n", "\n").split("\n")
    start = next((i for i, line in enumerate(lines) if line.strip().lower() == "## summary"), None)
    if start is None:
        return ""
    section = []
    for line in lines[start + 1:]:
        if HEADING.match(line):
            break
        if line.lstrip().startswith("> 🤖"):
            continue
        section.append(line)
    return "\n".join(plain_rows(section)).strip()


def plain_rows(lines):
    """Discord draws no tables: each table row becomes a list line ("- a · b"), and the header and rule rows go."""
    out = []
    for index, line in enumerate(lines):
        if not TABLE_ROW.match(line):
            out.append(line)
        elif TABLE_RULE.match(line) or (index + 1 < len(lines) and TABLE_RULE.match(lines[index + 1])):
            continue
        else:
            out.append("- " + " · ".join(cell.strip() for cell in line.strip().strip("|").split("|")))
    return out


def cut(text, limit):
    """`text` within `limit` characters, ended at a paragraph break (else a line, else a word) with an ellipsis."""
    if len(text) <= limit:
        return text
    room = text[: limit - 1]
    for mark in ("\n\n", "\n", " "):
        end = room.rfind(mark)
        if end > 0:
            return room[:end].rstrip() + "…"
    return room + "…"


def payload(kind, state, title, url, body):
    """The webhook message: one embed, no pings."""
    prefix = f"{state.capitalize()}: "
    embed = {
        "title": prefix + cut(title, TITLE_LIMIT - len(prefix)),
        "url": url,
        "color": COLOURS[state],
        "footer": {"text": f"{kind.capitalize()} {state}"},
    }
    text = summary(body)
    if text:
        embed["description"] = cut(text, DESCRIPTION_LIMIT)
    # 4096 is SUPPRESS_NOTIFICATIONS: a merge post never pushes or plays a sound, whatever a member's settings.
    return {"embeds": [embed], "allowed_mentions": {"parse": []}, "flags": 4096}


def main():
    env = os.environ
    message = payload(env["KIND"], env["STATE"], env["PR_TITLE"], env["PR_URL"], env.get("PR_BODY", ""))
    request = urllib.request.Request(
        env["WEBHOOK"],
        data=json.dumps(message).encode(),
        headers={"Content-Type": "application/json", "User-Agent": "horizon-merge-updates"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        print(f"posted: HTTP {response.status}")


if __name__ == "__main__":
    sys.exit(main())
