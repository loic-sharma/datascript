//! Every `CodeStatus::Active` diagnostic code in `datascript_diagnostics::codes::REGISTRY` must
//! appear as a rendered diagnostic in at least one non-skipped golden test under `tests/cases/`,
//! unless listed in `ALLOWLIST` with a reason (M2-30).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use datascript_diagnostics::Code;
use datascript_diagnostics::codes::{self, CodeStatus, REGISTRY};
use datascript_tests::TestCase;
use datascript_tests::runner::{discover, repo_root};

/// Active codes that intentionally do not appear in `.dstest` expected sections, with a reason
/// for each.
const ALLOWLIST: &[(Code, &str)] = &[
    (
        codes::DS3007,
        "requires ExecContext::sandbox (D-016); covered by datascript-datafusion and datascript-server integration tests",
    ),
    (
        codes::DS4002,
        "fallback for untraceable DataFusion runtime errors; covered by unit tests",
    ),
    (
        codes::DS4060,
        "emitted by UI runtimes when a result hits the UI row limit (D-062); covered by datascript-engine and datascript-server tests",
    ),
    (
        codes::DS5001,
        "omitted from golden output because the `render:` line already shows the chart (TESTING.md)",
    ),
    (
        codes::DS9001,
        "internal compiler error and caught panics; covered by unit tests",
    ),
];

/// Extracts codes from rendered diagnostic header lines (`error[DS1001]:`, `warning[DS2103]:`,
/// `info[DS1007]:`) inside a golden expected section body.
fn extract_rendered_codes(body: &str, out: &mut BTreeSet<String>) {
    for line in body.lines() {
        let rest = line
            .strip_prefix("error[")
            .or_else(|| line.strip_prefix("warning["))
            .or_else(|| line.strip_prefix("info["));
        if let Some(rest) = rest
            && let Some((code, _)) = rest.split_once("]:")
            && code.len() == 6
            && code.starts_with("DS")
            && code[2..].bytes().all(|b| b.is_ascii_digit())
        {
            out.insert(code.to_string());
        }
    }
}

#[test]
fn every_active_diagnostic_code_has_golden_coverage() {
    let cases_dir = repo_root().join("tests").join("cases");
    let files = discover(&cases_dir).expect("discover golden cases");
    assert!(
        !files.is_empty(),
        "expected golden test cases under {}",
        cases_dir.display()
    );

    let mut covered = BTreeSet::new();
    for path in &files {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let case =
            TestCase::parse(&text).unwrap_or_else(|e| panic!("malformed {}: {e}", path.display()));
        if case.skip.is_some() {
            continue;
        }
        for section in &case.expected {
            extract_rendered_codes(&section.body, &mut covered);
        }
    }

    let allowlist: BTreeMap<Code, &str> = ALLOWLIST.iter().copied().collect();
    assert_eq!(
        allowlist.len(),
        ALLOWLIST.len(),
        "duplicate entry in ALLOWLIST"
    );

    let mut missing = Vec::new();
    let mut stale_allowlist = Vec::new();
    for info in REGISTRY {
        if info.status != CodeStatus::Active {
            assert!(
                !allowlist.contains_key(&info.code),
                "allowlist entry {} is not CodeStatus::Active",
                info.code
            );
            continue;
        }
        let code_str = info.code.to_string();
        let is_covered = covered.contains(&code_str);
        if is_covered {
            if allowlist.contains_key(&info.code) {
                stale_allowlist.push(code_str);
            }
        } else if !allowlist.contains_key(&info.code) {
            missing.push(format!("{} ({})", info.code, info.summary));
        }
    }

    assert!(
        missing.is_empty(),
        "Active diagnostic codes with no golden test coverage under tests/cases/:\n  {}\n\
         Add a golden case or, if untestable in .dstest, add an ALLOWLIST entry with a reason.",
        missing.join("\n  ")
    );
    assert!(
        stale_allowlist.is_empty(),
        "Codes in ALLOWLIST now have golden coverage and should be removed from ALLOWLIST: {}",
        stale_allowlist.join(", ")
    );
}
