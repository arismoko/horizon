//! Raw listeners share the points registry's native/erased order. Their only game access is the
//! call-scoped internals view; unlike API points they receive no Host.
use crate::Refusal;
use crate::frame::{Frame, Root};
use crate::view::{self, Resolved};
use horizon_mod_package::manifest::inject::{Access, Kind};
use horizon_mod_package::manifest::{Internal, Manifest};
use mod_api::__private::wasm_host::{CallHost, Exports};
use mod_points::__private::SlotListener;
use mod_points::internals::bindings::exports::horizon::mod_internals::raw_listeners::HostKind;
use mod_points::internals::{Place, Table};
use mod_points::places::{IDS, NAMES, TARGETS};
use mod_points::register;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

type Listed = wasmtime::component::TypedFunc<(), (Vec<(String, String)>,)>;
type Fire = wasmtime::component::TypedFunc<(u32, HostKind), ()>;

type Key = (&'static str, usize);
static OPEN: Mutex<BTreeMap<Key, Vec<Listener>>> = Mutex::new(BTreeMap::new());
static REGISTERED: OnceLock<BTreeMap<Key, Vec<Listener>>> = OnceLock::new();

struct Listener {
    exports: Exports,
    fire: Fire,
    table: &'static Table,
    descriptor: usize,
    index: u32,
    declared: Arc<[Resolved]>,
}

pub(crate) struct Listeners(Vec<Listener>);

impl Listeners {
    pub(crate) fn register(self) -> Result<(), mod_points::RegisterError> {
        let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
        for listener in self.0 {
            let place = &listener.table.places[listener.descriptor];
            let key = (listener.exports.id(), place.place);
            let listeners = open.entry(key).or_default();
            // One erased callback per mod and place. It fans out repeated injections in guest
            // order, since the registry's erased callback carries a place but no injection id.
            if listeners.is_empty() {
                // SAFETY: load checked the descriptor against this exact place's raw shape;
                // `fire` uses it only for the callback's synchronous lifetime.
                let raw = unsafe { SlotListener::erased(key.0, IDS[key.1], fire) };
                register(&raw)?;
            }
            listeners.push(listener);
        }
        Ok(())
    }
}

/// A manifest and its compiled view must describe the same internals before table resolution.
fn check_declared(manifest: &[Internal], listed: &[(String, String)]) -> Result<(), Refusal> {
    for internal in manifest {
        if !listed
            .iter()
            .any(|(path, hash)| path == &internal.path && hash == &internal.hash)
        {
            return Err(Refusal::ChangedInternal {
                path: internal.path.clone(),
            });
        }
    }
    Ok(())
}

pub(crate) fn listen(manifest: &Manifest, exports: &Exports) -> Result<Option<Listeners>, Refusal> {
    let wasm = manifest
        .wasm
        .as_ref()
        .expect("wasm loading checked the package");
    if wasm.internals().is_empty() {
        return Ok(None);
    }
    let table = crate::INTERNALS.get().copied().unwrap_or(&Table::EMPTY);
    for path in wasm.internals().iter().map(|i| i.path.as_str()).chain(
        manifest
            .inject
            .iter()
            .filter(|i| i.access == Access::Internals)
            .map(|i| i.at.as_str()),
    ) {
        if table.native_only.contains(&path) {
            return Err(Refusal::missing_internal(table, &manifest.id, path));
        }
    }
    if table.types.is_empty()
        && table.thunks.is_empty()
        && table.members.is_empty()
        && table.places.is_empty()
        && table.native_only.is_empty()
    {
        return Err(Refusal::NoInternalsTable);
    }
    let interface = format!(
        "{}-raw-listeners",
        horizon_mod_package::manifest::export_prefix(&manifest.id)
    );
    let declared: Listed = exports
        .func(&interface, "declared")
        .map_err(Refusal::Load)?;
    let listed = exports
        .load(|store| Ok(declared.call(store, ())?.0))
        .ok_or_else(|| {
            Refusal::Points("its raw listeners didn't answer their declared internals".into())
        })?;
    check_declared(wasm.internals(), &listed)?;
    let pairs: Vec<_> = listed
        .iter()
        .map(|(path, hash)| (path.as_str(), hash.as_str()))
        .collect();
    let resolved = view::resolve(table, &manifest.id, &pairs)?;
    for ((path, hash), resolved) in listed.iter().zip(&resolved) {
        if !matches!(resolved, Resolved::Type(_))
            && !wasm
                .internals()
                .iter()
                .any(|entry| entry.path == *path && entry.hash == *hash)
        {
            return Err(Refusal::ChangedInternal { path: path.clone() });
        }
    }
    let declared: Arc<[Resolved]> = resolved.into();

    let injections: Vec<_> = manifest
        .inject
        .iter()
        .filter(|i| i.access == Access::Internals)
        .collect();
    if injections.is_empty() {
        return Ok(None);
    }
    let subscribed: Listed = exports
        .func(&interface, "subscribed")
        .map_err(Refusal::Load)?;
    let fire: Fire = exports.func(&interface, "fire").map_err(Refusal::Load)?;
    let listed = exports
        .load(|store| Ok(subscribed.call(store, ())?.0))
        .ok_or_else(|| {
            Refusal::Points("its raw listeners didn't answer which places they fill".into())
        })?;
    let mut expected = Vec::new();
    for injection in injections {
        let target = injection.at.as_str();
        let kind = match (injection.kind, injection.priority) {
            (Kind::Head, 0) => horizon_mod_package::manifest::InjectionKind::Head,
            (Kind::Return, 0) => horizon_mod_package::manifest::InjectionKind::Return,
            _ => {
                return Err(Refusal::Points(
                    "E_INJECT_NO_SLOT: wasm listener kind or priority pending".into(),
                ));
            }
        };
        let place = crate::points::target_place(target, kind)?;
        expected.push(place);
    }
    let mut listeners = Vec::new();
    for (index, (name, shape)) in listed.into_iter().enumerate() {
        let place = NAMES
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| Refusal::NoSuchPoint {
                point: name.clone(),
            })?;
        let found =
            expected
                .iter()
                .position(|p| *p == place)
                .ok_or_else(|| Refusal::UndeclaredPoint {
                    point: name.clone(),
                })?;
        expected.remove(found);
        let descriptor = table
            .places
            .iter()
            .position(|p| p.place == place && p.target == TARGETS[place])
            .ok_or_else(|| Refusal::missing_internal(table, &manifest.id, TARGETS[place]))?;
        let entry = &table.places[descriptor];
        if entry.hash != shape {
            return Err(Refusal::ChangedInternal {
                path: entry.target.into(),
            });
        }
        listeners.push(Listener {
            exports: exports.clone(),
            fire,
            table,
            descriptor,
            index: index as u32,
            declared: declared.clone(),
        });
    }
    if let Some(&place) = expected.first() {
        return Err(Refusal::UnfilledPoint {
            point: NAMES[place].into(),
        });
    }
    Ok(Some(Listeners(listeners)))
}

/// Builds roots only for the descriptor's receiver (if any) and its raw record.
fn roots(place: &Place, receiver: *mut (), record: *mut ()) -> Vec<Root> {
    let mut roots = Vec::with_capacity(2);
    if let Some(ty) = place.receiver {
        roots.push(Root {
            ptr: receiver,
            ty,
            writable: place.receiver_writable,
            receiver: true,
            born: None,
        });
    }
    roots.push(Root {
        ptr: record,
        ty: place.record,
        writable: true,
        receiver: false,
        born: None,
    });
    roots
}

/// # Safety
/// Called only by the points registry with the registered place's exact erased borrows.
unsafe fn fire(place: usize, receiver: *mut (), record: *mut ()) {
    // The points registry's `fire_raw` enters the listener's mod before every callback.
    let Some(id) = mod_api::crash::current_native_mod() else {
        return;
    };
    let registered = REGISTERED
        .get_or_init(|| std::mem::take(&mut *OPEN.lock().unwrap_or_else(|e| e.into_inner())));
    let Some(listeners) = registered.get(&(id, place)) else {
        return;
    };
    for listener in listeners {
        let descriptor = &listener.table.places[listener.descriptor];
        // SAFETY: registration matched this exact raw place and its descriptor. Each frame is
        // synchronous and discarded before another listener can observe the slot's borrows.
        let frame = unsafe { Frame::new(listener.table, roots(descriptor, receiver, record)) };
        let answer = listener.exports.call(CallHost::None, |store| {
            view::run(frame, &listener.declared, || {
                listener.fire.call(store, (listener.index, HostKind::None))
            })
        });
        if answer.is_none() && listener.exports.stopped() {
            mod_points::stop(id);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compiled_view_must_include_each_manifest_internal_at_the_same_hash() {
        let internal = Internal {
            path: "Game::world".into(),
            kind: Some("field".into()),
            hash: "world-hash".into(),
        };
        let expected = Err(Refusal::ChangedInternal {
            path: internal.path.clone(),
        });
        assert_eq!(
            check_declared(std::slice::from_ref(&internal), &[]),
            expected
        );
        assert_eq!(
            check_declared(
                std::slice::from_ref(&internal),
                &[(internal.path.clone(), "old-hash".into())]
            ),
            expected
        );
        assert_eq!(
            check_declared(
                std::slice::from_ref(&internal),
                &[
                    (internal.path.clone(), internal.hash.clone()),
                    ("World".into(), "type-hash".into())
                ]
            ),
            Ok(())
        );
    }

    #[test]
    fn erased_registration_runs_after_native_and_commits_its_journal() {
        use mod_points::internals::{Op, Reply, Type, Value};
        use wasmtime::component::Resource;
        unsafe fn counter(
            ptr: *mut (),
            _: bool,
            op: Op<'_>,
        ) -> Result<Reply, mod_points::internals::Refusal> {
            // SAFETY: this synthetic slot passes its exclusively borrowed i64 record.
            let value = unsafe { &mut *ptr.cast::<i64>() };
            match op {
                Op::Check(0, Value::S(_)) => Ok(Reply::Done),
                Op::Get(0) => Ok(Reply::Value(Value::S(*value))),
                Op::Set(0, Value::S(next)) => {
                    *value = next;
                    Ok(Reply::Done)
                }
                _ => Err(mod_points::internals::not_here("not on the fixture")),
            }
        }
        static TABLE: Table = Table {
            types: &[Type {
                path: "Counter",
                hash: "hash",
                node: counter,
                raw_fields: &[],
                raw_gets: &[],
                raw_index: false,
            }],
            thunks: &[],
            members: &[],
            native_only: &[],
            places: &[Place {
                place: mod_points::PointId::Tick as usize,
                target: "fixture",
                hash: "shape",
                receiver: None,
                receiver_writable: false,
                record: 0,
            }],
        };
        fn native() {}
        unsafe fn erased(_: usize, receiver: *mut (), record: *mut ()) {
            let factor = match mod_api::crash::current_native_mod() {
                Some("fixture_wasm") => 10,
                Some("fixture_wasm_other") => 100,
                id => panic!("wrong erased listener identity: {id:?}"),
            };
            // SAFETY: only the fixture slot below calls this callback, with its live record.
            let frame = unsafe { Frame::new(&TABLE, roots(&TABLE.places[0], receiver, record)) };
            view::run(frame, &[], || {
                let root = view::View.root(0).unwrap();
                let Value::S(n) = mod_points::internals::unflatten(
                    view::View.get(Resource::new_borrow(root.rep()), 0).unwrap(),
                )
                .unwrap() else {
                    panic!("a number")
                };
                view::View
                    .set(
                        Resource::new_borrow(root.rep()),
                        0,
                        mod_points::internals::flatten(Value::S(n * factor)),
                    )
                    .unwrap();
                Ok(())
            })
            .unwrap();
        }
        let point = mod_points::PointId::Tick;
        // SAFETY: no real game slot fires in this test. The synthetic slot below does not call
        // the typed pointer, and supplies the erased callback with the fixture descriptor's types.
        unsafe {
            register(&SlotListener::new(
                "fixture_native",
                point,
                native as *const (),
            ))
            .unwrap();
            register(&SlotListener::erased("fixture_wasm", point, erased)).unwrap();
            register(&SlotListener::new(
                "fixture_middle",
                point,
                native as *const (),
            ))
            .unwrap();
            register(&SlotListener::erased("fixture_wasm_other", point, erased)).unwrap();
        }
        mod_points::__private::seal();
        let mut value: i64 = 1;
        mod_points::__private::fire_raw(point, "fixture", &mut |kind| {
            match kind {
                mod_points::__private::RawKind::Typed(_) => value += 1,
                mod_points::__private::RawKind::Erased(callback) => {
                    // SAFETY: this synthetic slot's record is the live i64 its table names.
                    unsafe {
                        callback(
                            point as usize,
                            std::ptr::null_mut(),
                            (&raw mut value).cast(),
                        )
                    };
                }
            }
        });
        assert_eq!(
            value, 2100,
            "native, wasm, native, wasm retain their identities and order"
        );
        mod_points::stop("fixture_wasm");
        value = 1;
        mod_points::__private::fire_raw(point, "fixture", &mut |kind| match kind {
            mod_points::__private::RawKind::Typed(_) => value += 1,
            mod_points::__private::RawKind::Erased(callback) => {
                // SAFETY: the same synthetic slot's exclusively borrowed i64 record.
                unsafe {
                    callback(
                        point as usize,
                        std::ptr::null_mut(),
                        (&raw mut value).cast(),
                    )
                };
            }
        });
        assert_eq!(
            value, 300,
            "stopping one wasm listener never shifts another's identity"
        );
    }

    #[test]
    fn raw_roots_preserve_shared_receivers_and_free_function_layouts() {
        let mut receiver = 0;
        let mut record = 0;
        let place = Place {
            place: 0,
            target: "Game::read",
            hash: "shape",
            receiver: Some(3),
            receiver_writable: false,
            record: 4,
        };
        let roots = roots(&place, (&raw mut receiver).cast(), (&raw mut record).cast());
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].ty, 3);
        assert!(!roots[0].writable);
        assert!(roots[0].receiver);
        assert_eq!(roots[1].ty, 4);
        assert!(roots[1].writable);
        assert!(!roots[1].receiver);
        let place = Place {
            receiver: None,
            ..place
        };
        let roots = super::roots(&place, std::ptr::null_mut(), (&raw mut record).cast());
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].ty, 4);
        assert!(!roots[0].receiver);
    }
}

#[cfg(test)]
#[path = "raw/controlled_exports.rs"]
mod controlled_exports;
