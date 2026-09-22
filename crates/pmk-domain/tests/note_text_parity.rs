#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Diffs the note renderers against the legacy JavaScript.
//!
//! The note body is persisted and re-read by the diary UI, so it is contract,
//! not presentation: a stray space would alter how every existing inspection
//! looks the next time it is saved. `tools/phase11/legacy_expected.mjs` runs
//! the original functions -- copied verbatim into
//! `tools/phase11/legacy_note_text.mjs` -- over the shared case list and
//! writes `tests/fixtures/legacy-note-text.json`. This test renders the same
//! cases in Rust and asserts the strings are identical.
//!
//! Regenerate after changing a renderer:
//!   node tools/phase11/legacy_expected.mjs \
//!     > backend/crates/pmk-domain/tests/fixtures/legacy-note-text.json
//! A diff in that file is a deliberate divergence and needs justifying.

use pmk_domain::forms::{
    eto_note, inspection_entry_summary, inspection_note, EtoInput, EtoNumber, InspectionDraftInput,
    InspectionItemInput, JobHeader,
};
use pmk_domain::ids::JobId;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    kind: String,
    // inspection
    #[serde(default)]
    date: String,
    #[serde(default)]
    input: Option<DraftJson>,
    #[serde(default)]
    photos: HashMap<String, usize>,
    // eto
    #[serde(default)]
    po: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    job: Option<JobJson>,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    details: String,
    // po number
    #[serde(default)]
    job_number: String,
    #[serde(default)]
    job_id: i32,
    #[serde(default)]
    n: i32,
    // summary
    #[serde(default)]
    inspection_type: String,
    #[serde(default)]
    stage: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DraftJson {
    #[serde(default)]
    inspector: String,
    #[serde(default)]
    inspection_type: String,
    #[serde(default)]
    stage: String,
    #[serde(default)]
    observations: String,
    #[serde(default)]
    items: Vec<ItemJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemJson {
    client_key: String,
    #[serde(default)]
    room: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    actioned: bool,
}

#[derive(Deserialize, Default)]
struct JobJson {
    #[serde(default)]
    job_number: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    address: String,
}

#[derive(Deserialize)]
struct Expected {
    name: String,
    text: String,
}

#[test]
fn every_note_renderer_matches_the_legacy_output_byte_for_byte() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/note-text-cases.json")).unwrap();
    let expected: Vec<Expected> =
        serde_json::from_str(include_str!("fixtures/legacy-note-text.json")).unwrap();
    assert_eq!(
        cases.len(),
        expected.len(),
        "the fixture is stale -- rerun tools/phase11/legacy_expected.mjs"
    );

    let mut failures = Vec::new();
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(case.name, want.name, "case order drifted");
        let got = render(case);
        if got != want.text {
            failures.push(format!(
                "\n  case {:?}\n    legacy: {:?}\n    rust:   {:?}",
                case.name, want.text, got
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases diverge from the legacy renderer:{}",
        failures.len(),
        cases.len(),
        failures.join("")
    );
}

fn render(case: &Case) -> String {
    match case.kind.as_str() {
        "inspection" => {
            let d = case.input.as_ref().unwrap();
            let draft = InspectionDraftInput {
                inspector: d.inspector.clone(),
                inspection_type: d.inspection_type.clone(),
                stage: d.stage.clone(),
                observations: d.observations.clone(),
                weather_data: None,
                revision: 0,
                items: d
                    .items
                    .iter()
                    .map(|i| InspectionItemInput {
                        client_key: i.client_key.clone(),
                        room: i.room.clone(),
                        description: i.description.clone(),
                        actioned: i.actioned,
                        sort_order: None,
                    })
                    .collect(),
            };
            let photos = |k: &str| *case.photos.get(k).unwrap_or(&0);
            let date = case.date.parse().unwrap();
            inspection_note(&draft, date, &photos)
        }
        "eto" => {
            let j = case.job.as_ref().cloned().unwrap_or_default();
            eto_note(
                &case.po,
                &case.user,
                &JobHeader {
                    job_number: j.job_number,
                    name: j.name,
                    address: j.address,
                },
                &EtoInput {
                    job_id: JobId(1),
                    reason: case.reason.clone(),
                    details: case.details.clone(),
                },
            )
        }
        "po" => EtoNumber(case.n).po_number(&case.job_number, JobId(case.job_id)),
        "summary" => inspection_entry_summary(&case.inspection_type, &case.stage),
        other => panic!("unknown case kind {other}"),
    }
}

impl Clone for JobJson {
    fn clone(&self) -> Self {
        Self {
            job_number: self.job_number.clone(),
            name: self.name.clone(),
            address: self.address.clone(),
        }
    }
}
