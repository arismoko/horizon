#!/usr/bin/env python3
"""Separate load-state ownership from phase vocabulary after its verbatim move."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
API = ROOT / 'crates/horizon-mod-api/src/load_state.rs'
VOCAB = ROOT / 'crates/horizon-mod-vocab/src/world/phase.rs'
s = API.read_text()


def section(start, end):
    return s[s.index(start):s.index(end, s.index(start))]


phase_state = section('#[derive(Clone, Copy)]\nstruct PhaseState', '/// A refused engine')
phase_error = section('/// A refused engine', '// no wire: thread-bound phase authority')
owner = section('// no wire: thread-bound phase authority', '/// The catalog-owned access class')
packages = section('static PACKAGES:', '// no wire: shared immutable loader inventory')
registration = section('static REGISTRATION_DATA:', '/// Read-only view of the startup package inventory')
tests = section('#[cfg(test)]\nmod tests {', '#[cfg(test)]\n#[derive(Default)]\npub(crate) struct TestReader')
fixtures = s[s.index('#[cfg(test)]\n#[derive(Default)]\npub(crate) struct TestReader'):]
# Tests at the original layer retain only injected test data, never production TLS.
package_tests = section('#[cfg(any(test, feature = "testing"))]\nthread_local! {\n    static TEST_PACKAGES', 'fn inventory(')
registration_tests = section('#[cfg(any(test, feature = "testing"))]\nthread_local! {\n    static TEST_REGISTRATION_DATA', 'fn registration_data()')
package_tests = package_tests.replace('any(test, feature = "testing")', 'test')
registration_tests = registration_tests.replace('any(test, feature = "testing")', 'test')
head = s[:s.index('#[derive(Clone, Copy)]\nstruct PhaseState')].replace('sync::{Arc, OnceLock}', 'sync::Arc')
values = section('/// The catalog-owned access class', 'static PACKAGES:')
registration_values = section('// no wire: shared immutable loader inventory', 'static REGISTRATION_DATA:')
contexts = section('/// Read-only view of the startup package inventory', '#[cfg(test)]\nmod tests {')
contexts = contexts.replace('any(test, feature = "testing")', 'test')
contexts = contexts.replace('PACKAGES.get().cloned().unwrap_or_else(|| Arc::new([]))', 'crate::dispatch_scope::backend().map_or_else(|| Arc::from([]), |backend| (backend.packages)())')
vocab = head + phase_error + '''
/// The current owning phase, or the initial phase before the platform installs its reader.
pub fn current_phase() -> ModPhase {
    crate::dispatch_scope::backend().map_or(if cfg!(test) { ModPhase::World } else { ModPhase::Packages }, |backend| (backend.current_phase)())
}
''' + values + registration_values + '''
fn registration_data() -> Option<RegistrationData> {
    #[cfg(test)]
    if let Some(data) = TEST_REGISTRATION_DATA.with_borrow(Clone::clone) { return Some(data); }
    crate::dispatch_scope::backend().and_then(|backend| (backend.registration_data)())
}
''' + contexts + package_tests + registration_tests + fixtures
VOCAB.write_text(vocab)

imports = '''//! Platform-owned phase machine and load snapshots.
use crate::*;
use mod_vocab::__private::{ModPhase, PhaseError, RegistrationData};
use crate::__private::game::RegistrationContextNew;
use std::{collections::BTreeMap, sync::{Arc, OnceLock}};
'''
owner = owner.replace('    pub fn new(initial: ModPhase) -> Self {', '    pub fn new(initial: ModPhase) -> Self {\n        let _ = crate::__private::initialize();')
packages = packages.replace('entry.id == p.id', 'entry.fields().id == p.id()').replace('crate::mods()', 'crate::__private::mods()')
for signature in ['pub fn install_packages(native: &[PackageInfo], content: &[PackageInfo]) {', 'pub fn install_guest_packages(packages: Vec<PackageInfo>) {', '    let previous = TEST_PACKAGES.replace(Some(packages.into()));']:
    packages = packages.replace(signature, signature + '\n    let _ = crate::__private::initialize();')
registration = registration.replace('fn registration_data()', 'pub(crate) fn registration_data()')
registration = registration.replace('    TEST_REGISTRATION_DATA.set(Some(data));', '    let _ = crate::__private::initialize();\n    TEST_REGISTRATION_DATA.set(Some(data));')
registration = registration.replace('pub fn install_registration_data(data: RegistrationData) {', 'pub fn install_registration_data(data: RegistrationData) {\n    let _ = crate::__private::initialize();')
reader_fixture = fixtures[:fixtures.index('#[test]\nfn client_only_attribute')]
tests = tests.replace('crate::install(mods.clone())', 'crate::__private::install(mods.clone())')
tests = tests.replace('crate::install_parts', 'crate::__private::install_parts').replace('crate::parts::tests::rule_schema()', 'crate::__private::part_tests::rule_schema()')
tests = tests.replace('let view = LoadedMods {\n            entries: entries.into(),\n        };', 'TEST_PACKAGES.set(Some(entries.into()));\n        let view = RegistrationContext::new().mods().clone();\n        TEST_PACKAGES.set(None);')
API.write_text(imports + phase_state + owner + packages + registration + '''
pub(crate) fn current_packages() -> Arc<[PackageInfo]> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(packages) = TEST_PACKAGES.with_borrow(Clone::clone) { return packages; }
    PACKAGES.get().cloned().unwrap_or_else(|| Arc::from([]))
}
''' + reader_fixture + tests)
