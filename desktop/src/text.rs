//! Bounded, lossless text segmentation. No model or language service is needed.
pub const CHUNK_CHARS: usize = 180;

fn closing(ch: char) -> bool {
    matches!(
        ch,
        '”' | '’' | '」' | '』' | '》' | '）' | ')' | ']' | '"' | '\''
    )
}

fn sentence_mark(chars: &[char], at: usize) -> bool {
    let ch = chars[at];
    if matches!(ch, '。' | '！' | '？' | '…') {
        return true;
    }
    if !matches!(ch, '.' | '!' | '?') {
        return false;
    }
    // Internal punctuation in numbers, domains, email and URLs isn't a sentence.
    if let Some(next) = chars.get(at + 1) {
        if next.is_ascii_alphanumeric() || matches!(next, '/' | '_' | '-' | '=' | '&') {
            return false;
        }
    }
    if ch == '.' {
        let begin = chars[..at]
            .iter()
            .rposition(|c| !c.is_ascii_alphabetic() && *c != '.')
            .map_or(0, |i| i + 1);
        let token: String = chars[begin..at]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        if (token.len() == 1 && token.chars().all(|c| c.is_ascii_alphabetic()))
            || matches!(
                token.as_str(),
                "mr" | "mrs"
                    | "ms"
                    | "dr"
                    | "prof"
                    | "sr"
                    | "jr"
                    | "st"
                    | "vs"
                    | "e.g"
                    | "i.e"
                    | "a.m"
                    | "p.m"
            )
            || (token.contains('.') && token.split('.').all(|part| part.len() == 1))
        {
            return false;
        }
    }
    true
}

pub fn segments(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let limit = (start + CHUNK_CHARS).min(chars.len());
        if limit == chars.len() {
            result.push(chars[start..].iter().collect());
            break;
        }
        let mut natural = None;
        let mut clause = None;
        let mut word = None;
        for at in start..limit {
            let ch = chars[at];
            if ch.is_whitespace() {
                word = Some(at + 1);
            }
            if matches!(ch, ',' | '，' | ';' | '；' | ':' | '：' | '、')
                && !(ch.is_ascii() && chars.get(at + 1).is_some_and(|c| !c.is_whitespace()))
            {
                clause = Some(at + 1);
            }
            if matches!(ch, '\n' | '\r') || sentence_mark(&chars, at) {
                let mut end = at + 1;
                // Keep punctuation runs, closing quotes and paragraph whitespace together.
                while end < chars.len()
                    && (closing(chars[end])
                        || chars[end].is_whitespace()
                        || matches!(chars[end], '。' | '！' | '？' | '!' | '?' | '.' | '…'))
                {
                    end += 1;
                }
                if end <= limit {
                    natural = Some(end);
                    continue;
                }
                // Leave the punctuation/quote group for the next chunk when possible.
                break;
            }
        }
        let end = natural
            .filter(|end| end - start >= CHUNK_CHARS / 2)
            .unwrap_or_else(|| {
                if limit == chars.len() {
                    limit
                } else {
                    // Avoid tiny fragments when a long clause has only an early comma.
                    clause
                        .filter(|end| end - start >= CHUNK_CHARS / 3)
                        .or(word.filter(|end| end - start >= CHUNK_CHARS / 3))
                        .unwrap_or(limit)
                }
            });
        result.push(chars[start..end].iter().collect());
        start = end;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_sentences_keep_quotes_and_paragraphs() {
        let text = "他说：“你好！”\r\n\r\nHello, world. Next sentence? 好的。";
        assert_eq!(segments(text), [text]);
    }

    #[test]
    fn numbers_abbreviations_and_addresses_stay_intact() {
        let text = "Dr. Smith uses v1.2.3 at example.com, e.g. version 3.14. Email a@b.com! Done.";
        assert_eq!(segments(text), [text]);
    }

    #[test]
    fn short_sentences_are_packed_to_the_resource_limit() {
        let text = "你好。Hello!\n".repeat(100);
        let parts = segments(&text);
        assert_eq!(parts.concat(), text);
        assert!(
            parts[..parts.len() - 1]
                .iter()
                .all(|p| (90..=180).contains(&p.chars().count()))
        );
    }

    #[test]
    fn long_english_uses_word_boundaries() {
        let text = "internationalization and accessibility ".repeat(80);
        let parts = segments(&text);
        assert!(
            parts
                .iter()
                .all(|s| s.ends_with(' ') && s.chars().count() <= CHUNK_CHARS)
        );
        assert_eq!(parts.concat(), text);
    }

    #[test]
    fn long_chinese_prefers_clauses() {
        let clause = format!("{}，", "中文".repeat(40));
        let text = clause.repeat(4);
        let parts = segments(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|s| s.ends_with('，')));
        assert_eq!(parts.concat(), text);
    }

    #[test]
    fn very_long_unicode_is_never_truncated_or_reordered() {
        for text in [
            "中英 Rust 🙂".repeat(4000),
            "字".repeat(21001),
            " \n\r\t".repeat(6000),
        ] {
            let parts = segments(&text);
            assert!(
                parts
                    .iter()
                    .all(|s| !s.is_empty() && s.chars().count() <= CHUNK_CHARS)
            );
            assert_eq!(parts.concat(), text);
        }
        assert!(segments("").is_empty());
    }
}
