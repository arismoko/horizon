use mod_points::internals::{
    self as I, Arg, Member, Op, Place, Refusal, Reply, Table, Thunk, Type, Value,
};

#[derive(Debug, PartialEq)]
pub struct Game {
    pub left: i64,
    pub right: i64,
    pub hidden: Vec<i32>,
    pub refused_sets: usize,
    pub reads: usize,
}
pub struct Record {
    pub message: String,
    pub cancel: bool,
}

unsafe fn game(ptr: *mut (), writable: bool, op: Op<'_>) -> Result<Reply, Refusal> {
    // SAFETY: the synthetic point below supplies the live, exclusively borrowed fixture Game.
    let game = unsafe { &mut *ptr.cast::<Game>() };
    match op {
        Op::Check(0, Value::Str(_)) => {
            game.refused_sets += 1;
            Err(I::not_here("left is i64, not String"))
        }
        Op::Check(0, Value::S(_)) => Ok(Reply::Done),
        Op::Get(0) => Ok(Reply::Value(Value::S(game.left))),
        Op::Check(1, Value::S(_)) => Ok(Reply::Done),
        Op::Get(1) => {
            game.reads += 1;
            Ok(Reply::Value(Value::S(game.right)))
        }
        Op::Set(0, Value::S(n)) if writable => {
            assert_ne!(n, -777, "fixture final journal apply panic");
            game.left = n;
            Ok(Reply::Done)
        }
        Op::Set(1, Value::S(n)) if writable => {
            game.right = n;
            Ok(Reply::Done)
        }
        _ => Err(I::not_here("not a declared game field")),
    }
}
unsafe fn record(ptr: *mut (), writable: bool, op: Op<'_>) -> Result<Reply, Refusal> {
    // SAFETY: the synthetic point supplies the live Record for this call only.
    let record = unsafe { &mut *ptr.cast::<Record>() };
    match op {
        Op::Check(0, Value::Str(_)) => Ok(Reply::Done),
        Op::Get(0) => Ok(Reply::Value(Value::Str(record.message.clone()))),
        Op::Check(1, Value::B(_)) => Ok(Reply::Done),
        Op::Get(1) => Ok(Reply::Value(Value::B(record.cancel))),
        Op::Set(0, Value::Str(s)) if writable => {
            record.message = s;
            Ok(Reply::Done)
        }
        Op::Set(1, Value::B(b)) if writable => {
            record.cancel = b;
            Ok(Reply::Done)
        }
        _ => Err(I::not_here("not on the raw record")),
    }
}
unsafe fn add(receiver: Option<*mut ()>, args: Vec<Arg>) -> Result<I::CallReply, Refusal> {
    // SAFETY: the checked thunk descriptor requires a writable Game receiver.
    let game = unsafe { &mut *receiver.unwrap().cast::<Game>() };
    let [arg] = I::fields::<_, 1>(args)?;
    let delta: i64 = I::int(I::value(arg)?)?;
    assert_ne!(delta, -999, "fixture thunk panic");
    game.left += delta;
    Ok(I::CallReply::new(Reply::Value(Value::S(
        game.left + game.right,
    ))))
}

struct Small {
    count: u32,
}
unsafe fn small(ptr: *mut (), _: bool, op: Op<'_>) -> Result<Reply, Refusal> {
    // SAFETY: the descriptor identifies a live Small when this node is reached.
    let small = unsafe { &*ptr.cast::<Small>() };
    match op {
        Op::Read => Ok(Reply::Value(Value::Rec(vec![Value::U(small.count.into())]))),
        _ => Err(I::not_here("not on Small")),
    }
}
fn mutate_copies(n: &mut u32, small: &mut Small) -> u32 {
    *n += 5;
    small.count += 7;
    *n + small.count
}
unsafe fn mutate(_: Option<*mut ()>, args: Vec<Arg>) -> Result<I::CallReply, Refusal> {
    let [n, small] = I::fields::<_, 2>(args)?;
    let mut n = I::int(I::value(n)?)?;
    let [count] = I::fields::<_, 1>(I::rec(I::value(small)?)?)?;
    let mut small = Small {
        count: I::int(count)?,
    };
    let result = mutate_copies(&mut n, &mut small);
    Ok(I::CallReply {
        result: Reply::Value(Value::U(result.into())),
        updates: vec![
            Value::U(n.into()),
            Value::Rec(vec![Value::U(small.count.into())]),
        ],
    })
}

pub fn table() -> &'static Table {
    let point = mod_points::PointId::HandleChatInput;
    let place = point as usize;
    Box::leak(Box::new(Table {
        types: &[
            Type {
                path: "horizon_client::game::Game",
                hash: "6957fb66139038f6ab07ec4121dc6253b9f31532f1f3e45450b5439e695da605",
                node: game,
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
            Type {
                path: "horizon_client::Small",
                hash: "small-hash",
                node: small,
                raw_fields: &[],
                raw_gets: &[],
                raw_index: false,
            },
        ],
        members: &[
            Member {
                path: "horizon_client::game::Game::left",
                hash: "71b38399f108e572a5ef928b5cfdf52872f61b4615115d91f588221900ab97e0",
                owner: 0,
                field: 0,
            },
            Member {
                path: "horizon_client::game::Game::right",
                hash: "7b6f34593a32100b46a93d4562b17b8adeb9c7e5645fe9f8efbccc519aadfcc2",
                owner: 0,
                field: 1,
            },
        ],
        thunks: &[
            Thunk {
                path: "horizon_client::game::Game::add",
                hash: "638665cf01d552d5765711831232b653f7b98d9c7bdddb54a58182e433e5cd0e",
                receiver: Some(0),
                args: &[None],
                writes: 1,
                call: add,
            },
            Thunk {
                path: "horizon_client::mutate_copies",
                hash: "mutate-copies-hash",
                receiver: None,
                args: &[None, None],
                writes: 0,
                call: mutate,
            },
        ],
        places: Box::leak(Box::new([Place {
            place,
            target: mod_points::places::TARGETS[place],
            signature_hash: "fixture-signature",
            hash: mod_points::places::SHAPES[place],
            receiver: Some(0),
            receiver_writable: true,
            record: 1,
        }])),
        ..Table::EMPTY
    }))
}
