//! A guard in front of lopdf. When a stream's `/Length` is a reference, lopdf
//! reads it by parsing the object it points at, and if that is a stream with
//! a referenced `/Length` of its own, the one after it, with no bound on the
//! depth. A chain of a few thousand such streams overflows the stack, and a
//! stack overflow aborts the process: it cannot be caught
//! (GHSA-4597-5x94-x3xg, lopdf 0.47.0 and earlier).
//!
//! A length is a number, so a `/Length` pointing at an object that has a
//! `/Length` of its own is never right. Such links are counted before lopdf
//! sees the file, and too many refuse it.
//!
//! lopdf parses the objects its cross-reference table points at. That table
//! is not read here: every place an object could start is tried instead, so
//! nothing lopdf could parse is missed, and the grammar is lopdf's own. This
//! only ever counts more links than lopdf would follow, never fewer.

use std::collections::{HashMap, HashSet};

use crate::Error;

/// More links than this refuse the document. A correct file has none, and
/// the smallest stack the library may run on (a browser's) holds far more.
const MAX_LINKS: usize = 32;

/// lopdf's limit on nesting, which bounds the recursion here too.
const MAX_NESTING_DEPTH: usize = 100;

/// Refuse a PDF whose stream lengths chain deep enough to overflow lopdf.
pub(crate) fn check(pdf: &[u8]) -> Result<(), Error> {
    match links(pdf) {
        Some(n) if n <= MAX_LINKS => Ok(()),
        Some(n) => Err(Error::Open(format!(
            "{n} streams take their length from an object with a length of its own; \
             a chain this long would overflow the parser's stack"
        ))),
        None => Err(Error::Open(
            "its objects overlap too much to check their stream lengths".into(),
        )),
    }
}

/// How many objects have a `/Length` pointing at an object with a referenced
/// `/Length` of its own. `None` when the objects overlap so much that
/// checking them would cost far more than the file's size.
fn links(pdf: &[u8]) -> Option<usize> {
    let budget = pdf.len().saturating_mul(4).saturating_add(1 << 20);
    let mut work = 0usize;
    let mut lengths: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut i = 0;
    while i < pdf.len() {
        if !pdf[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let run = i;
        while i < pdf.len() && pdf[i].is_ascii_digit() {
            i += 1;
        }
        let Some(body) = header_end(pdf, i) else {
            continue;
        };
        let mut parser = Parser { pdf, pos: body };
        let mut found = Vec::new();
        parser.dictionary(MAX_NESTING_DEPTH, Some(&mut found));
        work = work.saturating_add(parser.pos - body);
        if work > budget {
            return None;
        }
        if !found.is_empty() {
            for id in suffix_ids(&pdf[run..i]) {
                lengths.entry(id).or_default().extend_from_slice(&found);
            }
        }
    }
    let chained: HashSet<u32> = lengths
        .iter()
        .filter(|(_, to)| to.iter().any(|t| lengths.contains_key(t)))
        .map(|(&id, _)| id)
        .collect();
    Some(chained.len())
}

/// After an object number ending at `i`: the generation, `obj`, and the
/// space after it, as lopdf reads them. Where the object's value starts.
fn header_end(pdf: &[u8], i: usize) -> Option<usize> {
    let p = Parser { pdf, pos: i };
    let gen_start = p.space(i);
    let gen_end = digits_end(pdf, gen_start);
    if gen_end == gen_start || parse::<u16>(&pdf[gen_start..gen_end]).is_none() {
        return None;
    }
    let obj = p.space(gen_end);
    pdf[obj..].starts_with(b"obj").then(|| p.space(obj + 3))
}

/// The object numbers lopdf could read from a run of digits: an offset may
/// point anywhere in it, so every suffix that fits a `u32`.
fn suffix_ids(run: &[u8]) -> Vec<u32> {
    let mut ids: Vec<u32> = Vec::new();
    let (mut value, mut place) = (0u64, 1u64);
    for &d in run.iter().rev() {
        let d = u64::from(d - b'0');
        if d != 0 {
            value += d.saturating_mul(place);
            if place > u64::from(u32::MAX) || value > u64::from(u32::MAX) {
                break;
            }
        }
        place = place.saturating_mul(10);
        let id = value as u32;
        if ids.last() != Some(&id) {
            ids.push(id);
        }
    }
    ids
}

fn digits_end(pdf: &[u8], mut i: usize) -> usize {
    while i < pdf.len() && pdf[i].is_ascii_digit() {
        i += 1;
    }
    i
}

fn parse<T: std::str::FromStr>(digits: &[u8]) -> Option<T> {
    std::str::from_utf8(digits).ok()?.parse().ok()
}

fn is_whitespace(c: u8) -> bool {
    b" \t\n\r\0\x0C".contains(&c)
}

fn is_regular(c: u8) -> bool {
    !is_whitespace(c) && !b"()<>[]{}/%".contains(&c)
}

/// lopdf's grammar for direct objects, enough to find where each ends and
/// which value a dictionary's `/Length` has. A `None` is where lopdf fails.
struct Parser<'a> {
    pdf: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn at(&self, tag: &[u8]) -> bool {
        self.pdf[self.pos..].starts_with(tag)
    }

    fn peek(&self) -> Option<u8> {
        self.pdf.get(self.pos).copied()
    }

    fn eat(&mut self, tag: &[u8]) -> Option<()> {
        self.at(tag).then(|| self.pos += tag.len())
    }

    /// Whitespace and comments from `i`. A comment needs its end of line.
    fn space(&self, mut i: usize) -> usize {
        loop {
            match self.pdf.get(i) {
                Some(&c) if is_whitespace(c) => i += 1,
                Some(b'%') => {
                    let Some(eol) = self.pdf[i..].iter().position(|&c| c == b'\r' || c == b'\n')
                    else {
                        return i;
                    };
                    i += eol + 1;
                    if self.pdf[i - 1] == b'\r' && self.pdf.get(i) == Some(&b'\n') {
                        i += 1;
                    }
                }
                _ => return i,
            }
        }
    }

    fn skip_space(&mut self) {
        self.pos = self.space(self.pos);
    }

    /// `<< (name value)* >>`. With `found`, the targets of the referenced
    /// `/Length` values at this level are pushed to it.
    fn dictionary(&mut self, depth: usize, mut found: Option<&mut Vec<u32>>) -> Option<()> {
        self.eat(b"<<")?;
        self.skip_space();
        loop {
            if self.eat(b">>").is_some() {
                return Some(());
            }
            let key = self.name()?;
            self.skip_space();
            let reference = self.direct_object(depth)?;
            if let (Some(found), Some(id)) = (found.as_deref_mut(), reference)
                && key == b"Length"
            {
                found.push(id);
            }
        }
    }

    /// A value and the space after it. `Some(Some(id))` for a reference.
    fn direct_object(&mut self, depth: usize) -> Option<Option<u32>> {
        if depth == 0 {
            return None;
        }
        let reference = self.value(depth - 1)?;
        self.skip_space();
        Some(reference)
    }

    /// One value, tried in lopdf's order.
    fn value(&mut self, depth: usize) -> Option<Option<u32>> {
        if self.eat(b"null").is_some() || self.eat(b"true").is_some() {
            return Some(None);
        }
        if self.eat(b"false").is_some() {
            return Some(None);
        }
        if let Some(id) = self.reference() {
            return Some(Some(id));
        }
        match self.peek()? {
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number(),
            b'/' => self.name().map(|_| ()),
            b'(' => self.literal_string(),
            b'[' => self.array(depth),
            b'<' => {
                let start = self.pos;
                if self.hex_string().is_some() {
                    return Some(None);
                }
                self.pos = start;
                self.dictionary(depth, None)
            }
            _ => None,
        }
        .map(|()| None)
    }

    /// `num space gen space R`, the numbers as `u32` and `u16`.
    fn reference(&mut self) -> Option<u32> {
        let start = self.pos;
        let id_end = digits_end(self.pdf, start);
        let id = parse::<u32>(&self.pdf[start..id_end]);
        let gen_start = self.space(id_end);
        let gen_end = digits_end(self.pdf, gen_start);
        let generation = parse::<u16>(&self.pdf[gen_start..gen_end]);
        let r = self.space(gen_end);
        match (id, generation, self.pdf.get(r)) {
            (Some(id), Some(_), Some(b'R')) => {
                self.pos = r + 1;
                Some(id)
            }
            _ => None,
        }
    }

    /// A real (`1.`, `.5`, `1.5`) or an integer that fits an `i64`.
    fn number(&mut self) -> Option<()> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let int_end = digits_end(self.pdf, self.pos);
        let has_int = int_end > self.pos;
        if self.pdf.get(int_end) == Some(&b'.') {
            let frac_end = digits_end(self.pdf, int_end + 1);
            if has_int || frac_end > int_end + 1 {
                self.pos = frac_end;
                return Some(());
            }
        }
        if has_int && parse::<i64>(&self.pdf[start..int_end]).is_some() {
            self.pos = int_end;
            return Some(());
        }
        None
    }

    /// `/` and the name, `#xx` decoded. Ends at the first byte that is not
    /// part of one, as lopdf's does.
    fn name(&mut self) -> Option<Vec<u8>> {
        self.eat(b"/")?;
        let mut out = Vec::new();
        loop {
            match self.pdf.get(self.pos..self.pos + 3) {
                Some([b'#', h, l]) if h.is_ascii_hexdigit() && l.is_ascii_hexdigit() => {
                    let hex = [*h, *l];
                    out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
                    self.pos += 3;
                    continue;
                }
                _ => {}
            }
            match self.peek() {
                Some(c) if c != b'#' && is_regular(c) => {
                    out.push(c);
                    self.pos += 1;
                }
                _ => return Some(out),
            }
        }
    }

    /// `( ... )`, nested parentheses balanced, a backslash escaping what
    /// follows.
    fn literal_string(&mut self) -> Option<()> {
        self.eat(b"(")?;
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => self.pos = (self.pos + 1).min(self.pdf.len()),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `<` hex digits and whitespace `>`.
    fn hex_string(&mut self) -> Option<()> {
        self.eat(b"<")?;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'>' => return Some(()),
                c if c.is_ascii_hexdigit() || is_whitespace(c) => {}
                _ => return None,
            }
        }
        None
    }

    /// `[ value* ]`.
    fn array(&mut self, depth: usize) -> Option<()> {
        self.eat(b"[")?;
        self.skip_space();
        loop {
            if self.eat(b"]").is_some() {
                return Some(());
            }
            self.direct_object(depth)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Objects 1..=n, each a stream whose `/Length` is the next one.
    fn chain(n: u32) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        for id in 1..=n {
            let length = if id == n {
                "1".to_string()
            } else {
                format!("{} 0 R", id + 1)
            };
            out.extend(
                format!("{id} 0 obj\n<< /Length {length} >>\nstream\nA\nendstream\nendobj\n")
                    .bytes(),
            );
        }
        out
    }

    #[test]
    fn a_chain_is_counted() {
        assert_eq!(links(&chain(1)), Some(0));
        assert_eq!(links(&chain(2)), Some(0));
        assert_eq!(links(&chain(3)), Some(1));
        // Object 0 is read from the 0 that ends 10, 20 and so on: counting
        // more than lopdf would follow is the safe side.
        assert_eq!(links(&chain(1200)), Some(1199));
        assert!(check(&chain(9)).is_ok());
        assert!(matches!(check(&chain(100)), Err(Error::Open(_))));
    }

    #[test]
    fn a_length_kept_in_a_number_object_is_no_link() {
        let mut pdf = Vec::new();
        for id in 1..=100 {
            pdf.extend(
                format!(
                    "{id} 0 obj << /Length {} 0 R >> stream\nA\nendstream endobj\n\
                     {} 0 obj 1 endobj\n",
                    id + 1000,
                    id + 1000
                )
                .bytes(),
            );
        }
        assert_eq!(links(&pdf), Some(0));
    }

    #[test]
    fn spellings_lopdf_accepts_are_counted() {
        // An escaped name, comments as space, no space before `R` or after
        // `obj`, other keys and strings around it, a digit run before the id.
        let pdf = b"1 0 obj<</Len#67th%x\n2%y\n0R>>endobj\n\
                    92 0 obj << /A (x) /B <00> /C [1 2.5 /D] /Length 3 0 R >> endobj\n\
                    3 0 obj << /Length 1 0 R >> endobj\n";
        assert_eq!(links(pdf), Some(4));
    }

    #[test]
    fn a_length_only_inside_a_string_or_a_nested_dictionary_is_no_link() {
        let pdf = b"1 0 obj << /A (/Length 2 0 R) /B << /Length 2 0 R >> >> endobj\n\
                    2 0 obj << /Length 1 0 R >> endobj\n";
        assert_eq!(links(pdf), Some(0));
    }

    #[test]
    fn suffixes_of_a_digit_run() {
        assert_eq!(suffix_ids(b"912"), vec![2, 12, 912]);
        assert_eq!(suffix_ids(b"0012"), vec![2, 12]);
        assert_eq!(
            suffix_ids(b"99999999999"),
            vec![
                9, 99, 999, 9999, 99999, 999999, 9999999, 99999999, 999999999
            ]
        );
    }
}
