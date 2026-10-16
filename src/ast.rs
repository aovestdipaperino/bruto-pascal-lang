/// AST types for Mini-Pascal.
///
/// Every node carries a `Span` for error reporting and DWARF debug info generation.
/// Source location (1-based line and column), plus the file it is in:
/// 0 for the program being compiled, otherwise an id from
/// [`crate::source_map`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub column: u32,
    pub file: u32,
}

impl Span {
    pub fn new(line: u32, column: u32) -> Self {
        Self {
            line,
            column,
            file: crate::source_map::MAIN_FILE,
        }
    }

    pub fn in_file(line: u32, column: u32, file: u32) -> Self {
        Self { line, column, file }
    }

    /// `line L:C`, plus the file name when it isn't the main program.
    pub fn describe(&self) -> String {
        crate::source_map::describe(self.line, self.column, self.file)
    }
}

/// Top-level program: `program Name; [uses ...;] [const ...] [type ...] [var ...] {proc|func} begin ... end.`
#[derive(Debug, Clone)]
pub struct Program {
    pub name: String,
    pub uses: Vec<String>,
    /// Every unit loaded for this program (lower case), direct or not,
    /// filled in when `uses` is resolved. `Unit.Name` qualifiers naming
    /// these are dropped by the semantic pass.
    pub units: Vec<String>,
    /// Overloaded routine names: the base name -> one symbol per distinct
    /// signature (`name`, `name$2`, ...). Filled by the semantic pass.
    pub overloads: std::collections::HashMap<String, Vec<String>>,
    pub labels: Vec<i64>,
    pub consts: Vec<ConstDecl>,
    pub type_decls: Vec<TypeDecl>,
    pub vars: Vec<VarDecl>,
    pub procedures: Vec<ProcDecl>,
    pub body: Block,
    pub span: Span,
    /// Whether the program (or a unit it uses) raises or handles
    /// exceptions, so it needs the exception runtime in `rtl/objpas.pp`.
    pub uses_exceptions: bool,
    /// The unit declaring each type that came from one (lower-case type
    /// name -> the unit's name as written; `System` for the built-in
    /// ones). For the names FPC gives specializations.
    pub type_units: std::collections::HashMap<String, String>,
}

/// A unit module: `unit Name; interface ... implementation ... end.`
///
/// Interface decls are visible to anything that `uses` this unit;
/// implementation decls are private to the unit's own bodies. We
/// flatten both during compilation but keep the split for parsing.
#[derive(Debug, Clone)]
pub struct Unit {
    pub name: String,
    /// Every unit used, in the interface and the implementation.
    pub uses: Vec<String>,
    /// The units the interface uses, which must not lead back here; the
    /// implementation's may.
    pub interface_uses: Vec<String>,
    pub interface_consts: Vec<ConstDecl>,
    pub interface_types: Vec<TypeDecl>,
    pub interface_vars: Vec<VarDecl>,
    pub interface_proc_headers: Vec<ProcDecl>,
    pub impl_consts: Vec<ConstDecl>,
    pub impl_types: Vec<TypeDecl>,
    pub impl_vars: Vec<VarDecl>,
    pub procedures: Vec<ProcDecl>,
    pub init: Option<Block>,
    pub span: Span,
    /// Whether the unit raises or handles exceptions.
    pub uses_exceptions: bool,
}

/// Type alias declaration: `type Name = T;`
#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: String,
    pub ty: PascalType,
    pub span: Span,
    /// A generic type's parameters (`generic TList<T> = class`); empty
    /// for an ordinary type.
    pub type_params: Vec<String>,
}

/// Constant declaration: `name = value` or `name: type = value` (typed/mutable)
#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<PascalType>,
    pub value: Expr,
    pub span: Span,
}

/// Parameter passing mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamMode {
    Value,
    Var, // pass by reference
}

/// A single parameter group: `a, b: integer` or `var x: integer`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamGroup {
    pub mode: ParamMode,
    pub names: Vec<String>,
    pub ty: PascalType,
    /// `X: T = Value`: the value a call that leaves the parameter out
    /// passes.
    pub default: Option<DefaultValue>,
    /// Declared `const`: a counted value (an interface) isn't counted
    /// again for the call, as in FPC.
    pub is_const: bool,
}

/// A parameter's default value. Two declarations are the same whatever
/// their defaults (an implementation may repeat or leave them out).
#[derive(Debug, Clone)]
pub struct DefaultValue(pub Expr);

impl PartialEq for DefaultValue {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for DefaultValue {}

/// Procedure or function declaration
#[derive(Debug, Clone)]
pub struct ProcDecl {
    pub name: String,
    pub params: Vec<ParamGroup>,
    pub return_type: Option<PascalType>, // None = procedure, Some = function
    pub kind: RoutineKind,
    /// The object type a method implementation belongs to: `TFoo` in
    /// `procedure TFoo.Bar;`.
    pub owner: Option<String>,
    /// A `forward` declaration or a unit interface header: no body here.
    pub is_forward: bool,
    /// `external ['lib'] [name 'sym']`: implemented outside Pascal, by the
    /// C function `sym` (the routine's own name if not given).
    pub external: Option<String>,
    /// `varargs`: an external C function that takes further arguments
    /// after its declared parameters.
    pub varargs: bool,
    pub consts: Vec<ConstDecl>,
    pub type_decls: Vec<TypeDecl>,
    pub vars: Vec<VarDecl>,
    pub nested_procs: Vec<ProcDecl>,
    pub body: Block,
    pub span: Span,
    /// A generic routine's type parameters (`generic function Max<T>`).
    pub type_params: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineKind {
    Procedure,
    Function,
    Constructor,
    Destructor,
}

impl ProcDecl {
    /// A routine with just its header: no declarations and an empty body.
    pub fn header(
        name: String,
        kind: RoutineKind,
        params: Vec<ParamGroup>,
        return_type: Option<PascalType>,
        span: Span,
    ) -> Self {
        Self {
            name,
            params,
            return_type,
            kind,
            owner: None,
            is_forward: false,
            external: None,
            varargs: false,
            consts: Vec::new(),
            type_decls: Vec::new(),
            vars: Vec::new(),
            nested_procs: Vec::new(),
            body: Block {
                statements: Vec::new(),
                span,
                end_span: span,
            },
            span,
            type_params: Vec::new(),
        }
    }
}

/// Variable declaration: `name1, name2 : type`
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub names: Vec<String>,
    pub ty: PascalType,
    /// `absolute other`: the variables share `other`'s memory.
    pub absolute: Option<String>,
    pub span: Span,
}

/// Supported types in Mini-Pascal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PascalType {
    /// The 64-bit signed integer (`integer`, `int64`, `ptrint`, ...).
    Integer,
    /// A Turbo Pascal short string: `max + 1` bytes stored in place, the
    /// first holding the length. `string[N]`, `ShortString`, and `string`
    /// itself under `{$H-}` or `{$mode objfpc}`. In expressions it is
    /// read as an ordinary (C) string.
    ShortString {
        max: u32,
    },
    /// A fixed-width integer other than the 64-bit signed one: `byte`,
    /// `word`, `longint`, `qword` and so on. Stored at its real width;
    /// widened to 64 bits as soon as it is loaded into an expression.
    SizedInt {
        bits: u32,
        signed: bool,
    },
    Real,
    /// `Single`: a 32-bit float in memory, widened to `Real` as soon as
    /// it is loaded into an expression.
    Single,
    String,
    Boolean,
    Char,
    /// FPC's `WideChar`: one UTF-16 code unit. An `i16` in memory and
    /// in expressions (so it can be told apart from a `Char`, an `i8`,
    /// and from integers, which are widened to 64 bits).
    WideChar,
    /// FPC's `UnicodeString`: UTF-16 code units, NUL-terminated, behind
    /// one pointer (nil is the empty string). In expressions it is the
    /// struct `{ ptr }`, so conversions to and from `String` (UTF-8) can
    /// be chosen from the value alone.
    UnicodeString,
    Pointer(Box<PascalType>),
    Array {
        lo: i64,
        hi: i64,
        elem: Box<PascalType>,
    },
    Record {
        fields: Vec<(String, PascalType)>,
        /// Optional variant part: (tag_name, tag_type, variants)
        variant: Option<Box<RecordVariant>>,
        /// For the record an `object` type lowers to, the object type's
        /// name, so its methods can be found.
        object: Option<String>,
        /// `packed record`: no padding between fields.
        packed: bool,
    },
    /// An `object` type as declared. Code generation lowers it to a
    /// `Record` (with `object` set) plus a method table.
    Object(Box<ObjectType>),
    /// `class of T`: a class reference, the address of T's (or a
    /// descendant's) VMT.
    ClassOf(String),
    /// A `var` / `out` (or untyped `const`) parameter of a procedural
    /// type: passed by reference.
    VarParam(Box<PascalType>),
    /// A reference to an interface: `{ instance, method table }`.
    Interface(String),
    /// `specialize TList<Integer>` (or `TList<Integer>`): generic type
    /// `name` with these arguments. The generics pass replaces it with the
    /// name of the instantiation it makes.
    Specialize {
        name: String,
        args: Vec<PascalType>,
    },
    /// `array of T` as a variable's type: a dynamic array, a reference
    /// counted heap block of elements indexed from 0 (nil when empty).
    DynArray(Box<PascalType>),
    /// `array of T` as a parameter's type: an open array, any array of T
    /// (static, dynamic or `[a, b]`) passed as its first element's address
    /// and its highest index. `array of const` is an open array of
    /// `TVarRec`.
    OpenArray(Box<PascalType>),
    /// Enumerated type: (val1, val2, val3)
    Enum {
        /// The type name (e.g., "Color") — set during type section parsing
        name: String,
        values: Vec<String>,
        /// Each value's ordinal: its position, unless given (`(A = 1, B =
        /// 5)`; a value without one follows the previous).
        ordinals: Vec<i64>,
        /// Bytes a value takes (1, 2 or 4), as FPC's `{$PACKENUM}` and
        /// mode give it.
        size: u8,
    },
    /// Subrange type: lo..hi (stored as i64)
    Subrange {
        lo: i64,
        hi: i64,
    },
    /// Set of ordinal type — stored as 256-bit bitmask (4 x i64)
    Set {
        elem: Box<PascalType>,
    },
    /// File of <type> — stored as opaque pointer to bruto_file struct.
    /// `text` is `File { elem: Char }`.
    File {
        elem: Box<PascalType>,
    },
    /// Procedural type — function pointer with a known signature.
    /// `return_type = None` means procedure (void); Some => function.
    /// A `nested` one (`... is nested`) is a closure that can hold a
    /// local routine together with the variables it uses.
    Proc {
        params: Vec<PascalType>,
        return_type: Option<Box<PascalType>>,
        nested: bool,
        /// `... of object`: a method pointer, FPC's `TMethod` (`{ Code,
        /// Data }`), called with `Data` as the method's `Self`.
        of_object: bool,
    },
    /// Conformant array parameter (ISO 7185):
    ///   `array[lo..hi: integer] of T`
    /// `lo_name`/`hi_name` become integer locals inside the procedure body.
    ConformantArray {
        lo_name: String,
        hi_name: String,
        elem: Box<PascalType>,
    },
    /// A named type alias (resolved to canonical type during compilation)
    Named(String),
    /// What an untyped `Pointer` points to. It has no size or value of its
    /// own; it only appears as `Pointer(Box::new(Untyped))`.
    Untyped,
    /// Array whose bounds are constant expressions (`array[0..Max-1] of T`).
    /// The semantic pass folds the bounds and turns it into `Array`; code
    /// generation never sees it.
    ArrayExpr {
        lo: ConstExpr,
        hi: ConstExpr,
        elem: Box<PascalType>,
    },
    /// Subrange whose bounds are constant expressions (`1..MaxItems`),
    /// folded into `Subrange` by the semantic pass.
    SubrangeExpr {
        lo: ConstExpr,
        hi: ConstExpr,
    },
}

impl PascalType {
    /// Call `f` with the values of every enumeration written inside this
    /// type, as in `Phase: (phFocused, phPreProcess)` in a record or
    /// object, each of which names a constant.
    pub fn for_each_enum(&self, f: &mut impl FnMut(&[String], &[i64], u8)) {
        match self {
            PascalType::Enum {
                values,
                ordinals,
                size,
                ..
            } => f(values, ordinals, *size),
            PascalType::Record {
                fields, variant, ..
            } => {
                for (_, t) in fields {
                    t.for_each_enum(f);
                }
                if let Some(v) = variant {
                    v.tag_type.for_each_enum(f);
                    for (_, vfields) in &v.variants {
                        for (_, t) in vfields {
                            t.for_each_enum(f);
                        }
                    }
                }
            }
            PascalType::Object(obj) => {
                for (_, t) in &obj.fields {
                    t.for_each_enum(f);
                }
            }
            PascalType::Array { elem, .. }
            | PascalType::ArrayExpr { elem, .. }
            | PascalType::Set { elem }
            | PascalType::Pointer(elem) => elem.for_each_enum(f),
            _ => {}
        }
    }
}

/// A compile-time integer expression used where the language needs a
/// constant: array and subrange bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstExpr {
    Int(i64),
    Char(u8),
    /// A named constant or enumeration value.
    Name(String),
    Neg(Box<ConstExpr>),
    Binary(ConstOp, Box<ConstExpr>, Box<ConstExpr>),
    /// `ord`, `chr`, `succ`, `pred` or `abs` applied to a constant.
    Call(String, Box<ConstExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

impl ConstExpr {
    /// Convert a parsed expression into a constant expression, or `None`
    /// if it uses anything a constant can't contain.
    pub fn from_expr(expr: &Expr) -> Option<ConstExpr> {
        Some(match expr {
            Expr::IntLit(n, _) => ConstExpr::Int(*n),
            Expr::CharLit(c, _) => ConstExpr::Char(*c),
            Expr::WideCharLit(c, _) => ConstExpr::Int(i64::from(*c)),
            Expr::StrLit(s, _) if s.len() == 1 => ConstExpr::Char(s.as_bytes()[0]),
            Expr::Var(name, _) => ConstExpr::Name(name.clone()),
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } => ConstExpr::Neg(Box::new(ConstExpr::from_expr(operand)?)),
            Expr::BinOp {
                op, left, right, ..
            } => {
                let op = match op {
                    BinOp::Add => ConstOp::Add,
                    BinOp::Sub => ConstOp::Sub,
                    BinOp::Mul => ConstOp::Mul,
                    BinOp::Div => ConstOp::Div,
                    BinOp::Mod => ConstOp::Mod,
                    BinOp::And => ConstOp::And,
                    BinOp::Or => ConstOp::Or,
                    BinOp::Xor => ConstOp::Xor,
                    BinOp::Shl => ConstOp::Shl,
                    BinOp::Shr => ConstOp::Shr,
                    _ => return None,
                };
                ConstExpr::Binary(
                    op,
                    Box::new(ConstExpr::from_expr(left)?),
                    Box::new(ConstExpr::from_expr(right)?),
                )
            }
            Expr::Call { name, args, .. } if args.len() == 1 => {
                let lower = name.to_ascii_lowercase();
                if !matches!(
                    lower.as_str(),
                    "ord"
                        | "chr"
                        | "succ"
                        | "pred"
                        | "abs"
                        | "length"
                        | "sizeof"
                        | "low"
                        | "high"
                        | "lo"
                        | "hi"
                ) {
                    return None;
                }
                ConstExpr::Call(lower, Box::new(ConstExpr::from_expr(&args[0])?))
            }
            _ => return None,
        })
    }

    /// The literal value, if this is a bare integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            ConstExpr::Int(n) => Some(*n),
            ConstExpr::Neg(inner) => inner.as_int().map(|n| -n),
            _ => None,
        }
    }
}

/// A Turbo Pascal `object` type declaration, or an Object Pascal
/// `class` (`is_class`), whose instances live on the heap and are used
/// through references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectType {
    pub parent: Option<String>,
    /// The object's own fields, in declaration order.
    pub fields: Vec<(String, PascalType)>,
    pub methods: Vec<MethodHeader>,
    pub is_class: bool,
    /// `TFoo = class;`: declared here, defined later in the same section.
    pub is_forward: bool,
    pub properties: Vec<PropertyDecl>,
    /// `class var` fields: one variable shared by the class and its
    /// descendants.
    pub class_vars: Vec<(String, PascalType)>,
    /// The type arguments of a parent that is a specialization
    /// (`class(TBox<T>)`); the generics pass makes it a plain name.
    pub parent_args: Vec<PascalType>,
    /// An `interface` type: methods (and properties) only.
    pub is_interface: bool,
    /// The interfaces a class implements (`class(TInterfacedObject,
    /// IFoo)`).
    pub interfaces: Vec<String>,
    /// An interface's GUID (`['{...}']`), if given.
    pub guid: Option<String>,
    /// A CORBA interface (`{$interfaces corba}`): not counted, and not
    /// descending from IInterface.
    pub is_corba: bool,
    /// Method resolution clauses: `function IFoo.Bar = MyBar;` as
    /// (interface, its method, the class's method).
    pub resolutions: Vec<(String, String, String)>,
    /// Fields and properties declared in a `published` section.
    pub published: Vec<String>,
    /// Fields and properties declared before any visibility section:
    /// published in a class compiled with `{$M+}` or descending from one.
    pub default_section: Vec<String>,
    /// Declared with `{$M+}` (`{$TYPEINFO ON}`).
    pub type_info: bool,
}

/// A class property: `property Name[Index: T]: T read Getter write
/// Setter; default;`. Reading it reads the field or calls the getter;
/// writing it writes the field or calls the setter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDecl {
    pub name: String,
    /// An array property's index parameters.
    pub params: Vec<ParamGroup>,
    pub ty: PascalType,
    pub read: Option<String>,
    pub write: Option<String>,
    /// `index N`: passed to the getter and setter before the value.
    pub index: Option<i64>,
    /// The class's default array property: `Obj[I]` means `Obj.Name[I]`.
    pub is_default: bool,
    /// `implements IFoo, IBar`: the class's IFoo is what the property
    /// gives.
    pub implements: Vec<String>,
    /// `default V`: the value streaming leaves out.
    pub default_value: Option<DefaultValue>,
    /// `stored False` / `stored IsStored`.
    pub stored: Option<DefaultValue>,
    pub span: Span,
}

/// A method declared in an object type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodHeader {
    pub name: String,
    pub kind: RoutineKind,
    pub params: Vec<ParamGroup>,
    pub return_type: Option<PascalType>,
    pub is_virtual: bool,
    pub is_abstract: bool,
    /// `override`: replaces the parent's virtual method of the same name.
    pub is_override: bool,
    /// `class procedure` / `class function`: `Self` is the class.
    pub is_class_method: bool,
    pub span: Span,
}

/// Variant part of a record type: `case tag: type of ...`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordVariant {
    pub tag_name: String,
    pub tag_type: PascalType,
    /// (case labels, fields). Labels are folded to `ConstExpr::Int` by the
    /// semantic pass.
    pub variants: Vec<(Vec<ConstExpr>, Vec<(String, PascalType)>)>,
    /// Part of a `packed record`: no padding.
    pub packed: bool,
}

/// A begin/end block containing statements.
#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
    /// Span of the closing `end` keyword (for breakpoint support).
    pub end_span: Span,
}

#[derive(Debug, Clone)]
pub enum Statement {
    Assignment {
        target: String,
        expr: Expr,
        span: Span,
    },
    If {
        condition: Expr,
        then_branch: Block,
        else_branch: Option<Block>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Block,
        span: Span,
    },
    For {
        var: String,
        from: Expr,
        to: Expr,
        downto: bool,
        body: Block,
        span: Span,
    },
    /// `for var in collection do body`: the characters of a string, the
    /// elements of an array, or what an enumerator object (`T.Create(...)`
    /// with `MoveNext` and `Current`) yields.
    ForIn {
        var: String,
        collection: Expr,
        body: Block,
        span: Span,
    },
    RepeatUntil {
        body: Vec<Statement>,
        condition: Expr,
        span: Span,
    },
    WriteLn {
        args: Vec<WriteArg>,
        span: Span,
    },
    Write {
        args: Vec<WriteArg>,
        span: Span,
    },
    ReadLn {
        targets: Vec<String>,
        span: Span,
    },
    Block(Block),
    /// Pointer dereference assignment: `p^ := expr`
    DerefAssignment {
        target: String,
        expr: Expr,
        span: Span,
    },
    /// `new(p)`, or `new(p, Init(args))` to construct an object.
    New {
        target: Expr,
        constructor: Option<(String, Vec<Expr>)>,
        span: Span,
    },
    /// `dispose(p)`, or `dispose(p, Done)` to destroy an object first.
    Dispose {
        target: Expr,
        destructor: Option<(String, Vec<Expr>)>,
        span: Span,
    },
    /// A method call statement: `obj.Method(args)`, `p^.Done`.
    MethodCall {
        object: Expr,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// `str(value[:width[:precision]], target)`: format a number into a
    /// string variable.
    Str {
        value: WriteArg,
        target: Expr,
        span: Span,
    },
    /// A call through a computed procedural value, as a statement:
    /// `TErrorProc(StreamError)(Self)`.
    IndirectCall {
        call: Expr,
        span: Span,
    },
    /// `inherited Method(args)` as a statement.
    Inherited {
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// Array index assignment: `a[i] := expr`
    IndexAssignment {
        target: String,
        index: Expr,
        expr: Expr,
        span: Span,
    },
    /// Multi-dimensional array index assignment: `a[i, j] := expr`
    MultiIndexAssignment {
        target: String,
        indices: Vec<Expr>,
        expr: Expr,
        span: Span,
    },
    /// Record field assignment: `r.field := expr`
    FieldAssignment {
        target: String,
        field: String,
        expr: Expr,
        span: Span,
    },
    /// Procedure call: `proc(args)`
    ProcCall {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// Case/of statement: `case expr of ... end`
    Case {
        expr: Expr,
        branches: Vec<CaseBranch>,
        else_branch: Option<Vec<Statement>>,
        span: Span,
    },
    /// Goto statement: `goto 10`
    Goto {
        label: i64,
        span: Span,
    },
    /// Label marker: `10: statement`
    Label {
        label: i64,
        span: Span,
    },
    /// With statement: `with target do statement`, where the target is a
    /// record or object (a variable, `p^`, a field, a cast...).
    With {
        target: Expr,
        body: Block,
        span: Span,
    },
    /// Assignment to any place expression, e.g. `PRec(p)^.Id := 7`.
    PlaceAssignment {
        target: Expr,
        expr: Expr,
        span: Span,
    },
    /// `exit` or `exit(value)`: leave the current routine, optionally
    /// setting a function's result first.
    Exit {
        value: Option<Expr>,
        span: Span,
    },
    /// `break`: leave the innermost loop.
    Break {
        span: Span,
    },
    /// `continue`: start the innermost loop's next iteration.
    Continue {
        span: Span,
    },
    /// Chained LValue assignment: a[i].field := expr, p^.next := expr, etc.
    ChainedAssignment {
        target: String,
        chain: Vec<LValueAccess>,
        expr: Expr,
        span: Span,
    },
    /// `try ... except ... end` or `try ... finally ... end`.
    Try {
        body: Block,
        handler: TryHandler,
        span: Span,
    },
    /// `raise Obj [at Address]`, or a bare `raise` (in an exception
    /// handler: raise the exception being handled again).
    Raise {
        exception: Option<Expr>,
        span: Span,
    },
}

/// What follows a `try` block.
#[derive(Debug, Clone)]
pub enum TryHandler {
    /// `except`: `on [Name:] Class do Statement` handlers, tried in
    /// order, then `else` statements. With no handlers, the statements
    /// after `except` (in `otherwise`) handle every exception.
    Except {
        handlers: Vec<ExceptHandler>,
        otherwise: Option<Block>,
    },
    /// `finally`: statements run however the `try` block is left.
    Finally(Block),
}

/// `on [Name:] Class do Statement` in an `except` section.
#[derive(Debug, Clone)]
pub struct ExceptHandler {
    /// The name the exception object is given in `body`.
    pub var: Option<String>,
    pub class: String,
    pub body: Block,
    pub span: Span,
}

/// A string literal's text holds the characters written in the source
/// (encoded in UTF-8 when compiled, as FPC copies a UTF-8 source's bytes),
/// and raw bytes (`#233`, or the bytes of a source that isn't UTF-8) as
/// the private-use characters from this one on: byte `b` is U+F780 + `b`.
pub const RAW_BYTE_BASE: u32 = 0xF780;

/// The character a raw byte 128..=255 travels as in a literal's text.
pub fn raw_byte_char(b: u8) -> char {
    char::from_u32(RAW_BYTE_BASE + u32::from(b)).unwrap_or('\u{FFFD}')
}

/// The raw byte a literal's character stands for, if it is one.
pub fn raw_byte_of(c: char) -> Option<u8> {
    let n = u32::from(c);
    (RAW_BYTE_BASE + 0x80..=RAW_BYTE_BASE + 0xFF)
        .contains(&n)
        .then(|| (n - RAW_BYTE_BASE) as u8)
}

/// A string literal's bytes: its raw bytes as they are, its characters
/// in UTF-8.
pub fn literal_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        match raw_byte_of(c) {
            Some(b) => bytes.push(b),
            None => bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    bytes
}

/// A string literal as Unicode text (for a UnicodeString or WideChar): a
/// raw byte is the character of that code (`#233` is U+00E9, as in FPC).
pub fn literal_unicode(text: &str) -> String {
    text.chars()
        .map(|c| match raw_byte_of(c) {
            Some(b) => char::from(b),
            None => c,
        })
        .collect()
}

/// The marker call `WriteStr(S, ...)`'s target rides on, as the first
/// argument of a `Write`.
pub const WRITESTR_TARGET: &str = "writestr$target";

/// A single write/writeln argument with optional formatting: `expr[:width[:precision]]`.
#[derive(Debug, Clone)]
pub struct WriteArg {
    pub expr: Expr,
    pub width: Option<Expr>,
    pub precision: Option<Expr>,
}

/// A single step in a chained LValue access path.
#[derive(Debug, Clone)]
pub enum LValueAccess {
    Field(String),
    Index(Expr),
    Deref,
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Self::Assignment { span, .. }
            | Self::DerefAssignment { span, .. }
            | Self::If { span, .. }
            | Self::While { span, .. }
            | Self::For { span, .. }
            | Self::ForIn { span, .. }
            | Self::RepeatUntil { span, .. }
            | Self::WriteLn { span, .. }
            | Self::Write { span, .. }
            | Self::ReadLn { span, .. }
            | Self::New { span, .. }
            | Self::Dispose { span, .. }
            | Self::IndexAssignment { span, .. }
            | Self::MultiIndexAssignment { span, .. }
            | Self::FieldAssignment { span, .. }
            | Self::ProcCall { span, .. }
            | Self::Case { span, .. }
            | Self::Goto { span, .. }
            | Self::Label { span, .. }
            | Self::With { span, .. }
            | Self::Exit { span, .. }
            | Self::PlaceAssignment { span, .. }
            | Self::Str { span, .. }
            | Self::IndirectCall { span, .. }
            | Self::MethodCall { span, .. }
            | Self::Inherited { span, .. }
            | Self::Break { span }
            | Self::Continue { span }
            | Self::ChainedAssignment { span, .. }
            | Self::Try { span, .. }
            | Self::Raise { span, .. } => *span,
            Self::Block(b) => b.span,
        }
    }
}

/// A single case branch: one or more values -> a statement list
#[derive(Debug, Clone)]
pub struct CaseBranch {
    pub values: Vec<CaseValue>,
    pub body: Vec<Statement>,
    pub span: Span,
}

/// A value or range in a case label
#[derive(Debug, Clone)]
pub enum CaseValue {
    Single(Expr),
    Range(Expr, Expr),
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(i64, Span),
    RealLit(f64, Span),
    CharLit(u8, Span),
    /// A character code above 255, `#$2502`: a `WideChar`.
    WideCharLit(u16, Span),
    StrLit(String, Span),
    /// A string from a `{$codepage cp437}` source with characters above
    /// 127: the bytes (as an AnsiString holds them) and the Unicode text
    /// they mean (as a UnicodeString gets it).
    CodePageStrLit(String, String, Span),
    BoolLit(bool, Span),
    Var(String, Span),
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    /// Pointer dereference: `p^`
    Deref(Box<Expr>, Span),
    /// Function call: `func(args)`
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// Array indexing: `a[i]`
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// Record field access: `r.field`
    FieldAccess {
        record: Box<Expr>,
        field: String,
        span: Span,
    },
    /// Set constructor: [1, 3, 5..10]
    SetConstructor {
        elements: Vec<SetElement>,
        span: Span,
    },
    /// `nil` pointer literal
    Nil(Span),
    /// Address of a variable, element, field or routine: `@x`
    AddrOf(Box<Expr>, Span),
    /// A method call: `obj.Method(args)`, `p^.Method`.
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// `inherited Method(args)`: the parent type's method, called on Self.
    Inherited {
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// A call through a computed procedural value: `TFunc(p)(args)`.
    IndirectCall {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// A typed constant's record value: `(X: 1; Y: 2)`.
    RecordInit {
        fields: Vec<(String, Expr)>,
        span: Span,
    },
    /// A typed constant's array value: `(1, 2, 3)`.
    ArrayInit {
        elements: Vec<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub enum SetElement {
    Single(Expr),
    Range(Expr, Expr),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::IntLit(_, s)
            | Self::RealLit(_, s)
            | Self::CharLit(_, s)
            | Self::WideCharLit(_, s)
            | Self::StrLit(_, s)
            | Self::CodePageStrLit(_, _, s)
            | Self::BoolLit(_, s)
            | Self::Var(_, s) => *s,
            Self::BinOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::Deref(_, span)
            | Self::AddrOf(_, span)
            | Self::MethodCall { span, .. }
            | Self::Inherited { span, .. }
            | Self::RecordInit { span, .. }
            | Self::ArrayInit { span, .. }
            | Self::IndirectCall { span, .. }
            | Self::Call { span, .. }
            | Self::Index { span, .. }
            | Self::FieldAccess { span, .. }
            | Self::SetConstructor { span, .. } => *span,
            Self::Nil(s) => *s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,     // integer division (div)
    RealDiv, // real division (/)
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And, // logical on booleans, bitwise on integers (also Or, Xor, Not)
    Or,
    Xor,
    Shl,
    Shr, // logical shift, as in FPC
    In,  // element membership: x in S
    Is,  // `Obj is TClass`
    As,  // `Obj as TClass`
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}
