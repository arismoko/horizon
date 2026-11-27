//! The command tree players receive (Commands.sendCommands and
//! ClientboundCommandsPacket): vanilla's tree from the data generator's
//! commands report, cut down to the nodes the player's permission level may
//! use and the commands of the server kind (dedicated servers have `op`,
//! `ban`, `stop` ...; an integrated server `publish`), numbered breadth first
//! as vanilla does, with the argument parsers' network properties and the
//! custom suggestion providers (`command_suggestions`). The report sorts each
//! node's children; the tree keeps vanilla's registration order
//! (`command_order`).
use super::{command_order::ORDER, command_suggestions::SUGGESTIONS};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::OnceLock,
};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    packets::play::{CommandNode, Commands},
    registry::Builtin,
};

/// Commands.CommandSelection: registered only on dedicated servers.
const DEDICATED_ONLY: &[&str] = &[
    "ban",
    "ban-ip",
    "banlist",
    "deop",
    "op",
    "pardon",
    "pardon-ip",
    "perf",
    "save-all",
    "save-off",
    "save-on",
    "setidletimeout",
    "stop",
    "transfer",
    "whitelist",
];
/// Registered only on integrated servers.
const INTEGRATED_ONLY: &[&str] = &["publish", "unpublish"];
/// Commands that check permissions only on dedicated servers
/// (`checkPermissions` in SeedCommand and VersionCommand).
const OPEN_WHEN_INTEGRATED: &[&str] = &["seed", "version"];
pub struct CommandTree {
    pub(super) root: Value,
}

/// Extend mod commands with game-session tooling, never vanilla's parity tree.
pub(super) fn with_session_tools(extra: &Value) -> Value {
    use crate::command::vanilla::{MODPROFILE_ACTIONS, MODPROFILE_PERMISSION};
    let children: serde_json::Map<_, _> = MODPROFILE_ACTIONS
        .into_iter()
        .map(|(name, _)| {
            (
                name.to_owned(),
                serde_json::json!({"type":"literal", "executable":true}),
            )
        })
        .collect();
    let mut root = extra.clone();
    root["children"]["modprofile"] = serde_json::json!({
        "type":"literal", "permissions":{"permission":{"level":MODPROFILE_PERMISSION.1}},
        "children": children
    });
    root
}

/// A node of the permission-filtered tree, before wire numbering.
#[derive(Default)]
struct Node {
    name: String,
    kind: u8,
    executable: bool,
    restricted: bool,
    children: Vec<usize>,
    redirect: Option<Vec<String>>,
    parser: Option<(String, Value)>,
    suggestions: Option<String>,
}

/// PermissionLevel names in the report's requirements.
fn required_level(node: &Value) -> u8 {
    match node["permissions"]["permission"]["level"].as_str() {
        Some("moderators") => 1,
        Some("gamemasters") => 2,
        Some("admins") => 3,
        Some("owners") => 4,
        _ => 0,
    }
}

impl CommandTree {
    /// commands.json next to registries.json (the data generator's reports).
    pub fn load(resources: &Path) -> Option<Self> {
        let dir = Builtin::find(resources)?.parent()?.to_path_buf();
        let text = std::fs::read_to_string(dir.join("commands.json")).ok()?;
        Some(Self {
            root: serde_json::from_str(&text).ok()?,
        })
    }

    /// The ClientboundCommandsPacket for a player with `level` permissions.
    pub fn packet(&self, level: u8, dedicated: bool, builtin: &Builtin) -> Commands {
        self.packet_with(level, dedicated, builtin, &Value::Null)
    }

    /// [`CommandTree::packet`] with more commands after vanilla's: `extra`
    /// is a root in the report's format (server mods' commands). A root vanilla registers on
    /// this kind of server takes the mod's nodes beside its own (`add_mod_commands`); a mod's
    /// other roots follow vanilla's.
    pub fn packet_with(
        &self,
        level: u8,
        dedicated: bool,
        builtin: &Builtin,
        extra: &Value,
    ) -> Commands {
        self.packet_with_features(
            level,
            dedicated,
            builtin,
            extra,
            &[],
            &crate::gamerules::rule_registry(),
        )
    }

    pub fn packet_with_features(
        &self,
        level: u8,
        dedicated: bool,
        builtin: &Builtin,
        extra: &Value,
        features: &[String],
        registry: &crate::gamerules::RuleRegistry,
    ) -> Commands {
        self.graph_with_features(level, dedicated, extra, features, registry)
            .packet(builtin, &[])
    }

    /// Build the usable graph without assigning wire IDs yet.
    pub(super) fn graph_with_features(
        &self,
        level: u8,
        dedicated: bool,
        extra: &Value,
        features: &[String],
        registry: &crate::gamerules::RuleRegistry,
    ) -> CommandGraph {
        let minecart_improvements = features
            .iter()
            .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS);
        let CommandGraph {
            mut nodes,
            mut paths,
        } = CommandGraph::default();
        let suggestions: HashMap<&str, &'static str> = SUGGESTIONS.iter().copied().collect();
        let sunrise = if crate::command::vanilla::game_dispatcher()
            .find("pregen")
            .is_some()
        {
            crate::pregen::command_tree()
        } else {
            Value::Null
        };
        let mut root = self.root.clone();
        if let Some(gamerule) = root.get_mut("children").and_then(|c| c.get_mut("gamerule")) {
            let mut rules = serde_json::Map::new();
            for rule in registry.available_rules(minecart_improvements) {
                let value = match rule.default {
                    crate::gamerules::Value::Bool(_) => {
                        serde_json::json!({"type": "argument", "parser": "brigadier:bool", "executable": true})
                    }
                    crate::gamerules::Value::Int(_) => {
                        let mut bounds = serde_json::Map::new();
                        if rule.min != i32::MIN {
                            bounds.insert("min".into(), rule.min.into());
                        }
                        if rule.max != i32::MAX {
                            bounds.insert("max".into(), rule.max.into());
                        }
                        serde_json::json!({"type": "argument", "parser": "brigadier:integer", "properties": bounds, "executable": true})
                    }
                };
                for name in
                    std::iter::once(rule.name.as_str()).chain(rule.name.strip_prefix("minecraft:"))
                {
                    rules.insert(name.to_owned(), serde_json::json!({"type":"literal", "executable": true, "children": {"value": value}}));
                }
            }
            gamerule["children"] = Value::Object(rules);
        }
        // Commands.CommandSelection: vanilla's roots of the other server kind are not registered.
        if let Some(children) = root.get_mut("children").and_then(Value::as_object_mut) {
            children.retain(|name, _| selected(name, dedicated));
        }
        add_mod_commands(&mut root, extra);
        for root in [&root, &sunrise] {
            fill(
                root,
                0,
                &mut Vec::new(),
                level,
                dedicated,
                &suggestions,
                &mut nodes,
                &mut paths,
            );
        }
        CommandGraph { nodes, paths }
    }
}

/// One recipient's usable command nodes, in registration order and without wire indices.
pub struct CommandGraph {
    nodes: Vec<Node>,
    paths: HashMap<Vec<String>, usize>,
}

impl Default for CommandGraph {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default()],
            paths: HashMap::from([(Vec::new(), 0)]),
        }
    }
}

impl CommandGraph {
    /// Names of the usable root children, indexed in registration order.
    pub fn roots(&self) -> impl ExactSizeIterator<Item = &str> + Clone {
        self.nodes[0]
            .children
            .iter()
            .map(|&i| self.nodes[i].name.as_str())
    }

    /// Apply root indexes collected from this graph, then number and construct the packet once.
    pub(super) fn packet(self, builtin: &Builtin, hidden: &[usize]) -> Commands {
        let Self { mut nodes, paths } = self;
        let redirects: Vec<_> = nodes
            .iter_mut()
            .map(|node| {
                node.redirect
                    .take()
                    .and_then(|path| paths.get(&path).copied())
            })
            .collect();
        let mut omit = Vec::new();
        if !hidden.is_empty() {
            // Follow incoming redirects as well as children so no surviving alias
            // reaches a hidden branch (including aliases to descendants or aliases).
            let mut aliases = vec![Vec::new(); nodes.len()];
            for (index, &target) in redirects.iter().enumerate() {
                if let Some(target) = target {
                    aliases[target].push(index);
                }
            }
            omit.resize(nodes.len(), false);
            let mut pending: Vec<_> = hidden.iter().map(|&i| nodes[0].children[i]).collect();
            while let Some(index) = pending.pop() {
                if !std::mem::replace(&mut omit[index], true) {
                    pending.extend(aliases[index].iter().copied());
                    pending.extend(nodes[index].children.iter().copied());
                }
            }
        }

        // ClientboundCommandsPacket.enumerateNodes: children in registration order, then redirect.
        let mut ids = vec![None; nodes.len()];
        let mut order = Vec::new();
        let mut queue = VecDeque::from([0usize]);
        while let Some(n) = queue.pop_front() {
            if (!omit.is_empty() && omit[n]) || ids[n].is_some() {
                continue;
            }
            ids[n] = Some(order.len() as i32);
            order.push(n);
            queue.extend(nodes[n].children.iter().copied());
            queue.extend(redirects[n]);
        }
        let argument_types = builtin.get("minecraft:command_argument_type");
        let nodes = order
            .into_iter()
            .map(|n| {
                let node = std::mem::take(&mut nodes[n]);
                CommandNode {
                    kind: node.kind,
                    executable: node.executable,
                    restricted: node.restricted,
                    children: node.children.into_iter().filter_map(|c| ids[c]).collect(),
                    redirect: redirects[n].and_then(|target| ids[target]),
                    name: node.name,
                    parser: node.parser.map(|(parser, props)| {
                        let id = argument_types.and_then(|t| t.id(&parser)).unwrap_or(0);
                        (id, properties(&parser, &props))
                    }),
                    suggestions: node.suggestions,
                }
            })
            .collect();
        Commands { nodes, root: 0 }
    }
}

/// Whether vanilla registers root `name` on this kind of server (Commands.CommandSelection).
fn selected(name: &str, dedicated: bool) -> bool {
    !(dedicated && INTEGRATED_ONLY.contains(&name) || !dedicated && DEDICATED_ONLY.contains(&name))
}

/// The mods' roots added to vanilla's report as NeoForge's `RegisterCommandsEvent` adds them to
/// vanilla's dispatcher (Brigadier's `CommandNode.addChild`): a root vanilla registers merges
/// into vanilla's node, which keeps its type, permission and suggestions; a mod's executable
/// node makes it executable at the mod's execution level (the later executor replaces
/// vanilla's), and the mod's children are added beside vanilla's, after them. Any other root is
/// the mod's.
fn add_mod_commands(root: &mut Value, extra: &Value) {
    let Some(mods) = extra.get("children").and_then(Value::as_object) else {
        return;
    };
    if !root["children"].is_object() {
        root["children"] = Value::Object(serde_json::Map::new());
    }
    let Some(children) = root["children"].as_object_mut() else {
        return;
    };
    for (name, node) in mods {
        match children.get_mut(name) {
            Some(vanilla) => merge_node(vanilla, node),
            None => {
                children.insert(name.clone(), node.clone());
            }
        }
    }
}

/// `CommandNode.addChild` of a mod's `node` onto the report node `into` of the same name.
fn merge_node(into: &mut Value, node: &Value) {
    if node["executable"] == true {
        into["executable"] = true.into();
        into["execution_level"] = node["execution_level"].clone();
    }
    let Some(added) = node["children"].as_object() else {
        return;
    };
    let Some(into) = into.as_object_mut() else {
        return;
    };
    let children = into
        .entry("children")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(children) = children.as_object_mut() else {
        return;
    };
    for (name, child) in added {
        match children.get_mut(name) {
            Some(existing) => merge_node(existing, child),
            None => {
                children.insert(name.clone(), child.clone());
            }
        }
    }
}

/// Commands.fillUsableCommands over the report: the children the source may use.
#[allow(clippy::too_many_arguments)]
fn fill(
    source: &Value,
    target: usize,
    path: &mut Vec<String>,
    level: u8,
    dedicated: bool,
    suggestions: &HashMap<&str, &'static str>,
    nodes: &mut Vec<Node>,
    paths: &mut HashMap<Vec<String>, usize>,
) {
    let Some(children) = source["children"].as_object() else {
        return;
    };
    let mut names: Vec<&String> = children.keys().collect();
    names.sort();
    // Vanilla's registration order; names it doesn't list (the game rules,
    // registered in an identity-hashed order) keep the report's.
    static ORDERS: OnceLock<HashMap<&str, Vec<&str>>> = OnceLock::new();
    let orders = ORDERS.get_or_init(|| {
        ORDER
            .iter()
            .map(|(path, names)| (*path, names.split(' ').collect()))
            .collect()
    });
    if let Some(order) = orders.get(path.join(" ").as_str()) {
        names.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(order.len()));
    }
    for name in names {
        let child = &children[name];
        let required =
            if path.is_empty() && !dedicated && OPEN_WHEN_INTEGRATED.contains(&name.as_str()) {
                0
            } else {
                required_level(child)
            };
        if required > level {
            continue;
        }
        path.push(name.clone());
        let argument = child["type"] == "argument";
        let index = nodes.len();
        nodes.push(Node {
            name: name.clone(),
            kind: if argument { 2 } else { 1 },
            executable: child["executable"].as_bool().unwrap_or(false)
                && child["execution_level"].as_u64().unwrap_or(0) <= u64::from(level),
            restricted: required > 0,
            children: Vec::new(),
            redirect: redirect(child),
            parser: argument.then(|| {
                (
                    child["parser"].as_str().unwrap_or_default().to_owned(),
                    child["properties"].clone(),
                )
            }),
            suggestions: if argument {
                // A mod's argument type the client does not know asks the server, like a vanilla
                // argument whose provider is `ask_server`; vanilla's own providers are the
                // static table by path.
                child["suggestions"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        suggestions
                            .get(path.join(" ").as_str())
                            .map(|s| (*s).to_owned())
                    })
            } else {
                None
            },
        });
        nodes[target].children.push(index);
        paths.insert(path.clone(), index);
        fill(
            child,
            index,
            path,
            level,
            dedicated,
            suggestions,
            nodes,
            paths,
        );
        path.pop();
    }
}

/// The redirect target's path. The report leaves out redirects to the root
/// (`execute ... run`, `return run`): a node that neither runs nor leads
/// anywhere forks to the root.
fn redirect(node: &Value) -> Option<Vec<String>> {
    if let Some(path) = node["redirect"].as_array() {
        return Some(
            path.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
        );
    }
    let dead_end = node["children"].as_object().is_none_or(|c| c.is_empty())
        && !node["executable"].as_bool().unwrap_or(false);
    dead_end.then(Vec::new)
}

/// ArgumentTypeInfo.serializeToNetwork for the report's properties.
fn properties(parser: &str, p: &Value) -> Vec<u8> {
    let mut w = Writer::new();
    let flags = |w: &mut Writer| {
        w.u8(u8::from(p.get("min").is_some()) | u8::from(p.get("max").is_some()) << 1);
    };
    match parser {
        "brigadier:integer" => {
            flags(&mut w);
            for key in ["min", "max"] {
                if let Some(v) = p[key].as_i64() {
                    w.i32(v as i32);
                }
            }
        }
        "brigadier:long" => {
            flags(&mut w);
            for key in ["min", "max"] {
                if let Some(v) = p[key].as_i64() {
                    w.i64(v);
                }
            }
        }
        "brigadier:float" => {
            flags(&mut w);
            for key in ["min", "max"] {
                if let Some(v) = p[key].as_f64() {
                    w.f32(v as f32);
                }
            }
        }
        "brigadier:double" => {
            flags(&mut w);
            for key in ["min", "max"] {
                if let Some(v) = p[key].as_f64() {
                    w.f64(v);
                }
            }
        }
        "brigadier:string" => w.varint(match p["type"].as_str() {
            Some("phrase") => 1,
            Some("greedy") => 2,
            _ => 0,
        }),
        "minecraft:entity" => {
            let single = u8::from(p["amount"] == "single");
            let players = u8::from(p["type"] == "players");
            w.u8(single | players << 1);
        }
        "minecraft:score_holder" => w.u8(u8::from(p["amount"] == "multiple")),
        "minecraft:time" => w.i32(p["min"].as_i64().unwrap_or(0) as i32),
        "minecraft:resource"
        | "minecraft:resource_key"
        | "minecraft:resource_or_tag"
        | "minecraft:resource_or_tag_key"
        | "minecraft:resource_selector" => w.string(p["registry"].as_str().unwrap_or_default()),
        _ => {}
    }
    w.data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_filter_removes_branches_and_remaps_the_ordered_wire_graph() {
        let node = |name: &str, children: Vec<i32>, redirect: Option<i32>| CommandNode {
            name: name.into(),
            kind: u8::from(!name.is_empty()),
            executable: children.is_empty() && redirect.is_none(),
            restricted: false,
            children,
            redirect,
            parser: None,
            suggestions: None,
        };
        let tree = CommandTree {
            root: serde_json::json!({"type": "root", "children": {
                "a_hide": {"type": "literal", "children": {"hidden_child": {"type": "literal", "executable": true}}},
                "b_keep": {"type": "literal", "children": {"kept_child": {"type": "literal", "executable": true}}},
                "c_hidden_alias": {"type": "literal", "redirect": ["a_hide"]},
                "d_indirect_alias": {"type": "literal", "redirect": ["c_hidden_alias"]},
                "e_child_alias": {"type": "literal", "redirect": ["a_hide", "hidden_child"]},
                "f_kept_alias": {"type": "literal", "redirect": ["b_keep"]}
            }}),
        };
        let graph = tree.graph_with_features(
            0,
            true,
            &Value::Null,
            &[],
            &crate::gamerules::rule_registry(),
        );
        assert_eq!(graph.roots().next(), Some("a_hide"));
        let packet = graph.packet(&Builtin::default(), &[0]);

        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        let encoded = Clientbound::Commands(packet).encode();
        let mut reader = Reader::new(&encoded);
        let id = reader.varint().unwrap();
        let Clientbound::Commands(decoded) = Clientbound::decode(id, reader.rest()).unwrap() else {
            panic!("expected outgoing command tree");
        };
        assert_eq!(
            decoded,
            Commands {
                root: 0,
                nodes: vec![
                    node("", vec![1, 2], None),
                    node("b_keep", vec![3], None),
                    node("f_kept_alias", vec![], Some(1)),
                    node("kept_child", vec![], None),
                ],
            },
        );
    }

    #[test]
    fn pregen_completion_keeps_permissions_and_radius_bounds() {
        let tree = CommandTree {
            root: crate::pregen::command_tree(),
        };
        let builtin = Builtin::default();
        assert_eq!(tree.packet(0, false, &builtin).nodes.len(), 1);
        let packet = tree.packet(2, false, &builtin);
        let root = packet.nodes.iter().find(|n| n.name == "pregen").unwrap();
        assert!(root.restricted && !root.executable);
        let radius = packet.nodes.iter().find(|n| n.name == "radius").unwrap();
        assert!(radius.executable);
        assert_eq!(
            radius.parser.as_ref().unwrap().1,
            vec![3, 0, 0, 0, 0, 1, 201, 195, 128]
        );
        let shapes: Vec<_> = radius
            .children
            .iter()
            .map(|&i| packet.nodes[i as usize].name.as_str())
            .collect();
        assert_eq!(shapes, ["circle", "square"]);
        assert_eq!(
            packet
                .nodes
                .iter()
                .filter(|n| n.name == "center" && n.executable)
                .count(),
            2
        );
    }

    #[test]
    fn integer_bounds_and_entity_flags() {
        let p: Value = serde_json::from_str(r#"{"min": 1, "max": 1000}"#).unwrap();
        assert_eq!(
            properties("brigadier:integer", &p),
            vec![3, 0, 0, 0, 1, 0, 0, 3, 0xE8]
        );
        let p: Value = serde_json::from_str(r#"{"amount": "single", "type": "players"}"#).unwrap();
        assert_eq!(properties("minecraft:entity", &p), vec![3]);
        let p: Value = serde_json::from_str(r#"{"type": "greedy"}"#).unwrap();
        assert_eq!(properties("brigadier:string", &p), vec![2]);
    }

    /// `StringArgumentSerializer` writes the `StringType` ordinal as a varint: `SINGLE_WORD` 0,
    /// `QUOTABLE_PHRASE` 1, `GREEDY_PHRASE` 2; the report names them `word`, `phrase`, `greedy`.
    #[test]
    fn string_modes_are_brigadiers_ordinals() {
        for (mode, wire) in [("word", 0), ("phrase", 1), ("greedy", 2)] {
            let p = serde_json::json!({ "type": mode });
            assert_eq!(properties("brigadier:string", &p), vec![wire], "{mode}");
        }
    }

    /// A mod's root of a vanilla name merges into vanilla's node, as NeoForge's
    /// `RegisterCommandsEvent` registers into vanilla's dispatcher (Brigadier's
    /// `CommandNode.addChild`): one `stop`, vanilla's (still executable, still restricted), with
    /// the mod's `now` beside its own nodes; a mod's executable node replaces vanilla's executor
    /// at the mod's level. On a server kind vanilla does not register `stop` on, the mod's root
    /// is the only one.
    #[test]
    fn a_mods_root_merges_into_vanillas_of_its_name() {
        let report: Value = serde_json::from_str(
            r#"{"type":"root","children":{
                "me":{"type":"literal","children":{"action":{"type":"argument","parser":"minecraft:message","executable":true}}},
                "stop":{"type":"literal","executable":true,"permissions":{"type":"require","permission":{"level":"owners"}},
                    "children":{"later":{"type":"literal","executable":true}}},
                "seed":{"type":"literal","executable":true}}}"#,
        )
        .unwrap();
        let mods: Value = serde_json::from_str(
            r#"{"type":"root","children":{
                "stop":{"type":"literal","executable":false,"children":{"now":{"type":"literal","executable":true,"execution_level":0}}},
                "seed":{"type":"literal","executable":true,"execution_level":3}}}"#,
        )
        .unwrap();
        let tree = CommandTree { root: report };
        let builtin = Builtin::default();
        let packet = tree.packet_with(4, true, &builtin, &mods);
        let roots: Vec<_> = packet.nodes[0]
            .children
            .iter()
            .map(|&c| &packet.nodes[c as usize])
            .collect();
        let stops: Vec<_> = roots.iter().filter(|n| n.name == "stop").collect();
        assert_eq!(stops.len(), 1, "one root of the name");
        assert!(stops[0].executable && stops[0].restricted, "vanilla's node");
        let children: Vec<_> = stops[0]
            .children
            .iter()
            .map(|&c| packet.nodes[c as usize].name.as_str())
            .collect();
        assert_eq!(
            children,
            ["later", "now"],
            "vanilla's usages stay, the mod's follow"
        );
        assert!(roots.iter().any(|n| n.name == "me"), "other roots stay");
        // The mod's executor replaced vanilla's at `seed`, at the mod's execution level.
        let seed = |level| {
            let packet = tree.packet_with(level, true, &builtin, &mods);
            packet
                .nodes
                .iter()
                .find(|n| n.name == "seed")
                .unwrap()
                .executable
        };
        assert!(seed(3));
        assert!(!seed(2));
        let integrated = tree.packet_with(4, false, &builtin, &mods);
        let stop = integrated.nodes.iter().find(|n| n.name == "stop").unwrap();
        assert!(
            !stop.executable,
            "the mod's own root where vanilla has none"
        );
        assert!(!integrated.nodes.iter().any(|n| n.name == "later"));
    }

    #[test]
    fn levels_filter_and_redirects_resolve() {
        let report: Value = serde_json::from_str(
            r#"{"type":"root","children":{
                "say":{"type":"literal","children":{"message":{"type":"argument","parser":"minecraft:message","executable":true}},
                       "permissions":{"type":"minecraft:require","permission":{"type":"minecraft:command_level","level":"gamemasters"}}},
                "me":{"type":"literal","children":{"action":{"type":"argument","parser":"minecraft:message","executable":true}}},
                "stop":{"type":"literal","executable":true},
                "tell":{"type":"literal","redirect":["me"]}}}"#,
        )
        .unwrap();
        let tree = CommandTree { root: report };
        let builtin = Builtin::default();
        let guest = tree.packet(0, false, &builtin);
        let names: Vec<_> = guest.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["", "me", "tell", "action"]);
        // `tell` redirects to `me` (node 1).
        assert_eq!(guest.nodes[2].redirect, Some(1));
        let op = tree.packet(2, true, &builtin);
        let names: Vec<_> = op.nodes.iter().map(|n| n.name.as_str()).collect();
        // The root's children in vanilla's registration order.
        assert_eq!(
            names,
            ["", "me", "tell", "say", "stop", "action", "message"]
        );
        assert!(op.nodes[3].restricted);
    }

    #[test]
    fn native_execution_levels_filter_runnable_flags_not_public_children() {
        let mods = serde_json::json!({"type": "root", "children": {
            "foo": {"type": "literal", "executable": true, "execution_level": 4,
                "children": {"bar": {"type": "literal", "executable": true, "execution_level": 0}}}
        }});
        let tree = CommandTree { root: Value::Null };
        for level in 0..=4 {
            let packet = tree.packet_with(level, true, &Builtin::default(), &mods);
            let foo = packet.nodes.iter().find(|n| n.name == "foo").unwrap();
            assert_eq!(foo.executable, level == 4);
            assert!(!foo.restricted, "public child keeps the root traversable");
            assert!(
                packet
                    .nodes
                    .iter()
                    .find(|n| n.name == "bar")
                    .unwrap()
                    .executable
            );
            let dispatcher =
                crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
            let registries = crate::command::Registries::default();
            assert_eq!(
                dispatcher.parse("foo", &registries).executable().is_ok(),
                level == 4
            );
            assert!(
                dispatcher
                    .parse("foo bar", &registries)
                    .executable()
                    .is_ok()
            );
        }
    }

    #[test]
    fn a_mods_custom_argument_is_a_word_that_asks_the_server() {
        let mut builtin = Builtin::default();
        builtin.registries.insert(
            "minecraft:command_argument_type".into(),
            sunrise_protocol::registry::Ids::from_names(
                sunrise_protocol::packets::play::ARGUMENT_TYPES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            ),
        );
        let mods = serde_json::json!({"type": "root", "children": {
            "machine": {"type": "literal", "children": {
                "use": {"type": "literal", "children": {
                    "mode": {"type": "argument", "parser": "brigadier:string",
                        "properties": {"type": "word"}, "custom_argument": "test:machine",
                        "suggestions": "minecraft:ask_server", "executable": true}
                }}
            }}
        }});
        let tree = CommandTree { root: Value::Null };
        let packet = tree.packet_with(0, true, &builtin, &mods);
        let mode_node = packet.nodes.iter().find(|n| n.name == "mode").unwrap();
        assert_eq!(
            mode_node.suggestions.as_deref(),
            Some("minecraft:ask_server")
        );
        let (id, properties) = mode_node.parser.as_ref().unwrap();
        assert_eq!(
            sunrise_protocol::packets::play::ARGUMENT_TYPES[*id as usize],
            "brigadier:string"
        );
        let expected = sunrise_protocol::packets::play::ArgumentProperties::String(0);
        assert_eq!(
            sunrise_protocol::packets::play::ArgumentProperties::decode(*id, properties),
            Ok(expected),
            "a vanilla client reads the custom argument as one word"
        );

        let dispatcher =
            crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
        let registries = crate::command::Registries::default();
        assert!(
            dispatcher
                .parse("machine use press", &registries)
                .executable()
                .is_ok()
        );
        let typed = "machine use pr";
        let (_, _, asks) = dispatcher.client_suggestions_asking(
            &dispatcher.parse(typed, &registries),
            typed.len(),
            &registries,
        );
        assert!(asks, "the client asks the server for the type's values");
    }

    #[test]
    fn native_argument_alternatives_survive_packet_reconstruction() {
        let mut builtin = Builtin::default();
        builtin.registries.insert(
            "minecraft:command_argument_type".into(),
            sunrise_protocol::registry::Ids::from_names(
                sunrise_protocol::packets::play::ARGUMENT_TYPES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            ),
        );
        let mods = serde_json::json!({"type": "root", "children": {
            "foo": {"type": "literal", "children": {
                "a": {"type": "argument", "parser": "brigadier:integer", "executable": true, "execution_level": 4,
                    "children": {"bar": {"type": "literal", "executable": true, "execution_level": 0}}},
                "b": {"type": "argument", "parser": "brigadier:string", "properties": {"type": "word"}, "executable": true, "execution_level": 0}
            }},
            "denied": {"type": "literal", "children": {
                "a": {"type": "argument", "parser": "brigadier:integer", "executable": true, "execution_level": 4,
                    "children": {"bar": {"type": "literal", "executable": true, "execution_level": 0}}}
            }}
        }});
        let tree = CommandTree { root: Value::Null };
        for level in 0..=4 {
            let packet = tree.packet_with(level, true, &builtin, &mods);
            let dispatcher =
                crate::command::network::from_packet(&packet, &crate::command::Dispatcher::new());
            let registries = crate::command::Registries::default();
            let parsed = dispatcher.parse("foo 7", &registries);
            let (context, _) = parsed.executable().unwrap();
            assert_eq!(context.get("b").is_some(), level < 4);
            assert!(
                dispatcher
                    .parse("foo 7 bar", &registries)
                    .executable()
                    .is_ok()
            );
            assert_eq!(
                dispatcher
                    .parse("denied 7", &registries)
                    .executable()
                    .is_ok(),
                level == 4
            );
        }
    }

    #[test]
    fn server_mods_commands_follow_vanillas() {
        let report: Value = serde_json::from_str(
            r#"{"type":"root","children":{"me":{"type":"literal","children":{"action":{"type":"argument","parser":"minecraft:message","executable":true}}}}}"#,
        )
        .unwrap();
        let mods: Value = serde_json::from_str(
            r#"{"type":"root","children":{"beacon":{"type":"literal","executable":true,"children":{
                "height":{"type":"argument","parser":"brigadier:integer","properties":{"min":1,"max":64},"executable":true}}}}}"#,
        )
        .unwrap();
        let tree = CommandTree { root: report };
        let packet = tree.packet_with(0, true, &Builtin::default(), &mods);
        let names: Vec<_> = packet.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["", "me", "beacon", "action", "height"]);
        let beacon = &packet.nodes[2];
        assert!(beacon.executable && !beacon.restricted);
        assert_eq!(beacon.children, [4]);
        let height = packet.nodes[4].parser.as_ref().unwrap();
        assert_eq!(height.1, vec![3, 0, 0, 0, 1, 0, 0, 0, 64]);
        // No mods: vanilla's tree as it is.
        assert_eq!(
            tree.packet_with(0, true, &Builtin::default(), &Value::Null)
                .nodes
                .len(),
            3
        );
    }
}

/// Sunrise selector metadata sent beside Commands.
pub mod selector_sync;
