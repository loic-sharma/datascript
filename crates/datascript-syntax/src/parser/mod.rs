//! The parser: an event-based recursive-descent parser, as in rust-analyzer.
//!
//! The grammar functions in [`grammar`] look at the significant (non-trivia) tokens and emit
//! events: start a node, add a token, finish a node. [`sink`] then builds the rowan green tree
//! from the events and puts the trivia back, so the tree is lossless.
//!
//! The parser never fails and never loops: every grammar loop either consumes a token or stops.
//! Text that does not fit the grammar is reported and wrapped in an `ERROR` node. Mistakes we can
//! recognize (a forgotten `|`, SQL habits, `=` for `==`, …) get targeted diagnostics from
//! [`targeted`]; DS0101 is the last resort.

mod grammar;
mod sink;
mod targeted;

use datascript_diagnostics::{Diagnostic, FileId, Span, codes};
use rowan::GreenNode;

use crate::SyntaxKind::{self, *};
use crate::Token;

pub(crate) use grammar::OPERATORS;

/// Parses `tokens` (the lexer's output for `text`, trivia included) into a green tree, and
/// returns it with the parse errors.
pub(crate) fn parse_tokens(
    text: &str,
    tokens: &[Token],
    file: FileId,
) -> (GreenNode, Vec<Diagnostic>) {
    let significant: Vec<Token> = tokens
        .iter()
        .copied()
        .filter(|t| !t.kind.is_trivia())
        .collect();
    let mut line_start = Vec::with_capacity(significant.len());
    let mut joined = Vec::with_capacity(significant.len());
    let mut prev_end = 0usize;
    for (i, token) in significant.iter().enumerate() {
        let gap = &text[prev_end..token.start as usize];
        line_start.push(i == 0 || gap.contains(['\n', '\r']));
        joined.push(i > 0 && gap.is_empty());
        prev_end = token.end() as usize;
    }
    let mut parser = Parser {
        text,
        file,
        tokens: significant,
        line_start,
        joined,
        pos: 0,
        events: Vec::new(),
        errors: Vec::new(),
        last_error_at: None,
        depth: 0,
        nesting: 0,
        eq_compares: false,
        in_arg_list: false,
        too_deep: false,
        last_unclosed: None,
    };
    grammar::program(&mut parser);
    let green = sink::build_tree(text, tokens, parser.events);
    (green, parser.errors)
}

/// One step of the parse. See [`sink::build_tree`].
#[derive(Debug)]
enum Event {
    /// Starts a node. `forward_parent` is the distance to the start event of a node that must
    /// open *before* this one (set by [`CompletedMarker::precede`]).
    Start {
        kind: SyntaxKind,
        forward_parent: Option<u32>,
    },
    /// Finishes the innermost open node.
    Finish,
    /// Adds the next significant token, with the trivia before it.
    Token,
    /// An abandoned or already-consumed start event.
    Tombstone,
}

/// A set of token kinds, for quick membership tests.
#[derive(Debug, Clone, Copy)]
struct TokenSet(u128);

impl TokenSet {
    const fn new(kinds: &[SyntaxKind]) -> TokenSet {
        let mut bits = 0u128;
        let mut i = 0;
        while i < kinds.len() {
            // A set is a `u128`: a kind past 127 would be dropped silently. In a `const` this
            // fails the build.
            assert!(
                (kinds[i] as u16) < 128,
                "TokenSet holds only the first 128 kinds"
            );
            bits |= 1u128 << (kinds[i] as u16);
            i += 1;
        }
        TokenSet(bits)
    }

    const fn contains(self, kind: SyntaxKind) -> bool {
        (kind as u16) < 128 && self.0 & (1u128 << (kind as u16)) != 0
    }
}

struct Parser<'t> {
    text: &'t str,
    file: FileId,
    /// Significant tokens only.
    tokens: Vec<Token>,
    /// Whether a token is the first significant token on its line.
    line_start: Vec<bool>,
    /// Whether a token directly follows the previous significant token, with no trivia between.
    joined: Vec<bool>,
    pos: usize,
    events: Vec<Event>,
    errors: Vec<Diagnostic>,
    /// The token index of the last error, so one bad token gives one error, not a cascade.
    last_error_at: Option<usize>,
    /// How many brackets the parser is inside. At depth 0 a `)` or `,` is never a terminator.
    depth: u32,
    /// How many expressions the parser is inside, to stop before the stack overflows.
    nesting: u32,
    /// Whether `=` here is a mistyped comparison (DS0110): in a `where` predicate, a `join ... on`
    /// condition, or the arguments of `iff` and friends.
    eq_compares: bool,
    /// Whether the innermost bracket is a call's argument list, where a `,` starts the next
    /// argument even after a pipeline (`f(T | where a, b)`).
    in_arg_list: bool,
    /// Whether DS0100 was reported, after which unclosed brackets are not reported again.
    too_deep: bool,
    /// Where the last unclosed bracket was reported (token index) and the index of its error.
    last_unclosed: Option<(usize, usize)>,
}

impl Parser<'_> {
    // --- Looking at tokens ---

    fn nth(&self, n: usize) -> SyntaxKind {
        self.tokens.get(self.pos + n).map_or(EOF, |t| t.kind)
    }

    fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    fn at_set(&self, set: TokenSet) -> bool {
        set.contains(self.current())
    }

    fn nth_text(&self, n: usize) -> &str {
        self.tokens
            .get(self.pos + n)
            .map_or("", |t| t.text(self.text))
    }

    /// Whether the `n`th token is an identifier with this text: a contextual keyword.
    fn nth_at_kw(&self, n: usize, kw: &str) -> bool {
        self.nth(n) == IDENT && self.nth_text(n) == kw
    }

    fn at_kw(&self, kw: &str) -> bool {
        self.nth_at_kw(0, kw)
    }

    /// Whether the `n`th token directly follows the one before it, with no trivia.
    fn nth_joined(&self, n: usize) -> bool {
        self.joined.get(self.pos + n).copied().unwrap_or(false)
    }

    fn at_line_start(&self) -> bool {
        self.line_start.get(self.pos).copied().unwrap_or(true)
    }

    /// Byte offset of the current token (the end of the input at EOF).
    fn offset(&self) -> u32 {
        self.tokens
            .get(self.pos)
            .map_or(self.text.len() as u32, |t| t.start)
    }

    /// Byte offset just past the previous significant token.
    fn prev_end(&self) -> u32 {
        self.pos
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .map_or(0, |t| t.end())
    }

    fn prev_text(&self) -> Option<&str> {
        let i = self.pos.checked_sub(1)?;
        Some(self.tokens.get(i)?.text(self.text))
    }

    fn span(&self, start: u32, end: u32) -> Span {
        Span::new(self.file, start, end)
    }

    /// The span of the significant token at index `i`.
    fn token_span(&self, i: usize) -> Span {
        let t = self.tokens[i];
        self.span(t.start, t.end())
    }

    /// The source text between two byte offsets.
    fn source(&self, start: u32, end: u32) -> &str {
        &self.text[start as usize..end as usize]
    }

    // --- Consuming tokens ---

    /// Adds the current token to the tree. Does nothing at EOF.
    fn bump(&mut self) {
        if self.pos < self.tokens.len() {
            self.events.push(Event::Token);
            self.pos += 1;
        }
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    // --- Nodes ---

    fn start(&mut self) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::Start {
            kind: TOMBSTONE_KIND,
            forward_parent: None,
        });
        Marker {
            event: pos,
            token: self.pos,
        }
    }

    // --- Errors ---

    /// Reports `diagnostic` as the error at the current token, unless that token already has
    /// one (from the parser, or from the lexer if it is an `ERROR` token).
    fn report(&mut self, diagnostic: Diagnostic) {
        if self.last_error_at == Some(self.pos) || self.at(ERROR) {
            return;
        }
        self.last_error_at = Some(self.pos);
        self.errors.push(diagnostic);
    }

    /// The current token in words, for "found ..." in messages.
    fn found(&self) -> String {
        if self.at(EOF) {
            return "the end of the input".to_string();
        }
        let text = self.nth_text(0);
        if text.chars().count() > 24 {
            let short: String = text.chars().take(20).collect();
            format!("`{short}…`")
        } else {
            format!("`{text}`")
        }
    }

    /// The span of the current token, or an empty span just after the last token at EOF.
    fn current_span(&self) -> Span {
        match self.tokens.get(self.pos) {
            Some(t) => self.span(t.start, t.end()),
            None => self.span(self.prev_end(), self.prev_end()),
        }
    }

    /// A DS0101 error at the current token: "expected {what}, found {current}".
    fn expected(&self, what: &str) -> Diagnostic {
        let label = if self.at(EOF) {
            "the input ends here"
        } else {
            ""
        };
        Diagnostic::error(
            codes::DS0101,
            format!("expected {what}, found {}", self.found()),
        )
        .primary(self.current_span(), label)
    }

    fn error_expected(&mut self, what: &str) {
        let d = self.expected(what);
        self.report(d);
    }

    /// "after `prev`" for messages, or nothing at the start of the input.
    fn after_prev(&self) -> String {
        match self.prev_text() {
            Some(prev) => format!(" after `{prev}`"),
            None => String::new(),
        }
    }

    // --- Recovery ---

    /// Wraps the current token, and the following ones until `stop` holds, in an `ERROR` node.
    /// Bracketed groups are skipped as a whole. Always consumes at least one token (unless at
    /// EOF), so callers can loop on it.
    fn recover(&mut self, stop: impl Fn(&Parser<'_>) -> bool) {
        if self.at(EOF) {
            return;
        }
        let m = self.start();
        loop {
            self.bump_group();
            if self.at(EOF) || stop(self) {
                break;
            }
        }
        m.complete(self, ERROR);
    }

    /// Bumps the current token; if it opens a bracket, bumps up to and including the matching
    /// close bracket, stopping early at `;`, `let` or the end.
    fn bump_group(&mut self) {
        let mut stack = Vec::new();
        loop {
            match self.current() {
                L_PAREN => stack.push(R_PAREN),
                L_BRACK => stack.push(R_BRACK),
                L_BRACE => stack.push(R_BRACE),
                kind if stack.last() == Some(&kind) => {
                    stack.pop();
                }
                _ => {}
            }
            self.bump();
            if stack.is_empty() || self.at(EOF) || self.at(SEMICOLON) || self.at(LET_KW) {
                break;
            }
        }
    }
}

/// Placeholder kind of a start event whose node kind is not known yet.
const TOMBSTONE_KIND: SyntaxKind = ERROR;

/// An open node. Finish it with [`Marker::complete`] or drop it with [`Marker::abandon`].
#[derive(Debug)]
struct Marker {
    event: u32,
    /// The token index where the node starts.
    token: usize,
}

impl Marker {
    fn complete(self, p: &mut Parser<'_>, kind: SyntaxKind) -> CompletedMarker {
        if let Event::Start { kind: slot, .. } = &mut p.events[self.event as usize] {
            *slot = kind;
        }
        p.events.push(Event::Finish);
        CompletedMarker { event: self.event }
    }

    /// Completes the node if it holds at least one token, and abandons it otherwise, so the tree
    /// never has empty nodes.
    fn complete_if_nonempty(self, p: &mut Parser<'_>, kind: SyntaxKind) -> Option<CompletedMarker> {
        if p.pos > self.token {
            Some(self.complete(p, kind))
        } else {
            self.abandon(p);
            None
        }
    }

    fn abandon(self, p: &mut Parser<'_>) {
        let idx = self.event as usize;
        if idx == p.events.len() - 1 {
            p.events.pop();
        } else {
            p.events[idx] = Event::Tombstone;
        }
    }
}

/// A finished node, which can still be wrapped in a new parent with [`CompletedMarker::precede`].
#[derive(Debug, Clone, Copy)]
struct CompletedMarker {
    event: u32,
}

impl CompletedMarker {
    /// The kind of the finished node.
    fn kind(self, p: &Parser<'_>) -> SyntaxKind {
        match p.events[self.event as usize] {
            Event::Start { kind, .. } => kind,
            _ => TOMBSTONE_KIND,
        }
    }

    /// Starts a new node that will contain this one: `a` becomes the left side of `a + b`.
    fn precede(self, p: &mut Parser<'_>) -> Marker {
        let mut new = p.start();
        if let Event::Start { forward_parent, .. } = &mut p.events[self.event as usize] {
            *forward_parent = Some(new.event - self.event);
        }
        // The new node contains the completed one, so it is never empty.
        new.token = 0;
        new
    }
}
