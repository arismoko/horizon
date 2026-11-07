#!/usr/bin/env python3
"""Restore engine recipe snapshots and emit explicit boundary field mappings."""
from pathlib import Path
import re
root=Path(__file__).resolve().parents[3]
api=root/'crates/horizon-mod-api/src'
client=root/'crates/horizon-client/src'
s=(api/'recipe_rule.rs').read_text().replace('use crate::{Ingredient, ItemStack, Tag};','use super::{Ingredient, ItemStack};\nuse horizon_core::nbt::Tag;')
c=(api/'crafting.rs').read_text()
s+='\n'+c[c.index('/// A recipe in force'):c.index('/// What a crafting grid makes')].replace('pub struct Recipe {','pub struct RecipeSnapshot {').replace('impl Recipe {','impl RecipeSnapshot {')
s=s.replace('#[non_exhaustive]\n', '')
s='use std::sync::Arc;\nuse serde_json::Value;\n'+s.replace('//! Typed rules for recipes whose result is not a fixed crafting stack.','// Engine-owned recipe snapshots.')
(client/'crafting/snapshots.rs').write_text(s)
for p in ['crafting.rs','stations.rs']:
 s=(client/p).read_text()
 for n in ['RecipeTemplate','RecipeRule','RecipeTrimPattern','FireworkShapeIngredient','Recipe']:
  s=re.sub(r'mod_api::'+n+r'\b', 'crate::crafting::'+('RecipeSnapshot' if n=='Recipe' else n),s)
 if p=='crafting.rs': s=s.replace('mod ingredient;', 'mod snapshots;\npub use snapshots::*;\nmod ingredient;')
 (client/p).write_text(s)
# Emission is declarative: match every enum variant, convert nested snapshots.
source=(api/'recipe_rule.rs').read_text()
body=source[source.index('pub enum RecipeRule {')+len('pub enum RecipeRule {'):]
body=re.sub(r'\s*///[^\n]*','',body)
variants=re.findall(r'(\w+)\s*\{([^}]+)\}|(RepairItem)',body)
out='\nimpl TryIntoApi for crate::crafting::RecipeRule {\n type Api = mod_api::RecipeRule;\n type Error = StackReadError;\n fn try_into_api(&self) -> Result<Self::Api, Self::Error> {\n Ok(match self {\n'
for name,fields,unit in variants:
 if unit:
  out+='Self::RepairItem => mod_api::RecipeRule::RepairItem,\n'; continue
 fs=re.findall(r'(\w+):\s*([^,]+)',fields)
 out+=f'Self::{name} {{ '+', '.join(n for n,t in fs)+f' }} => mod_api::RecipeRule::{name} {{\n'
 for n,t in fs:
  expr=f'{n}.try_into_api()?'
  if t.strip() in ['usize','bool','i32']: expr=f'*{n}'
  out+=f'{n}: {expr},\n'
 out+='},\n'
out+='})\n }\n}\n'
(client/'game/modding/api_values.rs').write_text((client/'game/modding/api_values.rs').read_text()+out)
