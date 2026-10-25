//! Offline Hungarian spelling using the bundled Magyar Ispell 1.9 dictionary (MPL-2.0).
//! Source, upstream revision and redistribution terms: `assets/spelling/hu/` and `NOTICE`.
//! Spellbook handles the compound/affix rules missing from our smaller affix engine.

mod encoding;
pub mod grammar;

use spellbook::Dictionary;
use std::sync::OnceLock;

const AFF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hu_HU.aff.deflate"));
const DIC: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hu_HU.dic.deflate"));
const MAX_WORD_BYTES: usize = 120;

/// Parse and validate a candidate dictionary before a maintainer replaces the bundled files.
/// The original bytes stay untouched on disk; only the parser's input is normalized.
pub fn parse_dictionary(aff: &[u8], dic: &[u8]) -> Result<Dictionary, String> {
    if aff.len() > 16 << 20 || dic.len() > 16 << 20 {
        return Err("Hungarian dictionary exceeds the 16 MiB per-file limit".into());
    }
    let aff = encoding::normalize_aff(aff)?;
    let dic = std::str::from_utf8(dic).map_err(|e| format!("Hungarian .dic: {e}"))?;
    Dictionary::new(&aff, dic).map_err(|e| format!("Hungarian dictionary: {e}"))
}

static DICT: OnceLock<Result<Dictionary, String>> = OnceLock::new();
static INPUTS: OnceLock<Result<(String, String), String>> = OnceLock::new();
static LOADER: std::sync::Mutex<Option<spellbook::DictionaryBuilder<'static>>> = std::sync::Mutex::new(None);
static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
std::thread_local! { static INTERACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Yielding mode is local to the UI thread; CLI and updater calls still complete synchronously.
/// Poll once per frame. Each batch has 1,024 input rows (one complete affix directive may
/// exceed that budget). The fixed bundled dictionary's largest directive has 3,116 rows.
pub fn poll_interactive_loading() {
    INTERACTIVE.set(true);
    if DICT.get().is_some() {
        return;
    }
    let mut loader = LOADER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = match loader.as_mut() {
        None => return,
        Some(builder) => builder.advance(1024),
    };
    match result {
        Ok(false) => (),
        Ok(true) => {
            let result = loader.take().and_then(|b| b.finish()).ok_or_else(|| "Hungarian dictionary builder did not finish".to_string());
            publish_dictionary(result);
        }
        Err(e) => {
            *loader = None;
            publish_dictionary(Err(format!("Hungarian dictionary: {e}")));
        }
    }
}

/// Whether a UI requested the dictionary and should show a loading indication/repaint.
pub fn is_loading() -> bool {
    STARTED.load(std::sync::atomic::Ordering::Relaxed) && DICT.get().is_none()
}

fn publish_dictionary(result: Result<Dictionary, String>) {
    if let Err(e) = &result {
        log::error!("{e}");
    }
    if DICT.set(result).is_ok() {
        crate::DICTIONARY_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn dictionary() -> Option<&'static Dictionary> {
    if let Some(result) = DICT.get() {
        return result.as_ref().ok();
    }
    if INTERACTIVE.get() {
        let mut loader = LOADER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if loader.is_none() && DICT.get().is_none() {
            STARTED.store(true, std::sync::atomic::Ordering::Relaxed);
            let inputs = INPUTS.get_or_init(|| {
                let inflate = |bytes| {
                    miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, 16 << 20)
                        .map_err(|e| format!("Hungarian dictionary decompression: {e:?}"))
                };
                let aff = encoding::normalize_aff(&inflate(AFF)?)?;
                let dic = String::from_utf8(inflate(DIC)?).map_err(|e| format!("Hungarian .dic: {e}"))?;
                Ok((aff, dic))
            });
            let builder = inputs
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|(aff, dic)| spellbook::DictionaryBuilder::new(aff, dic).map_err(|e| format!("Hungarian dictionary: {e}")));
            match builder {
                Ok(builder) => *loader = Some(builder),
                Err(e) => publish_dictionary(Err(e)),
            }
        }
        return DICT.get().and_then(|r| r.as_ref().ok());
    }
    DICT.get_or_init(|| load_compressed(AFF, DIC)).as_ref().ok()
}

/// Inflate with a hard allocation limit, then validate the complete pair.
fn load_compressed(aff: &[u8], dic: &[u8]) -> Result<Dictionary, String> {
    let inflate =
        |bytes| miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, 16 << 20).map_err(|e| format!("Hungarian dictionary decompression: {e:?}"));
    parse_dictionary(&inflate(aff)?, &inflate(dic)?)
}

/// Whether the bundled dictionary can actually be checked.
pub(crate) fn has_dictionary() -> bool {
    dictionary().is_some()
}

/// A cached loading failure, without triggering a parse just to read status.
pub(crate) fn load_error() -> Option<String> {
    DICT.get().and_then(|result| result.as_ref().err()).cloned()
}

pub fn is_correct(word: &str) -> bool {
    let word = word.trim_matches(['\'', '’']);
    if word.is_empty() || crate::in_user_dictionary(&word.to_lowercase()) {
        return true;
    }
    // Other scripts are outside the Hungarian dictionary's scope.
    if !word.chars().any(|c| c.is_alphabetic() && c <= '\u{024f}') {
        return true;
    }
    if word.len() > MAX_WORD_BYTES {
        return false;
    }
    dictionary().is_none_or(|d| d.check(word))
}

pub fn suggest(word: &str, max: usize) -> Vec<String> {
    if max == 0 || word.is_empty() || word.len() > MAX_WORD_BYTES || word.chars().any(char::is_control) {
        return Vec::new();
    }
    let Some(d) = dictionary() else { return Vec::new() };
    bounded_suggestions(word, max.min(16), |candidate| d.check(candidate))
}

/// Examine at most 256 dictionary candidates. Unlike full n-gram search this never scans
/// the dictionary, expands every stem, or recurses into subwords. Accent and j/ly corrections
/// come first, followed by case, transposition, deletion, replacement and insertion.
fn bounded_suggestions(word: &str, max: usize, mut check: impl FnMut(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut attempts = 0;
    let mut seen = std::collections::HashSet::new();
    let mut offer = |candidate: String| {
        if attempts >= 256 || out.len() >= max {
            return false;
        }
        if candidate != word && seen.insert(candidate.clone()) {
            attempts += 1;
            if check(&candidate) {
                out.push(candidate);
            }
        }
        attempts < 256 && out.len() < max
    };
    // Byte indices are obtained from the original UTF-8 string only.
    for (i, c) in word.char_indices() {
        let alternatives = match c {
            'a' | 'á' => "aá",
            'e' | 'é' => "eé",
            'i' | 'í' => "ií",
            'o' | 'ó' | 'ö' | 'ő' => "oóöő",
            'u' | 'ú' | 'ü' | 'ű' => "uúüű",
            'A' | 'Á' => "AÁ",
            'E' | 'É' => "EÉ",
            'I' | 'Í' => "IÍ",
            'O' | 'Ó' | 'Ö' | 'Ő' => "OÓÖŐ",
            'U' | 'Ú' | 'Ü' | 'Ű' => "UÚÜŰ",
            _ => "",
        };
        for replacement in alternatives.chars() {
            let mut candidate = word.to_string();
            candidate.replace_range(i..i + c.len_utf8(), &replacement.to_string());
            if !offer(candidate) {
                return out;
            }
        }
    }
    for (wrong, right) in [("j", "ly"), ("ly", "j")] {
        for (i, _) in word.match_indices(wrong) {
            let mut candidate = word.to_string();
            candidate.replace_range(i..i + wrong.len(), right);
            if !offer(candidate) {
                return out;
            }
        }
    }
    let mut chars = word.chars();
    if let Some(first) = chars.next()
        && !offer(first.to_uppercase().collect::<String>() + chars.as_str())
    {
        return out;
    }
    if !offer(word.to_lowercase()) {
        return out;
    }
    let indices: Vec<(usize, char)> = word.char_indices().collect();
    for pair in indices.windows(2) {
        let [(a, ca), (b, cb)] = pair else { continue };
        let mut candidate = word.to_string();
        candidate.replace_range(*a..b + cb.len_utf8(), &format!("{cb}{ca}"));
        if !offer(candidate) {
            return out;
        }
    }
    for (i, c) in &indices {
        let mut candidate = word.to_string();
        candidate.replace_range(*i..i + c.len_utf8(), "");
        if !offer(candidate) {
            return out;
        }
    }
    // Frequent letters first. The global attempt bound applies across all strategies.
    for replacement in "eétaolnksirzgmbvdfhupjcöyőúüűxqw".chars() {
        for (i, c) in &indices {
            let mut candidate = word.to_string();
            candidate.replace_range(*i..i + c.len_utf8(), &replacement.to_string());
            if !offer(candidate) {
                return out;
            }
        }
        for i in indices.iter().map(|(i, _)| *i).chain(std::iter::once(word.len())) {
            let mut candidate = word.to_string();
            candidate.insert(i, replacement);
            if !offer(candidate) {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueKind, Options, ProofLang, check_text, is_correct_in_with, suggest_in};

    #[test]
    fn bundled_dictionary_checks_real_hungarian_and_compounds_without_panicking() {
        assert!(dictionary().is_some());
        for w in ["magyar", "helyesírás", "ellenőrzés", "árvíztűrő", "tükörfúrógép", "ház", "házak", "házainkban", "könyvespolc", "Budapest"]
        {
            assert!(is_correct(w), "{w}");
        }
        for w in ["muszály", "hejesírás", "szépp", "konyv", "árvíztürő", "budapest"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn suggestions_use_accents_and_hungarian_spelling() {
        for (wrong, right) in [("muszály", "muszáj"), ("hejesírás", "helyesírás"), ("konyv", "könyv"), ("árvíztürő", "árvíztűrő")] {
            let suggestions = suggest_in(wrong, 5, ProofLang::Hu);
            assert!(suggestions.contains(&right.to_string()), "{wrong}: {suggestions:?}");
            assert!(suggestions.len() <= 5);
        }
        let _ = suggest("A", 5); // Regression: title-case suggestions used an end index.
        assert!(suggest("konyv", 0).is_empty());
        assert!(suggest(&"ő".repeat(1000), 5).is_empty());
    }

    #[test]
    fn suggestion_work_is_bounded_even_without_matches() {
        for word in ["szépp", "Xyzqwerty", "elkelkáposztástalanítottátok", &"ő".repeat(60)] {
            let mut calls = 0;
            let out = bounded_suggestions(word, 16, |_| {
                calls += 1;
                false
            });
            assert!(out.is_empty());
            assert!(calls <= 256, "{word}: {calls}");
        }
        let mut calls = 0;
        assert_eq!(
            bounded_suggestions("konyv", 2, |_| {
                calls += 1;
                true
            })
            .len(),
            2
        );
        assert_eq!(calls, 2);
    }

    #[test]
    fn corrupt_dictionary_is_a_visible_error() {
        assert!(load_compressed(b"invalid compressed input", DIC).is_err());
        assert!(parse_dictionary(b"SET UTF-8\n", &[0xff]).is_err());
        assert!(has_dictionary());
        assert!(load_error().is_none());
        assert!(crate::unreadable_dictionary(Some("hu-HU")).is_none());
    }

    #[test]
    fn mixed_runs_ranges_options_and_user_dictionary() {
        let text = "Magyar hejesírás. English recieve.";
        let boundary = text.find("English").unwrap();
        let issues = check_text(text, &[(0..boundary, ProofLang::Hu), (boundary..text.len(), ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == ProofLang::Hu
            && i.kind == IssueKind::Spelling
            && &text[i.start..i.end] == "hejesírás"
            && i.message == "Lehetséges helyesírási hiba"));
        assert!(issues.iter().any(|i| i.lang == ProofLang::En && &text[i.start..i.end] == "recieve"));
        let opts = Options { ignore_uppercase: false, ..Options::default() };
        assert!(!is_correct_in_with("HEJESÍRÁS", ProofLang::Hu, &opts));
        assert!(is_correct_in_with("HEJESÍRÁS", ProofLang::Hu, &Options::default()));
        crate::add_word("hungarianregressioncustomword");
        assert!(is_correct("hungarianregressioncustomword"));
        crate::remove_word("hungarianregressioncustomword");
        assert!(crate::missing_dictionary(Some("hu-HU")).is_none());
        assert!(crate::hyphen::hyphen_points_in("tükörfúrógép", &crate::hyphen::Limits::default(), ProofLang::Hu).is_empty());
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config { cases: 64, failure_persistence: None, ..Default::default() })]
        #[test]
        fn arbitrary_unicode_is_safe(s in "\\PC{0,48}") {
            let _ = is_correct(&s);
            let suggestions = suggest(&s, 3);
            proptest::prop_assert!(suggestions.len() <= 3);
        }
    }
}
