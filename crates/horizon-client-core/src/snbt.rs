//! Tag text output: `StringTagVisitor` (compact SNBT) and
//! `TextComponentTagVisitor` (the coloured form commands print).
use crate::text::{self, Style, Text};
use horizon_core::java_number::{double_to_string, float_to_string};
use horizon_core::nbt::Tag;
use horizon_world::snbt_text::string_of;
pub use horizon_world::snbt_text::{quote_and_escape, to_snbt};

// ---- the coloured form -----------------------------------------------------


struct Pretty {
    indentation: String,
    result: Text,
    indent_depth: usize,
    depth: usize,
}
fn is_numeric(t: &Tag) -> bool {
    matches!(
        t,
        Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_)
    )
}
impl Pretty {
    fn lit(&mut self, s: &str, color: Option<u32>) {
        let t = match color {
            Some(c) => Text::styled(s, Style::color(c)),
            None => Text::literal(s),
        };
        self.result.push(t);
    }
    fn number(&mut self, s: &str, suffix: &str) {
        self.lit(s, Some(text::GOLD));
        if !suffix.is_empty() {
            self.lit(suffix, Some(text::RED));
        }
    }
    fn quoted(&mut self, input: &str, color: u32) {
        let q = quote_and_escape(input);
        let (open, inner) = (&q[..1], &q[1..q.len() - 1]);
        self.lit(open, None);
        self.lit(inner, Some(color));
        self.lit(open, None);
    }
    fn array<T: std::fmt::Display>(&mut self, prefix: &str, items: &[T], suffix: &str) {
        self.lit("[", None);
        self.lit(prefix, Some(text::RED));
        self.lit(";", None);
        for (i, v) in items.iter().enumerate().take(128) {
            self.lit(" ", None);
            self.number(&v.to_string(), suffix);
            if i != items.len() - 1 {
                self.lit(",", None);
            }
        }
        if items.len() > 128 {
            self.lit("<...>", Some(text::GRAY));
        }
        self.lit("]", None);
    }
    fn sub(&mut self, tag: &Tag, indent: bool) {
        if indent {
            self.indent_depth += 1;
        }
        self.depth += 1;
        self.visit(tag);
        if indent {
            self.indent_depth -= 1;
        }
        self.depth -= 1;
    }
    fn visit(&mut self, tag: &Tag) {
        match tag {
            Tag::End => {}
            Tag::Byte(v) => self.number(&v.to_string(), "b"),
            Tag::Short(v) => self.number(&v.to_string(), "s"),
            Tag::Int(v) => self.number(&v.to_string(), ""),
            Tag::Long(v) => self.number(&v.to_string(), "L"),
            Tag::Float(v) => self.number(&float_to_string(f32::from_bits(*v)), "f"),
            Tag::Double(v) => self.number(&double_to_string(f64::from_bits(*v)), "d"),
            Tag::String(s) => self.quoted(&string_of(s), text::GREEN),
            Tag::ByteArray(a) => {
                let v: Vec<i8> = a.iter().map(|b| *b as i8).collect();
                self.array("B", &v, "b");
            }
            Tag::IntArray(a) => self.array("I", a, ""),
            Tag::LongArray(a) => self.array("L", a, "L"),
            Tag::List(l) => self.list(l),
            Tag::Compound(c) => self.compound(c),
        }
    }
    fn list(&mut self, l: &[Tag]) {
        if l.is_empty() {
            self.lit("[", None);
            self.lit("]", None);
        } else if self.depth >= 64 {
            self.lit("[", None);
            self.lit("<...>", Some(text::GRAY));
            self.lit("]", None);
        } else if l.len() >= 8 || l.iter().all(is_numeric) {
            self.lit("[", None);
            for (i, t) in l.iter().enumerate() {
                if i != 0 {
                    self.lit(",", None);
                    self.lit(" ", None);
                }
                self.sub(t, false);
            }
            self.lit("]", None);
        } else {
            self.lit("[", None);
            if !self.indentation.is_empty() {
                self.lit("\n", None);
            }
            let entry = self.indentation.repeat(self.indent_depth + 1);
            let spacing = if self.indentation.is_empty() {
                " "
            } else {
                "\n"
            };
            for (i, t) in l.iter().enumerate().take(128) {
                self.lit(&entry, None);
                self.sub(t, true);
                if i != l.len() - 1 {
                    self.lit(",", None);
                    self.lit(spacing, None);
                }
            }
            if l.len() > 128 {
                self.lit(&entry, None);
                self.lit("<...>", Some(text::GRAY));
            }
            if !self.indentation.is_empty() {
                self.lit("\n", None);
                let close = self.indentation.repeat(self.indent_depth);
                self.lit(&close, None);
            }
            self.lit("]", None);
        }
    }
    fn compound(&mut self, c: &horizon_core::nbt::Compound) {
        if c.is_empty() {
            self.lit("{", None);
            self.lit("}", None);
        } else if self.depth >= 64 {
            self.lit("{", None);
            self.lit("<...>", Some(text::GRAY));
            self.lit("}", None);
        } else {
            self.lit("{", None);
            if !self.indentation.is_empty() {
                self.lit("\n", None);
            }
            let entry = self.indentation.repeat(self.indent_depth + 1);
            let spacing = if self.indentation.is_empty() {
                " "
            } else {
                "\n"
            };
            let n = c.len();
            for (i, (k, v)) in c.iter().enumerate() {
                self.lit(&entry, None);
                let key = string_of(k);
                if !key.is_empty()
                    && key
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '+' | '-'))
                {
                    self.lit(&key, Some(text::AQUA));
                } else {
                    self.quoted(&key, text::AQUA);
                }
                self.lit(":", None);
                self.lit(" ", None);
                self.sub(v, true);
                if i + 1 < n {
                    self.lit(",", None);
                    self.lit(spacing, None);
                }
            }
            if !self.indentation.is_empty() {
                self.lit("\n", None);
                let close = self.indentation.repeat(self.indent_depth);
                self.lit(&close, None);
            }
            self.lit("}", None);
        }
    }
}
/// `NbtUtils.toPrettyComponent`: the coloured, unindented form.
pub fn pretty(tag: &Tag) -> Text {
    pretty_indented(tag, "")
}
pub fn pretty_indented(tag: &Tag, indentation: &str) -> Text {
    let mut p = Pretty {
        indentation: indentation.to_owned(),
        result: Text::empty(),
        indent_depth: 0,
        depth: 0,
    };
    p.visit(tag);
    p.result
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::parse_snbt;

    fn rt(s: &str) -> String {
        to_snbt(&parse_snbt(s).unwrap().0)
    }
    #[test]
    fn pretty_preserves_each_compounds_source_history() {
        let tag = parse_snbt(
            r#"{id:"minecraft:piston",blockState:{id:"minecraft:piston_head",properties:{facing:"east",short:"false",type:"normal"}},components:{},x:1,y:100,z:0,facing:5b,progress:0.0f,source:1b,extending:1b}"#,
        ).unwrap().0;
        let ordinary = r#"{blockState: {id: "minecraft:piston_head", properties: {facing: "east", short: "false", type: "normal"}}, components: {}, x: 1, facing: 5b, progress: 0.0f, y: 100, z: 0, id: "minecraft:piston", source: 1b, extending: 1b}"#;
        assert_eq!(pretty(&tag).plain(), ordinary);
        // A separate save writer inserts fields, components, then metadata.
        // Nested StateDefinition/DFU property writes are in reverse name order.
        let written = parse_snbt(
            r#"{blockState:{id:"minecraft:piston_head",properties:{type:"normal",short:"false",facing:"east"}},facing:5b,progress:0.0f,extending:1b,source:1b,components:{},id:"minecraft:piston",x:1,y:100,z:0}"#,
        ).unwrap().0;
        assert_eq!(
            pretty(&written).plain(),
            r#"{blockState: {id: "minecraft:piston_head", properties: {short: "false", facing: "east", type: "normal"}}, components: {}, x: 1, facing: 5b, progress: 0.0f, y: 100, z: 0, source: 1b, id: "minecraft:piston", extending: 1b}"#,
        );
        let state = written.get("blockState").unwrap();
        assert_eq!(
            pretty(state).plain(),
            r#"{id: "minecraft:piston_head", properties: {short: "false", facing: "east", type: "normal"}}"#,
        );
        assert_eq!(
            pretty(state.get("properties").unwrap()).plain(),
            r#"{short: "false", facing: "east", type: "normal"}"#,
        );
        let props = tag.get("blockState").unwrap().get("properties").unwrap();
        assert_eq!(
            pretty(props).plain(),
            r#"{facing: "east", short: "false", type: "normal"}"#
        );
        assert_eq!(pretty(&tag).plain(), ordinary);
    }
    #[test]
    fn mannequin_compound_orders_pose_before_immovable_in_shared_bucket() {
        let tag = parse_snbt(
            r#"{AbsorptionAmount:0.0f,Air:300s,Brain:{memories:{}},DeathTime:0s,FallFlying:0b,Fire:0s,Health:20.0f,HurtTime:0s,Invulnerable:0b,Motion:[0.0d,0.0d,0.0d],OnGround:0b,PortalCooldown:0,Pos:[2.5d,100.0d,2.5d],Rotation:[0.0f,0.0f],UUID:[I;369,0,0,1],attributes:[],current_impulse_context_reset_grace_time:0,description:{text:"Guard"},hide_description:1b,pose:"crouching",immovable:1b,profile:{name:"alex"}}"#,
        )
        .unwrap()
        .0;
        let rendered = pretty(&tag).plain();
        let pose_idx = rendered.find("pose: \"crouching\"").unwrap();
        let immovable_idx = rendered.find("immovable: 1b").unwrap();
        assert!(
            pose_idx < immovable_idx,
            "expected `pose` before `immovable` in bucket 5, got: {rendered}"
        );
    }
    #[test]
    fn interaction_compound_orders_width_before_interaction_in_shared_bucket() {
        // `width` and `interaction` share a bucket at every table size;
        // `Interaction.addAdditionalSaveData` writes `width` first.
        let tag = parse_snbt(
            r#"{Air:300s,Fire:0s,Invulnerable:0b,Motion:[0.0d,0.0d,0.0d],OnGround:0b,PortalCooldown:0,Pos:[4.5d,100.0d,4.5d],Rotation:[0.0f,0.0f],UUID:[I;66,0,0,1],fall_distance:0.0d,height:1.25f,width:1.0f,interaction:{player:[I;1,2,3,4],timestamp:9L},response:0b}"#,
        )
        .unwrap()
        .0;
        assert_eq!(
            pretty(&tag).plain(),
            "{Motion: [0.0d, 0.0d, 0.0d], Invulnerable: 0b, fall_distance: 0.0d, Air: 300s, OnGround: 0b, PortalCooldown: 0, Rotation: [0.0f, 0.0f], Pos: [4.5d, 100.0d, 4.5d], response: 0b, Fire: 0s, width: 1.0f, interaction: {player: [I; 1, 2, 3, 4], timestamp: 9L}, UUID: [I; 66, 0, 0, 1], height: 1.25f}",
        );
    }
    #[test]
    fn compact_form_matches_vanilla() {
        assert_eq!(
            rt("{b:1b,a:\"x\",c:[1,2],d:1.5f,e:2d,f:3L,g:4s}"),
            "{a:\"x\",b:1b,c:[1,2],d:1.5f,e:2.0d,f:3L,g:4s}"
        );
        assert_eq!(rt("{\"a b\":1}"), "{\"a b\":1}");
        assert_eq!(rt("{true:1}"), "{\"true\":1}");
        assert_eq!(rt("[B;1b,2b]"), "[B;1B,2B]");
        assert_eq!(rt("[I;1,2]"), "[I;1,2]");
        assert_eq!(rt("[L;1L]"), "[L;1L]");
        assert_eq!(quote_and_escape("say \"hi\""), "'say \"hi\"'");
        assert_eq!(quote_and_escape("it's \"x\""), "\"it's \\\"x\\\"\"");
        assert_eq!(quote_and_escape("a\nb\\"), "\"a\\nb\\\\\"");
    }
    #[test]
    fn java_number_text() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(0.1), "0.1");
        assert_eq!(double_to_string(1.0e7), "1.0E7");
        assert_eq!(double_to_string(12345678.9), "1.23456789E7");
        assert_eq!(double_to_string(0.0001), "1.0E-4");
        assert_eq!(double_to_string(-3.5), "-3.5");
        assert_eq!(float_to_string(0.3), "0.3");
        assert_eq!(float_to_string(100.0), "100.0");
        assert_eq!(double_to_string(f64::NAN), "NaN");
    }
    #[test]
    fn pretty_form_is_coloured_without_changing_text() {
        let tag = parse_snbt("{Pos:[1.5d,2.0d],id:\"minecraft:pig\",n:3b}")
            .unwrap()
            .0;
        let t = pretty(&tag);
        assert_eq!(
            t.plain(),
            "{Pos: [1.5d, 2.0d], id: \"minecraft:pig\", n: 3b}"
        );
        // keys aqua, strings green, numbers gold.
        let colors: Vec<_> = t.spans.iter().filter_map(|s| s.style.color).collect();
        assert!(colors.contains(&text::AQUA));
        assert!(colors.contains(&text::GREEN));
        assert!(colors.contains(&text::GOLD));
        let long = parse_snbt("{l:[{a:1},{b:2}]}").unwrap().0;
        assert_eq!(pretty(&long).plain(), "{l: [{a: 1}, {b: 2}]}");
    }
    #[test]
    fn block_entity_metadata_follows_its_fields() {
        // A conduit's `/data get block`, as vanilla 26.3 printed it: `Target`
        // and `components` share a bucket, and saveAdditional wrote `Target` first.
        let tag = parse_snbt(
            r#"{Target:[I;1,2,3,4],components:{},id:"minecraft:conduit",x:4098,y:300,z:4098}"#,
        )
        .unwrap()
        .0;
        assert_eq!(
            pretty(&tag).plain(),
            r#"{Target: [I; 1, 2, 3, 4], components: {}, x: 4098, y: 300, z: 4098, id: "minecraft:conduit"}"#,
        );
    }
}
