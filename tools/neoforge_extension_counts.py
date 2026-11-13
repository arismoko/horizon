"""Render extension verdict totals from method rows at book-build time."""
from collections import Counter
import re

PAGE = 'reference/neoforge-extensions.md'


def counts(content):
    result = {}
    for name, section in re.findall(r'## `(I\w+Extension)`\n(.*?)(?=\n## |\Z)', content, re.S):
        rows = re.findall(r'^\| `([^`]+)` \| (Covered|Partial|Missing|n/a)\b', section, re.M)
        if name == 'IBlockStateExtension':
            tally = result['IBlockExtension'].copy()
            for _, verdict in rows:
                tally[verdict] -= 1
        else:
            tally = Counter(verdict for _, verdict in rows)
        result[name] = tally
    return result


def render(content):
    tallies = counts(content)
    table = ['| Interface | Public methods | Covered | Partial | Missing | n/a |',
             '| --- | ---: | ---: | ---: | ---: | ---: |']
    for name, tally in tallies.items():
        values = [sum(tally.values())] + [tally[v] for v in ('Covered', 'Partial', 'Missing', 'n/a')]
        table.append(f'| `{name}` | ' + ' | '.join(map(str, values)) + ' |')
    return content.replace('<!-- extension-counts -->', '\n'.join(table))


def preprocess(context, book):
    def visit(items):
        for item in items:
            chapter = item.get('Chapter')
            if chapter:
                if chapter.get('source_path') == PAGE:
                    chapter['content'] = render(chapter['content'])
                visit(chapter['sub_items'])
    visit(book['sections'])
    return book
