//! A small Markdown reader for digests, producing a tree that the web app
//! renders as DOM nodes, so no HTML string (and no `innerHTML`) is involved.
//!
//! Digests are written by the judge's own renderer, so only that subset is
//! understood: `#` headings, paragraphs, `-`/`*` and `1.` lists, `---`
//! rules, `**strong**`, `*em*`/`_em_` and `` `code` ``. Anything else,
//! including HTML and links, is shown as the literal text it is.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<Inline>>,
    },
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Code(String),
}

enum Line<'a> {
    Blank,
    Heading(u8, &'a str),
    Rule,
    Item {
        ordered: bool,
        number: u64,
        text: &'a str,
    },
    Text(&'a str),
}

fn classify(raw: &str) -> Line<'_> {
    let line = raw.trim();
    if line.is_empty() {
        return Line::Blank;
    }
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        return Line::Heading(hashes as u8, line[hashes..].trim());
    }
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| compact.chars().all(|c| c.to_string() == *m))
    {
        return Line::Rule;
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Line::Item {
                ordered: false,
                number: 0,
                text: rest.trim(),
            };
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && (line[digits..].starts_with(". ") || line[digits..].starts_with(") "))
    {
        let number = line[..digits].parse().unwrap_or(1);
        return Line::Item {
            ordered: true,
            number,
            text: line[digits + 2..].trim(),
        };
    }
    Line::Text(line)
}

pub fn parse(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    // (ordered, start, item texts)
    let mut list: Option<(bool, u64, Vec<String>)> = None;

    fn flush_para(para: &mut Vec<&str>, out: &mut Vec<Block>) {
        if !para.is_empty() {
            out.push(Block::Paragraph(inlines(&para.join(" "))));
            para.clear();
        }
    }
    fn flush_list(list: &mut Option<(bool, u64, Vec<String>)>, out: &mut Vec<Block>) {
        if let Some((ordered, start, items)) = list.take() {
            out.push(Block::List {
                ordered,
                start,
                items: items.iter().map(|t| inlines(t)).collect(),
            });
        }
    }

    for raw in text.lines() {
        match classify(raw) {
            Line::Blank => {
                flush_para(&mut para, &mut out);
                flush_list(&mut list, &mut out);
            }
            Line::Heading(level, t) => {
                flush_para(&mut para, &mut out);
                flush_list(&mut list, &mut out);
                out.push(Block::Heading(level, inlines(t)));
            }
            Line::Rule => {
                flush_para(&mut para, &mut out);
                flush_list(&mut list, &mut out);
                out.push(Block::Rule);
            }
            Line::Item {
                ordered,
                number,
                text,
            } => {
                flush_para(&mut para, &mut out);
                if list.as_ref().is_some_and(|l| l.0 != ordered) {
                    flush_list(&mut list, &mut out);
                }
                list.get_or_insert_with(|| (ordered, number.max(1), Vec::new()))
                    .2
                    .push(text.to_owned());
            }
            Line::Text(t) => match list.as_mut() {
                // An indented line continues the previous list item.
                Some(l) if raw.starts_with("  ") => {
                    if let Some(last) = l.2.last_mut() {
                        last.push(' ');
                        last.push_str(t);
                    }
                }
                _ => {
                    flush_list(&mut list, &mut out);
                    para.push(t);
                }
            },
        }
    }
    flush_para(&mut para, &mut out);
    flush_list(&mut list, &mut out);
    out
}

/// Inline emphasis and code spans. Unmatched markers stay as text.
pub fn inlines(s: &str) -> Vec<Inline> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<Inline> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    let push_text = |text: &mut String, out: &mut Vec<Inline>| {
        if !text.is_empty() {
            out.push(Inline::Text(std::mem::take(text)));
        }
    };
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..=chars.len().saturating_sub(pat.len()))
            .find(|&j| chars[j..j + pat.len()] == *pat && (j == 0 || chars[j - 1] != '\\'))
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            text.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`'
            && let Some(end) = find(i + 1, &['`'])
            && end > i + 1
        {
            push_text(&mut text, &mut out);
            out.push(Inline::Code(chars[i + 1..end].iter().collect()));
            i = end + 1;
            continue;
        }
        if c == '*'
            && chars.get(i + 1) == Some(&'*')
            && let Some(end) = find(i + 2, &['*', '*'])
            && end > i + 2
        {
            push_text(&mut text, &mut out);
            out.push(Inline::Strong(inlines(
                &chars[i + 2..end].iter().collect::<String>(),
            )));
            i = end + 2;
            continue;
        }
        let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
        if (c == '*' || (c == '_' && word_start))
            && chars
                .get(i + 1)
                .is_some_and(|n| !n.is_whitespace() && *n != c)
            && let Some(end) = (i + 1..chars.len()).find(|&j| {
                chars[j] == c
                    && chars[j - 1] != '\\'
                    && !chars[j - 1].is_whitespace()
                    && chars.get(j + 1) != Some(&c)
                    && (c == '*' || chars.get(j + 1).is_none_or(|n| !n.is_alphanumeric()))
            })
        {
            push_text(&mut text, &mut out);
            out.push(Inline::Emph(inlines(
                &chars[i + 1..end].iter().collect::<String>(),
            )));
            i = end + 1;
            continue;
        }
        text.push(c);
        i += 1;
    }
    push_text(&mut text, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn headings_paragraphs_rules() {
        let doc = parse(
            "# Friday\n\n## Summary · 02:30 UTC\n\nAsha **proposed** a *thing*\nand `code`.\n\n---\n#nohash\n",
        );
        assert_eq!(
            doc,
            vec![
                Block::Heading(1, vec![t("Friday")]),
                Block::Heading(2, vec![t("Summary · 02:30 UTC")]),
                Block::Paragraph(vec![
                    t("Asha "),
                    Inline::Strong(vec![t("proposed")]),
                    t(" a "),
                    Inline::Emph(vec![t("thing")]),
                    t(" and "),
                    Inline::Code("code".into()),
                    t("."),
                ]),
                Block::Rule,
                Block::Paragraph(vec![t("#nohash")]),
            ]
        );
    }

    #[test]
    fn lists() {
        let doc = parse(
            "- **Asha:** managers\n- *Underneath:* budgets\n  continued\n\n2. one\n3. two\nafter\n",
        );
        assert_eq!(
            doc[0],
            Block::List {
                ordered: false,
                start: 1,
                items: vec![
                    vec![Inline::Strong(vec![t("Asha:")]), t(" managers")],
                    vec![
                        Inline::Emph(vec![t("Underneath:")]),
                        t(" budgets continued")
                    ],
                ],
            }
        );
        assert_eq!(
            doc[1],
            Block::List {
                ordered: true,
                start: 2,
                items: vec![vec![t("one")], vec![t("two")]]
            }
        );
        assert_eq!(doc[2], Block::Paragraph(vec![t("after")]));
    }

    #[test]
    fn html_and_links_are_just_text() {
        let src =
            "Hi <img src=x onerror=alert(1)> [x](javascript:alert(1)) <script>alert(2)</script>";
        assert_eq!(parse(src), vec![Block::Paragraph(vec![t(src)])]);
    }

    #[test]
    fn unmatched_and_escaped_markers_stay_literal() {
        assert_eq!(inlines("2 * 3 = 6"), vec![t("2 * 3 = 6")]);
        assert_eq!(
            inlines("snake_case_name stays"),
            vec![t("snake_case_name stays")]
        );
        assert_eq!(inlines("a **b"), vec![t("a **b")]);
        assert_eq!(inlines(r"\*not em\*"), vec![t("*not em*")]);
        assert_eq!(
            inlines("_em_ and **strong _nested_**"),
            vec![
                Inline::Emph(vec![t("em")]),
                t(" and "),
                Inline::Strong(vec![t("strong "), Inline::Emph(vec![t("nested")])]),
            ]
        );
    }
}
