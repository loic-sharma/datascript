//! Completion, hover and signature help: the compiler's answers (`datascript_compiler::ide`)
//! converted to LSP types, plus the one thing the compiler cannot do, listing files.

use std::path::Path;

use datascript_compiler::ide::{self, CompletionKind, Completions, FileCompletion};
use datascript_compiler::source::format_of_path;
use datascript_diagnostics::{FileId, SourceFile, Span};
use tower_lsp_server::ls_types::{
    self as lsp, Command, CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation,
    InsertTextFormat, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel,
    SignatureInformation, TextEdit,
};

use crate::convert::range_of;

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
