#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Diffs the ETO redaction against the legacy JavaScript.
//!
//! An ETO note carries the builder's commercial position -- why the extra work
//! is being charged -- next to what the client is entitled to see. Getting
//! this wrong sends the first to the client. The legacy did it with one
//! regex, which `tools/phase14/legacy_eto_redaction.mjs` holds verbatim; this
//! asserts the Rust agrees on every case.
//!
//! Regenerate after changing the redaction:
//!   node tools/phase14/legacy_eto_redaction.mjs \
//!     > backend/crates/pmk-domain/tests/fixtures/legacy-eto-redaction.json

use pmk_domain::email::redact_internal_eto_reason;
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct Fixture {
    cases: BTreeMap<String, String>,
    expected: BTreeMap<String, String>,
}

#[test]
fn redaction_removes_at_least_what_the_legacy_removed() {
    let f: Fixture =
        serde_json::from_str(include_str!("fixtures/legacy-eto-redaction.json")).unwrap();

    let mut failures = Vec::new();
    for (name, input) in &f.cases {
        let ours = redact_internal_eto_reason(input);
        let theirs = &f.expected[name];

        // The heading and the text under it must be gone from ours wherever
        // they are gone from theirs. Exact string equality is *not* the bar:
        // dropping more is safe, dropping less is not.
        if !theirs.contains("INTERNAL REASON") && ours.contains("INTERNAL REASON") {
            failures.push(format!("\n  {name}: we kept the internal heading"));
        }
        for secret in ["we quoted low", "secret terms", "line two"] {
            if !theirs.contains(secret) && ours.contains(secret) {
                failures.push(format!("\n  {name}: we kept {secret:?}"));
            }
        }
        // And everything the legacy kept, we keep.
        for kept in ["EXTRA TO ORDER", "WHAT IS REQUIRED", "Poured the slab"] {
            if theirs.contains(kept) && !ours.contains(kept) {
                failures.push(format!("\n  {name}: we dropped {kept:?}"));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} divergences across {} cases:{}",
        failures.len(),
        f.cases.len(),
        failures.join("")
    );
}

#[test]
fn the_fixture_contains_something_to_redact() {
    // A fixture of only plain notes would pass against a function that does
    // nothing at all.
    let f: Fixture =
        serde_json::from_str(include_str!("fixtures/legacy-eto-redaction.json")).unwrap();
    assert!(
        f.cases.values().any(|c| c.contains("INTERNAL REASON")),
        "no case carries an internal reason"
    );
    assert!(
        f.expected.values().any(|e| !e.contains("INTERNAL REASON")),
        "the legacy removed nothing in any case"
    );
}
