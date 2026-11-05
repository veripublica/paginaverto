//! From page text to blocks: drop running heads, page numbers and line-break
//! hyphens, then find the headings. The outline, when it is a real table of
//! contents, says where chapters start; the look of the text does otherwise.

use std::collections::{HashMap, HashSet};

use regex::Regex;

use crate::document::{Block, BlockBox, BlockKind, Figure};
use crate::outline::Mark;
use crate::text::{self, Marked, Page, TextBlock};

/// A heading starts a chapter only when at least this many characters of body
/// text follow it before the next heading. Outline entries need none.
const MIN_CHAPTER_BODY: usize = 400;

/// A heading has at most this many words.
const HEADING_WORDS: usize = 12;

/// Three recurrences of an edge block within this many pages make a head.
const NEAR_PAGES: usize = 4;

enum Item {
    Heading {
        text: Marked,
        boxes: Vec<BlockBox>,
        /// From the outline: this level, and no test of what follows it.
        given: Option<u32>,
        /// Text from the outline alone, found nowhere on the page.
        outline_only: bool,
        /// The page it is on.
        page: u32,
    },
    Body(Marked, Vec<BlockBox>),
    Figure(Figure),
}

/// What the pages came to.
pub(crate) struct Built {
    /// In reading order, not yet numbered.
    pub blocks: Vec<Block>,
    /// The pages themselves gave something: text or a figure, not only outline
    /// titles.
    pub has_content: bool,
    /// The text read is mostly non-letters.
    pub fails_quality_gate: bool,
}

/// Build the blocks from the pages, in order.
pub(crate) fn build(pages: &[Page], marks: Option<&[Mark]>) -> Built {
    let heads = running_heads(pages);
    let dehyphen = Regex::new(r"(\p{L})-\s+(\p{Ll})").expect("a valid pattern");
    let marks = marks.unwrap_or_default();
    // A heading the outline does not know sits below every level it has.
    let rule_level = marks.iter().map(|m| m.level + 1).max().unwrap_or(1);
    let mut by_page: HashMap<u32, Vec<&Mark>> = HashMap::new();
    for mark in marks {
        by_page.entry(mark.page).or_default().push(mark);
    }

    // One stream of headings, paragraphs and figures. A page's figures come
    // after its text.
    let mut items: Vec<Item> = Vec::new();
    let mut carried: Vec<&Mark> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let mut on_page = std::mem::take(&mut carried);
        on_page.extend(by_page.remove(&page.number).unwrap_or_default());
        let found = find_marks(page, &on_page);
        for mark in on_page {
            if found.values().any(|f| std::ptr::eq(f.mark, mark)) {
                continue;
            }
            // Outlines often point at the page before: a blank, or a picture
            // facing the chapter's first page.
            let on_next = mark.page == page.number
                && pages
                    .get(index + 1)
                    .is_some_and(|next| !find_marks(next, &[mark]).is_empty());
            if on_next {
                carried.push(mark);
            } else if mark.level == 1 {
                items.push(Item::Heading {
                    text: Marked::plain(mark.title.clone()),
                    boxes: Vec::new(),
                    given: Some(1),
                    outline_only: true,
                    page: page.number,
                });
            }
        }
        let big: HashSet<&str> = page.big_font.iter().map(String::as_str).collect();
        let mut first_on_page = true;
        let mut skip_to = 0;
        for (at, block) in page.blocks.iter().enumerate() {
            if at < skip_to {
                continue;
            }
            if let Some(f) = found.get(&at) {
                skip_to = f.end;
                first_on_page = false;
                let text = match f.whole {
                    true => block.text.clone(),
                    false => Marked::plain(f.mark.title.clone()),
                };
                items.push(Item::Heading {
                    text,
                    boxes: union(&page.blocks[at..f.end]),
                    given: Some(f.mark.level),
                    outline_only: false,
                    page: page.number,
                });
                continue;
            }
            if is_page_number(block.str()) || heads.contains(&template(block.str())) {
                continue;
            }
            let clean = without_hyphens(&block.text, &dehyphen);
            if clean.text.is_empty() {
                continue;
            }
            let continues =
                first_on_page && continue_across_page(&mut items, &clean, block, &dehyphen);
            first_on_page = false;
            if continues {
                continue;
            }
            let boxes = block.place.into_iter().collect();
            if is_heading(&clean.text, page.born_digital, &big) {
                items.push(Item::Heading {
                    text: clean,
                    boxes,
                    given: None,
                    outline_only: false,
                    page: page.number,
                });
            } else {
                items.push(Item::Body(clean, boxes));
            }
        }
        items.extend(page.figures.iter().cloned().map(Item::Figure));
    }

    let has_content = items.iter().any(|i| {
        !matches!(
            i,
            Item::Heading {
                outline_only: true,
                ..
            }
        )
    });
    let fails_quality_gate = fails_quality_gate(&items);

    let listed = marks
        .is_empty()
        .then(|| listed_titles(pages, &items))
        .flatten();

    // A heading without enough body after it (front matter, stacked titles)
    // is demoted to a paragraph.
    let mut blocks = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let (kind, boxes) = match item {
            Item::Body(text, boxes) => (BlockKind::Paragraph { runs: text.runs() }, boxes.clone()),
            Item::Figure(f) => (BlockKind::Figure(f.clone()), Vec::new()),
            Item::Heading {
                text, boxes, given, ..
            } => {
                let body: usize = items[i + 1..]
                    .iter()
                    .take_while(|it| !matches!(it, Item::Heading { .. }))
                    .map(|it| match it {
                        Item::Body(text, _) => text.text.chars().count(),
                        _ => 0,
                    })
                    .sum();
                let kind = match given {
                    Some(level) => BlockKind::Heading {
                        level: *level,
                        runs: text.runs(),
                    },
                    None if body >= MIN_CHAPTER_BODY => {
                        let unlisted = listed
                            .as_ref()
                            .is_some_and(|l| !l.contains(&text::letters_only(&text.text)));
                        BlockKind::Heading {
                            level: if unlisted { rule_level + 1 } else { rule_level },
                            runs: text.runs(),
                        }
                    }
                    None => BlockKind::Paragraph { runs: text.runs() },
                };
                (kind, boxes.clone())
            }
        };
        blocks.push(Block {
            id: format!("b{}", blocks.len() + 1),
            kind,
            boxes,
            confidence: None,
        });
    }
    Built {
        blocks,
        has_content,
        fails_quality_gate,
    }
}

/// Pages of a book's front or back, as a share of its pages, where a printed
/// table of contents can be; and at least this many pages each way.
const CONTENTS_SHARE: usize = 5;
const CONTENTS_MIN_PAGES: usize = 12;

/// A page listing at least this many headings found elsewhere is a table of
/// contents, and the pages next to it listing at least the second number go on
/// with it.
const CONTENTS_TITLES: usize = 5;
const CONTENTS_GOES_ON: usize = 2;

/// The headings a printed table of contents lists, if the book has one. The
/// look of the text finds subheadings as readily as chapters, and only this
/// page tells them apart. Without an outline to ask, it is the book's own word.
fn listed_titles(pages: &[Page], items: &[Item]) -> Option<HashSet<String>> {
    let mut titles: HashMap<String, HashSet<u32>> = HashMap::new();
    for item in items {
        if let Item::Heading {
            text,
            given: None,
            page,
            ..
        } = item
        {
            let letters = text::letters_only(&text.text);
            // Shorter ones are found by chance.
            if letters.chars().count() >= 4 {
                titles.entry(letters).or_default().insert(*page);
            }
        }
    }
    let edge = (pages.len() / CONTENTS_SHARE).max(CONTENTS_MIN_PAGES);
    // What each page at the edges lists, by page index.
    let lists: Vec<Vec<&str>> = pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            if i >= edge && i + edge < pages.len() {
                return Vec::new();
            }
            let all: Vec<String> = page
                .blocks
                .iter()
                .map(|b| text::letters_only(b.str()))
                .collect();
            let all = format!(" {} ", all.join(" "));
            titles
                .iter()
                .filter(|(t, on)| {
                    on.iter().any(|&p| p != page.number) && all.contains(&format!(" {t} "))
                })
                .map(|(t, _)| t.as_str())
                .collect()
        })
        .collect();
    let mut listed = HashSet::new();
    for (i, list) in lists.iter().enumerate() {
        let contents = list.len() >= CONTENTS_TITLES;
        let beside = |other: Option<&Vec<&str>>| other.is_some_and(|l| l.len() >= CONTENTS_TITLES);
        let goes_on = list.len() >= CONTENTS_GOES_ON
            && (beside(lists.get(i + 1)) || i.checked_sub(1).is_some_and(|j| beside(lists.get(j))));
        if contents || goes_on {
            listed.extend(list.iter().map(|t| t.to_string()));
        }
    }
    (!listed.is_empty()).then_some(listed)
}

/// Real prose is about 80% letters. When at least 50 non-space characters
/// decoded and fewer than 45% of them are letters, the fonts did not decode.
/// Titles that only the outline holds do not count: they say nothing of the
/// fonts.
fn fails_quality_gate(items: &[Item]) -> bool {
    let (mut nonspace, mut letters) = (0usize, 0usize);
    let texts = items.iter().filter_map(|item| match item {
        Item::Heading {
            text,
            outline_only: false,
            ..
        }
        | Item::Body(text, _) => Some(text.text.as_str()),
        _ => None,
    });
    for ch in texts.flat_map(str::chars) {
        if !ch.is_whitespace() {
            nonspace += 1;
            if ch.is_alphabetic() {
                letters += 1;
            }
        }
    }
    nonspace >= 50 && (letters as f32) < nonspace as f32 * 0.45
}

/// An outline entry found on its page.
struct Found<'a> {
    mark: &'a Mark,
    /// The blocks up to here hold its title.
    end: usize,
    /// One block holds the whole title, so its own text and styles are the
    /// heading. Otherwise the outline's wording joins the pieces.
    whole: bool,
}

/// Where the page's outline entries sit, by first block. Letters only, so that
/// numbering and capitals do not count, and a title broken over blocks is found
/// when its first block is at least half of it. A running head on the same page
/// is passed over only when it comes first, and that is rare enough to live with.
fn find_marks<'a>(page: &Page, marks: &[&'a Mark]) -> HashMap<usize, Found<'a>> {
    let mut out = HashMap::new();
    if marks.is_empty() {
        return out;
    }
    let letters: Vec<String> = page
        .blocks
        .iter()
        .map(|b| text::letters_only(b.str()))
        .collect();
    let mut from = 0;
    for mark in marks.iter().filter(|m| !m.letters.is_empty()) {
        for at in from..letters.len() {
            let found = title_at(&mark.letters, &letters[at..]);
            if let Some((count, whole)) = found {
                out.insert(
                    at,
                    Found {
                        mark,
                        end: at + count,
                        whole,
                    },
                );
                from = at + count;
                break;
            }
        }
    }
    out
}

/// Does the title start at the first of these blocks? How many blocks it takes,
/// and whether the first alone holds it.
fn title_at(title: &str, blocks: &[String]) -> Option<(usize, bool)> {
    let first = blocks.first()?;
    if first == title {
        return Some((1, true));
    }
    if first.is_empty() || first.len() * 2 < title.len() {
        return None;
    }
    let mut rest = title.strip_prefix(first.as_str())?.strip_prefix(' ')?;
    for (i, block) in blocks.iter().enumerate().skip(1) {
        if block == rest {
            return Some((i + 1, false));
        }
        rest = rest.strip_prefix(block.as_str())?.strip_prefix(' ')?;
    }
    None
}

/// One box per page around these blocks.
fn union(blocks: &[TextBlock]) -> Vec<BlockBox> {
    let mut places = blocks.iter().filter_map(|b| b.place);
    let Some(mut out) = places.next() else {
        return Vec::new();
    };
    for b in places {
        let (right, bottom) = (out.x + out.width, out.y + out.height);
        out.x = out.x.min(b.x);
        out.y = out.y.min(b.y);
        out.width = right.max(b.x + b.width) - out.x;
        out.height = bottom.max(b.y + b.height) - out.y;
    }
    vec![out]
}

/// The text with its line-break hyphens gone, whitespace collapsed.
fn without_hyphens(text: &Marked, dehyphen: &Regex) -> Marked {
    let cut: Vec<_> = dehyphen
        .captures_iter(&text.text)
        .filter_map(|c| Some(c.get(1)?.end()..c.get(2)?.start()))
        .collect();
    if cut.is_empty() {
        text.squashed()
    } else {
        text.without(&cut).squashed()
    }
}

/// A page's first block continues the paragraph the last page ended on, if
/// that one stopped mid-sentence and this one starts in lower case. Figures
/// between them stay where they are. Returns whether it was joined.
fn continue_across_page(
    items: &mut [Item],
    clean: &Marked,
    block: &TextBlock,
    dehyphen: &Regex,
) -> bool {
    if !clean.text.chars().next().is_some_and(char::is_lowercase) {
        return false;
    }
    let last = items
        .iter_mut()
        .rev()
        .find(|i| !matches!(i, Item::Figure(_)));
    let Some(Item::Body(body, boxes)) = last else {
        return false;
    };
    if body
        .text
        .ends_with(['.', '!', '?', ':', ';', '…', '"', '”', '’', ')'])
    {
        return false;
    }
    body.join(clean);
    *body = without_hyphens(body, dehyphen);
    boxes.extend(block.place);
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
            seen.entry(template(first.str())).or_default().push(n);
        }
        if page.blocks.len() > 1
            && let Some(last) = page.blocks.last()
        {
            seen.entry(template(last.str())).or_default().push(n);
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
            blocks: blocks
                .iter()
                .map(|s| TextBlock::plain(s.to_string()))
                .collect(),
            born_digital: true,
            ..Page::default()
        }
    }

    /// The pages' blocks, numbered from page 1.
    fn blocks(pages: &[Page]) -> Vec<Block> {
        blocks_with(pages, None)
    }

    fn blocks_with(pages: &[Page], marks: Option<&[Mark]>) -> Vec<Block> {
        let pages: Vec<Page> = pages
            .iter()
            .enumerate()
            .map(|(i, p)| Page {
                number: i as u32 + 1,
                ..p.clone()
            })
            .collect();
        build(&pages, marks).blocks
    }

    fn body(blocks: &[Block]) -> String {
        paragraphs(blocks).join(" ")
    }

    fn paragraphs(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Paragraph { .. }))
            .map(Block::text)
            .collect()
    }

    fn headings(blocks: &[Block]) -> Vec<(String, u32)> {
        blocks
            .iter()
            .filter_map(|b| Some((b.text(), b.level()?)))
            .collect()
    }

    fn mark(page: u32, title: &str, level: u32) -> Mark {
        Mark {
            page,
            title: title.to_string(),
            letters: text::letters_only(title),
            level,
        }
    }

    #[test]
    fn joins_line_break_hyphens() {
        let text = body(&blocks(&[page(&["The inter-  national group met today."])]));
        assert!(text.contains("international"), "{text}");
        assert!(!text.contains("inter-"), "{text}");
    }

    #[test]
    fn joins_turkish_line_break_hyphens() {
        let text = body(&blocks(&[page(&[
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
        let text = body(&blocks(&pages));
        assert!(!text.contains("MY BOOK TITLE"), "{text}");
        assert!(text.contains("alpha") && text.contains("hotel"), "{text}");
    }

    #[test]
    fn strips_page_numbers() {
        let text = body(&blocks(&[page(&[
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
        let got = blocks(&[page(&["CHAPTER ONE", &b])]);
        assert_eq!(headings(&got), [("CHAPTER ONE".to_string(), 1)]);
    }

    #[test]
    fn turkish_caps_heading_starts_a_chapter() {
        let b = "söz ".repeat(120);
        let got = blocks(&[page(&["BİRİNCİ BÖLÜM", &b])]);
        assert_eq!(headings(&got), [("BİRİNCİ BÖLÜM".to_string(), 1)]);
    }

    #[test]
    fn paragraph_continues_across_a_page() {
        let text = body(&blocks(&[
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
        let text = body(&blocks(&[page(&["Öğren-"]), page(&["cilerle geldi."])]));
        assert_eq!(text, "Öğrencilerle geldi.");
    }

    #[test]
    fn finished_sentence_does_not_continue() {
        let got = blocks(&[page(&["It ended."]), page(&["and this began."])]);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn a_head_on_three_pages_goes_even_in_a_long_book() {
        let mut pages: Vec<Page> = (0..300)
            .map(|i| page(&[&format!("Paragraph number {i} with its own words.")]))
            .collect();
        for p in &mut pages[10..13] {
            p.blocks
                .insert(0, TextBlock::plain("A SHORT CHAPTER".to_string()));
        }
        let text = body(&blocks(&pages));
        assert!(!text.contains("A SHORT CHAPTER"), "{text}");
    }

    #[test]
    fn chapter_body_counts_characters_not_bytes() {
        // 300 characters, 600 bytes: too little body to start a chapter.
        let b = "ş".repeat(300);
        let got = blocks(&[page(&["BİRİNCİ BÖLÜM", &b])]);
        assert!(headings(&got).is_empty());
    }

    #[test]
    fn heading_without_body_is_demoted() {
        let got = blocks(&[page(&["TITLE", "AUTHOR", "PUBLISHER"])]);
        assert!(headings(&got).is_empty());
    }

    #[test]
    fn an_outline_entry_found_on_its_page_is_a_heading_at_its_level() {
        let marks = [mark(1, "1. Her Name", 1), mark(1, "Inner Part", 2)];
        let pages = [page(&[
            "Her Name",
            "A short one.",
            "Inner Part",
            "Another.",
        ])];
        let got = blocks_with(&pages, Some(&marks));
        assert_eq!(
            headings(&got),
            [("Her Name".to_string(), 1), ("Inner Part".to_string(), 2)]
        );
    }

    #[test]
    fn with_a_table_of_contents_a_lookalike_heading_starts_no_chapter() {
        let b = "söz ".repeat(120);
        let marks = [mark(1, "Her Name", 1), mark(2, "Second", 1)];
        let pages = [
            page(&["Her Name", &b, "LOUD WORDS", &b]),
            page(&["Second", &b]),
        ];
        let got = blocks_with(&pages, Some(&marks));
        let levels: Vec<_> = headings(&got).into_iter().map(|h| h.1).collect();
        assert_eq!(levels, [1, 2, 1]);
    }

    #[test]
    fn an_outline_entry_not_on_its_page_still_starts_a_chapter() {
        let marks = [mark(2, "Hidden Title", 1), mark(2, "Hidden Inner", 2)];
        let pages = [page(&["First page."]), page(&["Second page."])];
        let got = blocks_with(&pages, Some(&marks));
        assert_eq!(headings(&got), [("Hidden Title".to_string(), 1)]);
        assert_eq!(got[1].text(), "Hidden Title");
        assert!(got[1].boxes.is_empty());
    }

    #[test]
    fn a_title_broken_over_blocks_is_one_heading() {
        let marks = [mark(1, "A Long Title Over Two", 1), mark(2, "Next", 1)];
        let pages = [
            page(&["A Long Title", "Over Two", "Body."]),
            page(&["Next"]),
        ];
        let got = blocks_with(&pages, Some(&marks));
        assert_eq!(got[0].text(), "A Long Title Over Two");
        assert_eq!(paragraphs(&got), ["Body."]);
    }

    #[test]
    fn a_title_repeated_as_a_running_head_stays_at_its_entry() {
        let filler = |i: usize| {
            format!(
                "Page {} words here.",
                ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"][i]
            )
        };
        let marks = [mark(1, "Her Name", 1), mark(6, "Other Place", 1)];
        let pages: Vec<Page> = (0..6).map(|i| page(&["Her Name", &filler(i)])).collect();
        let got = blocks_with(&pages, Some(&marks));
        let names: Vec<_> = headings(&got).into_iter().map(|h| h.0).collect();
        assert_eq!(names, ["Her Name", "Other Place"]);
    }

    #[test]
    fn an_entry_on_the_next_page_is_found_there() {
        let marks = [mark(1, "Her Name", 1), mark(9, "Far Away", 1)];
        let pages = [page(&[]), page(&["Her Name", "Body."])];
        let got = blocks_with(&pages, Some(&marks));
        assert_eq!(got[0].text(), "Her Name");
        assert_eq!(paragraphs(&got), ["Body."]);
    }

    #[test]
    fn a_printed_table_of_contents_tells_chapters_from_subheadings() {
        let b = "söz ".repeat(120);
        let titles = [
            "FIRST ONE",
            "SECOND ONE",
            "THIRD ONE",
            "FOURTH ONE",
            "FIFTH ONE",
        ];
        let contents = titles.map(|t| format!("{t} ... 9")).join(" | ");
        let mut pages = vec![page(&[&contents])];
        for t in titles {
            // Its own words, or the body would recur as a running foot.
            let own = format!("{} {b}", t.to_lowercase());
            pages.push(page(&[t, &b, "A SUBHEADING HERE", &own]));
        }
        let got = blocks(&pages);
        let levels: Vec<u32> = headings(&got).into_iter().map(|h| h.1).collect();
        assert_eq!(levels.iter().filter(|&&l| l == 1).count(), 5);
        assert_eq!(levels.iter().filter(|&&l| l == 2).count(), 5);
    }

    #[test]
    fn without_a_contents_page_every_lookalike_heading_starts_a_chapter() {
        let b = "söz ".repeat(120);
        let pages = [page(&["FIRST ONE", &b]), page(&["SECOND ONE", &b])];
        let levels: Vec<u32> = headings(&blocks(&pages)).into_iter().map(|h| h.1).collect();
        assert_eq!(levels, [1, 1]);
    }

    #[test]
    fn outline_titles_alone_are_no_content() {
        let pages = [Page {
            number: 1,
            ..Page::default()
        }];
        let marks = [mark(1, "Her Name", 1)];
        let built = build(&pages, Some(&marks));
        assert_eq!(built.blocks.len(), 1);
        assert!(!built.has_content);
    }

    #[test]
    fn hyphens_go_and_italics_stay_in_step() {
        let mut text = Marked::plain("plain ".to_string());
        text.join(&Marked {
            text: "kita- bı".to_string(),
            italic: vec![true; 8],
        });
        let got = without_hyphens(&text, &Regex::new(r"(\p{L})-\s+(\p{Ll})").unwrap());
        assert_eq!(got.text, "plain kitabı");
        let italic: String = got
            .runs()
            .into_iter()
            .filter(|r| r.italic)
            .map(|r| r.text)
            .collect();
        assert_eq!(italic, "kitabı");
    }
}
