//! Generic repeated-word and spacing checks with Hungarian messages.
//! No Hungarian-specific grammar, comma or sentence-capital rules are inferred.

use crate::{Issue, IssueKind, ProofLang, words};

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Hu }
}

pub fn check(text: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), &mut issues);
    spaces(text, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    // A replacement consumes its entire range. Offer only disjoint edits, then recheck
    // after applying them; repetition takes precedence over spacing within that phrase.
    let mut end = 0;
    issues.retain(|issue| {
        if issue.start < end {
            return false;
        }
        end = issue.end;
        true
    });
    issues
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(char::is_alphabetic)
            && w0.to_lowercase() == w1.to_lowercase()
            && !matches!(w0.to_lowercase().as_str(), "az" | "is")
        {
            v.push(issue(*a0, *b1, format!("Lehetséges szóismétlés: „{w1}”"), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, v: &mut Vec<Issue>) {
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != ' ' {
            continue;
        }
        let mut end = i + 1;
        while chars.peek().is_some_and(|(_, c)| *c == ' ') {
            if let Some((at, _)) = chars.next() {
                end = at + 1;
            }
        }
        let mut after_space = text.get(end..).unwrap_or("").chars();
        let p = after_space.next();
        let after = after_space.next();
        let ratio = p == Some(':')
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(|c| c.is_ascii_digit())
            && after == Some(' ')
            && after_space.next().is_some_and(|c| c.is_ascii_digit());
        let punctuation = !ratio
            && match p {
                Some(',') => true,
                Some(';') => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                Some('.') => after != Some('.') && !after.is_some_and(|a| a.is_ascii_digit()),
                Some(':' | '!' | '?') => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if punctuation {
            // Remove the whole run in one edit, leaving punctuation outside the range.
            v.push(issue(i, end, "Felesleges szóköz az írásjel előtt".into(), vec![String::new()]));
        } else if end - i > 1 {
            v.push(issue(i, end, "Többszörös szóköz".into(), vec![" ".into()]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacements_have_disjoint_utf8_ranges() {
        for (text, expected) in [
            ("szó  ,", "szó,"),
            ("Őő  ;", "Őő;"),
            ("Ez ez   , jó.", "Ez, jó."),
            ("Őő őő őő", "Őő őő"),
            ("Arány 1 : 2. Öröm :) és ...", "Arány 1 : 2. Öröm :) és ..."),
        ] {
            let issues = check(text);
            assert!(issues.windows(2).all(|p| p[0].end <= p[1].start), "{text}: {issues:?}");
            let mut corrected = text.to_string();
            for issue in issues.iter().rev() {
                assert!(text.get(issue.start..issue.end).is_some());
                corrected.replace_range(issue.start..issue.end, &issue.suggestions[0]);
            }
            assert_eq!(corrected, expected);
        }
    }

    #[test]
    fn hungarian_messages_and_safe_ranges() {
        let text = "Ez ez ,   szép.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].suggestions, ["Ez"]);
        assert!(issues.iter().any(|i| &text[i.start..i.end] == "   " && i.suggestions == [" "]));
        for i in issues {
            assert_eq!(i.lang, ProofLang::Hu);
            assert!(text.get(i.start..i.end).is_some());
        }
        for text in ["Ő az az ember.", "Arány 1 : 2.", "Öröm :) és ...", "Szép, szép!"] {
            assert!(check(text).is_empty(), "{text}");
        }
    }
}
