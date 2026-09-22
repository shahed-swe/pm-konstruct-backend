//! Outgoing-email rules: who may be sent to, and what must not be included.

use crate::error::{DomainError, DomainResult};
use crate::identity::looks_like_email;

/// The most people one message may go to.
///
/// The send path is authenticated and the recipients are restricted to
/// colleagues, so this is not the thing stopping abuse -- it stops a mistake
/// (a pasted address list) from becoming a hundred messages.
pub const MAX_RECIPIENTS: usize = 10;

/// Normalises, deduplicates and checks a recipient list.
///
/// Deduplication happens **before** the cap, so a body that repeats the same
/// address eleven times is one recipient rather than a rejection.
pub fn normalise_recipients(raw: &[String]) -> DomainResult<Vec<String>> {
    if raw.is_empty() {
        return Err(DomainError::invalid(
            "to",
            "At least one recipient email is required",
        ));
    }

    let mut seen = Vec::new();
    for address in raw {
        let normalised = address.trim().to_lowercase();
        if !seen.contains(&normalised) {
            seen.push(normalised);
        }
    }

    if seen.len() > MAX_RECIPIENTS {
        return Err(DomainError::invalid(
            "to",
            format!(
                "Too many recipients. A maximum of {MAX_RECIPIENTS} recipients are allowed per email."
            ),
        ));
    }

    let invalid: Vec<&str> = seen
        .iter()
        .filter(|a| !looks_like_email(a))
        .map(String::as_str)
        .collect();
    if !invalid.is_empty() {
        return Err(DomainError::invalid(
            "to",
            format!("Invalid email address(es): {}", invalid.join(", ")),
        ));
    }

    Ok(seen)
}

/// Rejects recipients who are not entitled to the job.
///
/// Restricting the list to colleagues with access is what stops an
/// authenticated user from turning the diary-email feature into a relay for
/// arbitrary addresses, and stops a site diary reaching someone who is not on
/// that job.
pub fn assert_recipients_allowed(recipients: &[String], allowed: &[String]) -> DomainResult<()> {
    let unauthorised: Vec<&str> = recipients
        .iter()
        .filter(|r| !allowed.iter().any(|a| a.eq_ignore_ascii_case(r)))
        .map(String::as_str)
        .collect();

    if unauthorised.is_empty() {
        return Ok(());
    }
    Err(DomainError::Forbidden(
        "One or more recipients do not have access to this job",
    ))
}

/// The heading that marks the part of an ETO note that must not be shared.
const INTERNAL_HEADING: &str = "INTERNAL REASON — DO NOT EMAIL OR SHARE";

/// The heading the shareable part resumes at.
const RESUME_HEADING: &str = "WHAT IS REQUIRED";

/// Strips the internal justification from an ETO note before it is emailed.
///
/// An ETO records why the extra work is being charged, which is the builder's
/// commercial position, alongside what the work actually is, which the client
/// is entitled to. The note keeps both under labelled headings; this removes
/// the first before the note leaves the company.
///
/// Anything that is not an ETO note passes through untouched.
#[must_use]
pub fn redact_internal_eto_reason(content: &str) -> String {
    if !is_eto_note(content) {
        return content.to_string();
    }

    let Some(start) = content.find(INTERNAL_HEADING) else {
        return content.to_string();
    };

    // Resume at the next shareable heading. Without one the rest of the note
    // is unlabelled, so everything from the heading on is dropped -- failing
    // toward saying less rather than more.
    let after = &content[start + INTERNAL_HEADING.len()..];
    let kept_tail = after
        .find(RESUME_HEADING)
        .map_or("", |offset| &after[offset..]);

    let mut out = content[..start].trim_end().to_string();
    if !kept_tail.is_empty() {
        out.push_str("\n\n");
        out.push_str(kept_tail);
    }
    out.trim().to_string()
}

/// Does this note look like an ETO?
///
/// Matches the legacy `^ETO\s+.+?—\s*EXTRA TO ORDER` on any line.
fn is_eto_note(content: &str) -> bool {
    content.lines().any(|line| {
        line.strip_prefix("ETO ").is_some_and(|rest| {
            rest.split_once('—').is_some_and(|(number, tail)| {
                !number.trim().is_empty() && tail.trim_start().starts_with("EXTRA TO ORDER")
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eto_note() -> String {
        [
            "ETO J-100-01 — EXTRA TO ORDER",
            "PURCHASE ORDER: J-100-01",
            "Raised by: Sarah",
            "Job: J-100 — Smith Residence",
            "",
            INTERNAL_HEADING,
            "The client will wear this; we quoted low to win the job.",
            "",
            RESUME_HEADING,
            "Two additional power points in the kitchen.",
            "",
            "Manager approval required before sharing.",
        ]
        .join("\n")
    }

    #[test]
    fn the_internal_reason_is_removed_from_an_eto_note() {
        let out = redact_internal_eto_reason(&eto_note());
        assert!(!out.contains(INTERNAL_HEADING), "{out}");
        assert!(!out.contains("we quoted low"), "{out}");
    }

    #[test]
    fn the_shareable_part_survives_intact() {
        let out = redact_internal_eto_reason(&eto_note());
        assert!(out.contains("ETO J-100-01 — EXTRA TO ORDER"), "{out}");
        assert!(out.contains("PURCHASE ORDER: J-100-01"), "{out}");
        assert!(out.contains(RESUME_HEADING), "{out}");
        assert!(out.contains("Two additional power points"), "{out}");
        assert!(out.contains("Manager approval required"), "{out}");
    }

    #[test]
    fn an_ordinary_note_passes_through_untouched() {
        let note = "Poured the slab today.\nWeather held.";
        assert_eq!(redact_internal_eto_reason(note), note);
    }

    #[test]
    fn a_note_merely_mentioning_the_heading_is_not_treated_as_an_eto() {
        // Only a note that opens with the ETO banner is redacted; otherwise a
        // note quoting the heading would be mangled.
        let note = format!("General note\n{INTERNAL_HEADING}\nsomething");
        assert_eq!(redact_internal_eto_reason(&note), note);
    }

    #[test]
    fn an_eto_note_with_no_resume_heading_drops_everything_after() {
        // Nothing labels the rest as shareable, so it fails toward saying
        // less rather than more.
        let note = format!("ETO J-1-01 — EXTRA TO ORDER\n\n{INTERNAL_HEADING}\nsecret terms");
        let out = redact_internal_eto_reason(&note);
        assert!(out.contains("EXTRA TO ORDER"), "{out}");
        assert!(!out.contains("secret terms"), "{out}");
        assert!(!out.contains(INTERNAL_HEADING), "{out}");
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact_internal_eto_reason(&eto_note());
        assert_eq!(redact_internal_eto_reason(&once), once);
    }

    #[test]
    fn recipients_are_lowercased_and_trimmed() {
        let out = normalise_recipients(&[" Site@Example.COM ".into()]).unwrap();
        assert_eq!(out, vec!["site@example.com"]);
    }

    #[test]
    fn duplicates_collapse_before_the_cap_applies() {
        // Eleven copies of one address is one recipient, not a rejection.
        let same = vec!["a@b.com".to_string(); MAX_RECIPIENTS + 1];
        assert_eq!(normalise_recipients(&same).unwrap().len(), 1);
    }

    #[test]
    fn duplicates_differing_only_in_case_also_collapse() {
        let out = normalise_recipients(&["A@B.com".into(), "a@b.com".into()]).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn too_many_distinct_recipients_are_rejected() {
        let many: Vec<String> = (0..=MAX_RECIPIENTS)
            .map(|i| format!("a{i}@b.com"))
            .collect();
        let e = normalise_recipients(&many).unwrap_err();
        assert!(matches!(e, DomainError::Invalid { field, .. } if field == "to"));
    }

    #[test]
    fn exactly_the_cap_is_allowed() {
        let many: Vec<String> = (0..MAX_RECIPIENTS).map(|i| format!("a{i}@b.com")).collect();
        assert_eq!(normalise_recipients(&many).unwrap().len(), MAX_RECIPIENTS);
    }

    #[test]
    fn an_empty_list_is_rejected() {
        assert!(normalise_recipients(&[]).is_err());
    }

    #[test]
    fn a_malformed_address_is_named_in_the_error() {
        let e = normalise_recipients(&["good@b.com".into(), "nope".into()])
            .unwrap_err()
            .to_string();
        assert!(e.contains("nope"), "{e}");
    }

    #[test]
    fn only_colleagues_with_job_access_may_be_sent_to() {
        // This is what stops the feature being used as a relay.
        let allowed = vec!["a@b.com".to_string(), "c@d.com".to_string()];
        assert!(assert_recipients_allowed(&["a@b.com".into()], &allowed).is_ok());
        assert!(assert_recipients_allowed(&["outsider@x.com".into()], &allowed).is_err());
    }

    #[test]
    fn the_allowlist_comparison_ignores_case() {
        let allowed = vec!["Site@Example.com".to_string()];
        assert!(assert_recipients_allowed(&["site@example.com".into()], &allowed).is_ok());
    }

    #[test]
    fn one_unauthorised_recipient_fails_the_whole_send() {
        // Partial delivery would be worse: the sender would believe everyone
        // received it.
        let allowed = vec!["a@b.com".to_string()];
        assert!(
            assert_recipients_allowed(&["a@b.com".into(), "outsider@x.com".into()], &allowed)
                .is_err()
        );
    }

    #[test]
    fn an_empty_allowlist_permits_nobody() {
        assert!(assert_recipients_allowed(&["a@b.com".into()], &[]).is_err());
    }
}
