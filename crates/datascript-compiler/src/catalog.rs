//! The function catalog: the one registry of functions that the binder, backends, LSP and docs
//! all read. It only describes functions; backends implement them.
//!
//! - [`FunctionCatalog::builtin`] holds the M1 set of LANGUAGE.md §7 (see `builtins.rs`).
//! - [`FunctionCatalog::lookup`] finds a function, or close names and the DataScript name of a
//!   function from SQL or other languages.
//! - [`FunctionDef::resolve`] picks the signature for some argument types and reports wrong
//!   arity ([`ArityError`], `DS2002`) or a wrong argument type ([`ArgTypeError`], `DS2006`).

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::sync::OnceLock;

use datascript_diagnostics::{Diagnostic, Span, Suggestions, codes, did_you_mean};

use crate::operators::Operand;
use crate::types::{Type, TypeClass};

/// Whether a function computes one value per row or one per group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    /// One value per row.
    Scalar,
    /// One value per group; allowed only in `summarize` (`DS2004`).
    Aggregate,
}

/// The type of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamType {
    /// Any type in the class. Each argument is checked on its own.
    Class(TypeClass),
    /// The signature's type variable `T`, restricted to a class. All `T` arguments are widened
    /// to one common type ([`Type::common`]), so `iff(p, 1, 2.5)` has `T = real`.
    Var(TypeClass),
}

impl ParamType {
    fn class(self) -> TypeClass {
        match self {
            ParamType::Class(class) | ParamType::Var(class) => class,
        }
    }
}

/// A named parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Param {
    /// Name shown in signatures and messages.
    pub name: &'static str,
    /// Accepted types.
    pub ty: ParamType,
}

/// How the result type follows from the argument types.
#[derive(Debug, Clone, Copy)]
pub enum ReturnRule {
    /// Always this type.
    Fixed(Type),
    /// The type of argument `n` (0-based) after conversion, which for a `T` parameter is `T`.
    SameAsArg(usize),
    /// Like [`ReturnRule::SameAsArg`], but an integer becomes `long`, as integer arithmetic does
    /// (D-021): `sum` and `abs` of an `int` are `long`, so `abs(toint(-2147483648))` does not
    /// overflow.
    IntegerAsLong(usize),
    /// Computed from the converted argument types. `display` is how the result reads in a
    /// signature. Not called when an argument has [`Type::Error`]; the result is then `Error`.
    Custom {
        rule: fn(&[Type]) -> Type,
        display: &'static str,
    },
}

/// A repeated group of parameters, as in `case(p1, v1, p2, v2, ..., else)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    /// The parameters of one repetition.
    pub params: Vec<Param>,
    /// The fewest repetitions allowed.
    pub min: usize,
}

/// One form of a function:
/// `required, [optional...], [repeat...]..., trailing`. A signature has optional parameters or
/// a repeated group, not both.
#[derive(Debug, Clone)]
pub struct Signature {
    /// Leading parameters that must be given.
    pub required: Vec<Param>,
    /// Parameters that may be left out, from the end.
    pub optional: Vec<Param>,
    /// A repeated group after the required parameters.
    pub repeat: Option<Repeat>,
    /// Parameters after the repeated group.
    pub trailing: Vec<Param>,
    /// The result type.
    pub returns: ReturnRule,
}

impl Signature {
    /// The fewest arguments this form takes.
    pub fn min_args(&self) -> usize {
        let repeat = self.repeat.as_ref().map_or(0, |r| r.params.len() * r.min);
        self.required.len() + repeat + self.trailing.len()
    }

    /// The most arguments this form takes, `None` if unbounded.
    pub fn max_args(&self) -> Option<usize> {
        match &self.repeat {
            Some(_) => None,
            None => Some(self.required.len() + self.optional.len() + self.trailing.len()),
        }
    }

    /// Whether this form takes `n` arguments.
    pub fn accepts_arity(&self, n: usize) -> bool {
        if n < self.min_args() {
            return false;
        }
        match &self.repeat {
            None => self.max_args().is_some_and(|max| n <= max),
            Some(repeat) => (n - self.min_args()).is_multiple_of(repeat.params.len().max(1)),
        }
    }

    /// The parameter argument `index` binds to in a call with `n` arguments. `n` must be an
    /// arity this form accepts.
    pub fn param_for(&self, index: usize, n: usize) -> &Param {
        if index < self.required.len() {
            return &self.required[index];
        }
        match &self.repeat {
            None => &self.optional[index - self.required.len()],
            Some(repeat) => {
                let trailing_start = n - self.trailing.len();
                if index >= trailing_start {
                    &self.trailing[index - trailing_start]
                } else {
                    &repeat.params[(index - self.required.len()) % repeat.params.len()]
                }
            }
        }
    }

    fn all_params(&self) -> impl Iterator<Item = &Param> {
        self.required
            .iter()
            .chain(&self.optional)
            .chain(self.repeat.iter().flat_map(|r| &r.params))
            .chain(&self.trailing)
    }

    /// The signature as shown to users: `substring(source: string, start: integer[, length:
    /// integer]) -> string`.
    pub fn display(&self, name: &str) -> String {
        let param = |p: &Param| match p.ty {
            ParamType::Class(class) => format!("{}: {}", p.name, class.name()),
            ParamType::Var(_) => format!("{}: T", p.name),
        };
        let mut parts: Vec<String> = self.required.iter().map(param).collect();
        let mut out = format!("{name}(");
        if let Some(repeat) = &self.repeat {
            for _ in 0..repeat.min {
                parts.extend(repeat.params.iter().map(param));
            }
            parts.push("...".to_string());
        }
        parts.extend(self.trailing.iter().map(param));
        out.push_str(&parts.join(", "));
        for (i, p) in self.optional.iter().enumerate() {
            let sep = if i == 0 && self.required.is_empty() {
                ""
            } else {
                ", "
            };
            let _ = write!(out, "[{sep}{}", param(p));
        }
        out.push_str(&"]".repeat(self.optional.len()));
        out.push_str(") -> ");
        match self.returns {
            ReturnRule::Fixed(ty) => out.push_str(ty.name()),
            ReturnRule::SameAsArg(n) | ReturnRule::IntegerAsLong(n) => {
                let widen = matches!(self.returns, ReturnRule::IntegerAsLong(_));
                let p = self
                    .all_params()
                    .nth(n)
                    .or_else(|| self.all_params().last());
                match p.map(|p| p.ty) {
                    Some(
                        ParamType::Var(TypeClass::Numeric) | ParamType::Class(TypeClass::Numeric),
                    ) if widen => out.push_str("long or real"),
                    Some(
                        ParamType::Var(TypeClass::Integer) | ParamType::Class(TypeClass::Integer),
                    ) if widen => out.push_str("long"),
                    Some(ParamType::Var(_)) | None => out.push('T'),
                    Some(ParamType::Class(class)) => out.push_str(class.name()),
                }
            }
            ReturnRule::Custom { display, .. } => out.push_str(display),
        }
        let var_class = self.all_params().find_map(|p| match p.ty {
            ParamType::Var(class) if class != TypeClass::Any => Some(class),
            _ => None,
        });
        if let Some(class) = var_class {
            let _ = write!(out, ", where T is {}", class.describe());
        }
        out
    }
}

/// One function in the catalog.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    /// The name, as called.
    pub name: &'static str,
    /// Scalar or aggregate.
    pub kind: FunctionKind,
    /// The forms, in order of preference: the first that accepts the arguments wins.
    pub signatures: Vec<Signature>,
    /// What the function does, in a sentence or two (shown on hover and in the reference).
    pub doc: &'static str,
    /// Example calls, each a valid DataScript expression.
    pub examples: Vec<&'static str>,
}

/// The result of resolving a call.
#[derive(Debug, Clone)]
pub struct Resolved<'c> {
    /// The function called.
    pub function: &'c FunctionDef,
    /// The form that matched.
    pub signature: &'c Signature,
    /// The type each argument converts to before the call. An untyped `null` gets the
    /// parameter's type when the parameter has one exact type or is `T`.
    pub arg_types: Vec<Type>,
    /// The result type.
    ///
    /// This can be the internal [`Type::Null`] when only untyped `null`s decide it
    /// (`abs(null)`, `sum(null)`, `iff(p, null, null)`, `coalesce(null, null)`). It is left for
    /// the caller because only the caller knows whether the context gives the value a type; the
    /// binder reports `DS2009` at the `null` argument when it does not (D-038).
    pub return_type: Type,
}

/// Why a call does not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// No form takes this many arguments (`DS2002`).
    Arity(ArityError),
    /// An argument has a type no form accepts at its position (`DS2006`).
    ArgType(ArgTypeError),
}

/// Wrong number of arguments (`DS2002`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArityError {
    /// The function.
    pub function: &'static str,
    /// How many arguments were given.
    pub found: usize,
}

/// What a mistyped argument should have been.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// One of these classes (from the forms that got furthest).
    OneOf(Vec<TypeClass>),
    /// The same type as argument `index` (a `T` parameter), which is `ty`.
    SameAs { index: usize, ty: Type },
}

/// An argument with a type no form accepts (`DS2006`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgTypeError {
    /// The function.
    pub function: &'static str,
    /// The argument's position (0-based).
    pub index: usize,
    /// The argument's type.
    pub found: Type,
    /// What it should have been.
    pub expected: Expected,
}

#[derive(Debug)]
struct Mismatch {
    index: usize,
    /// How many earlier arguments matched exactly: not `null`, not an error, not widened.
    exact: usize,
    expected: Expected,
}

impl FunctionDef {
    /// Picks the first form that accepts arguments of these types, and computes the argument
    /// conversions and result type.
    ///
    /// [`Type::Error`] arguments match any parameter, so an argument that already has an error
    /// never produces a second one. Arity is still checked: it does not depend on types.
    ///
    /// Untyped `null` arguments match any parameter too, and may leave the result type as
    /// [`Type::Null`] (see [`Resolved::return_type`]).
    ///
    /// When no form matches, the error names the argument where the best form failed: the form
    /// whose earlier arguments matched exactly (not through `null`, an error or widening) most
    /// often, and among equals the first form. Equally good forms failing at that argument have
    /// their expectations merged. So `bin(ts, 5)` with `ts: datetime` blames `5` (the datetime
    /// form matched `ts`), and `bin("x", 1h)` lists every form's first parameter.
    pub fn resolve(&self, args: &[Type]) -> Result<Resolved<'_>, ResolveError> {
        let n = args.len();
        let candidates: Vec<&Signature> = self
            .signatures
            .iter()
            .filter(|sig| sig.accepts_arity(n))
            .collect();
        if candidates.is_empty() {
            return Err(ResolveError::Arity(ArityError {
                function: self.name,
                found: n,
            }));
        }
        let mut mismatches = Vec::new();
        for sig in &candidates {
            match bind(sig, args) {
                Ok(arg_types) => {
                    let return_type = match sig.returns {
                        ReturnRule::Fixed(ty) => ty,
                        ReturnRule::SameAsArg(i) => arg_types[i],
                        ReturnRule::IntegerAsLong(i) if arg_types[i].is_integer() => Type::Long,
                        ReturnRule::IntegerAsLong(i) => arg_types[i],
                        ReturnRule::Custom { .. } if arg_types.iter().any(|t| t.is_error()) => {
                            Type::Error
                        }
                        ReturnRule::Custom { rule, .. } => rule(&arg_types),
                    };
                    return Ok(Resolved {
                        function: self,
                        signature: sig,
                        arg_types,
                        return_type,
                    });
                }
                Err(mismatch) => mismatches.push(mismatch),
            }
        }
        // Report the mismatch of the best form: most exact matches before it, then the first
        // form. Merge what the equally good forms failing at the same argument expected.
        let best = mismatches.iter().map(|m| m.exact).max().unwrap_or(0);
        let index = mismatches
            .iter()
            .find(|m| m.exact == best)
            .map_or(0, |m| m.index);
        let mut classes = Vec::new();
        let mut same_as = None;
        for mismatch in mismatches
            .into_iter()
            .filter(|m| m.exact == best && m.index == index)
        {
            match mismatch.expected {
                Expected::OneOf(cs) => {
                    for class in cs {
                        if !classes.contains(&class) {
                            classes.push(class);
                        }
                    }
                }
                same @ Expected::SameAs { .. } => same_as = same_as.or(Some(same)),
            }
        }
        let expected = match same_as {
            Some(same) if classes.is_empty() => same,
            _ => Expected::OneOf(classes),
        };
        Err(ResolveError::ArgType(ArgTypeError {
            function: self.name,
            index,
            found: args[index],
            expected,
        }))
    }

    /// Every form as shown to users.
    pub fn signature_lines(&self) -> Vec<String> {
        self.signatures
            .iter()
            .map(|sig| sig.display(self.name))
            .collect()
    }

    fn signatures_note(&self) -> String {
        match self.signature_lines().as_slice() {
            [one] => format!("signature: `{one}`"),
            many => {
                let list: Vec<String> = many.iter().map(|s| format!("`{s}`")).collect();
                format!("`{}` has these forms: {}", self.name, list.join("; "))
            }
        }
    }
}

/// Checks `args` against `sig`, which accepts their number.
fn bind(sig: &Signature, args: &[Type]) -> Result<Vec<Type>, Mismatch> {
    let n = args.len();
    let mut var: Option<(Type, usize)> = None;
    let mut arg_types = Vec::with_capacity(n);
    let mut exact = 0;
    for (index, &arg) in args.iter().enumerate() {
        let param = sig.param_for(index, n);
        let class = param.ty.class();
        if !class.accepts(arg) {
            return Err(Mismatch {
                index,
                exact,
                expected: Expected::OneOf(vec![class]),
            });
        }
        let widened = match param.ty {
            ParamType::Class(TypeClass::Exact(ty)) => arg != ty,
            ParamType::Class(_) => false,
            ParamType::Var(_) => var.is_some_and(|(t, _)| !t.is_null() && t != arg),
        };
        if !arg.is_null() && !arg.is_error() && !widened {
            exact += 1;
        }
        match param.ty {
            ParamType::Class(TypeClass::Exact(ty)) if !arg.is_error() => arg_types.push(ty),
            ParamType::Class(_) => arg_types.push(arg),
            ParamType::Var(_) => {
                var = match var {
                    None => Some((arg, index)),
                    Some((current, first)) => match current.common(arg) {
                        // Remember the first argument that fixed `T`, for messages.
                        Some(ty) if current.is_null() => Some((ty, index)),
                        Some(ty) => Some((ty, first)),
                        None => {
                            return Err(Mismatch {
                                index,
                                exact,
                                expected: Expected::SameAs {
                                    index: first,
                                    ty: current,
                                },
                            });
                        }
                    },
                };
                arg_types.push(arg);
            }
        }
    }
    if let Some((t, _)) = var {
        for (index, ty) in arg_types.iter_mut().enumerate() {
            if matches!(sig.param_for(index, n).ty, ParamType::Var(_)) {
                *ty = t;
            }
        }
    }
    Ok(arg_types)
}

/// A call as written, for building diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct CallSite<'a> {
    /// The whole call, `f(a, b)`.
    pub span: Span,
    /// The arguments.
    pub args: &'a [Operand<'a>],
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "argument" } else { "arguments" }
}

fn ordinal(index: usize) -> String {
    let n = index + 1;
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn join_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

impl ArityError {
    /// What the function accepts, in words: "2 or 3 arguments", "at least 1 argument",
    /// "an odd number of arguments, at least 3". Forms with a repeated group are described
    /// after the fixed counts: "1 argument, or at least 3 arguments".
    pub fn describe_accepted(function: &FunctionDef) -> String {
        let mut parts = Vec::new();
        let mut counts: Vec<usize> = function
            .signatures
            .iter()
            .filter(|s| s.repeat.is_none())
            .flat_map(|s| s.min_args()..=s.max_args().unwrap_or(s.min_args()))
            .collect();
        counts.sort_unstable();
        counts.dedup();
        match counts.as_slice() {
            [] => {}
            [0] => parts.push("no arguments".to_string()),
            [n] => parts.push(format!("{n} {}", plural(*n))),
            [a, b] => parts.push(format!("{a} or {b} arguments")),
            [first, .., last] if last - first + 1 == counts.len() => {
                parts.push(format!("{first} to {last} arguments"));
            }
            _ => {
                let all: Vec<String> = counts.iter().map(usize::to_string).collect();
                parts.push(format!("{} arguments", join_or(&all)));
            }
        }
        for sig in function.signatures.iter().filter(|s| s.repeat.is_some()) {
            let min = sig.min_args();
            let step = sig.repeat.as_ref().map_or(1, |r| r.params.len());
            let part = match step {
                1 => format!("at least {min} {}", plural(min)),
                2 => {
                    let parity = if min % 2 == 1 { "an odd" } else { "an even" };
                    format!("{parity} number of arguments, at least {min}")
                }
                _ => format!("at least {min} arguments, in groups of {step}"),
            };
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        match parts.as_slice() {
            [] => "no arguments".to_string(),
            [one] => one.clone(),
            // Each part may contain a comma, so join with ", or".
            [init @ .., last] => format!("{}, or {last}", init.join(", ")),
        }
    }

    /// The `DS2002` diagnostic. Extra arguments are underlined; otherwise the call is, with the
    /// names of the missing parameters when they are known.
    pub fn diagnostic(&self, function: &FunctionDef, call: CallSite<'_>) -> Diagnostic {
        let accepted = Self::describe_accepted(function);
        let n = self.found;
        let message = format!("`{}` takes {accepted}, found {n}", self.function);
        let max = function
            .signatures
            .iter()
            .map(Signature::max_args)
            .try_fold(0, |acc, m| m.map(|m| acc.max(m)));
        let mut diagnostic = Diagnostic::error(codes::DS2002, message);
        diagnostic = match max {
            Some(max) if n > max && !call.args.is_empty() => {
                let extra = n - max;
                let span = call.args[max].span.to(call.args[n - 1].span);
                let label = if max == 0 {
                    format!("`{}` takes no arguments", self.function)
                } else if extra == 1 {
                    "this argument is not expected".to_string()
                } else {
                    format!("these {extra} arguments are not expected")
                };
                diagnostic.primary(span, label)
            }
            _ => {
                // The form needing the fewest more arguments says what is missing.
                let missing = function
                    .signatures
                    .iter()
                    .filter(|s| s.repeat.is_none() && s.min_args() > n)
                    .min_by_key(|s| s.min_args())
                    .map(|s| {
                        s.required[n.min(s.required.len())..]
                            .iter()
                            .map(|p| format!("`{}`", p.name))
                            .collect::<Vec<_>>()
                    });
                let label = match missing {
                    Some(names) if !names.is_empty() => {
                        format!("missing {}", join_and(&names))
                    }
                    _ => format!("called with {n} {}", plural(n)),
                };
                diagnostic.primary(call.span, label)
            }
        };
        diagnostic.note(function.signatures_note())
    }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

impl ArgTypeError {
    /// The parameter name at this position if every form that takes this many arguments agrees
    /// on it.
    fn param_name(function: &FunctionDef, index: usize, n: usize) -> Option<&'static str> {
        let mut names = function
            .signatures
            .iter()
            .filter(|s| s.accepts_arity(n))
            .map(|s| s.param_for(index, n).name);
        let first = names.next()?;
        names.all(|name| name == first).then_some(first)
    }

    /// How argument `index` is named in a message: "argument `source` of `strlen`" or "the 2nd
    /// argument of `coalesce`".
    fn describe_arg(function: &FunctionDef, index: usize, n: usize) -> String {
        let repeated = function.signatures.iter().any(|s| s.repeat.is_some());
        match Self::param_name(function, index, n) {
            Some(name) if !repeated => format!("argument `{name}` of `{}`", function.name),
            _ => format!("the {} argument of `{}`", ordinal(index), function.name),
        }
    }

    /// The `DS2006` diagnostic, pointing at the argument and listing the signatures.
    pub fn diagnostic(&self, function: &FunctionDef, call: CallSite<'_>) -> Diagnostic {
        let n = call.args.len();
        let arg = call.args.get(self.index).copied();
        let found = self.found;
        let what = Self::describe_arg(function, self.index, n);
        let message = match &self.expected {
            Expected::OneOf(classes) => {
                let list: Vec<String> = classes.iter().map(|c| c.describe()).collect();
                format!("{what} must be {}, found `{found}`", join_or(&list))
            }
            Expected::SameAs { index, ty } => {
                let other = Self::describe_arg(function, *index, n);
                // "argument `else` of `iff` must have the same type as argument `then`"
                let other = other
                    .strip_suffix(&format!(" of `{}`", function.name))
                    .unwrap_or(&other)
                    .to_string();
                format!("{what} must have the same type as {other} (`{ty}`), found `{found}`")
            }
        };
        let mut diagnostic = Diagnostic::error(codes::DS2006, message);
        match arg {
            Some(arg) => {
                diagnostic = diagnostic.primary(arg.span, format!("this is `{found}`"));
            }
            None => diagnostic = diagnostic.primary(call.span, ""),
        }
        if let Expected::SameAs { index, ty } = &self.expected
            && let Some(other) = call.args.get(*index)
        {
            diagnostic = diagnostic.secondary(other.span, format!("this is `{ty}`"));
        }
        if let Some(help) = arg.and_then(|arg| self.help(arg)) {
            diagnostic = diagnostic.help(help);
        }
        diagnostic.note(function.signatures_note())
    }

    fn help(&self, arg: Operand<'_>) -> Option<String> {
        let targets: Vec<Type> = match &self.expected {
            Expected::OneOf(classes) => classes
                .iter()
                .filter_map(|c| match c {
                    TypeClass::Exact(ty) => Some(*ty),
                    TypeClass::Integer => Some(Type::Long),
                    TypeClass::Numeric => Some(Type::Real),
                    _ => None,
                })
                .collect(),
            Expected::SameAs { ty, .. } => vec![*ty],
        };
        let [target] = targets.as_slice() else {
            return None;
        };
        let conversion = match target {
            Type::String => "tostring",
            Type::Bool => "tobool",
            Type::Int | Type::Long => "tolong",
            Type::Real => "toreal",
            Type::DateTime => "todatetime",
            Type::TimeSpan => "totimespan",
            _ => return None,
        };
        Some(format!(
            "convert it first if that is what you meant: `{conversion}({})`",
            arg.text
        ))
    }
}

/// A name that is not in the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFunction {
    /// Close names in the catalog.
    pub suggestions: Suggestions,
    /// The DataScript function for a name from SQL, Python, Excel or another language
    /// (`len` → `strlen`), if known (DIAGNOSTICS.md rule 7). KQL names need no entry: the
    /// built-in functions use them.
    pub equivalent: Option<&'static str>,
    /// The milestone that adds this function, if it is in LANGUAGE.md §7 but not built yet
    /// ([`PLANNED`]).
    pub planned: Option<&'static str>,
}

/// Functions of LANGUAGE.md §7 that a later milestone adds, with that milestone, so the binder
/// can say "not available yet" instead of "unknown function".
pub const PLANNED: &[(&str, &str)] = &[
    ("arg_max", "M2"),
    ("arg_min", "M2"),
    ("make_list", "M2"),
    ("make_set", "M2"),
    ("split", "M2"),
];

/// The registry of functions.
#[derive(Debug, Clone)]
pub struct FunctionCatalog {
    functions: BTreeMap<&'static str, FunctionDef>,
}

/// Names users bring from SQL, Python or Excel, and the DataScript function to use instead.
/// KQL names are the catalog's own names (D-014).
const FOREIGN_NAMES: &[(&str, &str)] = &[
    ("any_value", "take_any"),
    ("ceil", "ceiling"),
    ("char_length", "strlen"),
    ("concat", "strcat"),
    ("count_distinct", "dcount"),
    ("countdistinct", "dcount"),
    ("current_timestamp", "now"),
    ("date_part", "datetime_part"),
    ("date_trunc", "bin"),
    ("datediff", "datetime_diff"),
    ("datepart", "datetime_part"),
    ("getdate", "now"),
    ("if", "iff"),
    ("ifnull", "coalesce"),
    ("len", "strlen"),
    ("length", "strlen"),
    ("ln", "log"),
    ("lower", "tolower"),
    ("mean", "avg"),
    ("nvl", "coalesce"),
    ("power", "pow"),
    ("replace", "replace_string"),
    ("stddev", "stdev"),
    ("substr", "substring"),
    ("todouble", "toreal"),
    ("upper", "toupper"),
    ("var", "variance"),
];

impl FunctionCatalog {
    /// Builds a catalog from definitions. Panics on a duplicate name: catalogs are built from
    /// code, so that is a bug.
    pub fn new(defs: impl IntoIterator<Item = FunctionDef>) -> Self {
        let mut functions = BTreeMap::new();
        for def in defs {
            let name = def.name;
            assert!(
                functions.insert(name, def).is_none(),
                "function `{name}` is defined twice"
            );
        }
        FunctionCatalog { functions }
    }

    /// The built-in functions (LANGUAGE.md §7).
    pub fn builtin() -> &'static FunctionCatalog {
        static CATALOG: OnceLock<FunctionCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| FunctionCatalog::new(crate::builtins::functions()))
    }

    /// The function called `name` (case-sensitive), or what the user may have meant.
    pub fn lookup(&self, name: &str) -> Result<&FunctionDef, UnknownFunction> {
        if let Some(def) = self.functions.get(name) {
            return Ok(def);
        }
        let lowered = name.to_ascii_lowercase();
        let equivalent = FOREIGN_NAMES
            .iter()
            .find(|(foreign, _)| *foreign == lowered)
            .map(|&(_, ours)| ours)
            .filter(|ours| self.functions.contains_key(ours));
        let planned = PLANNED
            .iter()
            .find(|(planned, _)| *planned == name)
            .map(|&(_, milestone)| milestone);
        Err(UnknownFunction {
            suggestions: did_you_mean(name, self.functions.keys().copied()),
            equivalent,
            planned,
        })
    }

    /// Every function, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = &FunctionDef> {
        self.functions.values()
    }

    /// Every function name, sorted.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.functions.keys().copied()
    }
}

impl fmt::Display for FunctionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FunctionKind::Scalar => "scalar",
            FunctionKind::Aggregate => "aggregate",
        })
    }
}
