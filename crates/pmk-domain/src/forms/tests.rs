use super::*;
use crate::ids::JobId;

fn no_photos(_: &str) -> usize {
    0
}

fn item(key: &str, room: &str, desc: &str, actioned: bool) -> InspectionItemInput {
    InspectionItemInput {
        client_key: key.to_string(),
        room: room.to_string(),
        description: desc.to_string(),
        actioned,
        sort_order: None,
    }
}

fn draft(items: Vec<InspectionItemInput>) -> InspectionDraftInput {
    InspectionDraftInput {
        inspector: String::new(),
        inspection_type: String::new(),
        stage: String::new(),
        observations: String::new(),
        weather_data: None,
        revision: 0,
        items,
    }
}

// ── R7: ETO numbering ───────────────────────────────────────────────────────

#[test]
fn the_first_eto_on_a_job_is_number_one() {
    // The sequence upsert inserts next_number = 2 and returns 1.
    assert_eq!(EtoNumber(1).po_number("J-100", JobId(7)), "J-100-01");
}

#[test]
fn eto_numbers_are_padded_to_two_digits() {
    assert_eq!(EtoNumber(9).po_number("J-100", JobId(7)), "J-100-09");
    assert_eq!(EtoNumber(10).po_number("J-100", JobId(7)), "J-100-10");
}

#[test]
fn three_digit_eto_numbers_are_not_truncated() {
    // padStart only ever adds characters. Truncating here would make the
    // 100th ETO on a job collide with the 10th.
    assert_eq!(EtoNumber(100).po_number("J-100", JobId(7)), "J-100-100");
    assert_eq!(EtoNumber(1234).po_number("J", JobId(7)), "J-1234");
}

#[test]
fn a_blank_job_number_falls_back_to_the_job_id() {
    assert_eq!(EtoNumber(3).po_number("", JobId(42)), "42-03");
    assert_eq!(EtoNumber(3).po_number("   ", JobId(42)), "42-03");
}

#[test]
fn the_job_number_is_trimmed_before_use() {
    assert_eq!(EtoNumber(1).po_number("  J-1  ", JobId(9)), "J-1-01");
}

#[test]
fn an_eto_needs_a_job_a_reason_and_details() {
    let ok = EtoInput {
        job_id: JobId(1),
        reason: "client changed the spec".into(),
        details: "add two power points".into(),
    };
    assert!(ok.validate().is_ok());

    let blank_reason = EtoInput {
        reason: "   ".into(),
        ..ok.clone()
    };
    assert!(blank_reason.validate().is_err());

    let blank_details = EtoInput {
        details: String::new(),
        ..ok.clone()
    };
    assert!(blank_details.validate().is_err());

    let bad_job = EtoInput {
        job_id: JobId(0),
        ..ok
    };
    assert!(bad_job.validate().is_err());
}

#[test]
fn the_eto_note_keeps_the_internal_reason_labelled() {
    let input = EtoInput {
        job_id: JobId(1),
        reason: "变更 client asked".into(),
        details: "two extra GPOs".into(),
    };
    let job = JobHeader {
        job_number: "J-100".into(),
        name: "Smith Residence".into(),
        address: "12 Rose St".into(),
    };
    let note = eto_note("J-100-01", "Sarah", &job, &input);

    // The email path redacts on this exact heading.
    assert!(note.contains("INTERNAL REASON — DO NOT EMAIL OR SHARE"));
    assert!(note.starts_with("ETO J-100-01 — EXTRA TO ORDER"));
    assert!(note.contains("PURCHASE ORDER: J-100-01"));
    assert!(note.contains("Job: J-100 — Smith Residence — 12 Rose St"));
    assert!(note.ends_with("Manager approval required before sharing."));
}

#[test]
fn the_job_line_skips_blank_parts() {
    let job = JobHeader {
        job_number: "J-1".into(),
        name: String::new(),
        address: "  ".into(),
    };
    assert_eq!(job.line(), "J-1");
}

#[test]
fn an_eto_number_round_trips_through_its_note() {
    let input = EtoInput {
        job_id: JobId(1),
        reason: "r".into(),
        details: "d".into(),
    };
    let note = eto_note("J-100-07", "Sarah", &JobHeader::default(), &input);
    assert_eq!(parse_eto_number(&note, 55), "J-100-07");
    assert_eq!(parse_eto_raised_by(note.as_str()).as_deref(), Some("Sarah"));
}

#[test]
fn an_unparseable_eto_note_falls_back_to_the_note_id() {
    assert_eq!(parse_eto_number("something else entirely", 55), "ETO-55");
    assert_eq!(parse_eto_raised_by("nothing here"), None);
}

// ── inspection drafts ───────────────────────────────────────────────────────

#[test]
fn an_empty_draft_carries_no_action_status() {
    // Otherwise every started-but-untouched draft would inflate the
    // dashboard's outstanding-actions count.
    let d = draft(vec![]);
    assert_eq!(inspection_action_status(&d.retained(), &no_photos), None);
}

#[test]
fn a_draft_of_only_blank_rows_still_carries_none() {
    let d = draft(vec![item("a", "", "", false), item("b", "  ", "  ", false)]);
    assert_eq!(inspection_action_status(&d.retained(), &no_photos), None);
}

#[test]
fn one_unticked_item_makes_the_whole_note_an_action() {
    let d = draft(vec![
        item("a", "Kitchen", "chip in benchtop", true),
        item("b", "Bath", "grout missing", false),
    ]);
    assert_eq!(
        inspection_action_status(&d.retained(), &no_photos),
        Some(ActionStatus::Action)
    );
}

#[test]
fn every_item_ticked_completes_the_note() {
    let d = draft(vec![item("a", "Kitchen", "chip", true)]);
    assert_eq!(
        inspection_action_status(&d.retained(), &no_photos),
        Some(ActionStatus::Completed)
    );
}

#[test]
fn a_row_with_only_a_photo_counts_as_filled() {
    let d = draft(vec![item("a", "", "", false)]);
    let with_photo = |k: &str| usize::from(k == "a");
    assert_eq!(
        inspection_action_status(&d.retained(), &with_photo),
        Some(ActionStatus::Action)
    );
}

#[test]
fn duplicate_client_keys_are_a_400_not_a_database_conflict() {
    let mut d = draft(vec![
        item("a", "Kitchen", "x", false),
        item("a", "Bath", "y", false),
    ]);
    d.revision = 0;
    let err = d.validate().unwrap_err();
    assert!(matches!(err, DomainError::Invalid { field, .. } if field == "items.clientKey"));
}

#[test]
fn a_blank_client_key_is_rejected() {
    let d = draft(vec![item("  ", "Kitchen", "x", false)]);
    assert!(d.validate().is_err());
}

#[test]
fn an_over_long_client_key_is_rejected_before_the_varchar_truncates_it() {
    let d = draft(vec![item(&"k".repeat(101), "Kitchen", "x", false)]);
    assert!(d.validate().is_err());
}

#[test]
fn a_negative_revision_is_rejected() {
    let mut d = draft(vec![]);
    d.revision = -1;
    assert!(d.validate().is_err());
}

#[test]
fn the_entry_summary_appends_only_the_parts_that_are_present() {
    assert_eq!(inspection_entry_summary("", ""), "Site Inspection");
    assert_eq!(
        inspection_entry_summary("Handover", ""),
        "Site Inspection — Handover"
    );
    assert_eq!(
        inspection_entry_summary("Handover", "Level 2"),
        "Site Inspection — Handover — Level 2"
    );
    assert_eq!(
        inspection_entry_summary("  ", " Level 2 "),
        "Site Inspection — Level 2"
    );
}

#[test]
fn an_untouched_draft_says_so_in_the_note() {
    let d = draft(vec![]);
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &no_photos,
    );
    assert!(note.contains("Draft started — no inspection items entered yet."));
    assert!(note.contains("INSPECTION ITEMS (0 total, 0 actioned):"));
    assert!(note.contains("📅 2026-03-04"));
}

#[test]
fn the_note_numbers_only_the_filled_rows() {
    // The client keeps a trailing blank row for the next entry; it must not
    // be numbered, or every save would renumber the list.
    let d = draft(vec![
        item("a", "Kitchen", "chip", true),
        item("b", "", "", false),
        item("c", "Bath", "grout", false),
    ]);
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &no_photos,
    );
    assert!(note.contains("INSPECTION ITEMS (2 total, 1 actioned):"));
    assert!(note.contains("#1 | Kitchen | chip | ✅ Actioned"));
    assert!(note.contains("#2 | Bath | grout | ⬜ Pending"));
    assert!(!note.contains("#3"));
}

#[test]
fn a_photo_count_appears_beside_the_description() {
    let d = draft(vec![item("a", "Kitchen", "chip", false)]);
    let two = |_: &str| 2usize;
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &two,
    );
    assert!(note.contains("#1 | Kitchen | chip [2 photo(s)] | ⬜ Pending"));
}

#[test]
fn an_empty_cell_renders_as_an_em_dash_so_the_columns_line_up() {
    let d = draft(vec![item("a", "", "chip", false)]);
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &no_photos,
    );
    assert!(note.contains("#1 | — | chip | ⬜ Pending"));
}

#[test]
fn the_detail_line_is_omitted_entirely_when_nothing_is_set() {
    let d = draft(vec![]);
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &no_photos,
    );
    let lines: Vec<&str> = note.lines().collect();
    // 📋 title, 📅 date, blank, then the item heading -- no detail row.
    assert_eq!(lines[2], "");
    assert!(lines[3].starts_with("INSPECTION ITEMS"));
}

#[test]
fn the_detail_line_joins_what_is_set() {
    let mut d = draft(vec![]);
    d.inspector = "Sarah".into();
    d.stage = "Level 2".into();
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &no_photos,
    );
    assert!(note.contains("👤 Inspector: Sarah  |  📍 Stage/Area: Level 2"));
    // Type was blank, so its label never appears.
    assert!(!note.contains("🏷 Type:"));
}

#[test]
fn observations_are_appended_under_their_own_heading() {
    let mut d = draft(vec![]);
    d.observations = "  slab looked wet  ".into();
    let note = inspection_note(
        &d,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        &no_photos,
    );
    assert!(note.ends_with("\n📝 OBSERVATIONS:\nslab looked wet"));
}

#[test]
fn a_stale_revision_is_a_conflict_the_user_can_act_on() {
    let e = stale_revision();
    assert!(matches!(e, DomainError::Conflict(ref m) if m.contains("Reload it before continuing")));
}
