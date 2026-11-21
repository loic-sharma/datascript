//! Property tests: the lexer never panics, and its tokens reproduce the input exactly.

use datascript_diagnostics::FileId;
use datascript_syntax::lex;
use proptest::prelude::*;

fn check_round_trip(text: &str) -> Result<(), TestCaseError> {
    let lexed = lex(text, FileId(0));
    let mut offset = 0;
    for token in &lexed.tokens {
        prop_assert_eq!(token.start, offset, "tokens must be contiguous");
        prop_assert!(token.len > 0, "tokens must not be empty");
        offset = token.end();
    }
    prop_assert_eq!(offset as usize, text.len());
    let joined: String = lexed.tokens.iter().map(|t| t.text(text)).collect();
    prop_assert_eq!(joined.as_str(), text);
    for diagnostic in &lexed.diagnostics {
        let primary = diagnostic
            .primary
            .as_ref()
            .expect("lexer diagnostics have a span");
        prop_assert!(primary.span.end as usize <= text.len());
        prop_assert!(text.is_char_boundary(primary.span.start as usize));
        prop_assert!(text.is_char_boundary(primary.span.end as usize));
    }
    Ok(())
}

/// Fragments that exercise the tricky paths, glued together at random.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("'".to_string()),
        Just("\"".to_string()),
        Just("@'".to_string()),
        Just("['".to_string()),
        Just("']".to_string()),
        Just("\\".to_string()),
        Just("\\u{".to_string()),
        Just("/*".to_string()),
        Just("*/".to_string()),
        Just("//".to_string()),
        Just("\n".to_string()),
        Just("datetime(".to_string()),
        Just("timespan(".to_string()),
        Just(")".to_string()),
        Just("!".to_string()),
        Just("in~".to_string()),
        Just("$left".to_string()),
        Just("1.5".to_string()),
        Just("..".to_string()),
        Just("e-".to_string()),
        Just("µs".to_string()),
        Just("ï".to_string()),
        Just("_".to_string()),
        "[0-9]{1,25}",
        "[a-z_]{1,8}",
        "[ -~]{1,4}",
        any::<char>().prop_map(String::from),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn arbitrary_text_round_trips(text in any::<String>()) {
        check_round_trip(&text)?;
    }

    #[test]
    fn fragments_round_trip(parts in prop::collection::vec(fragment(), 0..24)) {
        check_round_trip(&parts.concat())?;
    }
}
