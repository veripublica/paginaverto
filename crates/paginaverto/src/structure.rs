//! From page text to chapters: drop running heads, page numbers and
//! line-break hyphens, then find the headings that start chapters.
//!
//! Parity note (0.1.0): reproduces epublift cli-v4.5.1's
//! `src/pdf/structure.rs`. Quirks to fix later are marked `PARITY:`.

use std::collections::{HashMap, HashSet};

use regex::Regex;

use crate::text::{self, Page};
use crate::{Block, Chapter, Figure};

/// A heading starts a chapter only when at least this many bytes of body
/// text follow it before the next heading.
const MIN_CHAPTER_BODY: usize = 400;

/// A heading has at most this many words.
const HEADING_WORDS: usize = 12;

/// Three recurrences of an edge block within this many pages make a head.
const NEAR_PAGES: usize = 4;

enum Item {
    Heading(String),
    Body(String),
    Figure(Figure),
}

/// Build the chapters from the pages, in order. Chapters with neither a
/// title nor content are dropped, so the result may be empty.
pub(crate) fn chapters(pages: &[Page]) -> Vec<Chapter> {
    let heads = running_heads(pages);
    let dehyphen = Regex::new(r"(\p{L})-\s+(\p{Ll})").expect("a valid pattern");

    // One stream of headings, paragraphs and figures. A page's figures come
    // after its text.
    let mut items: Vec<Item> = Vec::new();
    for page in pages {
        let big: HashSet<&str> = page.big_font.iter().map(String::as_str).collect();
        let mut first_on_page = true;
        for block in &page.blocks {
            if is_page_number(block) || heads.contains(&template(block)) {
                continue;
            }
            let clean = text::squash(&dehyphen.replace_all(block, "$1$2"));
            if clean.is_empty() {
                continue;
            }
            let continues = first_on_page && continue_across_page(&mut items, &clean, &dehyphen);
            first_on_page = false;
            if continues {
                continue;
            }
            if is_heading(&clean, page.born_digital, &big) {
                items.push(Item::Heading(clean));
            } else {
                items.push(Item::Body(clean));
            }
        }
        items.extend(page.figures.iter().cloned().map(Item::Figure));
    }

    // A heading without enough body after it (front matter, stacked titles)
    // is demoted to a paragraph.
    let mut chapters = vec![Chapter::default()];
    for (i, item) in items.iter().enumerate() {
        let block = match item {
            Item::Body(p) => Block::Paragraph(p.clone()),
            Item::Figure(f) => Block::Figure(f.clone()),
            Item::Heading(h) => {
                // PARITY: counts bytes, not characters.
                let body: usize = items[i + 1..]
                    .iter()
                    .take_while(|it| !matches!(it, Item::Heading(_)))
                    .map(|it| match it {
                        Item::Body(p) => p.len(),
                        _ => 0,
                    })
                    .sum();
                if body >= MIN_CHAPTER_BODY {
                    chapters.push(Chapter {
                        title: Some(h.clone()),
                        blocks: Vec::new(),
                    });
                    continue;
                }
                Block::Paragraph(h.clone())
            }
        };
        if let Some(last) = chapters.last_mut() {
            last.blocks.push(block);
        }
    }
    chapters.retain(|c| c.title.is_some() || !c.blocks.is_empty());
    chapters
}

/// A page's first block continues the paragraph the last page ended on, if
/// that one stopped mid-sentence and this one starts in lower case. Figures
/// between them stay where they are. Returns whether it was joined.
fn continue_across_page(items: &mut [Item], block: &str, dehyphen: &Regex) -> bool {
    if !block.chars().next().is_some_and(char::is_lowercase) {
        return false;
    }
    let last = items
        .iter_mut()
        .rev()
        .find(|i| !matches!(i, Item::Figure(_)));
    let Some(Item::Body(body)) = last else {
        return false;
    };
    if body.ends_with(['.', '!', '?', ':', ';', '…', '"', '”', '’', ')']) {
        return false;
    }
    let joined = format!("{body} {block}");
    *body = text::squash(&dehyphen.replace_all(&joined, "$1$2"));
    true
}

/// A block reduced for recurrence: letters only, lowercased, roman-numeral
/// words dropped, so "INTRODUCTION ix" and "12 TITLE" variants collapse.
fn template(s: &str) -> String {
    text::letters_only(s)
        .split_whitespace()
        .filter(|w| !is_roman(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_roman(w: &str) -> bool {
    !w.is_empty() && w.len() <= 8 && w.chars().all(|c| "ivxlcdm".contains(c))
}

fn is_page_number(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.len() <= 6
        && (t.chars().all(|c| c.is_ascii_digit())
            || t.chars().all(|c| "ivxlcdmIVXLCDM".contains(c)))
}

/// Templates of the first and last block of each page that are running heads
/// or feet: those that recur on a 30th of the pages (at least 5), and short
/// ones that recur on 3 pages close together, since a head can name a chapter
/// of only a few pages.
fn running_heads(pages: &[Page]) -> HashSet<String> {
    let mut seen: HashMap<String, Vec<usize>> = HashMap::new();
    for (n, page) in pages.iter().enumerate() {
        if let Some(first) = page.blocks.first() {
            seen.entry(template(first)).or_default().push(n);
        }
        if page.blocks.len() > 1
            && let Some(last) = page.blocks.last()
        {
            seen.entry(template(last)).or_default().push(n);
        }
    }
    let threshold = (pages.len() / 30).max(5);
    seen.into_iter()
        .filter(|(t, on)| {
            t.len() > 3
                && (on.len() >= threshold
                    || (t.split_whitespace().count() <= HEADING_WORDS
                        && on.windows(3).any(|w| w[2] - w[0] <= NEAR_PAGES)))
        })
        .map(|(t, _)| t)
        .collect()
}

/// A heading is at most 12 words, has no ASCII digit and at least 3
/// letters, and is either mostly capitals (85%) or, on a born-digital page,
/// shown in a notably large font.
fn is_heading(s: &str, born_digital: bool, big_font: &HashSet<&str>) -> bool {
    if s.split_whitespace().count() > HEADING_WORDS || s.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if s.chars().filter(|c| c.is_alphabetic()).count() < 3 {
        return false;
    }
    let large_font = born_digital && big_font.contains(text::letters_only(s).as_str());
    text::mostly_caps(s) || large_font
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(blocks: &[&str]) -> Page {
        Page {
            blocks: blocks.iter().map(|s| s.to_string()).collect(),
            born_digital: true,
            ..Page::default()
        }
    }

    fn body(chapters: &[Chapter]) -> String {
        chapters
            .iter()
            .flat_map(|c| &c.blocks)
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(p.as_str()),
                Block::Figure(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn joins_line_break_hyphens() {
        let text = body(&chapters(&[page(&[
            "The inter-  national group met today.",
        ])]));
        assert!(text.contains("international"), "{text}");
        assert!(!text.contains("inter-"), "{text}");
    }

    #[test]
    fn joins_turkish_line_break_hyphens() {
        let text = body(&chapters(&[page(&[
            "Gazeteci- \nlerin çoğu, öğ- rencilerle geldi.",
        ])]));
        assert_eq!(text, "Gazetecilerin çoğu, öğrencilerle geldi.");
    }

    #[test]
    fn strips_a_recurring_running_head() {
        let words = [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
        ];
        let pages: Vec<Page> = words
            .iter()
            .map(|w| {
                let b = format!("This is the {w} section with enough genuine words to count.");
                page(&["MY BOOK TITLE", &b])
            })
            .collect();
        let text = body(&chapters(&pages));
        assert!(!text.contains("MY BOOK TITLE"), "{text}");
        assert!(text.contains("alpha") && text.contains("hotel"), "{text}");
    }

    #[test]
    fn strips_page_numbers() {
        let text = body(&chapters(&[page(&[
            "42",
            "Real body text that is long enough.",
            "xiv",
        ])]));
        assert!(!text.contains("42") && !text.contains("xiv"), "{text}");
        assert!(text.contains("Real body text"));
    }

    #[test]
    fn caps_heading_with_body_starts_a_chapter() {
        let b = "word ".repeat(120);
        let ch = chapters(&[page(&["CHAPTER ONE", &b])]);
        assert!(ch.iter().any(|c| c.title.as_deref() == Some("CHAPTER ONE")));
    }

    #[test]
    fn turkish_caps_heading_starts_a_chapter() {
        let b = "söz ".repeat(120);
        let ch = chapters(&[page(&["BİRİNCİ BÖLÜM", &b])]);
        assert!(
            ch.iter()
                .any(|c| c.title.as_deref() == Some("BİRİNCİ BÖLÜM"))
        );
    }

    #[test]
    fn paragraph_continues_across_a_page() {
        let text = body(&chapters(&[
            page(&["This sentence stops in the mid"]),
            page(&["dle of nothing, and ends here."]),
        ]));
        assert_eq!(
            text,
            "This sentence stops in the mid dle of nothing, and ends here."
        );
    }

    #[test]
    fn hyphen_joins_across_a_page() {
        let text = body(&chapters(&[page(&["Öğren-"]), page(&["cilerle geldi."])]));
        assert_eq!(text, "Öğrencilerle geldi.");
    }

    #[test]
    fn finished_sentence_does_not_continue() {
        let ch = chapters(&[page(&["It ended."]), page(&["and this began."])]);
        assert_eq!(ch[0].blocks.len(), 2);
    }

    #[test]
    fn a_head_on_three_pages_goes_even_in_a_long_book() {
        let mut pages: Vec<Page> = (0..300)
            .map(|i| page(&[&format!("Paragraph number {i} with its own words.")]))
            .collect();
        for p in &mut pages[10..13] {
            p.blocks.insert(0, "A SHORT CHAPTER".to_string());
        }
        let text = body(&chapters(&pages));
        assert!(!text.contains("A SHORT CHAPTER"), "{text}");
    }

    #[test]
    fn heading_without_body_is_demoted() {
        let ch = chapters(&[page(&["TITLE", "AUTHOR", "PUBLISHER"])]);
        assert!(ch.iter().all(|c| c.title.is_none()));
    }
}
