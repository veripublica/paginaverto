//! What a PDF may cost before it is refused. A PDF is an attack surface: a few
//! kilobytes can inflate to gigabytes. Everything is checked once, right after
//! loading, so the steps after it need no checks of their own.

use std::fmt;

use lopdf::xref::XrefEntry;
use lopdf::{Document, Object, ObjectId, Stream};

use crate::{Error, Input};

/// Deflate cannot expand by more than this, so a stream small enough can be
/// passed without decoding it.
const DEFLATE_MAX_RATIO: u64 = 1032;

/// Limits on one document. Going over any of them refuses the whole document
/// with [`Error::LimitExceeded`]. They bound the work, and with it the memory,
/// but are not a guarantee of a memory ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The size of the PDF file.
    pub max_file_bytes: u64,
    /// The number of pages.
    pub max_pages: u64,
    /// What one stream may decode to.
    pub max_stream_bytes: u64,
    /// What one page's content streams may decode to, together. Far lower
    /// than the stream limit, since this text is parsed into operations.
    pub max_page_content_bytes: u64,
    /// The pixels of one image, as its dictionary declares them.
    pub max_image_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 4 << 30,
            max_pages: 50_000,
            max_stream_bytes: 1 << 30,
            max_page_content_bytes: 128 << 20,
            max_image_pixels: 256_000_000,
        }
    }
}

/// Which of the [`Limits`] a document went over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitKind {
    /// [`Limits::max_file_bytes`]
    FileBytes,
    /// [`Limits::max_pages`]
    Pages,
    /// [`Limits::max_stream_bytes`]
    StreamBytes,
    /// [`Limits::max_page_content_bytes`]
    PageContentBytes,
    /// [`Limits::max_image_pixels`]
    ImagePixels,
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LimitKind::FileBytes => "bytes in the file",
            LimitKind::Pages => "pages",
            LimitKind::StreamBytes => "bytes in one decoded stream",
            LimitKind::PageContentBytes => "bytes in one page's decoded content",
            LimitKind::ImagePixels => "pixels in one image",
        })
    }
}

impl Limits {
    pub(crate) fn exceeded(what: LimitKind, limit: u64) -> Error {
        Error::LimitExceeded { what, limit }
    }

    /// Check a loaded document. Decodes only what could go over.
    pub(crate) fn check(&self, doc: &Document) -> Result<(), Error> {
        let pages = doc.get_pages();
        if pages.len() as u64 > self.max_pages {
            return Err(Self::exceeded(LimitKind::Pages, self.max_pages));
        }
        for object in doc.objects.values() {
            let Object::Stream(stream) = object else {
                continue;
            };
            if crate::text::pixel_area(stream) as u64 > self.max_image_pixels
                && stream.dict.get(b"Subtype").and_then(Object::as_name).ok()
                    == Some(b"Image".as_slice())
            {
                return Err(Self::exceeded(
                    LimitKind::ImagePixels,
                    self.max_image_pixels,
                ));
            }
            if self.stream_too_large(stream) {
                return Err(Self::exceeded(
                    LimitKind::StreamBytes,
                    self.max_stream_bytes,
                ));
            }
        }
        // The same stream can be listed many times, so each one passing does
        // not make a page pass.
        let max = usize_limit(self.max_page_content_bytes);
        for id in pages.into_values() {
            if let Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded {
                ..
            })) = doc.get_page_content_with_limit(id, max)
            {
                return Err(Self::exceeded(
                    LimitKind::PageContentBytes,
                    self.max_page_content_bytes,
                ));
            }
        }
        Ok(())
    }

    fn stream_too_large(&self, stream: &Stream) -> bool {
        let raw = stream.content.len() as u64;
        match stream.filters().as_deref() {
            Err(_) | Ok([]) => return raw > self.max_stream_bytes,
            Ok([b"FlateDecode"])
                if raw.saturating_mul(DEFLATE_MAX_RATIO) <= self.max_stream_bytes =>
            {
                return false;
            }
            _ => {}
        }
        matches!(
            stream.decompressed_content_with_limit(usize_limit(self.max_stream_bytes)),
            Err(lopdf::Error::Decompress(
                lopdf::DecompressError::MemoryLimitExceeded { .. }
            ))
        )
    }

    /// Refuse too large a file before reading it. A file that cannot be read
    /// is left for the load to report.
    pub(crate) fn check_file(&self, input: Input<'_>) -> Result<(), Error> {
        let bytes = match input {
            Input::Path(p) => std::fs::metadata(p).map_or(0, |m| m.len()),
            Input::Bytes(b) => b.len() as u64,
        };
        if bytes > self.max_file_bytes {
            return Err(Self::exceeded(LimitKind::FileBytes, self.max_file_bytes));
        }
        Ok(())
    }

    /// Load a PDF, its object streams bounded by the stream limit.
    pub(crate) fn load(&self, input: Input<'_>) -> Result<Document, Error> {
        let options =
            lopdf::LoadOptions::with_max_decompressed_size(usize_limit(self.max_stream_bytes));
        let doc = load_with(input, options).map_err(|e| match e {
            lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { .. }) => {
                Self::exceeded(LimitKind::StreamBytes, self.max_stream_bytes)
            }
            e => Error::Open(crate::with_causes(&e)),
        })?;
        if self.dropped_too_large_object_stream(input, &doc) {
            return Err(Self::exceeded(
                LimitKind::StreamBytes,
                self.max_stream_bytes,
            ));
        }
        Ok(doc)
    }

    /// lopdf drops an object stream it refuses, and the objects in it, without
    /// saying so: the document would then look like something it is not, such
    /// as a scan. So when a container is missing, look at the containers again,
    /// this time undecoded.
    fn dropped_too_large_object_stream(&self, input: Input<'_>, doc: &Document) -> bool {
        let missing = doc.reference_table.entries.values().any(|entry| {
            matches!(entry, XrefEntry::Compressed { container, .. }
                if !doc.objects.contains_key(&(*container, 0)))
        });
        if !missing {
            return false;
        }
        let Ok(raw) = load_with(input, lopdf::LoadOptions::with_filter(keep_undecoded)) else {
            return false;
        };
        raw.objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .any(|s| self.stream_too_large(s))
    }
}

fn load_with(input: Input<'_>, options: lopdf::LoadOptions) -> lopdf::Result<Document> {
    match input {
        Input::Path(p) => Document::load_with_options(p, options),
        Input::Bytes(b) => Document::load_mem_with_options(b, options),
    }
}

/// lopdf decodes only what is typed as an object stream, so an object stream
/// without its type stays as bytes. Only the `Some` matters to lopdf here.
fn keep_undecoded(id: ObjectId, object: &mut Object) -> Option<(ObjectId, Object)> {
    if let Object::Stream(stream) = object
        && stream.dict.has_type(b"ObjStm")
    {
        stream.dict.remove(b"Type");
    }
    Some((id, Object::Null))
}

fn usize_limit(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}
