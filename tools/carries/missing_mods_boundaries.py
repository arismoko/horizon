#!/usr/bin/env python3
"""Mechanical relocation for the #3933 carry; run once on 1bbf18324d5."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
paths = {name: root / ('crates/horizon-client/src/' + name) for name in (
    'game.rs', 'entry.rs', 'game/interface/dedicated/mod.rs',
    'game/interface/dedicated/missing_mods.rs', 'title.rs', 'title/missing_mods.rs')}
texts = {name: path.read_text() for name, path in paths.items()}

def replace(name, old, new):
    assert texts[name].count(old) == 1, (name, old)
    texts[name] = texts[name].replace(old, new)

game = texts['game.rs']
a = game.index('/// An intentional refusal')
b = game.index('pub(crate) fn run_with(', a)
refusal = game[a:b].replace('pub(crate) struct', 'struct')
replace('game.rs', game[a:b], '')
replace('game/interface/dedicated/missing_mods.rs', '/// `server.toml`', refusal + '''/// Whether this error is the already-explained, intentional world refusal.
pub(crate) fn is_missing_mods_refusal(error: &anyhow::Error) -> bool {
    error.is::<MissingModsRefusal>()
}

/// `server.toml`''')
replace('game/interface/dedicated/missing_mods.rs', 'crate::game::MissingModsRefusal(last)', 'MissingModsRefusal(last)')
texts['game/interface/dedicated/mod.rs'] = '// game-module-visibility: pub(crate)\n' + texts['game/interface/dedicated/mod.rs']
replace('game/interface/dedicated/mod.rs', 'mod missing_mods;', 'mod missing_mods;\npub(crate) use missing_mods::is_missing_mods_refusal;')
replace('entry.rs', 'let usage = error.is::<game::UsageError>() || error.is::<game::MissingModsRefusal>();\n        if !error.is::<game::MissingModsRefusal>() {', 'let missing_mods = game::dedicated::is_missing_mods_refusal(&error);\n        let usage = error.is::<game::UsageError>() || missing_mods;\n        if !missing_mods {')

title = texts['title.rs']
a = title.index("    /// The right stick's scroll:")
b = title.index('    pub fn failed(', a)
methods = title[a:b]
replace('title.rs', methods, '')
# The four dispatch points retain their existing action and return semantics.
title = texts['title.rs']
a = title.index('        if matches!(self.mode, Mode::MissingMods', title.index('    pub fn drag('))
b = title.index('            let (top, bottom, _, content)', a)
old = title[a:b]
methods += '''    fn drag_missing_mods(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if !matches!(self.mode, Mode::MissingMods { .. }) { return false; }
        let (top, bottom, _) = layout(size.0, size.1);
        let mut scroll = self.scroll.get();
        scroll.drag(cursor.1, top, bottom, self.message_lines.get() as f32 * 9.0);
        self.scroll.set(scroll);
        true
    }
'''
replace('title.rs', old, '''        if self.drag_missing_mods(cursor, size) { return; }
        if let Mode::Select { worlds, .. } = &self.mode {
''')
title = texts['title.rs']
a = title.index('        if matches!(self.mode, Mode::MissingMods', title.index('    pub fn scroll_worlds('))
b = title.index('        // CycleButton.mouseScrolled', a)
old = title[a:b]
methods += '''    fn wheel_missing_mods(&mut self, delta: f32, size: (f32, f32)) -> bool {
        if !matches!(self.mode, Mode::MissingMods { .. }) { return false; }
        let (top, bottom, _) = layout(size.0, size.1);
        let mut scroll = self.scroll.get();
        scroll.wheel(delta, 18.0, top, bottom, self.message_lines.get() as f32 * 9.0);
        self.scroll.set(scroll);
        true
    }
'''
replace('title.rs', old, '        if self.wheel_missing_mods(delta, size) { return false; }\n')
title = texts['title.rs']
a = title.index('            (Mode::MissingMods { .. }, None) => {')
b = title.index('            (Mode::Select { .. }, None)', a)
methods += '''    fn press_missing_mods(&mut self, cursor: (f32, f32), size: (f32, f32)) -> Action {
        let (top, bottom, _) = layout(size.0, size.1);
        self.scroll.get_mut().press(cursor, size.0 - 10.0, top, bottom, self.message_lines.get() as f32 * 9.0);
        Action::None
    }
'''
replace('title.rs', title[a:b], '            (Mode::MissingMods { .. }, None) => self.press_missing_mods((x, y), (w, h)),\n')
title = texts['title.rs']
a = title.index('        if let Mode::MissingMods { world, .. }', title.index('    pub fn confirm_key_size('))
b = title.index('        if self.rules.get_mut()', a)
methods += '''    fn confirm_missing_mods(&mut self) -> Option<(Action, bool)> {
        let Mode::MissingMods { world, .. } = &self.mode else { return None; };
        Some((self.answer_missing_mods(world.clone(), self.focused_button.unwrap_or(101)), true))
    }
'''
replace('title.rs', title[a:b], '        if let Some(answer) = self.confirm_missing_mods() { return answer; }\n')
replace('title/missing_mods.rs', 'impl Title {', 'impl Title {\n' + methods)
for name, path in paths.items():
    path.write_text(texts[name])
