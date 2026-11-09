use super::*;

fn opts(case: bool, word: bool, regex: bool) -> SearchOptions {
    SearchOptions {
        case_sensitive: case,
        whole_word: word,
        regex,
    }
}

#[test]
fn snapshot_search_finds_a_match_across_piece_boundaries() {
    crate::test_fixture::with_locale(|| {
        let prefix = "a".repeat(enzui_core::chunked_text::TARGET_CHUNK_BYTES - 3);
        let text = format!("{prefix}Needle tail");
        let store = enzui_core::chunked_text::TextStore::from_string(text);
        let result = find_all_snapshot_cancellable(
            store.current(),
            "needle",
            SearchOptions::default(),
            &crate::worker::Cancel::default(),
        );
        let Cancellable::Done(Ok(scan)) = result else {
            panic!("snapshot search did not finish")
        };
        assert_eq!(scan.total, 1);
        assert_eq!(scan.matches[0].start, prefix.len());
        assert_eq!(scan.matches[0].end, prefix.len() + "Needle".len());
    });
}

#[test]
fn snapshot_whole_word_search_is_exact_at_the_one_mib_window_boundary() {
    crate::test_fixture::with_locale(|| {
        let prefix = format!("{} ", "x".repeat(CANCEL_CHUNK_BYTES - 3));
        let crossing_start = prefix.len();
        assert!(crossing_start < CANCEL_CHUNK_BYTES);
        assert!(crossing_start + "needle".len() > CANCEL_CHUNK_BYTES);
        let text = format!("{prefix}needle sword needleish final needle");
        let store = enzui_core::chunked_text::TextStore::from_string(text);
        let cancel = crate::worker::Cancel::default();
        let mut largest = 0;
        let result = find_all_snapshot_cancellable_observed(
            store.current(),
            "needle",
            opts(true, true, false),
            &cancel,
            |bytes| largest = largest.max(bytes),
        );
        let Cancellable::Done(Ok(scan)) = result else {
            panic!("whole-word snapshot search did not finish")
        };
        assert_eq!(scan.total, 2);
        assert_eq!(scan.matches[0].start, crossing_start);
        assert_eq!(
            scan.matches[1].start,
            store.current().len() - "needle".len()
        );
        assert!(largest >= CANCEL_CHUNK_BYTES);
        assert!(largest <= CANCEL_CHUNK_BYTES + "needle".len() * 4 + 8);
        assert!(largest < store.current().len());
    });
}
