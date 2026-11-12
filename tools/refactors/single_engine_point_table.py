"""Replace mod-owned table collections with the single engine point registry."""
from pathlib import Path
p = Path(__file__).resolve().parents[2] / 'crates/horizon-mod-vocab/src/__private/points.rs'
s = p.read_text()
a = s.index('//! Slot tables'); b = s.index('\nuse ', a)
s = '//! The engine API point registry, shared by native and wasm listeners.\n' + s[b:]
s = s.replace('tables: Vec<OpenTable>', 'table: Option<OpenTable>').replace('tables: Vec::new()', 'table: None')
a = s.index('    let table = match open\n'); b = s.index('    let place =', a)
s = s[:a] + '''    let table = open.table.get_or_insert_with(|| OpenTable {
        table: slot.place.table,
        listeners: Vec::new(),
        slots: Vec::new(),
    });
    if !std::ptr::eq(table.table, slot.place.table) {
        return Err(RegisterError::Unsupported);
    }
''' + s[b:]
s = s.replace('for table in &open.tables {', 'for table in open.table.iter() {').replace('for table in &mut open.tables {', 'for table in open.table.iter_mut() {')
s = s.replace("/// Every table's registrations while mods load, and stops that came before the seal.", "/// Engine point registrations while mods load, and stops that came before the seal.").replace('/// sealed tables in `tables`.', '/// sealed engine table.').replace('/// Stops mod `id`\'s listeners in every table: none of them runs again, whether the tables are', '/// Stops mod `id`\'s engine listeners: none of them runs again, whether the slots are').replace('/// Seals every table, so no listener registers after it, and sets each slot a mod listens at. The', '/// Seals the engine table, refusing later registrations, and sets listened slots. The')
s = s.replace('// hidden: generated mod-point adapters share replacement semantics without exposing registry types.', '// hidden: generated engine point adapters share replacement semantics.')
p.write_text(s)
