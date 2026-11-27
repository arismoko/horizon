//! Compare the typed events omitted from the local layout projection. Components and NBT can
//! be arbitrarily deep in memory, so don't use recursive equality on their child trees.
use super::*;
use sunrise_game_data::identifier::parse_identifier_parts;

pub(super) fn same_events(a: LocalStyle<'_>, b: LocalStyle<'_>) -> bool {
    let mut nodes = Vec::new();
    if !same_click(a.click, b.click) || !same_hover(a.hover, b.hover, &mut nodes) {
        return false;
    }
    while let Some((a, b)) = nodes.pop() {
        if std::ptr::eq(a, b) {
            continue;
        }
        let (sa, sb) = (&a.style, &b.style);
        if sa.color.as_deref().and_then(component::parse_color)
            != sb.color.as_deref().and_then(component::parse_color)
            || sa.shadow_color != sb.shadow_color
            || sa.bold != sb.bold
            || sa.italic != sb.italic
            || sa.underlined != sb.underlined
            || sa.strikethrough != sb.strikethrough
            || sa.obfuscated != sb.obfuscated
            || sa.font != sb.font
            || sa.insertion != sb.insertion
            || !same_click(sa.click.as_ref(), sb.click.as_ref())
            || !same_hover(sa.hover.as_ref(), sb.hover.as_ref(), &mut nodes)
            || a.children.len() != b.children.len()
        {
            return false;
        }
        nodes.extend(a.children.iter().zip(&b.children));
        use TextContent::*;
        let same = match (&a.content, &b.content) {
            (
                Translatable {
                    key: ak,
                    fallback: af,
                    arguments: aa,
                },
                Translatable {
                    key: bk,
                    fallback: bf,
                    arguments: ba,
                },
            ) => {
                nodes.extend(aa.iter().zip(ba));
                ak == bk && af == bf && aa.len() == ba.len()
            }
            (
                Selector {
                    pattern: ap,
                    separator: as_,
                },
                Selector {
                    pattern: bp,
                    separator: bs,
                },
            ) => ap == bp && pair_children(as_.as_deref(), bs.as_deref(), &mut nodes),
            (
                Nbt {
                    path: ap,
                    interpret: ai,
                    source: as_,
                    separator: ax,
                },
                Nbt {
                    path: bp,
                    interpret: bi,
                    source: bs,
                    separator: bx,
                },
            ) => {
                ap == bp
                    && ai == bi
                    && as_ == bs
                    && pair_children(ax.as_deref(), bx.as_deref(), &mut nodes)
            }
            (
                Object(TextObject::Atlas {
                    atlas: aa,
                    sprite: as_,
                    fallback: af,
                }),
                Object(TextObject::Atlas {
                    atlas: ba,
                    sprite: bs,
                    fallback: bf,
                }),
            ) => aa == ba && as_ == bs && pair_children(af.as_deref(), bf.as_deref(), &mut nodes),
            (
                Object(TextObject::Player {
                    profile: ap,
                    hat: ah,
                    fallback: af,
                }),
                Object(TextObject::Player {
                    profile: bp,
                    hat: bh,
                    fallback: bf,
                }),
            ) => {
                ah == bh
                    && ap.id == bp.id
                    && ap.name == bp.name
                    && ap.properties.len() == bp.properties.len()
                    && ap.properties.iter().zip(&bp.properties).all(|(a, b)| {
                        a.name == b.name && a.value == b.value && a.signature == b.signature
                    })
                    && pair_children(af.as_deref(), bf.as_deref(), &mut nodes)
            }
            // All same-variant content containing nested Text was handled above.
            (a, b) => a == b,
        };
        if !same {
            return false;
        }
    }
    true
}

fn same_click(a: Option<&TextClick>, b: Option<&TextClick>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if std::ptr::eq(a, b) => true,
        (Some(TextClick::ShowDialog(a)), Some(TextClick::ShowDialog(b))) => same_tag(a, b),
        (
            Some(TextClick::Custom {
                id: ai,
                payload: ap,
            }),
            Some(TextClick::Custom {
                id: bi,
                payload: bp,
            }),
        ) => {
            // ClickEvent.Custom compares decoded Identifiers, not their input spelling.
            (ai == bi
                || parse_identifier_parts(ai)
                    .zip(parse_identifier_parts(bi))
                    .is_some_and(|(a, b)| a == b))
                && same_optional_tag(ap.as_ref(), bp.as_ref())
        }
        (a, b) => a == b,
    }
}

fn same_hover<'a, 'b>(
    a: Option<&'a TextHover>,
    b: Option<&'b TextHover>,
    nodes: &mut Vec<(&'a Text, &'b Text)>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if std::ptr::eq(a, b) => true,
        (Some(TextHover::Text(a)), Some(TextHover::Text(b))) => {
            nodes.push((a, b));
            true
        }
        (
            Some(TextHover::Item {
                id: ai,
                count: ac,
                components: ap,
            }),
            Some(TextHover::Item {
                id: bi,
                count: bc,
                components: bp,
            }),
        ) => ai == bi && ac == bc && same_optional_tag(ap.as_ref(), bp.as_ref()),
        (
            Some(TextHover::Entity {
                kind: ak,
                id: ai,
                name: an,
            }),
            Some(TextHover::Entity {
                kind: bk,
                id: bi,
                name: bn,
            }),
        ) => ak == bk && ai == bi && pair_children(an.as_deref(), bn.as_deref(), nodes),
        (None, None) => true,
        _ => false,
    }
}

fn pair_children<'a, 'b>(
    a: Option<&'a Text>,
    b: Option<&'b Text>,
    nodes: &mut Vec<(&'a Text, &'b Text)>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            nodes.push((a, b));
            true
        }
        (None, None) => true,
        _ => false,
    }
}

fn same_optional_tag(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_tag(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn same_tag(a: &Tag, b: &Tag) -> bool {
    let mut tags = vec![(a, b)];
    while let Some((a, b)) = tags.pop() {
        if std::ptr::eq(a, b) {
            continue;
        }
        match (a, b) {
            (Tag::List(a), Tag::List(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                tags.extend(a.iter().zip(b));
            }
            (Tag::Compound(a), Tag::Compound(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                for (key, left) in a {
                    let Some(right) = b.get(key) else {
                        return false;
                    };
                    tags.push((left, right));
                }
            }
            (a, b) if a != b => return false,
            _ => {}
        }
    }
    true
}
