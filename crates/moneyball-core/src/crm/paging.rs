//! Paging policy - pure, testable without a network: which records
//! stay (client-side date cutover) and when the page loop stops
//! (totalElements > window cutover > short page). Split from fetch.rs
//! (size cap); fetch.rs owns the HTTP loop that consults these.

use serde_json::Value;

use super::source::{self, PagingMode};

/// Drop records whose delivery (read via `map.delivery`) is older than
/// `from_epoch`. Records with unparseable or missing delivery are KEPT
/// so the validator can surface them with a precise per-row error.
pub(super) fn keep_records(batch: &[Value], map: &source::MapSpec, from_epoch: i64) -> Vec<Value> {
    batch
        .iter()
        .filter(|rec| {
            source::get_path(rec, &map.delivery)
                .and_then(super::parse_epoch)
                .map(|e| e >= from_epoch)
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

/// Pure per-page stop decision. Returns true when the page loop should
/// break after the just-pulled page. Factored out so it's testable
/// without a network.
pub(super) fn should_stop(
    total_pulled: usize,
    total_elements: Option<usize>,
    in_window: usize,
    page_kept: usize,
    page_n: usize,
    page_size: usize,
    paging_mode: PagingMode,
) -> bool {
    if let Some(te) = total_elements {
        if total_pulled >= te {
            return true;
        }
    }
    // Cutover: page is full AND zero in-window records AND we've
    // already seen in-window records on a prior page. We're past the
    // window - all remaining pages will be older too.
    if page_n == page_size && page_kept == 0 && in_window > 0 {
        return true;
    }
    if paging_mode == PagingMode::None || page_n < page_size {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crm::source::{MapSpec, PagingMode};
    use std::time::Duration;

    #[test]
    fn elapsed_uses_ms_seconds_and_minutes() {
        assert_eq!(
            super::super::fetch::fmt_elapsed(Duration::from_millis(0)),
            "0ms"
        );
        assert_eq!(
            super::super::fetch::fmt_elapsed(Duration::from_millis(340)),
            "340ms"
        );
        assert_eq!(
            super::super::fetch::fmt_elapsed(Duration::from_millis(1_200)),
            "1.2s"
        );
        assert_eq!(
            super::super::fetch::fmt_elapsed(Duration::from_secs(59)),
            "59.0s"
        );
        assert_eq!(
            super::super::fetch::fmt_elapsed(Duration::from_secs(60)),
            "1m00s"
        );
        assert_eq!(
            super::super::fetch::fmt_elapsed(Duration::from_secs(125)),
            "2m05s"
        );
    }

    fn map_with(delivery: &str) -> MapSpec {
        MapSpec {
            root: "content".into(),
            ad_id: "adId.adId".into(),
            stage: "stage.name".into(),
            delivery: delivery.into(),
            funnel: String::new(),
            stage_map: Default::default(),
        }
    }

    fn rec(delivery: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "createdAt": delivery, "adId": { "adId": "111" }, "stage": { "name": "Fresh" } })
    }

    #[test]
    fn keep_records_drops_older_than_window_keeps_others() {
        let map = map_with("createdAt");
        let from = 1_700_000_000i64;
        let batch = vec![
            rec(from.into()),
            rec((from - 1).into()),
            rec((from + 60).into()),
        ];
        let kept = keep_records(&batch, &map, from);
        assert_eq!(kept.len(), 2, "exactly the boundary + newer survive");
        assert_eq!(kept[0]["createdAt"], from);
        assert_eq!(kept[1]["createdAt"], from + 60);
    }

    #[test]
    fn keep_records_keeps_unparseable_for_validator_to_surface() {
        let map = map_with("createdAt");
        let batch = vec![
            rec(serde_json::Value::String("not-a-date".into())),
            rec(serde_json::Value::Null),
        ];
        let kept = keep_records(&batch, &map, 1_700_000_000);
        assert_eq!(kept.len(), 2, "unparseable records pass through");
    }

    #[test]
    fn keep_records_walks_dotted_path() {
        let mut map = map_with("meta.delivery");
        map.delivery = "meta.delivery".into();
        let batch = vec![serde_json::json!({"meta": {"delivery": 1_700_000_000}})];
        let kept = keep_records(&batch, &map, 1_700_000_000);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn should_stop_respects_total_elements_first() {
        assert!(should_stop(
            100,
            Some(100),
            50,
            50,
            200,
            200,
            PagingMode::Page
        ));
        assert!(!should_stop(
            99,
            Some(100),
            50,
            50,
            200,
            200,
            PagingMode::Page
        ));
    }

    #[test]
    fn should_stop_cuts_over_after_paging_past_window() {
        // Full page, zero in-window, but we've seen in-window before.
        assert!(should_stop(400, None, 200, 0, 200, 200, PagingMode::Page));
        // Same shape, but the FIRST page is empty - don't cut, keep paging.
        assert!(!should_stop(0, None, 0, 0, 200, 200, PagingMode::Page));
    }

    #[test]
    fn should_stop_breaks_on_short_page_or_none_mode() {
        // Short page (last page of the set).
        assert!(should_stop(150, None, 150, 150, 50, 200, PagingMode::Page));
        // Single-shot mode.
        assert!(should_stop(200, None, 200, 200, 200, 200, PagingMode::None));
    }

    #[test]
    fn should_stop_continues_when_full_page_with_in_window_records() {
        // Normal in-flight page: keep going.
        assert!(!should_stop(
            200,
            Some(1000),
            200,
            200,
            200,
            200,
            PagingMode::Page
        ));
    }
}
