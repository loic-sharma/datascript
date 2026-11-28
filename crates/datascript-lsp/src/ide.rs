//! Editor features: the compiler's answers (`datascript_compiler::ide`) converted to LSP types,
//! quick fixes from diagnostic suggestions, and listing files for path completion.

use std::collections::HashMap;
use std::path::Path;

use datascript_compiler::ide::{
    self, CompletionKind, Completions, DocumentSymbol, FileCompletion, RenameEdit, RenameTarget,
    SemanticToken, SemanticTokenKind, SemanticTokenModifiers, SymbolKind,
};
use datascript_compiler::source::format_of_path;
use datascript_diagnostics::{Applicability, Diagnostic, FileId, SourceFile, Span};
use tower_lsp_server::ls_types::{
    self as lsp, CodeAction, CodeActionKind, CodeActionOrCommand, Command, CompletionItem,
    CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat, MarkupContent,
    MarkupKind, ParameterInformation, ParameterLabel, PrepareRenameResponse, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens, SemanticTokensLegend, SignatureInformation, TextEdit, Uri,
    WorkspaceEdit,
};

use crate::convert::{self, range_of};

/// The most file names one completion lists.
const MAX_FILES: usize = 500;

fn markdown(value: String) -> MarkupContent {
    MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    }
}

fn span(file: FileId, start: usize, end: usize) -> Span {
    Span::new(file, start as u32, end as u32)
}

fn kind_of(kind: CompletionKind) -> CompletionItemKind {
    match kind {
        CompletionKind::Column => CompletionItemKind::FIELD,
        CompletionKind::Property => CompletionItemKind::PROPERTY,
        CompletionKind::Value => CompletionItemKind::ENUM_MEMBER,
        CompletionKind::Table => CompletionItemKind::STRUCT,
        CompletionKind::Variable => CompletionItemKind::VARIABLE,
        CompletionKind::Function => CompletionItemKind::FUNCTION,
        CompletionKind::Operator | CompletionKind::Keyword => CompletionItemKind::KEYWORD,
    }
}

/// The completion items: the compiler's, then the files in `base_dir` when it asks for them.
/// `base_dir` is `None` when the server must not read the disk (syntax-only).
pub(crate) fn completion_items(
    src: &SourceFile,
    file: FileId,
    completions: Completions,
    snippets: bool,
    base_dir: Option<&Path>,
) -> Vec<CompletionItem> {
    let range = range_of(src, span(file, completions.start, completions.end));
    let mut items: Vec<CompletionItem> = completions
        .items
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let snippet = c.snippet && snippets;
            let new_text = if c.snippet && !snippets {
                c.label.clone()
            } else {
                c.insert
            };
            CompletionItem {
                kind: Some(kind_of(c.kind)),
                detail: c.detail,
                documentation: c
                    .documentation
                    .map(|d| Documentation::MarkupContent(markdown(d))),
                // Keep the compiler's order: columns first, in table order.
                sort_text: Some(format!("{i:05}")),
                filter_text: Some(c.label.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
                insert_text_format: Some(if snippet {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                }),
                label: c.label,
                ..CompletionItem::default()
            }
        })
        .collect();
    if let (Some(files), Some(base_dir)) = (&completions.files, base_dir) {
        let first = items.len();
        items.extend(file_items(files, &base_dir.join(&files.dir), range, first));
    }
    items
}

/// The data files and directories in `dir`, as items that replace `range`.
fn file_items(
    files: &FileCompletion,
    dir: &Path,
    range: lsp::Range,
    first: usize,
) -> Vec<CompletionItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(bool, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().ok()?.is_dir();
            (is_dir || format_of_path(&name).is_some()).then_some((is_dir, name))
        })
        .collect();
    // Directories first, then files, each by name.
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.truncate(MAX_FILES);
    found
        .into_iter()
        .enumerate()
        .map(|(i, (is_dir, name))| {
            let label = if is_dir { format!("{name}/") } else { name };
            // Outside a string the name becomes one; a directory leaves it open to go on.
            let new_text = match (files.quote, is_dir) {
                (Some(_), _) => label.clone(),
                (None, true) => format!("'{}{label}", files.dir),
                (None, false) => format!("'{}{label}'", files.dir),
            };
            CompletionItem {
                kind: Some(if is_dir {
                    CompletionItemKind::FOLDER
                } else {
                    CompletionItemKind::FILE
                }),
                sort_text: Some(format!("{:05}", first + i)),
                filter_text: Some(new_text.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
                // Choosing a directory lists what is in it.
                command: is_dir.then(|| Command {
                    title: "list files".into(),
                    command: "editor.action.triggerSuggest".into(),
                    arguments: None,
                }),
                label,
                ..CompletionItem::default()
            }
        })
        .collect()
}

/// The compiler's hover as an LSP hover.
pub(crate) fn hover(src: &SourceFile, file: FileId, hover: ide::Hover) -> lsp::Hover {
    lsp::Hover {
        contents: lsp::HoverContents::Markup(markdown(hover.markdown)),
        range: Some(range_of(src, span(file, hover.start, hover.end))),
    }
}

/// The compiler's signature help as LSP signature help, with UTF-16 parameter offsets.
pub(crate) fn signature_help(help: ide::SignatureHelp) -> lsp::SignatureHelp {
    let active = help
        .signatures
        .get(help.active_signature)
        .and_then(|s| s.active_parameter);
    lsp::SignatureHelp {
        signatures: help
            .signatures
            .into_iter()
            .map(|s| SignatureInformation {
                documentation: Some(Documentation::MarkupContent(markdown(s.documentation))),
                parameters: Some(
                    s.parameters
                        .iter()
                        .map(|&(start, end)| ParameterInformation {
                            label: ParameterLabel::LabelOffsets([
                                utf16_len(&s.label[..start]),
                                utf16_len(&s.label[..end]),
                            ]),
                            documentation: None,
                        })
                        .collect(),
                ),
                // `None` would mean "use the top-level one"; past the last parameter means
                // "no parameter is active".
                active_parameter: Some(
                    s.active_parameter
                        .map_or(s.parameters.len() as u32, |p| p as u32),
                ),
                label: s.label,
            })
            .collect(),
        active_signature: Some(help.active_signature as u32),
        active_parameter: active.map(|p| p as u32),
    }
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Quick-fix code actions for every diagnostic overlapping `range` that carries a
/// `MachineApplicable` or `MaybeIncorrect` suggestion.
pub(crate) fn code_actions(
    src: &SourceFile,
    uri: &Uri,
    diagnostics: &[Diagnostic],
    range: lsp::Range,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();
    for diag in diagnostics {
        let diag_range = diag
            .primary
            .as_ref()
            .map_or_else(lsp::Range::default, |label| range_of(src, label.span));
        if !ranges_overlap(diag_range, range) {
            continue;
        }
        let lsp_diag = convert::to_lsp(diag, src, uri);
        for help in &diag.help {
            let Some(suggestion) = &help.suggestion else {
                continue;
            };
            let is_preferred = match suggestion.applicability {
                Applicability::MachineApplicable => true,
                Applicability::MaybeIncorrect => false,
                Applicability::HasPlaceholders => continue,
            };
            let edits: Vec<TextEdit> = suggestion
                .edits
                .iter()
                .map(|edit| TextEdit {
                    range: range_of(src, edit.span),
                    new_text: edit.replacement.clone(),
                })
                .collect();
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), edits);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: help.message.clone(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![lsp_diag.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..WorkspaceEdit::default()
                }),
                is_preferred: Some(is_preferred),
                ..CodeAction::default()
            }));
        }
    }
    actions
}

/// Whether two LSP ranges overlap, including touching at a point cursor.
fn ranges_overlap(a: lsp::Range, b: lsp::Range) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Whether a `codeAction` request's `context.only` filter allows quick fixes.
pub(crate) fn allows_quickfix(only: Option<&[CodeActionKind]>) -> bool {
    let Some(kinds) = only else {
        return true;
    };
    let quickfix_kind = CodeActionKind::QUICKFIX;
    let quickfix = quickfix_kind.as_str();
    kinds.iter().any(|k| {
        let s = k.as_str();
        s.is_empty()
            || s == quickfix
            || s.starts_with(&format!("{quickfix}."))
            || quickfix.starts_with(&format!("{s}."))
    })
}

/// Converts a compiler [`RenameTarget`] into an LSP prepare-rename response.
pub(crate) fn prepare_rename(
    src: &SourceFile,
    file: FileId,
    target: RenameTarget,
) -> PrepareRenameResponse {
    PrepareRenameResponse::RangeWithPlaceholder {
        range: range_of(src, span(file, target.start, target.end)),
        placeholder: target.placeholder,
    }
}

/// Converts compiler [`RenameEdit`]s into an LSP [`WorkspaceEdit`] for `uri`.
pub(crate) fn workspace_edit(
    src: &SourceFile,
    file: FileId,
    uri: &Uri,
    edits: Vec<RenameEdit>,
) -> WorkspaceEdit {
    let lsp_edits: Vec<TextEdit> = edits
        .into_iter()
        .map(|e| TextEdit {
            range: range_of(src, span(file, e.start, e.end)),
            new_text: e.replacement,
        })
        .collect();
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), lsp_edits);
    WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    }
}

/// Converts compiler [`DocumentSymbol`]s into LSP [`lsp::DocumentSymbol`]s.
pub(crate) fn document_symbols(
    src: &SourceFile,
    file: FileId,
    symbols: Vec<DocumentSymbol>,
) -> Vec<lsp::DocumentSymbol> {
    symbols
        .into_iter()
        .map(|s| {
            let kind = match s.kind {
                SymbolKind::Variable => lsp::SymbolKind::VARIABLE,
                SymbolKind::Table => lsp::SymbolKind::STRUCT,
                SymbolKind::Query => lsp::SymbolKind::OBJECT,
            };
            #[allow(deprecated)] // `deprecated` field on `ls_types::DocumentSymbol`
            lsp::DocumentSymbol {
                name: s.name,
                detail: s.detail,
                kind,
                tags: None,
                deprecated: None,
                range: range_of(src, span(file, s.start, s.end)),
                selection_range: range_of(src, span(file, s.selection_start, s.selection_end)),
                children: None,
            }
        })
        .collect()
}

/// The token types advertised in [`semantic_tokens_legend`], in index order.
pub const SEMANTIC_TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::COMMENT,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::TYPE,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::NUMBER,
    SemanticTokenType::STRING,
];

/// The token modifiers advertised in [`semantic_tokens_legend`], in bit order matching
/// [`SemanticTokenModifiers`].
pub const SEMANTIC_TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::READONLY,
    SemanticTokenModifier::DEFAULT_LIBRARY,
];

/// The server's semantic-tokens legend.
pub(crate) fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: SEMANTIC_TOKEN_TYPES.to_vec(),
        token_modifiers: SEMANTIC_TOKEN_MODIFIERS.to_vec(),
    }
}

fn token_type_index(kind: SemanticTokenKind) -> u32 {
    match kind {
        SemanticTokenKind::Comment => 0,
        SemanticTokenKind::Keyword => 1,
        SemanticTokenKind::Operator => 2,
        SemanticTokenKind::EnumMember => 3,
        SemanticTokenKind::Function => 4,
        SemanticTokenKind::Type => 5,
        SemanticTokenKind::Variable => 6,
        SemanticTokenKind::Property => 7,
        SemanticTokenKind::Number => 8,
        SemanticTokenKind::String => 9,
    }
}

fn modifier_bits(modifiers: SemanticTokenModifiers) -> u32 {
    let mut bits = 0u32;
    if modifiers.contains(SemanticTokenModifiers::DECLARATION) {
        bits |= 1 << 0;
    }
    if modifiers.contains(SemanticTokenModifiers::READONLY) {
        bits |= 1 << 1;
    }
    if modifiers.contains(SemanticTokenModifiers::DEFAULT_LIBRARY) {
        bits |= 1 << 2;
    }
    bits
}

/// Encodes compiler [`SemanticToken`]s into LSP delta-encoded [`SemanticTokens`], splitting any
/// multi-line token (such as a block comment) into one token per line.
pub(crate) fn semantic_tokens(src: &SourceFile, tokens: Vec<SemanticToken>) -> SemanticTokens {
    let text = src.text();
    let mut data = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for token in tokens {
        let token_type = token_type_index(token.kind);
        let token_modifiers_bitset = modifier_bits(token.modifiers);
        for (seg_start, seg_end) in split_lines(text, token.start, token.end) {
            let length = utf16_len(&text[seg_start..seg_end]);
            if length == 0 {
                continue;
            }
            let pos = src.utf16_line_col(seg_start as u32);
            let line = pos.line - 1;
            let start = pos.column - 1;
            let delta_line = line - prev_line;
            let delta_start = if delta_line == 0 {
                start - prev_start
            } else {
                start
            };
            data.push(lsp::SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type,
                token_modifiers_bitset,
            });
            prev_line = line;
            prev_start = start;
        }
    }

    SemanticTokens {
        result_id: None,
        data,
    }
}

/// Splits `start..end` into non-empty single-line byte ranges (ending at `\n`, `\r\n` or `\r`).
fn split_lines(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = start;
    while i < end {
        let mut line_end = i;
        while line_end < end && bytes[line_end] != b'\n' && bytes[line_end] != b'\r' {
            line_end += 1;
        }
        if line_end > i {
            out.push((i, line_end));
        }
        if line_end >= end {
            break;
        }
        i = if bytes[line_end] == b'\r' && line_end + 1 < end && bytes[line_end + 1] == b'\n' {
            line_end + 2
        } else {
            line_end + 1
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use datascript_compiler::{Capabilities, NoSchemas, check};

    use super::*;

    #[test]
    fn clients_without_snippets_get_plain_names() {
        let text = "from t | where str";
        let src = SourceFile::new("q.ds", text);
        let analysis = check(&src, FileId(0), &NoSchemas, &Capabilities::full());
        let completions = ide::complete(&src, &analysis, text.len());
        let items = completion_items(&src, FileId(0), completions, false, None);
        let strlen = items.iter().find(|i| i.label == "strlen").unwrap();
        assert_eq!(
            strlen.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );
        let Some(CompletionTextEdit::Edit(edit)) = &strlen.text_edit else {
            panic!("a text edit");
        };
        assert_eq!(edit.new_text, "strlen");
    }
}
