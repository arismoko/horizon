//! Typed `ContextIntProvider` and `ContextFloatProvider` evaluation.
use super::{Providers, Value};
use smallvec::{IntoIter, SmallVec};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
};
use sunrise_core::{
    java_number::float_to_string,
    random::{LevelRandom, RandomSource},
};
use sunrise_items::item_data::cooking_fuel::{CookingFuel, Resolvable};

type Arith = String;
type Node<'v> = Cow<'v, Value>;
type ResultValue = Result<Scalar, Arith>;
type Args<'v> = SmallVec<[(Kind, Option<Node<'v>>); 2]>;

/// The same numeric evaluation for commands and container processes. The
/// caller supplies the level's random source and its loot-condition context.
pub struct Evaluation<'a> {
    providers: &'a Providers,
    random: &'a mut LevelRandom,
}

pub trait ConditionContext<'v> {
    fn leaf(&mut self, condition: &Value) -> bool;

    fn named(&self, _id: &str) -> Option<Cow<'v, Value>> {
        None
    }

    fn valid(&self, _condition: &Value) -> bool {
        true
    }

    fn kind<'n>(&self, condition: &'n Value) -> Option<&'n str> {
        condition
            .get("condition")
            .or_else(|| condition.get("type"))?
            .as_str()
    }

    fn chance<'n>(&self, condition: &'n Value) -> Option<&'n Value> {
        condition.get("chance")
    }

    fn singleton_terms(&self) -> bool {
        true
    }
}

impl<F: FnMut(&Value) -> bool> ConditionContext<'_> for F {
    fn leaf(&mut self, condition: &Value) -> bool {
        self(condition)
    }
}

fn lookup<'a>(table: &'a BTreeMap<String, Value>, id: &str) -> Option<&'a Value> {
    if let Some(path) = id.strip_prefix(':') {
        table.get(&format!("minecraft:{path}"))
    } else if id.contains(':') {
        table.get(id)
    } else {
        table.get(&format!("minecraft:{id}"))
    }
}

/// Project a child of a borrowed definition without copying it. Command
/// predicate lookups can instead supply an owned node, independent of Game.
fn child<'v>(node: &Node<'v>, get: impl FnOnce(&Value) -> Option<&Value>) -> Option<Node<'v>> {
    match node {
        Cow::Borrowed(v) => get(v).map(Cow::Borrowed),
        Cow::Owned(v) => get(v).cloned().map(Cow::Owned),
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Int,
    Float,
}
impl Kind {
    fn zero(self) -> Scalar {
        match self {
            Self::Int => Scalar::Int(0),
            Self::Float => Scalar::Float(0.0),
        }
    }
}

#[derive(Clone, Copy)]
enum Scalar {
    Int(i32),
    Float(f32),
    Bool(bool),
}
impl Scalar {
    fn int(self) -> i32 {
        let Self::Int(v) = self else {
            unreachable!("integer operand")
        };
        v
    }
    fn float(self) -> f32 {
        let Self::Float(v) = self else {
            unreachable!("float operand")
        };
        v
    }
    fn truth(self) -> bool {
        let Self::Bool(v) = self else {
            unreachable!("condition result")
        };
        v
    }
}

/// A suspended provider or condition keeps its next operand and any previous
/// results here instead of holding a native call frame while visiting a holder.
enum Work<'v> {
    Number(Kind, Option<Node<'v>>),
    Holder(Kind, &'v Value),
    Leave(*const Value),
    LeavePredicate(String),
    Apply(
        Op,
        IntoIter<[(Kind, Option<Node<'v>>); 2]>,
        SmallVec<[Scalar; 2]>,
    ),
    Branch(Kind, Node<'v>),
    Dispatch(Kind, Node<'v>, usize),
    Condition(Option<Node<'v>>, bool),
    Logic(Node<'v>, usize, bool, bool),
    Invert,
    Chance,
}
type WorkStack<'v> = SmallVec<[Work<'v>; 8]>;

// `Validatable.validateHolder`'s active identity rule. Only the inline storage
// is bounded; deep paths spill into a set so distinct holders remain cheap.
enum Holders {
    Inline(SmallVec<[*const Value; 8]>),
    Deep(HashSet<*const Value>),
}
impl Holders {
    fn enter(&mut self, id: *const Value) -> bool {
        match self {
            Self::Inline(ids) => {
                if ids.contains(&id) {
                    return false;
                }
                if ids.len() == ids.inline_size() {
                    *self = Self::Deep(ids.iter().copied().chain([id]).collect());
                } else {
                    ids.push(id);
                }
                true
            }
            Self::Deep(ids) => ids.insert(id),
        }
    }
    fn leave(&mut self, id: *const Value) {
        match self {
            Self::Inline(ids) => {
                let last = ids.pop();
                debug_assert_eq!(last, Some(id));
            }
            Self::Deep(ids) => {
                ids.remove(&id);
            }
        }
    }
}

impl<'a> Evaluation<'a> {
    pub fn new(providers: &'a Providers, random: &'a mut LevelRandom) -> Self {
        Self { providers, random }
    }

    /// `AbstractFurnaceBlockEntity.getBurnDuration` and `getSpeedMultiplier`.
    /// A missing key uses its field default; a present one uses the safe getter.
    pub fn cooking_fuel<'v>(
        &mut self,
        fuel: &CookingFuel,
        context: &mut dyn ConditionContext<'v>,
    ) -> (i32, f32)
    where
        'a: 'v,
    {
        let time = match &fuel.burn_time {
            Resolvable::Constant(value) => *value,
            Resolvable::Reference(key) => self.providers.0.ints.get(key).map_or(0, |v| {
                self.run(Work::Holder(Kind::Int, v), context)
                    .map_or(0, Scalar::int)
            }),
        };
        let speed = match &fuel.speed_multiplier {
            Resolvable::Constant(value) => *value,
            Resolvable::Reference(key) => self.providers.0.floats.get(key).map_or(1.0, |v| {
                safe_float(self.run(Work::Holder(Kind::Float, v), context))
            }),
        };
        (time, speed)
    }

    pub fn int<'v>(
        &mut self,
        v: &'v Value,
        context: &mut dyn ConditionContext<'v>,
    ) -> Result<i32, Arith>
    where
        'a: 'v,
    {
        self.run(Work::Number(Kind::Int, Some(Cow::Borrowed(v))), context)
            .map(Scalar::int)
    }

    pub fn float<'v>(
        &mut self,
        v: &'v Value,
        context: &mut dyn ConditionContext<'v>,
    ) -> Result<f32, Arith>
    where
        'a: 'v,
    {
        self.run(Work::Number(Kind::Float, Some(Cow::Borrowed(v))), context)
            .map(Scalar::float)
    }

    fn run<'v>(&mut self, start: Work<'v>, context: &mut dyn ConditionContext<'v>) -> ResultValue
    where
        'a: 'v,
    {
        let mut work = WorkStack::new();
        let mut holders = Holders::Inline(SmallVec::new());
        let mut predicates = HashSet::new();
        let mut last = Ok(Scalar::Bool(false));
        work.push(start);
        while let Some(task) = work.pop() {
            match task {
                Work::Number(kind, Some(node)) => {
                    if let Some(result) = self.number(kind, node, &mut work) {
                        last = result;
                    }
                }
                Work::Number(kind, None) => last = Ok(kind.zero()),
                Work::Holder(kind, node) => {
                    let id = std::ptr::from_ref(node);
                    if holders.enter(id) {
                        work.push(Work::Leave(id));
                        work.push(Work::Number(kind, Some(Cow::Borrowed(node))));
                    } else {
                        last = Err("recursive number provider reference".into());
                    }
                }
                Work::Leave(id) => holders.leave(id),
                Work::LeavePredicate(id) => {
                    predicates.remove(&id);
                }
                Work::Apply(op, mut remaining, mut values) => {
                    let Ok(value) = last else { continue };
                    values.push(value);
                    // Vanilla's float Modulus evaluates the divisor first and
                    // does not read the dividend when it is zero.
                    if matches!(op, Op::FloatMod) && values.len() == 1 && value.float() == 0.0 {
                        last = Ok(Scalar::Float(f32::NAN));
                    } else if let Some((kind, arg)) = remaining.next() {
                        work.push(Work::Apply(op, remaining, values));
                        work.push(Work::Number(kind, arg));
                    } else {
                        last = op.apply(&values, self.random);
                    }
                }
                Work::Branch(kind, node) => {
                    let branch = if last.as_ref().is_ok_and(|v| v.truth()) {
                        "on_true"
                    } else {
                        "on_false"
                    };
                    work.push(Work::Number(kind, child(&node, |v| v.get(branch))));
                }
                Work::Dispatch(kind, node, index) => {
                    if index > 0 && last.as_ref().is_ok_and(|v| v.truth()) {
                        work.push(Work::Number(
                            kind,
                            child(&node, |v| v["cases"][index - 1].get("value")),
                        ));
                    } else if node["cases"]
                        .as_array()
                        .is_some_and(|cases| index < cases.len())
                    {
                        let condition = child(&node, |v| v["cases"][index].get("condition"));
                        work.push(Work::Dispatch(kind, node, index + 1));
                        work.push(Work::Condition(condition, true));
                    } else {
                        work.push(Work::Number(kind, child(&node, |v| v.get("default"))));
                    }
                }
                Work::Condition(node, validate) => {
                    if let Some(value) =
                        Self::condition(node, validate, context, &mut predicates, &mut work)
                    {
                        last = Ok(Scalar::Bool(value));
                    }
                }
                Work::Logic(node, index, all, array) => {
                    if index == 0 || last.as_ref().is_ok_and(|v| v.truth()) == all {
                        let term = child(&node, |v| {
                            let terms = if array { v } else { &v["terms"] };
                            match terms {
                                Value::Array(terms) => terms.get(index),
                                Value::Null => None,
                                Value::String(tag) if tag.starts_with('#') => None,
                                one if index == 0 && context.singleton_terms() => Some(one),
                                _ => None,
                            }
                        });
                        if let Some(term) = term {
                            work.push(Work::Logic(node, index + 1, all, array));
                            work.push(Work::Condition(Some(term), false));
                        } else {
                            last = Ok(Scalar::Bool(all));
                        }
                    }
                }
                Work::Invert => last = Ok(Scalar::Bool(!last.as_ref().is_ok_and(|v| v.truth()))),
                Work::Chance => {
                    // `LootItemRandomChanceCondition.test`: the safe chance
                    // may itself consume randomness before the comparison draw.
                    let probability = safe_float(last);
                    last = Ok(Scalar::Bool(self.random.next_float() < probability));
                }
            }
        }
        last
    }

    fn number<'v>(
        &mut self,
        kind: Kind,
        node: Node<'v>,
        work: &mut WorkStack<'v>,
    ) -> Option<ResultValue>
    where
        'a: 'v,
    {
        let value = match node.as_ref() {
            Value::Number(n) => match kind {
                Kind::Int => Scalar::Int(
                    n.as_i64()
                        .map_or_else(|| n.as_f64().unwrap_or(0.0) as i32, |i| i as i32),
                ),
                Kind::Float => Scalar::Float(n.as_f64().unwrap_or(0.0) as f32),
            },
            Value::Bool(b) if matches!(kind, Kind::Int) => Scalar::Int(i32::from(*b)),
            Value::String(id) => {
                let table = match kind {
                    Kind::Int => &self.providers.0.ints,
                    Kind::Float => &self.providers.0.floats,
                };
                if let Some(def) = lookup(table, id) {
                    work.push(Work::Holder(kind, def));
                    return None;
                }
                kind.zero()
            }
            Value::Object(o) => {
                let ty = o
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_start_matches("minecraft:");
                match ty {
                    "conditional" => {
                        let condition = child(&node, |v| v.get("condition"));
                        work.push(Work::Branch(kind, node));
                        work.push(Work::Condition(condition, true));
                        return None;
                    }
                    "number_dispatcher" => {
                        work.push(Work::Dispatch(kind, node, 0));
                        return None;
                    }
                    "weighted_list" => {
                        let index = self.weighted(o.get("distribution"));
                        let data =
                            index.and_then(|i| child(&node, |v| v["distribution"][i].get("data")));
                        work.push(Work::Number(kind, data));
                        return None;
                    }
                    _ => {}
                }
                if ty == "constant" {
                    work.push(Work::Number(kind, child(&node, |v| v.get("value"))));
                    return None;
                }
                if let Some((op, args)) = Op::parse(kind, ty, &node) {
                    let mut pending = args.into_iter();
                    if let Some((kind, arg)) = pending.next() {
                        work.push(Work::Apply(op, pending, SmallVec::new()));
                        work.push(Work::Number(kind, arg));
                        return None;
                    }
                    return Some(op.apply(&[], self.random));
                }
                kind.zero()
            }
            _ => kind.zero(),
        };
        Some(Ok(value))
    }

    fn condition<'v>(
        node: Option<Node<'v>>,
        validate: bool,
        context: &mut dyn ConditionContext<'v>,
        predicates: &mut HashSet<String>,
        work: &mut WorkStack<'v>,
    ) -> Option<bool> {
        let Some(node) = node else { return Some(false) };
        if validate && !context.valid(&node) {
            return Some(false);
        }
        match node.as_ref() {
            Value::String(raw) => {
                let id = if let Some(path) = raw.strip_prefix(':') {
                    format!("minecraft:{path}")
                } else if raw.contains(':') {
                    raw.clone()
                } else {
                    format!("minecraft:{raw}")
                };
                let Some(named) = context.named(raw) else {
                    return Some(false);
                };
                if !predicates.insert(id.clone()) {
                    return Some(false);
                }
                work.push(Work::LeavePredicate(id));
                work.push(Work::Condition(Some(named), false));
            }
            Value::Array(_) => work.push(Work::Logic(node, 0, true, true)),
            Value::Object(_) => match context
                .kind(&node)
                .unwrap_or("")
                .trim_start_matches("minecraft:")
            {
                "inverted" => {
                    work.push(Work::Invert);
                    work.push(Work::Condition(child(&node, |v| Some(&v["term"])), false));
                }
                "all_of" => work.push(Work::Logic(node, 0, true, false)),
                "any_of" => work.push(Work::Logic(node, 0, false, false)),
                "random_chance" => {
                    let Some(chance) = child(&node, |v| context.chance(v)) else {
                        return Some(false);
                    };
                    work.push(Work::Chance);
                    work.push(Work::Number(Kind::Float, Some(chance)));
                }
                _ => return Some(context.leaf(&node)),
            },
            _ => return Some(context.leaf(&node)),
        }
        None
    }

    fn weighted(&mut self, distribution: Option<&Value>) -> Option<usize> {
        let list = distribution?.as_array()?;
        let weight = |entry: &Value| entry.get("weight").and_then(Value::as_i64).unwrap_or(1);
        // WeightedRandom.getTotalWeight sums into a long and requires an int total.
        let total = list.iter().try_fold(0_i32, |total, entry| {
            let weight = i32::try_from(weight(entry)).ok()?;
            (weight >= 0).then_some(())?;
            total.checked_add(weight)
        })?;
        let mut pick = i64::from(self.random.next_int_bound(total).ok()?);
        list.iter().position(|entry| {
            pick -= weight(entry);
            pick < 0
        })
    }
}

fn safe_float(value: ResultValue) -> f32 {
    value
        .ok()
        .map(Scalar::float)
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
}

#[derive(Clone, Copy)]
enum Op {
    IntAdd,
    IntMul,
    IntAvg,
    IntMax,
    IntMin,
    IntSub,
    IntDiv,
    IntMod,
    FloorDiv,
    FloorMod,
    IntPow,
    IntAbs,
    IntNegate,
    FromFloat,
    IntUniform,
    Binomial,
    FloatAdd,
    FloatMul,
    FloatAvg,
    Length,
    FloatMax,
    FloatMin,
    FloatSub,
    FloatDiv,
    FloatMod,
    FloatPow,
    FloatAbs,
    FloatNegate,
    Floor,
    Ceil,
    Round,
    Truncate,
    Sqrt,
    Sin,
    Cos,
    FromInt,
    FloatUniform,
}
impl Op {
    fn parse<'v>(kind: Kind, ty: &str, node: &Node<'v>) -> Option<(Self, Args<'v>)> {
        use Kind::{Float, Int};
        let (op, fields): (_, &[(Kind, &str)]) = match (kind, ty) {
            (Int, "add") => (Self::IntAdd, &[]),
            (Int, "mul") => (Self::IntMul, &[]),
            (Int, "avg") => (Self::IntAvg, &[]),
            (Int, "max") => (Self::IntMax, &[]),
            (Int, "min") => (Self::IntMin, &[]),
            (Float, "add") => (Self::FloatAdd, &[]),
            (Float, "mul") => (Self::FloatMul, &[]),
            (Float, "avg") => (Self::FloatAvg, &[]),
            (Float, "length") => (Self::Length, &[]),
            (Float, "max") => (Self::FloatMax, &[]),
            (Float, "min") => (Self::FloatMin, &[]),
            (Int, "sub") => (Self::IntSub, &[(Int, "left"), (Int, "right")]),
            (Int, "div") => (Self::IntDiv, &[(Int, "left"), (Int, "right")]),
            (Int, "mod") => (Self::IntMod, &[(Int, "left"), (Int, "right")]),
            (Int, "floor_div") => (Self::FloorDiv, &[(Int, "left"), (Int, "right")]),
            (Int, "floor_mod") => (Self::FloorMod, &[(Int, "left"), (Int, "right")]),
            (Float, "sub") => (Self::FloatSub, &[(Float, "left"), (Float, "right")]),
            (Float, "div") => (Self::FloatDiv, &[(Float, "left"), (Float, "right")]),
            (Float, "mod") => (Self::FloatMod, &[(Float, "right"), (Float, "left")]),
            (Int, "pow") => (Self::IntPow, &[(Int, "base"), (Int, "exponent")]),
            (Float, "pow") => (Self::FloatPow, &[(Float, "base"), (Float, "exponent")]),
            (Int, "uniform") => (Self::IntUniform, &[(Int, "min"), (Int, "max")]),
            (Float, "uniform") => (Self::FloatUniform, &[(Float, "min"), (Float, "max")]),
            (Int, "binomial") => (Self::Binomial, &[(Int, "n"), (Float, "p")]),
            (Int, "from_float") => (Self::FromFloat, &[(Float, "input")]),
            (Float, "from_int") => (Self::FromInt, &[(Int, "input")]),
            (Int, "abs") => (Self::IntAbs, &[(Int, "input")]),
            (Int, "negate") => (Self::IntNegate, &[(Int, "input")]),
            (Float, "abs") => (Self::FloatAbs, &[(Float, "input")]),
            (Float, "negate") => (Self::FloatNegate, &[(Float, "input")]),
            (Float, "floor") => (Self::Floor, &[(Float, "input")]),
            (Float, "ceil") => (Self::Ceil, &[(Float, "input")]),
            (Float, "round") => (Self::Round, &[(Float, "input")]),
            (Float, "truncate") => (Self::Truncate, &[(Float, "input")]),
            (Float, "sqrt") => (Self::Sqrt, &[(Float, "input")]),
            (Float, "sin") => (Self::Sin, &[(Float, "input")]),
            (Float, "cos") => (Self::Cos, &[(Float, "input")]),
            _ => return None,
        };
        let args = if fields.is_empty() {
            match node.get("inputs") {
                Some(Value::Array(list)) => (0..list.len())
                    .map(|i| (kind, child(node, |v| v["inputs"].get(i))))
                    .collect(),
                Some(_) => smallvec::smallvec![(kind, child(node, |v| v.get("inputs")))],
                None => SmallVec::new(),
            }
        } else {
            fields
                .iter()
                .map(|&(ty, key)| (ty, child(node, |v| v.get(key))))
                .collect()
        };
        Some((op, args))
    }

    fn apply(self, values: &[Scalar], random: &mut LevelRandom) -> ResultValue {
        let int = |i: usize| values[i].int();
        let float = |i: usize| values[i].float();
        let ints = || values.iter().copied().map(Scalar::int);
        let floats = || values.iter().copied().map(Scalar::float);
        let overflow = || "integer overflow".to_owned();
        let long_to_int = |v: i64| {
            i32::try_from(v).map_err(|_| format!("Value {v} can't be safely converted to int"))
        };
        Ok(Scalar::Int(match self {
            Self::IntAdd => long_to_int(ints().map(i64::from).sum())?,
            Self::IntMul => long_to_int(ints().fold(1i64, |a, b| a.wrapping_mul(i64::from(b))))?,
            Self::IntAvg => {
                if values.is_empty() {
                    return Err("/ by zero".into());
                }
                long_to_int(ints().map(i64::from).sum::<i64>() / values.len() as i64)?
            }
            Self::IntMax => ints().fold(i32::MIN, i32::max),
            Self::IntMin => ints().fold(i32::MAX, i32::min),
            Self::IntSub => int(0).checked_sub(int(1)).ok_or_else(overflow)?,
            Self::IntDiv | Self::IntMod | Self::FloorDiv | Self::FloorMod => {
                let (a, b) = (int(0), int(1));
                if b == 0 {
                    return Err("/ by zero".into());
                }
                match self {
                    Self::IntDiv => a.wrapping_div(b),
                    Self::IntMod => a.wrapping_rem(b),
                    Self::FloorMod => {
                        let remainder = a.wrapping_rem(b);
                        if remainder != 0 && (remainder < 0) != (b < 0) {
                            remainder + b
                        } else {
                            remainder
                        }
                    }
                    _ => {
                        if a == i32::MIN && b == -1 {
                            return Err(overflow());
                        }
                        a.div_euclid(b) - if b < 0 && a.rem_euclid(b) != 0 { 1 } else { 0 }
                    }
                }
            }
            Self::IntPow => {
                let (base, exp) = (int(0), int(1));
                if base == 0 && exp == 0 {
                    return Err("Result of 0 to the power of 0 is undefined".into());
                }
                if exp < 0 {
                    return Err("Negative exponent".into());
                }
                base.checked_pow(exp as u32).ok_or_else(overflow)?
            }
            // Math.absExact's message.
            Self::IntAbs => int(0).checked_abs().ok_or_else(|| {
                "Overflow to represent absolute value of Integer.MIN_VALUE".to_owned()
            })?,
            Self::IntNegate => int(0).checked_neg().ok_or_else(overflow)?,
            Self::FromFloat => {
                let f = float(0);
                if !f.is_finite() {
                    return Err(format!(
                        "Value {} can't be safely converted to int",
                        float_to_string(f)
                    ));
                }
                long_to_int(f as i64)?
            }
            Self::IntUniform => {
                let (lo, hi) = (int(0), int(1));
                if lo >= hi {
                    lo
                } else {
                    // Mth.nextInt computes the inclusive bound as a Java int.
                    let bound = hi.wrapping_sub(lo).wrapping_add(1);
                    random
                        .next_int_bound(bound)
                        .map_err(|err| err.to_string())?
                        + lo
                }
            }
            Self::Binomial => (0..int(0).max(0))
                .filter(|_| random.next_float() < float(1))
                .count() as i32,
            _ => {
                return Ok(Scalar::Float(match self {
                    Self::FloatAdd => floats().fold(0.0, |a, b| a + b),
                    Self::FloatMul => floats().fold(1.0, |a, b| a * b),
                    Self::FloatAvg => floats().fold(0.0, |a, b| a + b) / values.len() as f32,
                    Self::Length => floats().fold(0.0, |a, b| a + b * b).sqrt(),
                    Self::FloatMax => floats().fold(-f32::MAX, f32::max),
                    Self::FloatMin => floats().fold(f32::MAX, f32::min),
                    Self::FloatSub => float(0) - float(1),
                    Self::FloatDiv => float(0) / float(1),
                    Self::FloatMod => float(1) % float(0),
                    Self::FloatPow => {
                        let (base, exp) = (float(0), float(1));
                        if base == 0.0 && exp == 0.0 {
                            f32::NAN
                        } else {
                            sunrise_core::jmath::pow(f64::from(base), f64::from(exp)) as f32
                        }
                    }
                    Self::FloatAbs => float(0).abs(),
                    Self::FloatNegate => -float(0),
                    Self::Floor => float(0).floor(),
                    Self::Ceil => float(0).ceil() as i32 as f32, // Mth.ceil returns an int.
                    Self::Round => sunrise_core::jmath::round_f32(float(0)) as f32,
                    Self::Truncate => float(0).trunc(),
                    Self::Sqrt => float(0).sqrt(),
                    Self::Sin => sunrise_core::geometry::trig::sin(f64::from(float(0))),
                    Self::Cos => sunrise_core::geometry::trig::cos(f64::from(float(0))),
                    Self::FromInt => int(0) as f32,
                    Self::FloatUniform => {
                        let (lo, hi) = (float(0), float(1));
                        if lo >= hi {
                            lo
                        } else {
                            random.next_float() * (hi - lo) + lo
                        }
                    }
                    _ => unreachable!("float operation"),
                }));
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Definitions;
    use serde_json::json;
    use std::sync::Arc;
    use sunrise_core::random::LegacyRandom;

    #[test]
    fn cooking_fuel_suspends_deep_arithmetic_and_chance_holders() {
        const DEPTH: i32 = 4096;
        let provider = |i| format!("test:depth_{i}");
        let mut defs = Definitions::default();
        defs.ints.insert(provider(DEPTH), json!(DEPTH * 2));
        defs.floats.insert(provider(DEPTH), json!(2.0));
        for i in 0..DEPTH {
            let next = provider(i + 1);
            defs.ints.insert(
                provider(i),
                json!({
                    "type": "minecraft:sub", "left": next, "right": 1,
                }),
            );
            defs.floats.insert(
                provider(i),
                json!({
                    "type": "minecraft:conditional", "on_true": 2.0, "on_false": -1.0,
                    "condition": {"condition": "minecraft:random_chance", "chance": {
                        "type": "minecraft:div", "left": next, "right": 2.0,
                    }},
                }),
            );
        }
        let providers = Providers(Arc::new(defs));
        let fuel = CookingFuel {
            burn_time: Resolvable::Reference(provider(0)),
            speed_multiplier: Resolvable::Reference(provider(0)),
        };
        // Each definition is shallow like a separate pack file. A fixed-size
        // native stack makes recursing across holders fail deterministically.
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut random = LevelRandom::for_level(LegacyRandom::new(31));
                let mut expected_random = random.clone();
                for _ in 0..DEPTH {
                    expected_random.next_float();
                }
                let (burn, speed) = Evaluation::new(&providers, &mut random)
                    .cooking_fuel(&fuel, &mut |_: &Value| false);
                assert_eq!(burn, DEPTH);
                assert_eq!(speed.to_bits(), 2.0_f32.to_bits());
                assert_eq!(random.next_long(), expected_random.next_long());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
