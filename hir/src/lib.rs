//! High-level intermediate representation. This IR is used for type analysis, validating code
//! correctness, and desugaring.

#![feature(let_chains)]

pub mod error;
pub mod lower;

use grammar::ast::StructDef;
use internment::Intern;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ident(Intern<String>);

impl Display for Ident {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The ID of a module.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleId(Intern<Vec<String>>);

impl Display for  ModuleId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.iter().map(AsRef::as_ref).collect::<Vec<_>>().join("."))
    }
}

/// The ID of a top-level item.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(
    /// The module in which the item is defined.
    ModuleId,
    /// The name of the item, which is unique within the module.
    Ident,
);

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ScopeId(usize);

impl ScopeId {
    pub const fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

/// HIR of a Terbium program.
#[derive(Default)]
pub struct Hir {
    /// A mapping of all modules within the program.
    pub modules: HashMap<ModuleId, Vec<Node>>,
    /// A mapping of all top-level functions in the program.
    pub funcs: HashMap<ItemId, Func>,
    /// A mapping of all constants in the program.
    pub consts: HashMap<ItemId, Const>,
    /// A mapping of all raw structs within the program.
    pub structs: HashMap<ItemId, StructDef>,
    /// A mapping of all types within the program.
    pub types: HashMap<ItemId, Ty>,
    /// A mapping of all lexical scopes within the program.
    pub scopes: HashMap<ScopeId, Scope>,
    /// The root scope of the program.
    pub root: ScopeId,
}

/// An HIR node.
#[derive(Clone, Debug)]
pub enum Node {
    Expr(Expr),
    Let {
        pat: Pattern,
        ty: Ty,
        value: Option<Expr>,
    },
    Const(Const),
    Func(Func),
    Break(Option<Ident>, Option<Expr>),
    Continue(Option<Ident>),
    Return(Option<Expr>),
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub label: Option<Ident>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct Const {
    pub name: Ident,
    pub ty: Ty,
    pub value: Expr,
}

/// A pattern that can be matched against.
#[derive(Clone, Debug)]
pub enum Pattern {
    Ident { ident: Ident, is_mut: bool },
    Tuple(Vec<Self>),
}

/// Visibility of a top-level item.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemVisibility {
    /// The item is visible to all other items in the program.
    Public,
    /// The item is visible to all other items in the library.
    Lib,
    /// The item is visible to all items in the parent module and its submodules.
    Super,
    /// The item is only visible to the current module. This is the default visibility.
    Private,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MemberVisibility {
    Public,
    Lib,
    Super,
    Mod,
    Sub,
    Private,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldVisibility {
    pub get: MemberVisibility,
    pub set: MemberVisibility,
}

#[derive(Clone, Debug)]
pub struct FuncParam {
    pub pat: Pattern,
    pub ty: Ty,
    pub default: Option<Expr>,
}

/// HIR of a top-level function.
#[derive(Clone, Debug)]
pub struct Func {
    /// The visibility of the item.
    pub visibility: ItemVisibility,
    /// The name of the function.
    pub name: Ident,
    /// The parameters of the function.
    pub params: Vec<FuncParam>,
    /// The return type of the function.
    pub ret_ty: Ty,
    /// The body of the function.
    pub body: ScopeId,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum IntWidth {
    Int8 = 8,
    Int16 = 16,
    #[default]
    Int32 = 32,
    Int64 = 64,
    Int128 = 128,
    Unknown = !0,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum IntSign {
    Signed,
    Unsigned,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum FloatWidth {
    Float32 = 32,
    #[default]
    Float64 = 64,
    Unknown = !0,
    // Float128 = 128,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PrimitiveTy {
    // integer "signedness" and bit-width are unified as one type since they coerce to each other
    Int(IntSign, IntWidth),
    Float(FloatWidth),
    Bool,
    Char,
    Void,
}

#[derive(Clone, Debug)]
pub enum Ty {
    Unknown,
    Primitive(PrimitiveTy),
    Generic(TyParam),
    Tuple(Vec<Ty>),
    Struct(ItemId, Vec<Ty>),
}

#[derive(Clone, Debug)]
pub struct TyParam {
    pub name: Ident,
    pub bound: Option<Box<Ty>>,
}

#[derive(Clone, Debug)]
pub struct StructField {
    pub vis: FieldVisibility,
    pub name: Ident,
    pub ty: Ty,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct StructTy {
    pub vis: ItemVisibility,
    pub name: Ident,
    pub ty_params: Vec<TyParam>,
    pub fields: Vec<StructField>,
}

#[derive(Clone, Debug)]
pub enum Literal {
    UInt(u128),
    Int(i128),
    Float(f64),
    Bool(bool),
    Char(char),
    String(String),
    Void,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Intrinsic {
    IntNeg,
    IntAdd,
    IntSub,
    IntMul,
    IntDiv,
    IntPow,
    IntMod,
    IntBitOr,
    IntBitAnd,
    IntBitNot,
    IntBitXor,
    IntShl,
    IntShr,
    IntEq,
    IntLt,
    IntLe,
    IntGt,
    IntGe,
    FloatPos,
    FloatNeg,
    FloatAdd,
    FloatSub,
    FloatMul,
    FloatDiv,
    FloatPow,
    FloatMod,
    FloatEq,
    FloatLt,
    FloatLe,
    FloatGt,
    FloatGe,
    BoolEq,
    BoolNot,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Op {
    Pos,
    Neg,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Mul,
    MulAssign,
    Div,
    DivAssign,
    Mod,
    ModAssign,
    Pow,
    PowAssign,
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
    Not,
    BitOr,
    BitOrAssign,
    BitAnd,
    BitAndAssign,
    BitXor,
    BitXorAssign,
    BitNot,
    Shl,
    ShlAssign,
    Shr,
    ShrAssign,
    Index,
    IndexMut,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StaticOp {
    New,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Literal(Literal),
    Ident(Ident),
    Tuple(Vec<Self>),
    Intrinsic(Intrinsic, Vec<Self>),
    Call {
        callee: Box<Self>,
        args: Vec<Self>,
        kwargs: Vec<(Ident, Self)>,
    },
    CallOp(Op, Box<Self>, Vec<Self>),
    CallStaticOp(StaticOp, Ty, Vec<Self>),
    Cast(Box<Self>, Ty),
    GetAttr(Box<Self>, Ident),
    SetAttr(Box<Self>, Ident, Box<Self>),
    Block(ScopeId),
    If(Box<Self>, ScopeId, ScopeId),
    While(Box<Self>, ScopeId, ScopeId),
    Loop(ScopeId),
    Assign(Pattern, Box<Self>),
}
