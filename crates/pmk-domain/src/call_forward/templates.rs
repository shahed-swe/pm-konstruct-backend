//! Call-forward templates: a saved programme, reusable across jobs.

use crate::call_forward::ItemType;
use crate::error::{DomainError, DomainResult};
use crate::ids::CallForwardTemplateId;

/// One line of a saved programme.
///
/// Identified by `local_id` rather than a database id: the template outlives
/// the job it was captured from, and those rows may be long gone by the time
/// it is applied somewhere else.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateItem {
    pub local_id: i32,
    /// The `local_id` of this item's parent, if it has one.
    pub local_parent_id: Option<i32>,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug, Clone)]
pub struct Template {
    pub id: CallForwardTemplateId,
    pub name: String,
    pub description: Option<String>,
    pub items: Vec<TemplateItem>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A template cannot hold more lines than a job's programme reasonably would.
pub const MAX_TEMPLATE_ITEMS: usize = 2000;

/// Checks a template can actually be applied.
///
/// Items are inserted in order and each child is pointed at the row its parent
/// produced, so a parent appearing *after* its child has no id to give. The
/// legacy resolved that to `null` and silently flattened the item to the top
/// level; here it is a 400 that says what is wrong.
pub fn validate_items(items: &[TemplateItem]) -> DomainResult<()> {
    if items.len() > MAX_TEMPLATE_ITEMS {
        return Err(DomainError::invalid(
            "items",
            format!("a template cannot hold more than {MAX_TEMPLATE_ITEMS} items"),
        ));
    }

    let mut seen: Vec<i32> = Vec::with_capacity(items.len());
    for item in items {
        if item.title.trim().is_empty() {
            return Err(DomainError::invalid(
                "items.title",
                "every template item needs a title",
            ));
        }
        if ItemType::parse(&item.item_type).is_none() {
            return Err(DomainError::invalid(
                "items.itemType",
                "must be one of HEADER, STAGE_CLAIM, TASK",
            ));
        }
        if seen.contains(&item.local_id) {
            return Err(DomainError::invalid(
                "items.localId",
                "template items must have distinct local ids",
            ));
        }
        if let Some(parent) = item.local_parent_id {
            if parent == item.local_id {
                return Err(DomainError::invalid(
                    "items.localParentId",
                    "an item cannot be its own parent",
                ));
            }
            if !seen.contains(&parent) {
                return Err(DomainError::invalid(
                    "items.localParentId",
                    "a parent must appear before the item that refers to it",
                ));
            }
        }
        seen.push(item.local_id);
    }
    Ok(())
}

/// One row of a job's programme, as capture reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammeRow {
    pub id: i32,
    pub parent_id: Option<i32>,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    /// Ordering among siblings. Not a global position: a tree UI numbers
    /// each parent's children from one, so two items at different depths can
    /// share a value.
    pub sort_order: i32,
}

/// Walks the programme parent-first, depth-first.
///
/// Templates require every parent to appear before its children, and reading
/// the rows in `sort_order` alone does not guarantee that -- a board that
/// numbers each parent's children from one will put a child at 1 and its
/// parent at 7. That produced a capture that failed validation with "a
/// parent must appear before the item that refers to it", from a programme
/// that was perfectly valid on screen.
///
/// An item whose parent is not in the set is treated as a root rather than
/// dropped, so a partial programme still captures everything it has.
#[must_use]
pub fn in_tree_order(rows: &[ProgrammeRow]) -> Vec<&ProgrammeRow> {
    use std::collections::{HashMap, HashSet};

    let present: HashSet<i32> = rows.iter().map(|r| r.id).collect();
    let mut children: HashMap<Option<i32>, Vec<&ProgrammeRow>> = HashMap::new();
    for row in rows {
        let parent = row.parent_id.filter(|p| present.contains(p));
        children.entry(parent).or_default().push(row);
    }
    for list in children.values_mut() {
        list.sort_by_key(|r| (r.sort_order, r.id));
    }

    // Iterative rather than recursive: a cycle in `parent_id` would blow the
    // stack, and `parent_id` had no foreign key in the legacy schema.
    let mut out: Vec<&ProgrammeRow> = Vec::with_capacity(rows.len());
    let mut emitted: HashSet<i32> = HashSet::new();
    let mut stack: Vec<&ProgrammeRow> = children
        .get(&None)
        .map(|roots| roots.iter().rev().copied().collect())
        .unwrap_or_default();

    while let Some(row) = stack.pop() {
        if !emitted.insert(row.id) {
            continue;
        }
        out.push(row);
        if let Some(kids) = children.get(&Some(row.id)) {
            for kid in kids.iter().rev() {
                stack.push(kid);
            }
        }
    }

    // Anything left is part of a cycle. Appending it keeps the capture
    // lossless; `validate_items` then reports the cycle rather than silently
    // producing a template with items missing.
    for row in rows {
        if !emitted.contains(&row.id) {
            out.push(row);
        }
    }

    out
}

/// Captures a job's programme as template items.
///
/// Database ids are replaced by 1-based positions, so the template is
/// self-contained and survives the original job being archived.
#[must_use]
pub fn capture(rows: &[ProgrammeRow]) -> Vec<TemplateItem> {
    let ordered = in_tree_order(rows);
    let position = |idx: usize| i32::try_from(idx).unwrap_or(i32::MAX);
    let positions: std::collections::HashMap<i32, i32> = ordered
        .iter()
        .enumerate()
        .map(|(idx, r)| (r.id, position(idx) + 1))
        .collect();

    ordered
        .iter()
        .enumerate()
        .map(|(idx, r)| TemplateItem {
            local_id: position(idx) + 1,
            // A parent outside the captured set becomes top-level: it cannot
            // be referred to, and dropping the item entirely would lose work
            // the template is meant to preserve.
            local_parent_id: r.parent_id.and_then(|p| positions.get(&p).copied()),
            title: r.title.clone(),
            item_type: r.item_type.clone(),
            supplier_trade: r.supplier_trade.clone(),
            sort_order: position(idx),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: i32,
        parent_id: Option<i32>,
        title: &str,
        item_type: &str,
        supplier_trade: Option<&str>,
    ) -> ProgrammeRow {
        ProgrammeRow {
            id,
            parent_id,
            title: title.into(),
            item_type: item_type.into(),
            supplier_trade: supplier_trade.map(Into::into),
            sort_order: id,
        }
    }

    fn ordered(id: i32, parent_id: Option<i32>, sort_order: i32) -> ProgrammeRow {
        ProgrammeRow {
            sort_order,
            ..row(id, parent_id, &format!("Item {id}"), "TASK", None)
        }
    }

    #[test]
    fn a_capture_puts_every_parent_before_its_children() {
        // The board numbers each parent's children from one, so a child can
        // have a lower sort order than its parent. Reading the rows in that
        // order made the capture fail validation on a programme that was
        // perfectly valid on screen.
        let rows = vec![
            ordered(10, None, 7),
            ordered(11, Some(10), 1),
            ordered(12, Some(10), 2),
            ordered(20, None, 8),
        ];

        let items = capture(&rows);
        assert!(validate_items(&items).is_ok());
        assert_eq!(
            items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            ["Item 10", "Item 11", "Item 12", "Item 20"],
        );
        assert_eq!(items[1].local_parent_id, Some(1));
        assert_eq!(items[2].local_parent_id, Some(1));
    }

    #[test]
    fn an_item_whose_parent_is_missing_is_captured_as_a_root() {
        // A partial programme, or a parent deleted from under it. Dropping
        // the item would lose the work the template exists to preserve.
        let rows = vec![ordered(5, Some(999), 1)];
        let items = capture(&rows);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].local_parent_id, None);
    }

    #[test]
    fn a_cycle_is_reported_rather_than_looping_forever() {
        // `parent_id` had no foreign key in the legacy schema, so two rows
        // can point at each other. The capture must terminate and the result
        // must be rejected, not silently lose items.
        let rows = vec![ordered(1, Some(2), 1), ordered(2, Some(1), 2)];
        let items = capture(&rows);
        assert_eq!(items.len(), 2, "no item may be dropped");
        assert!(validate_items(&items).is_err(), "the cycle must be reported");
    }

    fn item(local: i32, parent: Option<i32>) -> TemplateItem {
        TemplateItem {
            local_id: local,
            local_parent_id: parent,
            title: format!("Item {local}"),
            item_type: "TASK".into(),
            supplier_trade: None,
            sort_order: local - 1,
        }
    }

    #[test]
    fn a_flat_template_is_valid() {
        assert!(validate_items(&[item(1, None), item(2, None)]).is_ok());
    }

    #[test]
    fn a_child_after_its_parent_is_valid() {
        assert!(validate_items(&[item(1, None), item(2, Some(1))]).is_ok());
    }

    #[test]
    fn a_child_before_its_parent_is_rejected() {
        // Applying inserts in order, so the parent has no id to give yet. The
        // legacy silently flattened the item to the top level.
        let e = validate_items(&[item(2, Some(1)), item(1, None)]).unwrap_err();
        assert!(matches!(e, DomainError::Invalid { field, .. } if field == "items.localParentId"));
    }

    #[test]
    fn an_item_cannot_parent_itself() {
        assert!(validate_items(&[item(1, Some(1))]).is_err());
    }

    #[test]
    fn a_parent_that_is_not_in_the_template_is_rejected() {
        assert!(validate_items(&[item(1, Some(99))]).is_err());
    }

    #[test]
    fn duplicate_local_ids_are_rejected() {
        assert!(validate_items(&[item(1, None), item(1, None)]).is_err());
    }

    #[test]
    fn a_blank_title_is_rejected() {
        let mut i = item(1, None);
        i.title = "   ".into();
        assert!(validate_items(&[i]).is_err());
    }

    #[test]
    fn an_unknown_item_type_is_rejected() {
        let mut i = item(1, None);
        i.item_type = "MILESTONE".into();
        assert!(validate_items(&[i]).is_err());
        for t in ["HEADER", "STAGE_CLAIM", "TASK"] {
            let mut ok = item(1, None);
            ok.item_type = t.into();
            assert!(validate_items(&[ok]).is_ok(), "{t}");
        }
    }

    #[test]
    fn an_oversized_template_is_rejected() {
        let over = i32::try_from(MAX_TEMPLATE_ITEMS).unwrap_or(i32::MAX - 1) + 1;
        let many: Vec<_> = (1..=over).map(|i| item(i, None)).collect();
        assert!(validate_items(&many).is_err());
    }

    #[test]
    fn an_empty_template_is_valid_but_applies_nothing() {
        assert!(validate_items(&[]).is_ok());
    }

    #[test]
    fn capturing_replaces_database_ids_with_positions() {
        // Ids 50 and 51 in the job become 1 and 2 in the template, so the
        // template survives the job being archived.
        let rows = vec![
            row(50, None, "Frame", "HEADER", None),
            row(51, Some(50), "Frame inspection", "TASK", Some("Carpenter")),
        ];
        let items = capture(&rows);
        assert_eq!(items[0].local_id, 1);
        assert_eq!(items[0].local_parent_id, None);
        assert_eq!(items[1].local_id, 2);
        assert_eq!(items[1].local_parent_id, Some(1));
        assert_eq!(items[1].supplier_trade.as_deref(), Some("Carpenter"));
        assert!(
            validate_items(&items).is_ok(),
            "capture must produce a valid template"
        );
    }

    #[test]
    fn a_parent_outside_the_captured_set_becomes_top_level() {
        let rows = vec![row(51, Some(999), "Orphan", "TASK", None)];
        let items = capture(&rows);
        assert_eq!(items[0].local_parent_id, None);
        assert!(validate_items(&items).is_ok());
    }

    #[test]
    fn capture_preserves_the_order_it_was_given() {
        let rows: Vec<_> = (1..=5)
            .map(|i| ProgrammeRow {
                id: i * 10,
                parent_id: None,
                title: format!("T{i}"),
                item_type: "TASK".into(),
                supplier_trade: None,
                sort_order: i,
            })
            .collect();
        let items = capture(&rows);
        assert_eq!(
            items.iter().map(|i| i.sort_order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
    }
}
