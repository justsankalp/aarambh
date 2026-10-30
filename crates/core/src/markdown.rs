//! Markdown to a small tree that the web app renders as DOM nodes, so no
//! HTML string (and no `innerHTML`) is ever involved. Raw HTML in the source
//! becomes plain text; links keep only http(s) and in-page targets.

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Code(String),
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Code(String),
    /// `href` is `None` when the target wasn't safe to link to.
    Link {
        href: Option<String>,
        children: Vec<Inline>,
    },
    Break,
}

enum Open {
    Para,
    /// Text directly inside a tight list item.
    Implicit,
    Heading(u8),
    Strong,
    Emph,
    Link(Option<String>),
    /// Image alt text is kept as plain text.
    Image,
}

#[derive(Default)]
struct Builder {
    /// Block containers: the document, list items and quotes.
    blocks: Vec<Vec<Block>>,
    inlines: Vec<(Open, Vec<Inline>)>,
    lists: Vec<(bool, u64, Vec<Vec<Block>>)>,
    code: Option<String>,
}

fn safe_href(url: &str) -> Option<String> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || u.starts_with('#'))
        .then(|| u.to_owned())
}

impl Builder {
    fn push_block(&mut self, b: Block) {
        self.close_implicit();
        self.blocks.last_mut().expect("root container").push(b);
    }

    fn close_implicit(&mut self) {
        if matches!(self.inlines.last(), Some((Open::Implicit, _))) {
            let (_, content) = self.inlines.pop().expect("checked above");
            self.blocks
                .last_mut()
                .expect("root container")
                .push(Block::Paragraph(content));
        }
    }

    fn inline(&mut self, i: Inline) {
        if self.inlines.is_empty() {
            self.inlines.push((Open::Implicit, Vec::new()));
        }
        self.inlines.last_mut().expect("just ensured").1.push(i);
    }

    fn open(&mut self, o: Open) {
        if matches!(o, Open::Para | Open::Heading(_)) {
            self.close_implicit();
        } else if self.inlines.is_empty() {
            self.inlines.push((Open::Implicit, Vec::new()));
        }
        self.inlines.push((o, Vec::new()));
    }

    fn close_inline(&mut self) {
        let Some((kind, content)) = self.inlines.pop() else {
            return;
        };
        match kind {
            Open::Para | Open::Implicit => self.push_block(Block::Paragraph(content)),
            Open::Heading(l) => self.push_block(Block::Heading(l, content)),
            Open::Strong => self.inline(Inline::Strong(content)),
            Open::Emph => self.inline(Inline::Emph(content)),
            Open::Link(href) => self.inline(Inline::Link {
                href,
                children: content,
            }),
            Open::Image => content.into_iter().for_each(|c| self.inline(c)),
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        if let Some(code) = self.code.as_mut() {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let c = self.code.take().unwrap_or_default();
                    self.push_block(Block::Code(c.trim_end_matches('\n').to_owned()));
                }
                _ => {}
            }
            return;
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.open(Open::Para),
                Tag::Heading { level, .. } => self.open(Open::Heading(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                })),
                Tag::BlockQuote(_) | Tag::Item => {
                    self.close_implicit();
                    self.blocks.push(Vec::new());
                }
                Tag::List(start) => {
                    self.close_implicit();
                    self.lists
                        .push((start.is_some(), start.unwrap_or(1), Vec::new()));
                }
                Tag::CodeBlock(_) => {
                    self.close_implicit();
                    self.code = Some(String::new());
                }
                Tag::Strong => self.open(Open::Strong),
                Tag::Emphasis => self.open(Open::Emph),
                Tag::Link { dest_url, .. } => self.open(Open::Link(safe_href(&dest_url))),
                Tag::Image { .. } => self.open(Open::Image),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Strong
                | TagEnd::Emphasis
                | TagEnd::Link
                | TagEnd::Image => self.close_inline(),
                TagEnd::Item => {
                    self.close_implicit();
                    let item = self.blocks.pop().unwrap_or_default();
                    if let Some(l) = self.lists.last_mut() {
                        l.2.push(item);
                    }
                }
                TagEnd::List(_) => {
                    if let Some((ordered, start, items)) = self.lists.pop() {
                        self.push_block(Block::List {
                            ordered,
                            start,
                            items,
                        });
                    }
                }
                TagEnd::BlockQuote(_) => {
                    self.close_implicit();
                    let inner = self.blocks.pop().unwrap_or_default();
                    self.push_block(Block::Quote(inner));
                }
                _ => {}
            },
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                self.inline(Inline::Text(t.into_string()))
            }
            Event::Code(t) => self.inline(Inline::Code(t.into_string())),
            Event::SoftBreak => self.inline(Inline::Text(" ".into())),
            Event::HardBreak => self.inline(Inline::Break),
            Event::Rule => self.push_block(Block::Rule),
            _ => {}
        }
    }
}

pub fn parse(text: &str) -> Vec<Block> {
    let mut b = Builder {
        blocks: vec![Vec::new()],
        ..Default::default()
    };
    for ev in Parser::new(text) {
        b.event(ev);
    }
    while !b.inlines.is_empty() {
        b.close_inline();
    }
    let mut out = b.blocks.into_iter().next().unwrap_or_default();
    merge_text(&mut out);
    out
}

/// Joins adjacent text runs (the parser splits text at entities and HTML).
fn merge_text(blocks: &mut [Block]) {
    fn inlines(v: &mut Vec<Inline>) {
        let mut out: Vec<Inline> = Vec::with_capacity(v.len());
        for i in v.drain(..) {
            match (out.last_mut(), i) {
                (Some(Inline::Text(a)), Inline::Text(b)) => a.push_str(&b),
                (_, mut other) => {
                    if let Inline::Strong(c) | Inline::Emph(c) | Inline::Link { children: c, .. } =
                        &mut other
                    {
                        inlines(c);
                    }
                    out.push(other);
                }
            }
        }
        *v = out;
    }
    for b in blocks {
        match b {
            Block::Heading(_, v) | Block::Paragraph(v) => inlines(v),
            Block::List { items, .. } => items.iter_mut().for_each(|i| merge_text(i)),
            Block::Quote(inner) => merge_text(inner),
            Block::Code(_) | Block::Rule => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn headings_paragraphs_and_emphasis() {
        let doc = parse(
            "# Friday\n\n## Summary · 02:30 UTC\n\nAsha **proposed** a *thing*\nand `code`.\n\n---\n",
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
            ]
        );
    }

    #[test]
    fn tight_and_ordered_lists() {
        let doc = parse("- **Asha:** managers\n- *Underneath:* budgets\n\n2. one\n3. two\n");
        let Block::List {
            ordered: false,
            items,
            ..
        } = &doc[0]
        else {
            panic!("{doc:?}")
        };
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            vec![Block::Paragraph(vec![
                Inline::Strong(vec![t("Asha:")]),
                t(" managers")
            ])]
        );
        assert_eq!(
            doc[1],
            Block::List {
                ordered: true,
                start: 2,
                items: vec![
                    vec![Block::Paragraph(vec![t("one")])],
                    vec![Block::Paragraph(vec![t("two")])]
                ],
            }
        );
    }

    #[test]
    fn raw_html_is_kept_as_text() {
        let doc = parse("Hi <img src=x onerror=alert(1)> there\n\n<script>alert(2)</script>\n");
        let text = format!("{doc:?}");
        assert!(text.contains("<img src=x onerror=alert(1)>"));
        assert!(text.contains("<script>alert(2)</script>"));
        assert!(
            doc.iter().all(|b| matches!(b, Block::Paragraph(_))),
            "{doc:?}"
        );
    }

    #[test]
    fn only_safe_links_keep_their_target() {
        let doc = parse(
            "[a](https://x.org) [b](javascript:alert(1)) [c](#n-1) [d](data:text/html,x) ![alt text](https://img)",
        );
        let Block::Paragraph(inl) = &doc[0] else {
            panic!()
        };
        let links: Vec<_> = inl
            .iter()
            .filter_map(|i| match i {
                Inline::Link { href, .. } => Some(href.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            links,
            vec![
                Some("https://x.org".into()),
                None,
                Some("#n-1".into()),
                None
            ]
        );
        assert!(
            inl.contains(&t(" alt text")),
            "image becomes its alt text: {inl:?}"
        );
    }

    #[test]
    fn quotes_and_code_blocks() {
        let doc = parse("> quoted **bold**\n\n```\nlet x = 1;\n```\n");
        assert_eq!(
            doc[0],
            Block::Quote(vec![Block::Paragraph(vec![
                t("quoted "),
                Inline::Strong(vec![t("bold")])
            ])])
        );
        assert_eq!(doc[1], Block::Code("let x = 1;".into()));
    }
}
