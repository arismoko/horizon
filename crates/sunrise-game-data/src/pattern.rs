//! A small regular-expression subset for `pack.mcmeta` filter blocks (Java
//! `Pattern.asPredicate()` semantics: an unanchored search). Supports
//! literals, `.`, classes `[a-z_]`/`[^..]`, escapes, `* + ?` quantifiers,
//! `^`/`$`, and alternation with `|` inside groups `( )` / `(?: )`.

#[derive(Clone, Debug, PartialEq)]
enum Atom {
    Any,
    Lit(char),
    Class(Vec<(char, char)>, bool),
    Start,
    End,
}
#[derive(Clone, Debug, PartialEq)]
enum Node {
    Atom(Atom, usize, usize),
    /// Alternatives, each a sequence, repeated `min..=max` times.
    Group(Vec<Vec<Node>>, usize, usize),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern(Vec<Vec<Node>>);

/// The alternatives up to the end of the source, or up to the `)` closing a group
/// (left unread at `i`).
fn alternatives(chars: &[char], i: &mut usize, in_group: bool) -> Option<Vec<Vec<Node>>> {
    let mut alternatives = vec![Vec::new()];
    while *i < chars.len() {
        let atom = match chars[*i] {
            '|' => {
                alternatives.push(Vec::new());
                *i += 1;
                continue;
            }
            ')' if in_group => return Some(alternatives),
            '(' => {
                *i += 1;
                if chars.get(*i) == Some(&'?') {
                    if chars.get(*i + 1) != Some(&':') {
                        return None;
                    }
                    *i += 2;
                }
                let inner = alternatives_of_group(chars, i)?;
                let (min, max) = quantifier(chars, i);
                alternatives.last_mut()?.push(Node::Group(inner, min, max));
                continue;
            }
            '.' => Atom::Any,
            '^' => Atom::Start,
            '$' => Atom::End,
            '\\' => {
                *i += 1;
                match *chars.get(*i)? {
                    'd' => Atom::Class(vec![('0', '9')], false),
                    'w' => Atom::Class(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], false),
                    c => Atom::Lit(c),
                }
            }
            '[' => {
                *i += 1;
                let negate = chars.get(*i) == Some(&'^');
                if negate {
                    *i += 1;
                }
                let mut ranges = Vec::new();
                loop {
                    let mut c = *chars.get(*i)?;
                    if c == ']' && !ranges.is_empty() {
                        break;
                    }
                    if c == '\\' {
                        *i += 1;
                        c = *chars.get(*i)?;
                    }
                    if chars.get(*i + 1) == Some(&'-')
                        && chars.get(*i + 2).is_some_and(|e| *e != ']')
                    {
                        ranges.push((c, chars[*i + 2]));
                        *i += 3;
                    } else {
                        ranges.push((c, c));
                        *i += 1;
                    }
                }
                Atom::Class(ranges, negate)
            }
            ')' | '{' | '}' => return None,
            c => Atom::Lit(c),
        };
        *i += 1;
        let (min, max) = quantifier(chars, i);
        alternatives.last_mut()?.push(Node::Atom(atom, min, max));
    }
    (!in_group).then_some(alternatives)
}
/// A group's alternatives and its closing `)`, after the opening one.
fn alternatives_of_group(chars: &[char], i: &mut usize) -> Option<Vec<Vec<Node>>> {
    let inner = alternatives(chars, i, true)?;
    (chars.get(*i) == Some(&')')).then(|| *i += 1)?;
    Some(inner)
}
fn quantifier(chars: &[char], i: &mut usize) -> (usize, usize) {
    let bounds = match chars.get(*i) {
        Some('*') => (0, usize::MAX),
        Some('+') => (1, usize::MAX),
        Some('?') => (0, 1),
        _ => return (1, 1),
    };
    *i += 1;
    bounds
}

impl Pattern {
    pub fn new(src: &str) -> Option<Pattern> {
        let chars: Vec<char> = src.chars().collect();
        alternatives(&chars, &mut 0, false).map(Pattern)
    }
    pub fn is_match(&self, text: &str) -> bool {
        let text: Vec<char> = text.chars().collect();
        (0..=text.len()).any(|start| self.0.iter().any(|alt| run(alt, &text, start, &|_| true)))
    }
}
fn one(atom: &Atom, c: char) -> bool {
    match atom {
        Atom::Any => c != '\n',
        Atom::Lit(l) => *l == c,
        Atom::Class(r, negate) => r.iter().any(|(lo, hi)| *lo <= c && c <= *hi) != *negate,
        Atom::Start | Atom::End => false,
    }
}
/// Whether `nodes` match at `at` and `done` accepts the position they end at.
fn run(nodes: &[Node], text: &[char], at: usize, done: &dyn Fn(usize) -> bool) -> bool {
    let Some((node, rest)) = nodes.split_first() else {
        return done(at);
    };
    match node {
        Node::Atom(Atom::Start, ..) => at == 0 && run(rest, text, at, done),
        Node::Atom(Atom::End, ..) => at == text.len() && run(rest, text, at, done),
        Node::Atom(atom, min, max) => {
            let mut count = 0;
            while count < *max && at + count < text.len() && one(atom, text[at + count]) {
                count += 1;
            }
            (*min..=count).rev().any(|n| run(rest, text, at + n, done))
        }
        Node::Group(alts, min, max) => repeat(alts, *min, *max, 0, text, at, &|end| {
            run(rest, text, end, done)
        }),
    }
}
/// `count` repetitions of the group are matched; try one more, else leave it to `done`.
fn repeat(
    alts: &[Vec<Node>],
    min: usize,
    max: usize,
    count: usize,
    text: &[char],
    at: usize,
    done: &dyn Fn(usize) -> bool,
) -> bool {
    (count < max
        && alts.iter().any(|alt| {
            run(alt, text, at, &|end| {
                // An empty repetition past the minimum could loop forever.
                (end != at || count < min) && repeat(alts, min, max, count + 1, text, end, done)
            })
        }))
        || (count >= min && done(at))
}

#[cfg(test)]
mod tests {
    use super::Pattern;
    fn m(p: &str, t: &str) -> bool {
        Pattern::new(p).unwrap().is_match(t)
    }
    #[test]
    fn java_find_semantics() {
        assert!(m("stone", "minecraft/stone_bricks"));
        assert!(!m("^stone$", "stone_bricks"));
        assert!(m("^textures/.*\\.png$", "textures/block/a.png"));
        assert!(!m("^textures/.*\\.png$", "textures/block/a_png"));
        assert!(m("[a-c]+z", "xxabcz"));
        assert!(!m("^[^a]+$", "bab"));
        assert!(m("dirt|grass", "grass_block"));
        assert!(m(".*", ""));
        assert!(m(
            "^(worldgen/noise|dimension)/hidden[.]json$",
            "dimension/hidden.json"
        ));
        assert!(!m(
            "^(worldgen/noise|dimension)/hidden[.]json$",
            "worldgen/noise/kept.json"
        ));
        assert!(m("^a(?:bc)+d$", "abcbcd"));
        assert!(!m("^a(bc)+d$", "ad"));
        assert!(Pattern::new("(a").is_none());
        assert!(Pattern::new("a)").is_none());
    }
}
