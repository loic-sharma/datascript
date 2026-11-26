//! Signature help: the signatures of the function whose arguments the cursor is in.

use datascript_diagnostics::SourceFile;
use datascript_syntax::ast::{self, AstNode};
use datascript_syntax::{SyntaxKind, SyntaxNode};

use super::{Doc, callee_of, range};
use crate::Analysis;
use crate::catalog::{FunctionCatalog, Param, Signature};

/// The signatures of a call, and where the cursor is in them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInfo>,
    /// The signature that fits the arguments written so far.
    pub active_signature: usize,
    /// The argument the cursor is in, counting from 0.
    pub active_argument: usize,
}

/// One signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInfo {
    /// `substring(source: string, start: integer[, length: integer]) -> string`.
    pub label: String,
    /// The function's docs, in Markdown.
    pub documentation: String,
    /// The byte range of each parameter in [`SignatureInfo::label`], in order.
    pub parameters: Vec<(usize, usize)>,
    /// The parameter the cursor's argument binds to, as an index into `parameters`; `None` when
    /// this signature takes no argument there.
    pub active_parameter: Option<usize>,
}

/// The signatures of the innermost function call whose argument list holds byte `offset`.
pub fn signature_help(
    src: &SourceFile,
    analysis: &Analysis,
    offset: usize,
) -> Option<SignatureHelp> {
    let doc = Doc::new(src.text(), analysis);
    let i = doc.left_index(offset)?;
    let prev = &doc.tokens[i];
    let args = prev
        .parent_ancestors()
        .filter(|n| n.kind() == SyntaxKind::ARG_LIST)
        .find(|args| is_open_at(args, offset))?;
    let call = args
        .parent()
        .filter(|c| c.kind() == SyntaxKind::CALL_EXPR)?;
    let (name, _) = callee_of(&call)?;
    let f = FunctionCatalog::builtin().lookup(&name).ok()?;
    let commas: Vec<_> = args
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::COMMA)
        .collect();
    let active = commas.iter().filter(|c| range(c).end <= offset).count();
    let written = ast::ArgList::cast(args.clone()).map_or(0, |a| a.args().count());
    let signatures: Vec<SignatureInfo> = f
        .signatures
        .iter()
        .map(|sig| info(&name, f.doc, sig, active, written))
        .collect();
    let active_signature = f
        .signatures
        .iter()
        .position(|s| s.accepts_arity(written.max(active + 1)))
        .or_else(|| {
            f.signatures
                .iter()
                .position(|s| s.max_args().is_none_or(|m| m > active))
        })
        .unwrap_or(0);
    Some(SignatureHelp {
        signatures,
        active_signature,
        active_argument: active,
    })
}

/// Whether `offset` is after the `(` of `args` and not after its `)`.
fn is_open_at(args: &SyntaxNode, offset: usize) -> bool {
    let tokens: Vec<_> = args
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();
    let Some(open) = tokens.iter().find(|t| t.kind() == SyntaxKind::L_PAREN) else {
        return false;
    };
    if offset < range(open).end {
        return false;
    }
    match tokens.iter().find(|t| t.kind() == SyntaxKind::R_PAREN) {
        Some(close) => offset <= range(close).start,
        None => true,
    }
}

fn info(name: &str, doc: &str, sig: &Signature, active: usize, written: usize) -> SignatureInfo {
    let label = sig.display(name);
    let parameters = param_ranges(&label, sig);
    // The arity to read the active argument against: the smallest this form accepts that
    // covers what is written.
    let max = sig.max_args();
    let arity = (written.max(active + 1)..=max.unwrap_or(written.max(active + 1) + 8))
        .find(|&n| sig.accepts_arity(n));
    let active_parameter = arity.and_then(|n| active_param_index(sig, active, n));
    SignatureInfo {
        label,
        documentation: doc.to_owned(),
        parameters,
        active_parameter,
    }
}

/// The index in [`displayed_params`] that argument `active` binds to in a call of arity `n`.
fn active_param_index(sig: &Signature, active: usize, n: usize) -> Option<usize> {
    if active < sig.required.len() {
        return Some(active);
    }
    match &sig.repeat {
        None => {
            let index = sig.required.len() + sig.trailing.len() + (active - sig.required.len());
            (index < sig.required.len() + sig.trailing.len() + sig.optional.len()).then_some(index)
        }
        Some(repeat) => {
            if repeat.params.is_empty() {
                return None;
            }
            let repeat_shown = repeat.min * repeat.params.len();
            let trailing_start = n - sig.trailing.len();
            if active >= trailing_start {
                Some(sig.required.len() + repeat_shown + (active - trailing_start))
            } else {
                let in_repeat = active - sig.required.len();
                if repeat.min > 0 {
                    let shown_pos = if in_repeat < repeat_shown {
                        in_repeat
                    } else {
                        (repeat.min - 1) * repeat.params.len() + (in_repeat % repeat.params.len())
                    };
                    Some(sig.required.len() + shown_pos)
                } else {
                    // When `min == 0`, the repeated group is shown only as `...` after the
                    // required parameters (e.g. `coalesce(value: T, value: T, ...)`).
                    let base = sig.required.len().checked_sub(repeat.params.len())?;
                    Some(base + (in_repeat % repeat.params.len()))
                }
            }
        }
    }
}

/// The parameters in the order [`Signature::display`] shows them.
fn displayed_params(sig: &Signature) -> Vec<&Param> {
    let mut out: Vec<&Param> = sig.required.iter().collect();
    if let Some(repeat) = &sig.repeat {
        for _ in 0..repeat.min {
            out.extend(&repeat.params);
        }
    }
    out.extend(&sig.trailing);
    out.extend(&sig.optional);
    out
}

/// The range of each displayed parameter (`name: type`) in `label`.
fn param_ranges(label: &str, sig: &Signature) -> Vec<(usize, usize)> {
    let mut from = label.find('(').map_or(0, |i| i + 1);
    let mut out = Vec::new();
    for p in displayed_params(sig) {
        let needle = format!("{}: ", p.name);
        let Some(rel) = label[from..].find(&needle) else {
            continue;
        };
        let start = from + rel;
        let end = label[start..]
            .find([',', '[', ']', ')'])
            .map_or(label.len(), |e| start + e);
        out.push((start, end));
        from = end;
    }
    out
}
