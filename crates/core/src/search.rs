//! BM25 ranking, used to shortlist the notes most related to a new note so
//! the judge sends a handful of notes to the model, never the whole log.

use std::collections::HashMap;

use crate::id::Id;

const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Common English words that carry no topic.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "because", "been", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had",
    "has", "have", "he", "her", "his", "how", "i", "if", "in", "into", "is", "it", "its", "just",
    "me", "more", "most", "my", "no", "not", "of", "on", "or", "our", "out", "she", "so", "some",
    "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "to", "too",
    "up", "us", "was", "we", "were", "what", "when", "which", "who", "why", "will", "with",
    "would", "you", "your",
];

/// Lowercased word stems, without stopwords. Stemming is deliberately
/// crude (plural and -ing/-ed endings, then a trailing "e") so that
/// "clinics" ~ "clinic", "pricing" ~ "price" and "takes" ~ "take".
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() > 1 && !STOPWORDS.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

fn stem(w: &str) -> String {
    let long = |base: &str| base.chars().count() >= 3;
    let base = if let Some(b) = w.strip_suffix("ies").filter(|b| long(b)) {
        format!("{b}y")
    } else if let Some(b) = w
        .strip_suffix("es")
        .filter(|b| long(b) && ["s", "x", "z", "ch", "sh"].iter().any(|e| b.ends_with(e)))
    {
        b.to_owned()
    } else if let Some(b) = w
        .strip_suffix("ing")
        .or_else(|| w.strip_suffix("ed"))
        .filter(|b| long(b))
    {
        b.to_owned()
    } else if let Some(b) = w.strip_suffix('s').filter(|b| long(b) && !b.ends_with('s')) {
        b.to_owned()
    } else {
        w.to_owned()
    };
    match base.strip_suffix('e') {
        Some(b) if long(b) => b.to_owned(),
        _ => base,
    }
}

/// An in-memory BM25 index over short documents.
pub struct Index {
    docs: Vec<(Id, HashMap<String, u32>, usize)>,
    df: HashMap<String, u32>,
    avg_len: f64,
}

impl Index {
    pub fn new<'a>(docs: impl IntoIterator<Item = (Id, &'a str)>) -> Self {
        let mut df: HashMap<String, u32> = HashMap::new();
        let docs: Vec<_> = docs
            .into_iter()
            .map(|(id, text)| {
                let toks = tokenize(text);
                let mut tf: HashMap<String, u32> = HashMap::new();
                for t in &toks {
                    *tf.entry(t.clone()).or_default() += 1;
                }
                for t in tf.keys() {
                    *df.entry(t.clone()).or_default() += 1;
                }
                (id, tf, toks.len())
            })
            .collect();
        let avg_len = if docs.is_empty() {
            0.0
        } else {
            docs.iter().map(|d| d.2).sum::<usize>() as f64 / docs.len() as f64
        };
        Index { docs, df, avg_len }
    }

    /// Documents with a positive score, best first. Ties go to the newer id.
    pub fn search(&self, query: &str, k: usize) -> Vec<(Id, f64)> {
        let mut terms = tokenize(query);
        terms.sort();
        terms.dedup();
        let n = self.docs.len() as f64;
        let mut scored: Vec<(Id, f64)> = self
            .docs
            .iter()
            .map(|(id, tf, len)| {
                let score = terms
                    .iter()
                    .filter_map(|t| {
                        let f = f64::from(*tf.get(t)?);
                        let df = f64::from(self.df[t]);
                        let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                        let norm = 1.0 - B + B * (*len as f64) / self.avg_len.max(1.0);
                        Some(idf * f * (K1 + 1.0) / (f + K1 * norm))
                    })
                    .sum::<f64>();
                (*id, score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
        scored.truncate(k);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> Id {
        Id::new(1_761_843_481_000 + n * 1000, u128::from(n))
    }

    #[test]
    fn word_forms_match_and_stopwords_drop() {
        let same = |a: &str, b: &str| assert_eq!(tokenize(a), tokenize(b), "{a} ~ {b}");
        same("clinics", "clinic");
        same("pricing", "price");
        same("takes taking", "take take");
        same("stories studies", "story study");
        same("classes boxes", "class box");
        same("notes", "note");
        assert_eq!(
            tokenize("The clinic's budget is TOO high!").len(),
            3,
            "clinic, budget, high"
        );
        assert_eq!(tokenize("Ünïcode café").len(), 2);
        assert!(tokenize("it is what it is").is_empty());
    }

    #[test]
    fn ranks_the_most_relevant_note_first() {
        let docs = [
            (id(1), "Clinics will pay for intake automation"),
            (id(2), "We should hire a designer next quarter"),
            (
                id(3),
                "Intake at the clinic takes an hour every morning; intake forms are on paper",
            ),
            (id(4), "Pricing for schools is unclear"),
        ];
        let idx = Index::new(docs);
        let hits = idx.search("How long does clinic intake take?", 10);
        let ids: Vec<Id> = hits.iter().map(|h| h.0).collect();
        assert_eq!(
            ids,
            vec![id(3), id(1)],
            "only notes sharing words, best first"
        );
        assert!(hits[0].1 > hits[1].1);
    }

    #[test]
    fn rare_words_count_more_than_common_ones() {
        let idx = Index::new([
            (id(1), "clinic clinic budget"),
            (id(2), "clinic owner"),
            (id(3), "clinic manager"),
            (id(4), "budget review with the clinic"),
        ]);
        let hits = idx.search("owner budget", 2);
        assert_eq!(hits.len(), 2);
        assert!(
            hits.iter().all(|h| h.0 != id(3)),
            "matching only 'clinic' scores nothing here"
        );
    }

    #[test]
    fn ties_prefer_newer_notes_and_k_limits() {
        let idx = Index::new([(id(1), "wedge"), (id(2), "wedge"), (id(3), "wedge")]);
        let ids: Vec<Id> = idx.search("wedge", 2).into_iter().map(|h| h.0).collect();
        assert_eq!(ids, vec![id(3), id(2)]);
    }

    #[test]
    fn empty_inputs() {
        assert!(Index::new([]).search("anything", 5).is_empty());
        assert!(
            Index::new([(id(1), "text")])
                .search("the of and", 5)
                .is_empty()
        );
    }
}
