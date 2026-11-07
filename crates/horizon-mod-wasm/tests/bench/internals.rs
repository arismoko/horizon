//! A real guest crossing the internals imports, not just the native frame implementation.
use mod_points::internals::{
    self as I, Arg, Member, Op, Place, Refusal, Reply, Table, Thunk, Type, Value,
};

#[derive(Debug, PartialEq)]
struct Game {
    copy: i64,
    child: Child,
    fields: [i64; 12],
}
#[derive(Debug, PartialEq)]
struct Child {
    value: i64,
    padding: Vec<u8>,
}
struct Record {
    message: String,
    cancel: bool,
}

fn world() -> Game {
    Game {
        copy: 7,
        fields: [7; 12],
        child: Child {
            value: 11,
            padding: Vec::new(),
        },
    }
}
unsafe fn game(ptr: *mut (), writable: bool, op: Op<'_>) -> Result<Reply, Refusal> {
    // SAFETY: this test's descriptor names exactly the live Game supplied to fire_raw.
    let game = unsafe { &mut *ptr.cast::<Game>() };
    match op {
        Op::Check(0, Value::S(_)) => Ok(Reply::Done),
        Op::Get(0) => Ok(Reply::Value(Value::S(game.copy))),
        Op::Set(0, Value::S(n)) if writable => {
            game.copy = n;
            Ok(Reply::Done)
        }
        Op::Field(1) => Ok(Reply::Node((&raw mut game.child).cast(), 1, writable)),
        Op::Check(2..=13, Value::S(_)) => Ok(Reply::Done),
        Op::Get(n @ 2..=13) => Ok(Reply::Value(Value::S(game.fields[n as usize - 2]))),
        Op::Set(n @ 2..=13, Value::S(v)) if writable => {
            game.fields[n as usize - 2] = v;
            Ok(Reply::Done)
        }
        _ => Err(I::not_here("not a benchmark game field")),
    }
}
unsafe fn child(ptr: *mut (), _: bool, op: Op<'_>) -> Result<Reply, Refusal> {
    // SAFETY: only the Game's child step supplies this node.
    let child = unsafe { &*ptr.cast::<Child>() };
    match op {
        Op::Check(0, Value::S(_)) => Ok(Reply::Done),
        Op::Get(0) => Ok(Reply::Value(Value::S(child.value))),
        _ => Err(I::not_here("not a benchmark child field")),
    }
}
unsafe fn record(ptr: *mut (), writable: bool, op: Op<'_>) -> Result<Reply, Refusal> {
    // SAFETY: the descriptor below names this exact raw record layout.
    let record = unsafe { &mut *ptr.cast::<Record>() };
    match op {
        Op::Check(0, Value::Str(_)) => Ok(Reply::Done),
        Op::Get(0) => Ok(Reply::Value(Value::Str(record.message.clone()))),
        Op::Get(1) => Ok(Reply::Value(Value::B(record.cancel))),
        Op::Set(0, Value::Str(message)) if writable => {
            record.message = message;
            Ok(Reply::Done)
        }
        _ => Err(I::not_here("not a benchmark raw field")),
    }
}
#[inline(never)]
fn native_add(game: &mut Game, delta: i64) -> i64 {
    game.copy += delta;
    game.copy
}
unsafe fn add(receiver: Option<*mut ()>, args: Vec<Arg>) -> Result<I::CallReply, Refusal> {
    // SAFETY: Frame checked the receiver's type and exclusive write access.
    let game = unsafe { &mut *receiver.unwrap().cast::<Game>() };
    let [arg] = I::fields::<_, 1>(args)?;
    Ok(I::CallReply::new(Reply::Value(Value::S(native_add(
        game,
        I::int(I::value(arg)?)?,
    )))))
}

fn table() -> &'static Table {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("internals/internals.json")).unwrap();
    let hash = |path: &str| -> &'static str {
        Box::leak(
            catalog
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["path"] == path)
                .unwrap()["hash"]
                .as_str()
                .unwrap()
                .to_owned()
                .into_boxed_str(),
        )
    };
    let point = mod_points::PointId::HandleChatInput as usize;
    Box::leak(Box::new(Table {
        types: Box::leak(Box::new([
            Type {
                path: "horizon_client::game::Game",
                hash: hash("horizon_client::game::Game"),
                node: game,
                raw_fields: &[],
                raw_gets: &[],
                raw_index: false,
            },
            Type {
                path: "horizon_client::game::Child",
                hash: hash("horizon_client::game::Child"),
                node: child,
                raw_fields: &[],
                raw_gets: &[],
                raw_index: false,
            },
            Type {
                path: "fixture::Raw",
                hash: "",
                node: record,
                raw_fields: &[],
                raw_gets: &[],
                raw_index: false,
            },
        ])),
        members: Box::leak(
            vec![
                Member {
                    path: "horizon_client::game::Game::copy",
                    hash: hash("horizon_client::game::Game::copy"),
                    owner: 0,
                    field: 0,
                },
                Member {
                    path: "horizon_client::game::Game::child",
                    hash: hash("horizon_client::game::Game::child"),
                    owner: 0,
                    field: 1,
                },
            ]
            .into_iter()
            .chain((0..12).map(|i| {
                let path = Box::leak(format!("horizon_client::game::Game::f{i}").into_boxed_str());
                Member {
                    path,
                    hash: hash(path),
                    owner: 0,
                    field: i + 2,
                }
            }))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        ),
        thunks: Box::leak(Box::new([Thunk {
            path: "horizon_client::game::Game::add",
            hash: hash("horizon_client::game::Game::add"),
            receiver: Some(0),
            args: &[None],
            writes: 1,
            call: add,
        }])),
        places: Box::leak(Box::new([Place {
            place: point,
            target: mod_points::places::TARGETS[point],
            signature_hash: "fixture-signature",
            hash: mod_points::places::SHAPES[point],
            receiver: Some(0),
            receiver_writable: true,
            record: 2,
        }])),
        ..Table::EMPTY
    }))
}

fn wasm(mode: &str, count: i64) -> (Game, String) {
    let mut game = world();
    let mut record = Record {
        message: format!("{mode} {count}"),
        cancel: false,
    };
    mod_points::__private::fire_raw(
        mod_points::PointId::HandleChatInput,
        "handle_chat_input",
        &mut |kind| {
            let mod_points::__private::RawKind::Indexed(callback, index) = kind else {
                panic!("wasm listener")
            };
            // SAFETY: table() supplies exactly these receiver and record layouts.
            unsafe {
                callback(
                    mod_points::PointId::HandleChatInput as usize,
                    index,
                    (&raw mut game).cast(),
                    (&raw mut record).cast(),
                );
            }
        },
    );
    (game, record.message)
}
fn native(mode: &str, count: i64) -> (Game, String) {
    let mut game = world();
    let mut total = 0;
    for i in 0..std::hint::black_box(count) {
        let game = std::hint::black_box(&mut game);
        match mode {
            "get" => total += game.copy,
            "handle" => total += game.child.value,
            "set" => game.copy = i,
            "call" => total += native_add(game, 1),
            "get10-single" | "get10-batch" | "get12-single" | "get12-batch" => {
                total += game.fields[..if mode.starts_with("get10") { 10 } else { 12 }]
                    .iter()
                    .sum::<i64>()
            }
            "set10-single" | "set10-batch" | "set12-single" | "set12-batch" => {
                game.fields[..if mode.starts_with("set10") { 10 } else { 12 }].fill(i)
            }
            _ => unreachable!(),
        }
    }
    (game, total.to_string())
}

#[test]
#[ignore = "needs tests/bench/internals built to mod.wasm; run alone with --test-threads=1"]
fn internals_accesses_against_native() {
    horizon_mod_wasm::set_internals(table());
    let path = |name: &str| std::path::PathBuf::from(std::env::var(name).expect(name));
    let package = path("HORIZON_WASM_INTERNALS_BENCH");
    let loaded = horizon_mod_wasm::load(
        &horizon_mod_wasm::RuntimeConfig::default(),
        &std::fs::read(path("HORIZON_WASM_STD")).unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest: horizon_mod_package::Package::read(&package)
                .unwrap()
                .manifest,
            module: horizon_mod_package::Package::read_wasm(&package).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    for listeners in loaded.listening {
        listeners.register().unwrap();
    }
    mod_points::__private::seal();
    let mut report = String::from(
        "### Internals imports against native\n\nMedian of 9 batches after warm-up. Reads and calls use 256 accesses per call, with the zero-access call subtracted before division. Set uses one write and one apply per call (5,000 calls per batch), not coalesced repeated writes. Handle read includes field then get. Native accesses are black-boxed per iteration and measured in batches of 256 for timer resolution; each native write applies immediately.\n\n| Access | Wasm ns | Native ns |\n| --- | ---: | ---: |\n",
    );
    for mode in ["get", "handle", "set", "call"] {
        let count = if mode == "set" { 1 } else { 256 };
        let calls = if mode == "set" { 5_000 } else { 100 };
        assert_eq!(
            wasm(mode, count),
            native(mode, count),
            "{mode}: committed values and result"
        );
        let measure = |call: fn(&str, i64) -> (Game, String), count: i64, calls| {
            let base = super::timed(calls, || {
                std::hint::black_box(call(mode, 0));
            });
            let work = super::timed(calls, || {
                std::hint::black_box(call(mode, count));
            });
            (work.as_secs_f64() - base.as_secs_f64()).max(0.0) * 1e9 / count as f64
        };
        report.push_str(&format!(
            "| {mode} | {:.2} | {:.2} |\n",
            measure(wasm, count, calls),
            measure(native, 256, 100)
        ));
    }
    report.push_str("\n### Batched fields\n\nOne listener reads or journals writes to 10 or 12 distinct i64 fields; includes one apply for sets. Baseline zero-work listener subtracted. Median of 9 batches; ns per field.\n\n| Access | Fields | Single ns/field | Batch ns/field |\n| --- | ---: | ---: | ---: |\n");
    for fields in [10, 12] {
        for access in ["get", "set"] {
            let mut ns = Vec::new();
            for route in ["single", "batch"] {
                let mode = format!("{access}{fields}-{route}");
                assert_eq!(wasm(&mode, 1), native(&mode, 1));
                let base = super::timed(5_000, || {
                    std::hint::black_box(wasm(&mode, 0));
                });
                let work = super::timed(5_000, || {
                    std::hint::black_box(wasm(&mode, 1));
                });
                ns.push((work.as_secs_f64() - base.as_secs_f64()).max(0.0) * 1e9 / fields as f64);
            }
            report.push_str(&format!(
                "| {access} | {fields} | {:.2} | {:.2} |\n",
                ns[0], ns[1]
            ));
        }
    }
    report.push_str("\n### Reused list transport\n\n128 twelve-field batches per listener, baseline subtracted. Host counts include journal growth; guest counts cover only the access loop.\n\n| Access | ns/batch | Host alloc/batch | Host realloc/batch | Guest alloc/batch | Guest realloc/batch |\n| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for mode in ["get12-batch", "set12-batch"] {
        const BATCHES: i64 = 128;
        assert_eq!(wasm(mode, BATCHES), native(mode, BATCHES));
        let base = super::timed(100, || {
            std::hint::black_box(wasm(mode, 0));
        });
        let work = super::timed(100, || {
            std::hint::black_box(wasm(mode, BATCHES));
        });
        let count = |batches| {
            super::allocations::start(true);
            std::hint::black_box(wasm(mode, batches));
            super::allocations::stop()
        };
        let baseline = count(0);
        let allocations = count(BATCHES);
        let host = (allocations.0 - baseline.0, allocations.1 - baseline.1);
        if mode.starts_with("get") {
            assert!(
                host.0 <= BATCHES as usize && host.1 <= 2 * BATCHES as usize,
                "get adds no allocation above Wasmtime's per-import borrow ledger: {host:?}"
            );
        }
        let guest: Vec<usize> = wasm(&format!("{mode}-alloc"), BATCHES)
            .1
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        report.push_str(&format!(
            "| {mode} | {:.2} | {:.4} | {:.4} | {:.4} | {:.4} |\n",
            (work.as_secs_f64() - base.as_secs_f64()).max(0.0) * 1e9 / BATCHES as f64,
            host.0 as f64 / BATCHES as f64,
            host.1 as f64 / BATCHES as f64,
            guest[0] as f64 / BATCHES as f64,
            guest[1] as f64 / BATCHES as f64
        ));
        if mode.starts_with("set") {
            super::allocations::start(true);
            let mut journal = horizon_mod_wasm::journal::Journal::default();
            for i in 0..BATCHES {
                for field in 0..12 {
                    journal.set(
                        horizon_mod_wasm::journal::Path {
                            root: 0,
                            steps: Vec::new(),
                        },
                        field,
                        Value::S(i),
                    );
                }
            }
            let owned = super::allocations::stop();
            assert!(
                host.0 <= owned.0 + BATCHES as usize && host.1 <= owned.1 + 2 * BATCHES as usize,
                "set adds no allocation above the journal and Wasmtime borrow ledger: {host:?}"
            );
            report.push_str(&format!(
                "\nJournal alone, 1,536 scalar root writes: {} allocations, {} reallocations.\n\n",
                owned.0, owned.1
            ));
        }
    }
    assert_eq!(
        wasm("trap", 1),
        (world(), "trap 1".into()),
        "a real guest trap discards its pending set"
    );
    assert_eq!(
        wasm("get", 1),
        (world(), "get 1".into()),
        "the trapped group stays stopped"
    );
    println!("{report}");
    if let Ok(path) = std::env::var("HORIZON_WASM_INTERNALS_BENCH_REPORT") {
        std::fs::write(path, report).unwrap();
    }
}
