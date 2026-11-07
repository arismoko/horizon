//! The game's internals as a wasm mod reaches them (#589 W): the table the staged game generates
//! (`horizon_client::__internals::TABLE`) and what crosses through it. Only the game's generated
//! code and the wasm runtime use it; a mod sees its own generated view instead.
//!
//! Every pointer here is derived, within one call into a mod, from that call's roots (the slot's
//! own borrows); none is kept past the call. The runtime keeps that rule; this module only names
//! the shapes.

/// A copy of a plain game value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `()`, or a unit case.
    None,
    B(bool),
    C(char),
    S(i64),
    U(u64),
    F(f64),
    Str(String),
    /// An array, a slice, a list or a set's elements; a map's entries, each a two-field `Rec`.
    Seq(Vec<Value>),
    /// A tuple's or a struct's fields, in order.
    Rec(Vec<Value>),
    /// An enum's case, by index, with its fields; an `Option` is case 0 for `None` and 1 for `Some`.
    Var(u32, Vec<Value>),
}

/// Why a step, read, write or call can't be done.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// The node's call is over, or its path leads nowhere now.
    Gone(String),
    /// A value of the wrong shape for its place.
    Changed(String),
    /// Not on this node: another enum case, an index past the end, a key not present, a write
    /// under a shared node, or an operation its type doesn't have.
    NotHere(String),
}

/// One operation on a node. A field is numbered in its type's declaration order; an enum's fields
/// are numbered across its cases, in order.
#[derive(Debug)]
pub enum Op<'v> {
    /// A field whose type is walked: answers its node.
    Field(u32),
    /// A field whose type is a copy: answers its value.
    Get(u32),
    /// Writes a copy-typed field.
    Set(u32, Value),
    /// Checks a copy field with its codec without changing game state.
    Check(u32, &'v Value),
    /// A list's element: answers its node.
    Index(u64),
    /// A map's entry by key: answers its node.
    Key(&'v Value),
    /// Canonical wire representation of a native map key.
    CanonicalKey(&'v Value),
    /// The number of elements or entries.
    Len,
    /// An enum's or an `Option`'s case.
    Case,
    /// The whole node, when its type has a copy.
    Read,
}

/// What an operation answers.
#[derive(Debug, PartialEq)]
pub enum Reply {
    /// A node, of a type in the table, and whether it may be written (`false` under an `Arc` or
    /// `Rc`, or under a `&` the game holds).
    Node(*mut (), u32, bool),
    Value(Value),
    Done,
}

/// A type's operations, on a pointer to a value of that type, which may be written (and reached
/// through `&mut`) only when the flag says so.
pub type NodeFn = unsafe fn(*mut (), bool, Op<'_>) -> Result<Reply, Refusal>;

/// A function's argument: a copy, or a node standing for a reference argument.
#[derive(Debug)]
pub enum Arg {
    Value(Value),
    Node(*mut ()),
}

/// A function's result and the final values of its mutable copy arguments, in parameter order.
#[derive(Debug, PartialEq)]
pub struct CallReply<T = Reply> {
    pub result: T,
    pub updates: Vec<Value>,
}

impl<T> CallReply<T> {
    /// A call without mutable copy arguments.
    pub fn new(result: T) -> Self {
        Self {
            result,
            updates: Vec::new(),
        }
    }
}

/// A declared function's call: its receiver's node (none for a free function) and its arguments.
/// A reference it returns comes back as a node, valid until the next call.
pub type ThunkFn = unsafe fn(Option<*mut ()>, Vec<Arg>) -> Result<CallReply, Refusal>;

/// A declared function.
pub struct Thunk {
    /// Its `internals.json` path.
    pub path: &'static str,
    /// Its `internals.json` hash.
    pub hash: &'static str,
    /// Its receiver's node type, for a method; `None` for a free or associated function.
    pub receiver: Option<u32>,
    /// Each argument's node type, or `None` for one given as a copy.
    pub args: &'static [Option<u32>],
    /// Which nodes it writes through: bit 0 the receiver, bit `i + 1` argument `i`. The runtime
    /// refuses a shared node there.
    pub writes: u64,
    pub call: ThunkFn,
}

/// A walked type's identity and operations.
pub struct Type {
    pub path: &'static str,
    /// Empty for a wrapper or collection, reached only through a checked named type.
    pub hash: &'static str,
    pub node: NodeFn,
    /// Fields whose step creates no reference to the parent (including wrappers).
    pub raw_fields: &'static [u32],
    /// Primitive field reads that do not create references.
    pub raw_gets: &'static [u32],
    /// Index steps create no reference to the parent or its elements.
    pub raw_index: bool,
}

/// A raw point's receiver and record, as roots of a call.
pub struct Place {
    pub place: usize,
    pub target: &'static str,
    /// The pinned function signature, distinct from the wire record shape.
    pub signature_hash: &'static str,
    pub hash: &'static str,
    pub receiver: Option<u32>,
    pub receiver_writable: bool,
    pub record: u32,
}

/// A declared field's identity and declaration-order position in its owner.
pub struct Member {
    pub path: &'static str,
    pub hash: &'static str,
    pub owner: u32,
    pub field: u32,
}

/// The staged game's internals.
pub struct Table {
    pub types: &'static [Type],
    pub thunks: &'static [Thunk],
    pub places: &'static [Place],
    pub members: &'static [Member],
    /// Paths that must remain native, even when a mod declares them.
    pub native_only: &'static [&'static str],
}

impl Table {
    /// A game built without wasm internals support.
    pub const EMPTY: Self = Self {
        types: &[],
        thunks: &[],
        places: &[],
        members: &[],
        native_only: &[],
    };
}

/// The wrong shape for `what`.
pub fn changed(what: &str) -> Refusal {
    Refusal::Changed(what.to_owned())
}

/// Not on this node.
pub fn not_here(what: &str) -> Refusal {
    Refusal::NotHere(what.to_owned())
}

/// A whole number from `value`, if it fits `T`.
pub fn int<T: TryFrom<i64> + TryFrom<u64>>(value: Value) -> Result<T, Refusal> {
    match value {
        Value::S(n) => T::try_from(n).map_err(|_| changed("an integer out of range")),
        Value::U(n) => T::try_from(n).map_err(|_| changed("an integer out of range")),
        _ => Err(changed("an integer")),
    }
}

/// Exactly `N` fields of a `Rec` or a case, or arguments.
pub fn fields<T, const N: usize>(values: Vec<T>) -> Result<[T; N], Refusal> {
    values
        .try_into()
        .map_err(|_| changed("a count of fields or arguments"))
}

/// A `Rec`'s fields.
pub fn rec(value: Value) -> Result<Vec<Value>, Refusal> {
    match value {
        Value::Rec(fields) => Ok(fields),
        _ => Err(changed("a record")),
    }
}

/// A `Seq`'s elements.
pub fn seq(value: Value) -> Result<Vec<Value>, Refusal> {
    match value {
        Value::Seq(items) => Ok(items),
        _ => Err(changed("a sequence")),
    }
}

/// An argument given as a copy.
pub fn value(arg: Arg) -> Result<Value, Refusal> {
    match arg {
        Arg::Value(value) => Ok(value),
        Arg::Node(_) => Err(changed("a value, not a node")),
    }
}

/// An argument given as a node.
pub fn node(arg: Arg) -> Result<*mut (), Refusal> {
    match arg {
        Arg::Node(node) => Ok(node),
        Arg::Value(_) => Err(changed("a node, not a value")),
    }
}

/// The generic internals interface, generated from the same WIT on host and guest.
#[cfg(all(feature = "wasm-host", not(target_arch = "wasm32")))]
pub mod bindings {
    wasmtime::component::bindgen!({ path: "wit/internals.wit", world: "internals", imports: { default: trappable } });
}
#[cfg(target_arch = "wasm32")]
pub mod bindings {
    wit_bindgen::generate!({
        path: "wit/internals.wit", world: "internals",
        runtime_path: "mod_points::internals::rt",
        pub_export_macro: true,
        export_macro_name: "export_raw_listeners",
        default_bindings_module: "mod_points::internals::bindings",
    });
}
#[cfg(any(target_arch = "wasm32", feature = "wasm-host"))]
pub use bindings::horizon::mod_internals::view::Cell;
#[cfg(target_arch = "wasm32")]
pub use wit_bindgen::rt;

/// A plain value as pre-order cells. Compound counts count children, not all descendants.
#[cfg(any(target_arch = "wasm32", feature = "wasm-host"))]
pub fn flatten(value: Value) -> Vec<Cell> {
    let mut cells = Vec::new();
    flatten_into(value, &mut cells, &mut Vec::new());
    cells
}

/// Appends pre-order cells using caller-owned output and traversal buffers.
#[cfg(any(target_arch = "wasm32", feature = "wasm-host"))]
pub fn flatten_into(value: Value, cells: &mut Vec<Cell>, pending: &mut Vec<Value>) {
    pending.clear();
    let mut first = Some(value);
    while let Some(value) = first.take().or_else(|| pending.pop()) {
        cells.push(match value {
            Value::None => Cell::None,
            Value::B(v) => Cell::B(v),
            Value::C(v) => Cell::C(v),
            Value::S(v) => Cell::S(v),
            Value::U(v) => Cell::U(v),
            Value::F(v) => Cell::F(v),
            Value::Str(v) => Cell::Str(v),
            Value::Seq(children) => {
                let cell = Cell::Seq(children.len() as u32);
                pending.extend(children.into_iter().rev());
                cell
            }
            Value::Rec(children) => {
                let cell = Cell::Rec(children.len() as u32);
                pending.extend(children.into_iter().rev());
                cell
            }
            Value::Var(case, children) => {
                let cell = Cell::Var((case, children.len() as u32));
                pending.extend(children.into_iter().rev());
                cell
            }
        });
    }
}

/// One value from pre-order cells. Refuses missing children, trailing cells and excessive nesting.
#[cfg(any(target_arch = "wasm32", feature = "wasm-host"))]
pub fn unflatten(
    cells: impl IntoIterator<Item = Cell, IntoIter: ExactSizeIterator>,
) -> Result<Value, Refusal> {
    fn next(
        cells: &mut impl ExactSizeIterator<Item = Cell>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        if depth > 128 {
            return Err(changed("a value nested too deeply"));
        }
        let cell = cells.next().ok_or_else(|| changed("missing value cells"))?;
        let (count, case) = match cell {
            Cell::None => return Ok(Value::None),
            Cell::B(v) => return Ok(Value::B(v)),
            Cell::C(v) => return Ok(Value::C(v)),
            Cell::S(v) => return Ok(Value::S(v)),
            Cell::U(v) => return Ok(Value::U(v)),
            Cell::F(v) => return Ok(Value::F(v)),
            Cell::Str(v) => return Ok(Value::Str(v)),
            Cell::Seq(n) => (n, None),
            Cell::Rec(n) => (n, Some(None)),
            Cell::Var((case, n)) => (n, Some(Some(case))),
        };
        if count as usize > cells.len() {
            return Err(changed("missing value children"));
        }
        let children = (0..count)
            .map(|_| next(cells, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match case {
            None => Value::Seq(children),
            Some(None) => Value::Rec(children),
            Some(Some(case)) => Value::Var(case, children),
        })
    }
    let mut cells = cells.into_iter();
    let value = next(&mut cells, 0)?;
    if cells.len() != 0 {
        return Err(changed("trailing value cells"));
    }
    Ok(value)
}

#[cfg(all(feature = "wasm-host", not(target_arch = "wasm32")))]
impl From<Refusal> for bindings::horizon::mod_internals::view::Refusal {
    fn from(why: Refusal) -> Self {
        match why {
            Refusal::Gone(why) => Self::Gone(why),
            Refusal::Changed(why) => Self::Changed(why),
            Refusal::NotHere(why) => Self::NotHere(why),
        }
    }
}
