//! Turns the parser's events into a rowan green tree, putting trivia back in.

use rowan::{GreenNode, GreenNodeBuilder};

use super::Event;
use crate::{SyntaxKind, Token};

/// Builds the green tree from `events` over all `tokens` (trivia included) of `text`.
///
/// Trivia goes to the innermost node open when the next significant token (or node) starts, so
/// nodes begin and end at significant tokens. Leading trivia of the file and trailing trivia go
/// to the root.
pub(super) fn build_tree(text: &str, tokens: &[Token], mut events: Vec<Event>) -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    let mut next_token = 0usize;
    let mut open = 0usize;
    let mut forward_kinds = Vec::new();

    for i in 0..events.len() {
        match std::mem::replace(&mut events[i], Event::Tombstone) {
            Event::Start {
                kind,
                forward_parent,
            } => {
                // `precede` makes a node start before an earlier one: follow the chain of forward
                // parents and open the outermost first.
                forward_kinds.push(kind);
                let mut idx = i;
                let mut fp = forward_parent;
                while let Some(offset) = fp {
                    idx += offset as usize;
                    fp = match std::mem::replace(&mut events[idx], Event::Tombstone) {
                        Event::Start {
                            kind,
                            forward_parent,
                        } => {
                            forward_kinds.push(kind);
                            forward_parent
                        }
                        _ => unreachable!("a forward parent is always a start event"),
                    };
                }
                for kind in forward_kinds.drain(..).rev() {
                    if open > 0 {
                        attach_trivia(&mut builder, text, tokens, &mut next_token);
                    }
                    builder.start_node(kind.into_raw());
                    open += 1;
                }
            }
            Event::Finish => {
                open -= 1;
                if open == 0 {
                    // The root keeps whatever trivia (or stray tokens) are left.
                    while next_token < tokens.len() {
                        push_token(&mut builder, text, &tokens[next_token]);
                        next_token += 1;
                    }
                }
                builder.finish_node();
            }
            Event::Token => {
                attach_trivia(&mut builder, text, tokens, &mut next_token);
                push_token(&mut builder, text, &tokens[next_token]);
                next_token += 1;
            }
            Event::Tombstone => {}
        }
    }
    builder.finish()
}

fn attach_trivia(builder: &mut GreenNodeBuilder, text: &str, tokens: &[Token], next: &mut usize) {
    while let Some(token) = tokens.get(*next).filter(|t| t.kind.is_trivia()) {
        push_token(builder, text, token);
        *next += 1;
    }
}

fn push_token(builder: &mut GreenNodeBuilder, text: &str, token: &Token) {
    builder.token(token.kind.into_raw(), token.text(text));
}

impl SyntaxKind {
    fn into_raw(self) -> rowan::SyntaxKind {
        rowan::SyntaxKind(self.into())
    }
}
