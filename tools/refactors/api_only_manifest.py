from pathlib import Path
p=Path('crates/horizon-mod-package/src/manifest/inject.rs')
s=p.read_text(); s=s[:s.index('#[cfg(test)]')]
s=s.replace('    pub call: Option<String>,\n    pub nth: Option<usize>,\n    pub ctx: Option<String>,\n','')
a=s.index('    BeforeCall,'); b=s.index('\n}',a); s=s[:a]+s[b:]
a=s.index('    pub const ALL'); b=s.index('    /// The manifest',a); s=s[:a]+'    pub const ALL: [Self; 3] = [Self::Head, Self::Return, Self::Replace];\n\n'+s[b:]
a=s.index('            Self::BeforeCall'); b=s.index('\n        }',a); s=s[:a]+s[b:]
s=s.replace('Default, Deserialize, Serialize','Default, Serialize').replace('    Internals,\n','')
a=s.index('impl Kind {',s.index('pub enum Access')); b=s.index('impl Inject',a)
s=s[:a]+'''impl<'de> Deserialize<'de> for Access {
 fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
 let value = String::deserialize(d)?;
 if value == "api" { Ok(Self::Api) } else { Err(serde::de::Error::custom(format!("access `{value}` is unsupported; use access = \\\"api\\\" or [[native.mixin]] for native function replacement"))) }
 }
}
impl Kind { pub fn slot(self) -> SlotKind { match self { Self::Head | Self::Replace => SlotKind::Head, Self::Return => SlotKind::Return } } }

'''+s[b:]
a=s.index('            let call ='); b=s.index('\n        }\n        Ok(())',a); s=s[:a]+s[b:]
s=s.replace('    Site,\n',''); a=s.index('    pub calls:'); b=s.index('#[derive(Clone, Copy',a); s=s[:a]+'}\n\n'+s[b:]
s=s.replace("    pub resolved_call: Option<&'a str>,\n",'')
for x in ['NotListed','NativeOnly','WasmSitesUnsupported','ApiSitesUnsupported']:
 pass
a=s.index('pub enum NoSlotReason'); b=s.index('/// Escape-hatch',a)
s=s[:a]+'''pub enum NoSlotReason { NoApiRecord, ReplaceWithoutValue }
impl fmt::Display for NoSlotReason { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(match self { Self::NoApiRecord => "no API record: a parameter has no API type yet", Self::ReplaceWithoutValue => "replace needs a return its record carries" }) } }

'''+s[b:]
a=s.index('    Moved {'); b=s.index('    NoSlot {',a); s=s[:a]+s[b:]
s=s.replace('            Self::Moved { .. } => "E_INJECT_MOVED",\n','').replace('            Self::Lost { .. } => "E_INJECT_LOST",\n','')
a=s.index('            Self::Moved {'); b=s.index('            Self::NoSlot {',a); s=s[:a]+s[b:]
a=s.index('    let available ='); s=s[:a]+'''    if place.slots.contains(&inject.kind.slot()) { return Ok(inject.kind.slot()); }
    Err(InjectError::NoSlot { id, reason: NoSlotReason::NoApiRecord, declaration: inject.at.clone() })
}
'''
p.write_text(s)
p=Path('crates/horizon-mod-package/src/manifest/inject/mod_points.rs'); s=p.read_text(); s=s.replace('        calls: Vec::new(),\n','').replace('        native_only: false,\n','').replace('            resolved_call: None,\n',''); s=s.replace('        (super::Access::Internals, _) => Some(Refusal::CrossTargetInternals { id, library }),\n',''); p.write_text(s)
