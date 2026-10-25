//! Separate process: verifies the first interactive use, before any synchronous dictionary load.
#[test]
fn interactive_loading_yields_and_invalidates_cached_proofing() {
    use wordcraft_proof::{dictionary_generation, hu, is_correct_for, missing_dictionary};
    let before = dictionary_generation();
    hu::poll_interactive_loading();
    assert!(!hu::is_loading(), "opening an English UI must not allocate the Hungarian dictionary");
    let _ = is_correct_for("hejesírás", Some("hu-HU"));
    assert!(hu::is_loading(), "first word starts loading without parsing all entries");
    assert_eq!(missing_dictionary(Some("hu-HU")), Some("hu_HU"));
    let mut frames = 0;
    while hu::is_loading() && frames < 1000 {
        hu::poll_interactive_loading();
        frames += 1;
    }
    assert!(frames > 1 && frames < 1000, "{frames} batches");
    assert!(!hu::is_loading());
    assert!(dictionary_generation() > before, "layout must discard the unchecked first frame");
    assert!(wordcraft_proof::dictionary_load_error(wordcraft_proof::ProofLang::Hu).is_none());
    assert_eq!(missing_dictionary(Some("hu-HU")), None);
    assert!(!is_correct_for("hejesírás", Some("hu-HU")));
    for word in ["magyar", "árvíztűrő", "tükörfúrógép", "Budapest", "házainkban"] {
        assert!(is_correct_for(word, Some("hu-HU")), "{word}");
    }
}
