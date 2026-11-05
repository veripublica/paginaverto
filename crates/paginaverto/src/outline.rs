//! The PDF's own table of contents: the document outline, or bookmarks.
//!
//! Written here rather than taken from `lopdf::Document::get_toc`, which gives
//! up on the whole outline over one entry without a destination, and panics on
//! some malformed named destinations. A file can make this tree as deep or as
//! circular as it likes, so nothing here recurses and no node is visited twice.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

/// One entry of the PDF's outline, as the file gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutlineEntry {
    /// How deep it sits; the top level is 1.
    pub level: u32,
    /// Its title, whitespace collapsed to single spaces.
    pub title: String,
    /// The page it points to, from 1. `None` when it points nowhere this file
    /// can say: to another file, to a link, or to a page that does not exist.
    pub page: Option<u32>,
}

/// The outline's entries in reading order. Empty when there is none.
pub(crate) fn outline(doc: &Document) -> Vec<OutlineEntry> {
    let Some(root) = doc
        .catalog()
        .ok()
        .and_then(|c| dict_at(doc, c, b"Outlines"))
    else {
        return Vec::new();
    };
    let pages: HashMap<ObjectId, u32> =
        doc.get_pages().into_iter().map(|(n, id)| (id, n)).collect();
    let mut named = None;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack: Vec<(ObjectId, u32)> = Vec::new();
    if let Ok(Object::Reference(first)) = root.get(b"First") {
        stack.push((*first, 1));
    }
    while let Some((id, level)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Ok(node) = doc.get_dictionary(id) else {
            continue;
        };
        // The next sibling waits under the children, so entries come in reading order.
        if let Ok(Object::Reference(next)) = node.get(b"Next") {
            stack.push((*next, level));
        }
        if let Ok(Object::Reference(first)) = node.get(b"First") {
            stack.push((*first, level.saturating_add(1)));
        }
        if let Some(title) = title(doc, node) {
            let named = named.get_or_insert_with(|| named_destinations(doc));
            out.push(OutlineEntry {
                level,
                title,
                page: target(doc, node, named, &pages),
            });
        }
    }
    out
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn dict_at<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    resolve(doc, d.get(key).ok()?)?.as_dict().ok()
}

/// A text string from the file: UTF-16, UTF-8 or PDFDocEncoding. lopdf keeps a
/// UTF-8 byte order mark as a character, so it goes here.
pub(crate) fn text_string(o: &Object) -> Option<String> {
    let text = decode_text_string(o).ok().or_else(|| {
        o.as_str()
            .ok()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    })?;
    Some(text.replace('\u{FEFF}', ""))
}

fn title(doc: &Document, node: &Dictionary) -> Option<String> {
    let text = text_string(resolve(doc, node.get(b"Title").ok()?)?)?;
    let kept = crate::text::squash(&text);
    (!kept.is_empty()).then_some(kept)
}

/// The page an entry points to: its `/Dest`, or a `GoTo` action's `/D`.
fn target(
    doc: &Document,
    node: &Dictionary,
    named: &HashMap<Vec<u8>, Object>,
    pages: &HashMap<ObjectId, u32>,
) -> Option<u32> {
    let dest = match node.get(b"Dest") {
        Ok(d) => d,
        Err(_) => {
            let action = dict_at(doc, node, b"A")?;
            if action.get(b"S").ok()?.as_name().ok()? != b"GoTo" {
                return None;
            }
            action.get(b"D").ok()?
        }
    };
    page_of(doc, dest, named, pages)
}

/// A destination's page. A destination is an array starting with the page, a
/// name or string to look up, or a dictionary holding one under `/D`.
fn page_of(
    doc: &Document,
    dest: &Object,
    named: &HashMap<Vec<u8>, Object>,
    pages: &HashMap<ObjectId, u32>,
) -> Option<u32> {
    // Named destinations can point to each other; two steps is all a file needs.
    let mut dest = dest;
    for _ in 0..3 {
        match resolve(doc, dest)? {
            Object::Array(a) => {
                return match a.first()? {
                    Object::Reference(id) => pages.get(id).copied(),
                    // Some writers give the page's index, as remote links do.
                    Object::Integer(i) => {
                        let n = u32::try_from(*i).ok()?.checked_add(1)?;
                        pages.values().any(|&p| p == n).then_some(n)
                    }
                    _ => None,
                };
            }
            Object::Dictionary(d) => dest = d.get(b"D").ok()?,
            Object::Name(key) | Object::String(key, _) => dest = named.get(key.as_slice())?,
            _ => return None,
        }
    }
    None
}

/// Every named destination: the catalog's `/Dests` dictionary and the
/// `/Names` tree's `/Dests`, the newer form, which wins.
fn named_destinations(doc: &Document) -> HashMap<Vec<u8>, Object> {
    let mut out = HashMap::new();
    let Ok(catalog) = doc.catalog() else {
        return out;
    };
    if let Some(dests) = dict_at(doc, catalog, b"Dests") {
        for (name, value) in dests {
            out.insert(name.clone(), value.clone());
        }
    }
    let Some(tree) = dict_at(doc, catalog, b"Names").and_then(|n| n.get(b"Dests").ok()) else {
        return out;
    };
    let mut seen = HashSet::new();
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        if let Object::Reference(id) = node
            && !seen.insert(*id)
        {
            continue;
        }
        let Some(dict) = resolve(doc, node).and_then(|o| o.as_dict().ok()) else {
            continue;
        };
        if let Some(kids) = dict
            .get(b"Kids")
            .ok()
            .and_then(|k| resolve(doc, k))
            .and_then(|k| k.as_array().ok())
        {
            stack.extend(kids.iter());
        }
        if let Some(names) = dict
            .get(b"Names")
            .ok()
            .and_then(|n| resolve(doc, n))
            .and_then(|n| n.as_array().ok())
        {
            for pair in names.as_chunks::<2>().0 {
                if let Ok(key) = pair[0].as_str() {
                    out.insert(key.to_vec(), pair[1].clone());
                }
            }
        }
    }
    out
}

/// An outline entry a chapter can start at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mark {
    pub page: u32,
    pub title: String,
    /// The title's letters, for matching against the page's text.
    pub letters: String,
    /// 1 starts a chapter.
    pub level: u32,
}

/// The outline's entries, if it is a table of contents. Many are not: scanning
/// software writes one entry per page named after its number, and those have
/// next to no distinct titles. On the 18 PDFs measured, real outlines had every
/// title distinct and the others at most a fifth, so half is a safe line.
///
/// When a single entry sits at the top (the book's own title), the level below
/// it is the one that holds the chapters.
pub(crate) fn chapter_marks(entries: &[OutlineEntry]) -> Option<Vec<Mark>> {
    let placed: Vec<_> = entries.iter().filter(|e| e.page.is_some()).collect();
    if placed.len() < 2 || placed.len() * 2 < entries.len() {
        return None;
    }
    let letters: Vec<String> = entries
        .iter()
        .map(|e| crate::text::letters_only(&e.title))
        .collect();
    let distinct: HashSet<&str> = letters
        .iter()
        .map(String::as_str)
        .filter(|l| !l.is_empty())
        .collect();
    if distinct.len() * 2 < entries.len() {
        return None;
    }
    let mut top = placed.iter().map(|e| e.level).min()?;
    let deepest = placed.iter().map(|e| e.level).max()?;
    while top < deepest && placed.iter().filter(|e| e.level == top).count() == 1 {
        top += 1;
    }
    Some(
        entries
            .iter()
            .zip(letters)
            .filter_map(|(e, letters)| {
                Some(Mark {
                    page: e.page?,
                    title: e.title.clone(),
                    letters,
                    level: (e.level + 1).saturating_sub(top).max(1),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: u32, title: &str, page: u32) -> OutlineEntry {
        OutlineEntry {
            level,
            title: title.into(),
            page: Some(page),
        }
    }

    #[test]
    fn a_page_per_entry_outline_is_no_table_of_contents() {
        let entries: Vec<_> = (1..=40)
            .map(|n| entry(1, &format!("Page {n}"), n))
            .collect();
        assert_eq!(chapter_marks(&entries), None);
    }

    #[test]
    fn a_real_outline_keeps_its_levels() {
        let e = [entry(1, "One", 1), entry(2, "Inner", 2), entry(1, "Two", 3)];
        let levels: Vec<_> = chapter_marks(&e).unwrap().iter().map(|m| m.level).collect();
        assert_eq!(levels, [1, 2, 1]);
    }

    #[test]
    fn a_single_top_entry_gives_way_to_the_level_below() {
        let e = [entry(1, "Book", 1), entry(2, "One", 2), entry(2, "Two", 3)];
        let levels: Vec<_> = chapter_marks(&e).unwrap().iter().map(|m| m.level).collect();
        assert_eq!(levels, [1, 1, 1]);
    }

    #[test]
    fn one_entry_is_no_table_of_contents() {
        assert_eq!(chapter_marks(&[entry(1, "Only", 1)]), None);
    }
}
