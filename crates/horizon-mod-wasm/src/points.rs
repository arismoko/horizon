//! Wasm mods listening at the game's points (#589): a `[wasm]` package's `[[inject]]`
//! entries name the places its `mod.wasm` fills. At load each is checked against this game's
//! points and against the mod's own listeners, then the mod registers in the points crate's one
//! list of listeners, exactly as a native mod does: its places fire it through
//! checked export indices, as hook calls within its group's limits.
use crate::Refusal;
use horizon_mod_package::manifest::{
    InjectionKind, Manifest,
    inject::{Access, Inject, Kind},
};
use mod_api::__private::wasm_host::{CallHost, Exports, LoadError};
use mod_points::__private::wire::{Forward, HostKind, PointHost, Record, Value};
use mod_points::PointId;
use mod_points::places::{IDS, NAMES, SHAPES, TARGETS};
use std::collections::BTreeMap;

type Subscribed = wasmtime::component::TypedFunc<(), (Vec<(String, String)>,)>;
type Fire = wasmtime::component::TypedFunc<(u32, HostKind, Vec<Value>), (Vec<Value>,)>;

/// One wasm mod's listeners: its `listeners` export, and the places it fills in the order the
/// mod named them (a place's index there is the one `fire` takes).
struct Listeners {
    exports: Exports,
    fire: Fire,
}

impl Forward for Listeners {
    fn forward(&self, index: u32, host: PointHost<'_>, record: &mut dyn Record) {
        let (host, wire) = match host {
            PointHost::Read(h) => (CallHost::Read(h), HostKind::Read),
            PointHost::Server(h) | PointHost::Side(mod_api::HostSide::Server(h)) => {
                (CallHost::Server(h), HostKind::Server)
            }
            PointHost::Client(h) | PointHost::Side(mod_api::HostSide::Client(h)) => {
                (CallHost::Client(h), HostKind::Client)
            }
            PointHost::None => (CallHost::None, HostKind::None),
        };
        let fields = record.lower();
        let answer = self.exports.call(host, |store| {
            Ok(self.fire.call(store, (index, wire, fields))?.0)
        });
        let written = match answer {
            Some(fields) => record.write_back(fields),
            None if self.exports.stopped() => {
                mod_points::stop(self.exports.id());
                return;
            }
            // Refused (busy, or a call back into its own group): the place runs as if it
            // weren't listened at, as a hook's would.
            None => return,
        };
        if let Err(why) = written {
            log::error!(
                "wasm mod {} stopped listening at points: at export {}, {why}",
                self.exports.id(),
                index
            );
            mod_points::stop(self.exports.id());
        }
    }
}

/// Resolve the API function slots currently supported by the guest export path.
fn declared_place(inject: &Inject) -> Result<usize, Refusal> {
    let kind = match (inject.access, inject.kind) {
        (Access::Api, Kind::Head) => InjectionKind::Head,
        (Access::Api, Kind::Return) => InjectionKind::Return,
        _ => {
            return Err(Refusal::Points(format!(
                "{}: wasm injects currently require API head/return",
                inject.id
            )));
        }
    };
    let place = target_place(&inject.at, kind)?;
    if SHAPES[place].is_empty() {
        return Err(Refusal::NoSuchPoint {
            point: NAMES[place].into(),
        });
    }
    Ok(place)
}

/// The point at a target's head or return, shared by API and raw listener validation.
pub(crate) fn target_place(target: &str, kind: InjectionKind) -> Result<usize, Refusal> {
    let missing = || Refusal::NoSuchPoint {
        point: target.into(),
    };
    let head = TARGETS
        .iter()
        .position(|t| *t == target)
        .ok_or_else(missing)?;
    match kind {
        InjectionKind::Return => Some(head + 1)
            .filter(|&back| TARGETS.get(back) == Some(&target))
            .ok_or_else(missing),
        _ => Ok(head),
    }
}

/// A wasm mod's checked listeners, registered once every native mod's are, in the installed order.
/// It owns them until then: one dropped unregistered (its mod refused after the check) lets its
/// group go.
pub struct Listening {
    exports: Exports,
    fire: Option<Fire>,
    places: Vec<(PointId, i32, String)>,
    raw: Option<crate::raw::Listeners>,
}

impl Listening {
    /// The installed owner whose slots these are.
    pub fn id(&self) -> &'static str {
        self.exports.id()
    }

    /// Registers the listeners in the points crate's one list, which keeps them for good.
    pub fn register(self) -> Result<(), mod_points::RegisterError> {
        let id = self.exports.id();
        if let Some(fire) = self.fire {
            let listeners = Listeners {
                exports: self.exports,
                fire,
            };
            let listeners = Box::leak(Box::new(listeners));
            for (index, (place, priority, inject)) in self.places.into_iter().enumerate() {
                let inject = Box::leak(inject.into_boxed_str());
                mod_points::register(
                    &mod_points::__private::SlotListener::forwarded(
                        id,
                        place,
                        listeners,
                        index as u32,
                    )
                    .ordered(priority, inject),
                )?;
            }
        }
        if let Some(raw) = self.raw {
            raw.register()?;
        }
        Ok(())
    }
}

pub(crate) fn export_refusal(error: LoadError, point: &str) -> Refusal {
    match error {
        LoadError::MissingExport(_) => Refusal::UnfilledPoint {
            point: point.into(),
        },
        other => Refusal::Load(other),
    }
}

/// Checks a loaded wasm mod's listeners against its manifest and this game's points: the listeners
/// to register, none for a mod whose manifest lists no injections.
pub(crate) fn listen(manifest: &Manifest, exports: &Exports) -> Result<Option<Listening>, Refusal> {
    if manifest.wasm.is_none() {
        return Ok(None);
    }
    let raw = crate::raw::listen(manifest, exports)?;
    if !manifest.inject.iter().any(|i| i.access == Access::Api) {
        return Ok(raw.map(|raw| Listening {
            exports: exports.clone(),
            fire: None,
            places: vec![],
            raw: Some(raw),
        }));
    }
    let mut declared = BTreeMap::new();
    for inject in manifest.inject.iter().filter(|i| i.access == Access::Api) {
        declared.insert(inject.id.as_str(), (declared_place(inject)?, inject));
    }
    let interface = format!(
        "{}-listeners",
        horizon_mod_package::manifest::export_prefix(&manifest.id)
    );
    let refused = |error| {
        export_refusal(
            error,
            declared
                .first_key_value()
                .expect("nonempty inject declarations")
                .0,
        )
    };
    let subscribed: Subscribed = exports.func(&interface, "subscribed").map_err(refused)?;
    let fire: Fire = exports.func(&interface, "fire").map_err(refused)?;
    let listed = exports
        .load(|store| Ok(subscribed.call(store, ())?.0))
        .ok_or_else(|| {
            Refusal::Points("its listeners didn't answer which places they fill".into())
        })?;
    let mut places = Vec::with_capacity(listed.len());
    for (name, shape) in &listed {
        let Some((place, inject)) = declared.remove(name.as_str()) else {
            return Err(Refusal::UndeclaredPoint {
                point: name.clone(),
            });
        };
        if SHAPES[place] != shape {
            return Err(Refusal::ChangedPoint {
                point: name.clone(),
                built: shape.clone(),
                game: SHAPES[place].to_owned(),
            });
        }
        places.push((IDS[place], inject.priority, inject.id.clone()));
    }
    if let Some((&id, _)) = declared.first_key_value() {
        return Err(Refusal::UnfilledPoint {
            point: id.to_owned(),
        });
    }
    Ok(Some(Listening {
        exports: exports.clone(),
        fire: Some(fire),
        places,
        raw,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_points::places::NAMES;

    #[test]
    fn a_guest_export_with_the_wrong_type_retains_its_load_refusal() {
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve
            .push_str(
                "wrong.wit",
                "package test:wrong; world wrong { export fire: func(); }",
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("wrong")).unwrap();
        let mut module =
            wit_component::dummy_module(&resolve, world, wit_parser::ManglingAndAbi::Standard32);
        wit_component::embed_component_metadata(
            &mut module,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
            true,
        )
        .unwrap();
        let bytes = wit_component::ComponentEncoder::default()
            .module(&module)
            .unwrap()
            .encode()
            .unwrap();
        let engine = wasmtime::Engine::default();
        let component = wasmtime::component::Component::new(&engine, bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::component::Linker::<()>::new(&engine)
            .instantiate(&mut store, &component)
            .unwrap();
        let wrong: Result<Fire, _> = instance.get_typed_func(&mut store, "fire");
        let error = LoadError::ChangedExport(
            "test-listeners#fire".into(),
            wrong.err().unwrap().to_string(),
        );
        assert!(
            matches!(export_refusal(error, "listen"), Refusal::Load(LoadError::ChangedExport(name, _)) if name == "test-listeners#fire")
        );
        assert!(
            matches!(export_refusal(LoadError::MissingExport("test-listeners#fire".into()), "listen"), Refusal::UnfilledPoint { point } if point == "listen")
        );
    }

    const TICK: &str = "horizon_client::game::Game<'_>::tick";

    #[test]
    fn an_injection_names_the_place_its_kind_and_method_say() {
        let mut inject = Inject {
            id: "tick".into(),
            at: TICK.into(),
            kind: Kind::Head,
            with: "any_function".into(),
            access: Access::Api,
            priority: 0,
            hash: "signature".into(),
            call: None,
            nth: None,
            ctx: None,
        };
        assert_eq!(NAMES[declared_place(&inject).unwrap()], "tick");
        inject.priority = 10;
        inject.kind = Kind::Return;
        assert_eq!(NAMES[declared_place(&inject).unwrap()], "tick_return");
        for (access, kind, priority) in [
            (Access::Internals, Kind::Head, 0),
            (Access::Api, Kind::WrapCall, 0),
        ] {
            inject.access = access;
            inject.kind = kind;
            inject.priority = priority;
            assert!(matches!(declared_place(&inject), Err(Refusal::Points(_))));
        }
        inject.access = Access::Api;
        inject.kind = Kind::Head;
        inject.priority = 0;
        inject.at = "game::gone".into();
        assert!(matches!(
            declared_place(&inject),
            Err(Refusal::NoSuchPoint { .. })
        ));
        inject.access = Access::Api;
        inject.priority = 0;
        inject.kind = Kind::Head;
        inject.at = "horizon_client::game::commands::Game<'_>::acting_player_name".into();
        assert!(declared_place(&inject).is_ok());
        inject.kind = Kind::Return;
        assert!(matches!(
            declared_place(&inject),
            Err(Refusal::NoSuchPoint { .. })
        ));
    }
}
