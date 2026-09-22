#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Diffs the SSRF address checks against the legacy JavaScript.
//!
//! These decide whether the server will open a connection somewhere an
//! authenticated user named, so a rewrite that quietly relaxes one is worse
//! than no rewrite at all. `tools/phase14/legacy_ssrf.mjs` holds the original
//! `isPrivateIp` and `isBlockedSmtpHost` verbatim and writes their verdicts to
//! `tests/fixtures/legacy-ssrf.json`; this asserts the Rust port agrees on
//! every one.
//!
//! Regenerate after changing either function:
//!   node tools/phase14/legacy_ssrf.mjs \
//!     > backend/crates/pmk-domain/tests/fixtures/legacy-ssrf.json
//! A diff there is a deliberate change to what the server will connect to.

use pmk_domain::net::{is_blocked_smtp_host, is_private_ip};
use std::collections::BTreeMap;
use std::net::IpAddr;

#[derive(serde::Deserialize)]
struct Fixture {
    ips: BTreeMap<String, bool>,
    hosts: BTreeMap<String, bool>,
}

#[test]
fn the_address_checks_agree_with_the_legacy_implementation() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/legacy-ssrf.json")).unwrap();

    let mut disagreements = Vec::new();

    for (raw, &legacy) in &fixture.ips {
        let parsed: IpAddr = raw.parse().unwrap_or_else(|_| panic!("unparseable: {raw}"));
        let ours = is_private_ip(parsed);
        if ours != legacy {
            disagreements.push(format!(
                "\n  address {raw:?}: legacy says private={legacy}, we say {ours}"
            ));
        }
    }

    for (raw, &legacy) in &fixture.hosts {
        let ours = is_blocked_smtp_host(raw);
        if ours != legacy {
            disagreements.push(format!(
                "\n  host {raw:?}: legacy says blocked={legacy}, we say {ours}"
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "{} of {} checks disagree:{}",
        disagreements.len(),
        fixture.ips.len() + fixture.hosts.len(),
        disagreements.join("")
    );
}

#[test]
fn the_fixture_actually_covers_both_answers() {
    // A fixture where everything is blocked would pass against a function
    // that blocks everything.
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/legacy-ssrf.json")).unwrap();
    assert!(fixture.ips.values().any(|v| *v), "no blocked addresses");
    assert!(fixture.ips.values().any(|v| !*v), "no allowed addresses");
    assert!(fixture.hosts.values().any(|v| *v), "no blocked hosts");
    assert!(fixture.hosts.values().any(|v| !*v), "no allowed hosts");
}
