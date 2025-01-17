use std::{
    fmt::{self, Write},
    sync::LazyLock,
};

use derive_where::derive_where;
use flux_common::format::PadAdapter;
use itertools::Itertools;
use rustc_macros::{Decodable, Encodable};
use rustc_span::Symbol;

use crate::{big_int::BigInt, StringTypes, Types};

#[derive_where(Hash)]
pub enum Constraint<T: Types> {
    Pred(Pred<T>, #[derive_where(skip)] Option<T::Tag>),
    Conj(Vec<Self>),
    Guard(Pred<T>, Box<Self>),
    ForAll(T::Var, Sort, Pred<T>, Box<Self>),
}

#[derive(Clone, Hash)]
pub enum Sort {
    Int,
    Bool,
    Real,
    Unit,
    BitVec(usize),
    Pair(Box<Sort>, Box<Sort>),
    Func(PolyFuncSort),
    App(SortCtor, Vec<Sort>),
}

#[derive(Clone, Hash)]
pub enum SortCtor {
    Set,
    Map,
    // User { name: Symbol, arity: usize },
}

#[derive(Clone, Hash)]
pub struct FuncSort {
    inputs_and_output: Vec<Sort>,
}

#[derive(Clone, Hash)]
pub struct PolyFuncSort {
    params: usize,
    fsort: FuncSort,
}

#[derive_where(Hash)]
pub enum Pred<T: Types> {
    And(Vec<Self>),
    KVar(T::KVar, Vec<T::Var>),
    Expr(Expr<T>),
}

#[derive_where(Hash)]
pub enum Expr<T: Types> {
    Var(T::Var),
    Constant(Constant),
    BinaryOp(BinOp, Box<[Self; 2]>),
    App(Func<T>, Vec<Self>),
    UnaryOp(UnOp, Box<Self>),
    Pair(Box<[Self; 2]>),
    Proj(Box<Self>, Proj),
    IfThenElse(Box<[Self; 3]>),
    Unit,
}

#[derive_where(Hash)]
pub enum Func<T: Types> {
    Var(T::Var),
    /// interpreted (theory) function
    Itf(Symbol),
}

#[derive(Clone, Copy, Hash)]
pub enum Proj {
    Fst,
    Snd,
}

#[derive_where(Hash)]
pub struct Qualifier<T: Types> {
    pub name: String,
    pub args: Vec<(T::Var, Sort)>,
    pub body: Expr<T>,
    pub global: bool,
}

#[derive(Clone, Copy)]
pub struct Const<T: Types> {
    pub name: T::Var,
    pub val: i128,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Encodable, Decodable)]
pub enum BinOp {
    Iff,
    Imp,
    Or,
    And,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Encodable, Decodable)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encodable, Decodable)]
pub enum Constant {
    Int(BigInt),
    Real(i128),
    Bool(bool),
}

impl<T: Types> Constraint<T> {
    pub const TRUE: Self = Self::Pred(Pred::TRUE, None);

    /// Returns true if the constraint has at least one concrete RHS ("head") predicates.
    /// If `!c.is_concrete`  then `c` is trivially satisfiable and we can avoid calling fixpoint.
    pub fn is_concrete(&self) -> bool {
        match self {
            Constraint::Conj(cs) => cs.iter().any(Constraint::is_concrete),
            Constraint::Guard(_, c) | Constraint::ForAll(_, _, _, c) => c.is_concrete(),
            Constraint::Pred(p, _) => p.is_concrete() && !p.is_trivially_true(),
        }
    }
}

impl<T: Types> Pred<T> {
    pub const TRUE: Self = Pred::Expr(Expr::Constant(Constant::Bool(true)));

    pub fn is_trivially_true(&self) -> bool {
        match self {
            Pred::Expr(Expr::Constant(Constant::Bool(true))) => true,
            Pred::And(ps) => ps.is_empty(),
            _ => false,
        }
    }

    pub fn is_concrete(&self) -> bool {
        match self {
            Pred::And(ps) => ps.iter().any(Pred::is_concrete),
            Pred::KVar(_, _) => false,
            Pred::Expr(_) => true,
        }
    }
}

impl PolyFuncSort {
    pub fn new(
        params: usize,
        inputs: impl IntoIterator<Item = Sort>,
        output: Sort,
    ) -> PolyFuncSort {
        let mut inputs = inputs.into_iter().collect_vec();
        inputs.push(output);
        PolyFuncSort { params, fsort: FuncSort { inputs_and_output: inputs } }
    }
}

impl<T: Types> fmt::Display for Constraint<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constraint::Pred(pred, tag) => write!(f, "{}", PredTag(pred, tag)),
            Constraint::Conj(preds) => {
                match &preds[..] {
                    [] => write!(f, "((true))"),
                    [pred] => write!(f, "{pred}"),
                    preds => {
                        write!(f, "(and")?;
                        write!(PadAdapter::wrap_fmt(f, 2), "\n{}", preds.iter().join("\n"))?;
                        write!(f, "\n)")
                    }
                }
            }
            Constraint::Guard(body, head) => {
                write!(f, "(forall ((_ Unit) {body})")?;
                write!(PadAdapter::wrap_fmt(f, 2), "\n{head}")?;
                write!(f, "\n)")
            }
            Constraint::ForAll(x, sort, body, head) => {
                write!(f, "(forall (({x} {sort}) {body})")?;
                write!(PadAdapter::wrap_fmt(f, 2), "\n{head}")?;
                write!(f, "\n)")
            }
        }
    }
}

struct PredTag<'a, T: Types>(&'a Pred<T>, &'a Option<T::Tag>);

impl<T: Types> fmt::Display for PredTag<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let PredTag(pred, tag) = self;
        match pred {
            Pred::And(preds) => {
                match &preds[..] {
                    [] => write!(f, "((true))"),
                    [pred] => write!(f, "{}", PredTag(pred, tag)),
                    _ => {
                        write!(f, "(and")?;
                        let mut w = PadAdapter::wrap_fmt(f, 2);
                        for pred in preds {
                            write!(w, "\n{}", PredTag(pred, tag))?;
                        }
                        write!(f, "\n)")
                    }
                }
            }
            Pred::Expr(_) | Pred::KVar(..) => {
                if let Some(tag) = tag {
                    write!(f, "(tag {pred} \"{tag}\")")
                } else {
                    write!(f, "({pred})")
                }
            }
        }
    }
}

impl fmt::Display for SortCtor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SortCtor::Set => write!(f, "Set_Set"),
            SortCtor::Map => write!(f, "Map_t"),
        }
    }
}
impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Sort::Int => write!(f, "int"),
            Sort::Bool => write!(f, "bool"),
            Sort::Real => write!(f, "real"),
            Sort::Unit => write!(f, "Unit"),
            Sort::BitVec(size) => write!(f, "(BitVec Size{})", size),
            Sort::Pair(s1, s2) => write!(f, "(Pair {s1} {s2})"),
            Sort::Func(sort) => write!(f, "{sort}"),
            Sort::App(ctor, ts) => write!(f, "({ctor} {})", ts.iter().format(" ")),
        }
    }
}

impl fmt::Display for PolyFuncSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(func({}, [{}]))", self.params, self.fsort.inputs_and_output.iter().format("; "))
    }
}

impl fmt::Display for FuncSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(func(0, [{}]))", self.inputs_and_output.iter().format("; "))
    }
}

impl<T: Types> fmt::Display for Pred<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pred::And(preds) => {
                match &preds[..] {
                    [] => write!(f, "((true))"),
                    [pred] => write!(f, "{pred}"),
                    preds => write!(f, "(and {})", preds.iter().join(" ")),
                }
            }
            Pred::KVar(kvid, vars) => {
                write!(f, "(${kvid} {})", vars.iter().format(" "))
            }
            Pred::Expr(expr) => write!(f, "({expr})"),
        }
    }
}

impl<T: Types> Expr<T> {
    pub const ZERO: Expr<T> = Expr::Constant(Constant::ZERO);
    pub const ONE: Expr<T> = Expr::Constant(Constant::ONE);
    pub fn eq(self, other: Self) -> Self {
        Expr::BinaryOp(BinOp::Eq, Box::new([self, other]))
    }
}

struct FmtParens<'a, T: Types>(&'a Expr<T>);

impl<T: Types> fmt::Display for FmtParens<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Fixpoint parser has `=` at two different precedence levels depending on whether it is
        // used in a sequence of boolean expressions or not. To avoid complexity we parenthesize
        // all binary expressions no matter the parent operator.
        let should_parenthesize = matches!(&self.0, Expr::BinaryOp(..) | Expr::IfThenElse(..));
        if should_parenthesize {
            write!(f, "({})", self.0)
        } else {
            write!(f, "{}", self.0)
        }
    }
}

impl<T: Types> fmt::Display for Expr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Var(x) => write!(f, "{x}"),
            Expr::Constant(c) => write!(f, "{c}"),
            Expr::BinaryOp(op, box [e1, e2]) => {
                write!(f, "{} {op} {}", FmtParens(e1), FmtParens(e2))?;
                Ok(())
            }
            Expr::UnaryOp(op, e) => {
                if matches!(e.as_ref(), Expr::Constant(_) | Expr::Var(_)) {
                    write!(f, "{op}{e}")
                } else {
                    write!(f, "{op}({e})")
                }
            }
            Expr::Pair(box [e1, e2]) => write!(f, "(Pair ({e1}) ({e2}))"),
            Expr::Proj(e, Proj::Fst) => write!(f, "(fst {e})"),
            Expr::Proj(e, Proj::Snd) => write!(f, "(snd {e})"),
            Expr::Unit => write!(f, "Unit"),
            Expr::App(func, args) => {
                write!(f, "({func} {})", args.iter().map(FmtParens).format(" "),)
            }
            Expr::IfThenElse(box [p, e1, e2]) => {
                write!(f, "if {p} then {e1} else {e2}")
            }
        }
    }
}

impl<T: Types> fmt::Display for Func<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Func::Var(name) => write!(f, "{name}"),
            Func::Itf(itf) => write!(f, "{itf}"),
        }
    }
}

pub(crate) static DEFAULT_QUALIFIERS: LazyLock<Vec<Qualifier<StringTypes>>> = LazyLock::new(|| {
    // -----
    // UNARY
    // -----

    // (qualif EqZero ((v int)) (v == 0))
    let eqzero = Qualifier {
        args: vec![("v", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Eq, Box::new([Expr::Var("v"), Expr::ZERO])),
        name: String::from("EqZero"),
        global: true,
    };

    // (qualif GtZero ((v int)) (v > 0))
    let gtzero = Qualifier {
        args: vec![("v", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Gt, Box::new([Expr::Var("v"), Expr::ZERO])),
        name: String::from("GtZero"),
        global: true,
    };

    // (qualif GeZero ((v int)) (v >= 0))
    let gezero = Qualifier {
        args: vec![("v", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Ge, Box::new([Expr::Var("v"), Expr::ZERO])),
        name: String::from("GeZero"),
        global: true,
    };

    // (qualif LtZero ((v int)) (v < 0))
    let ltzero = Qualifier {
        args: vec![("v", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Lt, Box::new([Expr::Var("v"), Expr::ZERO])),
        name: String::from("LtZero"),
        global: true,
    };

    // (qualif LeZero ((v int)) (v <= 0))
    let lezero = Qualifier {
        args: vec![("v", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Le, Box::new([Expr::Var("v"), Expr::ZERO])),
        name: String::from("LeZero"),
        global: true,
    };

    // ------
    // BINARY
    // ------

    // (qualif Eq ((a int) (b int)) (a == b))
    let eq = Qualifier {
        args: vec![("a", Sort::Int), ("b", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Eq, Box::new([Expr::Var("a"), Expr::Var("b")])),
        name: String::from("Eq"),
        global: true,
    };

    // (qualif Gt ((a int) (b int)) (a > b))
    let gt = Qualifier {
        args: vec![("a", Sort::Int), ("b", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Gt, Box::new([Expr::Var("a"), Expr::Var("b")])),
        name: String::from("Gt"),
        global: true,
    };

    // (qualif Lt ((a int) (b int)) (a < b))
    let ge = Qualifier {
        args: vec![("a", Sort::Int), ("b", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Ge, Box::new([Expr::Var("a"), Expr::Var("b")])),
        name: String::from("Ge"),
        global: true,
    };

    // (qualif Ge ((a int) (b int)) (a >= b))
    let lt = Qualifier {
        args: vec![("a", Sort::Int), ("b", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Lt, Box::new([Expr::Var("a"), Expr::Var("b")])),
        name: String::from("Lt"),
        global: true,
    };

    // (qualif Le ((a int) (b int)) (a <= b))
    let le = Qualifier {
        args: vec![("a", Sort::Int), ("b", Sort::Int)],
        body: Expr::BinaryOp(BinOp::Le, Box::new([Expr::Var("a"), Expr::Var("b")])),
        name: String::from("Le"),
        global: true,
    };

    // (qualif Le1 ((a int) (b int)) (a < b - 1))
    let le1 = Qualifier {
        args: vec![("a", Sort::Int), ("b", Sort::Int)],
        body: Expr::BinaryOp(
            BinOp::Le,
            Box::new([
                Expr::Var("a"),
                Expr::BinaryOp(BinOp::Sub, Box::new([Expr::Var("b"), Expr::ONE])),
            ]),
        ),
        name: String::from("Le1"),
        global: true,
    };

    vec![eqzero, gtzero, gezero, ltzero, lezero, eq, gt, ge, lt, le, le1]
});

impl<T: Types> fmt::Display for Qualifier<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(qualif {} ({}) ({}))",
            self.name,
            self.args
                .iter()
                .format_with(" ", |(name, sort), f| f(&format_args!("({name} {sort})"))),
            self.body
        )
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Iff => write!(f, "<=>"),
            BinOp::Imp => write!(f, "=>"),
            BinOp::Or => write!(f, "||"),
            BinOp::And => write!(f, "&&"),
            BinOp::Eq => write!(f, "="),
            BinOp::Ne => write!(f, "/="),
            BinOp::Gt => write!(f, ">"),
            BinOp::Ge => write!(f, ">="),
            BinOp::Lt => write!(f, "<"),
            BinOp::Le => write!(f, "<="),
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Mod => write!(f, "mod"),
        }
    }
}

impl fmt::Debug for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnOp::Not => write!(f, "~"),
            UnOp::Neg => write!(f, "-"),
        }
    }
}

impl fmt::Debug for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constant::Int(n) => write!(f, "{n}"),
            Constant::Real(r) => write!(f, "{r}.0"),
            Constant::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl Constant {
    pub const ZERO: Constant = Constant::Int(BigInt::ZERO);
    pub const ONE: Constant = Constant::Int(BigInt::ONE);

    fn to_bool(self) -> Option<bool> {
        match self {
            Constant::Bool(b) => Some(b),
            _ => None,
        }
    }

    fn to_int(self) -> Option<BigInt> {
        match self {
            Constant::Int(n) => Some(n),
            _ => None,
        }
    }

    pub fn iff(&self, other: &Constant) -> Option<Constant> {
        let b1 = self.to_bool()?;
        let b2 = other.to_bool()?;
        Some(Constant::Bool(b1 == b2))
    }

    pub fn imp(&self, other: &Constant) -> Option<Constant> {
        let b1 = self.to_bool()?;
        let b2 = other.to_bool()?;
        Some(Constant::Bool(!b1 || b2))
    }

    pub fn or(&self, other: &Constant) -> Option<Constant> {
        let b1 = self.to_bool()?;
        let b2 = other.to_bool()?;
        Some(Constant::Bool(b1 || b2))
    }

    pub fn and(&self, other: &Constant) -> Option<Constant> {
        let b1 = self.to_bool()?;
        let b2 = other.to_bool()?;
        Some(Constant::Bool(b1 && b2))
    }

    pub fn eq(&self, other: &Constant) -> Constant {
        Constant::Bool(*self == *other)
    }

    pub fn ne(&self, other: &Constant) -> Constant {
        Constant::Bool(*self != *other)
    }

    pub fn gt(&self, other: &Constant) -> Option<Constant> {
        let n1 = self.to_int()?;
        let n2 = other.to_int()?;
        Some(Constant::Bool(n1 > n2))
    }

    pub fn ge(&self, other: &Constant) -> Option<Constant> {
        let n1 = self.to_int()?;
        let n2 = other.to_int()?;
        Some(Constant::Bool(n1 >= n2))
    }

    /// See [`BigInt::int_min`]
    pub fn int_min(bit_width: u32) -> Constant {
        Constant::Int(BigInt::int_min(bit_width))
    }

    /// See [`BigInt::int_max`]
    pub fn int_max(bit_width: u32) -> Constant {
        Constant::Int(BigInt::int_max(bit_width))
    }

    /// See [`BigInt::uint_max`]
    pub fn uint_max(bit_width: u32) -> Constant {
        Constant::Int(BigInt::uint_max(bit_width))
    }
}

impl From<i32> for Constant {
    fn from(c: i32) -> Self {
        Constant::Int(c.into())
    }
}

impl From<usize> for Constant {
    fn from(u: usize) -> Self {
        Constant::Int(u.into())
    }
}

impl From<u128> for Constant {
    fn from(c: u128) -> Self {
        Constant::Int(c.into())
    }
}

impl From<i128> for Constant {
    fn from(c: i128) -> Self {
        Constant::Int(c.into())
    }
}

impl From<bool> for Constant {
    fn from(b: bool) -> Self {
        Constant::Bool(b)
    }
}
