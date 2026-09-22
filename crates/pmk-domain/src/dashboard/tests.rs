use super::*;

#[test]
fn every_bucket_is_present_even_when_empty() {
    // The chart draws four bars; a missing bucket would shift the rest.
    let out = delay_severity(&[]);
    assert_eq!(out.len(), 4);
    assert_eq!(
        out.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["< 1 week", "1–2 weeks", "2–4 weeks", "> 4 weeks"]
    );
    assert!(out
        .iter()
        .all(|b| b.count == 0 && b.avg_days == 0 && b.max_days == 0));
}

#[test]
fn the_boundaries_are_inclusive_at_the_top() {
    // Exactly 7 days late is "< 1 week" -- the label reads loosely, but this
    // is the threshold the dashboard has always drawn.
    assert_eq!(DelayBucket::of(7), DelayBucket::UnderOneWeek);
    assert_eq!(DelayBucket::of(8), DelayBucket::OneToTwoWeeks);
    assert_eq!(DelayBucket::of(14), DelayBucket::OneToTwoWeeks);
    assert_eq!(DelayBucket::of(15), DelayBucket::TwoToFourWeeks);
    assert_eq!(DelayBucket::of(28), DelayBucket::TwoToFourWeeks);
    assert_eq!(DelayBucket::of(29), DelayBucket::OverFourWeeks);
}

#[test]
fn one_day_late_lands_in_the_first_bucket() {
    assert_eq!(DelayBucket::of(1), DelayBucket::UnderOneWeek);
}

#[test]
fn items_are_counted_summed_and_maxed_per_bucket() {
    let out = delay_severity(&[1, 3, 10, 10, 40]);
    assert_eq!(out[0].count, 2);
    assert_eq!(out[0].avg_days, 2); // (1+3)/2
    assert_eq!(out[0].max_days, 3);
    assert_eq!(out[1].count, 2);
    assert_eq!(out[1].avg_days, 10);
    assert_eq!(out[1].max_days, 10);
    assert_eq!(out[2].count, 0);
    assert_eq!(out[3].count, 1);
    assert_eq!(out[3].avg_days, 40);
    assert_eq!(out[3].max_days, 40);
}

#[test]
fn averages_round_to_the_nearest_day() {
    // 5/2 = 2.5 -> 3, matching Math.round.
    assert_eq!(delay_severity(&[2, 3])[0].avg_days, 3);
    // 7/2 = 3.5 -> 4
    assert_eq!(delay_severity(&[3, 4])[0].avg_days, 4);
    // 4/3 = 1.33 -> 1
    assert_eq!(delay_severity(&[1, 1, 2])[0].avg_days, 1);
}

#[test]
fn a_supervisor_seeing_no_jobs_short_circuits_every_count() {
    assert!(JobScope::Only(vec![]).is_empty());
    assert!(!JobScope::Only(vec![JobId(1)]).is_empty());
    // A manager is never "empty" -- All means the whole company.
    assert!(!JobScope::All.is_empty());
}

#[test]
fn only_a_restricted_scope_produces_an_id_list() {
    assert_eq!(JobScope::All.ids(), None);
    assert_eq!(
        JobScope::Only(vec![JobId(3), JobId(7)]).ids(),
        Some(vec![3, 7])
    );
}
