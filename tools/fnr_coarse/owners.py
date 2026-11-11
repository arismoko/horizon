"""Collapse the IR graph to source functions of horizon_client (owners)."""
import json, re
from collections import defaultdict

GAME = 'horizon_client::'


def strip_generics(s):
    out, depth = [], 0
    i = 0
    # remove ::<...> turbofish groups
    while i < len(s):
        if s.startswith('::<', i) and depth == 0:
            d = 0
            j = i + 2
            while j < len(s):
                if s[j] == '<':
                    d += 1
                elif s[j] == '>':
                    d -= 1
                    if d == 0:
                        break
                j += 1
            i = j + 1
            continue
        out.append(s[i])
        i += 1
    return ''.join(out)


def owner(name):
    """The horizon_client source function a symbol belongs to, or None (std, other crates, shims)."""
    if '{shim' in name or name.startswith('core::ptr::drop_in_place'):
        return None
    n = strip_generics(name)
    n = re.sub(r'::\{closure#\d+\}.*$', '', n)
    n = re.sub(r'::\{constant#\d+\}.*$', '', n)
    if n.startswith(GAME):
        return n
    if n.startswith('<'):
        # <Type>::f or <Type as Trait>::f : a game impl if the self type is a game type
        d = 0
        for j, c in enumerate(n):
            if c == '<':
                d += 1
            elif c == '>':
                d -= 1
                if d == 0:
                    break
        inner = n[1:j]
        selfty = inner.split(' as ')[0]
        if selfty.startswith(GAME) and '{closure' not in selfty:
            # a game type's method, possibly of a foreign trait
            return n
    return None


def load(path):
    g = json.load(open(path))
    name = g['name']
    own = {s: owner(name[s]) for s in g['defined']}
    return g, name, own


def key(n):
    """Match form: drop lifetimes and whitespace, and module path of inherent impls."""
    n = re.sub(r"<'_>|'_, |<'[a-z]+>|'[a-z]+, ", '', n).replace(' ', '')
    return n
