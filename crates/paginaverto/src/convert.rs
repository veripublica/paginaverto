//! `paginaverto convert`: read one PDF and write the document to a zip,
//! `document.json` and `figures/`.

use std::fs::{self, File};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use paginaverto::{
    Block, BlockBox, BlockKind, Document, Error, Input, LATIN_RECOGNITION, MODELS, ModelFile,
    ONNX_MODELS, Ocr, Options, OutlineEntry, Page, PageDone, PageProblem, Progress, Run,
    SCHEMA_VERSION, TextSource,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::{Envelope, InputObject, Item, VERSION, numbers, report_status};

/// What the command line asked for, already checked.
#[derive(Debug, PartialEq)]
pub(crate) struct Job {
    pub input: String,
    pub output: Option<String>,
    pub force: bool,
    pub language: Option<String>,
    pub models: Option<String>,
    pub threads: Option<usize>,
    pub json: bool,
}

pub(crate) fn run(job: &Job) -> ExitCode {
    let output = job
        .output
        .as_ref()
        .map_or_else(|| default_output(&job.input), PathBuf::from);
    let (status, object) = match convert(job, &output) {
        Ok(doc) => {
            let items = page_items(&doc);
            let errors = items.iter().filter(|i| i.severity == "error").count();
            if !job.json {
                print_human(job, &output, &doc, &items);
            }
            let warnings = items.len() - errors;
            let status = if errors > 0 { "problems" } else { "ok" };
            let summary = serde_json::json!({
                "pages": doc.pages.len(),
                "chapters": doc.chapters().len(),
                "blocks": doc.blocks.len(),
                "figures": doc.blocks.iter().filter(|b| matches!(b.kind, BlockKind::Figure(_))).count(),
                "text_layer": count_source(&doc, TextSource::TextLayer),
                "embedded_ocr": count_source(&doc, TextSource::EmbeddedOcr),
                "ocr": count_source(&doc, TextSource::Ocr),
                "none": count_source(&doc, TextSource::None),
                "errors": errors,
                "warnings": warnings,
            });
            let object = InputObject {
                path: &job.input,
                status,
                error: None,
                output: Some(output.display().to_string()),
                summary: Some(summary),
                items,
            };
            (status, object)
        }
        Err(message) => {
            if !job.json {
                eprintln!("error: {}: {message}", job.input);
            }
            let object = InputObject {
                path: &job.input,
                status: "error",
                error: Some(message),
                output: None,
                summary: None,
                items: Vec::new(),
            };
            ("error", object)
        }
    };
    if job.json
        && let Err(e) = Envelope::new(status, vec![object]).print()
    {
        eprintln!("error: could not write the report: {e}");
        return ExitCode::from(2);
    }
    report_status(status)
}

fn count_source(doc: &Document, source: TextSource) -> usize {
    doc.pages.iter().filter(|p| p.text_source == source).count()
}

/// `book.pdf` becomes `book.zip`, beside it. The format is the operation, so
/// the name takes no verb (CLI §4).
fn default_output(input: &str) -> PathBuf {
    Path::new(input).with_extension("zip")
}

fn convert(job: &Job, output: &Path) -> Result<Document, String> {
    // Before any work: nobody should wait for a refusal.
    check_output(Path::new(&job.input), output, job.force)?;
    let mut options = Options::default();
    options.language = job.language.clone();
    options.threads = job.threads;
    if let Some(dir) = &job.models {
        options.ocr = load_models(Path::new(dir))?;
    }
    // A counter for a person watching, on stderr (§5). Never in a log or a pipe.
    let counting = !job.json && io::stderr().is_terminal();
    if counting {
        options.progress = Some(Progress::new(|done: PageDone| {
            eprint!("\rpage {} of {}", done.done, done.total);
        }));
    }
    let result = paginaverto::read(Input::Path(job.input.as_ref()), &options);
    if counting {
        // Back to the start of the line, and clear it.
        eprint!("\r\x1b[K");
    }
    let doc = result.map_err(|e| match e {
        Error::ScanNeedsOcr => format!("{e}; give --ocr-models <DIR> and --language <TAG>"),
        e => e.to_string(),
    })?;
    write_atomically(output, &doc)
        .map_err(|e| format!("could not write {}: {e}", output.display()))?;
    Ok(doc)
}

/// CLI §4: never the input, and never an existing file without `-f`.
fn check_output(input: &Path, output: &Path, force: bool) -> Result<(), String> {
    let name = output.display();
    let resolved = |p: &Path| {
        fs::canonicalize(p).ok().or_else(|| {
            let parent = p.parent().filter(|d| !d.as_os_str().is_empty());
            let dir = fs::canonicalize(parent.unwrap_or(Path::new("."))).ok()?;
            Some(dir.join(p.file_name()?))
        })
    };
    if let (Some(a), Some(b)) = (resolved(input), resolved(output))
        && a == b
    {
        return Err(format!("the output '{name}' is the input"));
    }
    if !force && output.exists() {
        return Err(format!("'{name}' exists; use -f to replace it"));
    }
    Ok(())
}

/// The folder's two model files, ONNX if it has them, else RTen, checked against
/// their published SHA-256: a damaged model reads badly without saying so.
fn load_models(dir: &Path) -> Result<Ocr, String> {
    // The first file found is used: for recognition the Latin model, which reads
    // Turkish, before ocrs's own, which gives 0.2's text.
    let read = |files: &[ModelFile]| {
        let found = files
            .iter()
            .find_map(|file| Some((file, fs::read(dir.join(file.name)).ok()?)));
        let (file, bytes) = found.ok_or_else(|| {
            let names: Vec<&str> = files.iter().map(|f| f.name).collect();
            format!(
                "none of {} in {}; how to get them: {MODELS_GUIDE}",
                names.join(", "),
                dir.display()
            )
        })?;
        if sha256_hex(&bytes) != file.sha256 {
            return Err(not_the_model(file, dir));
        }
        Ok(bytes)
    };
    Ok(Ocr::Models {
        detection: read(&[ONNX_MODELS[0], MODELS[0]])?,
        recognition: read(&[LATIN_RECOGNITION, ONNX_MODELS[1], MODELS[1]])?,
    })
}

/// Where the model files and their download are explained.
const MODELS_GUIDE: &str = "https://github.com/veripublica/paginaverto/blob/main/AGENTS.md#ocr-reading-scans-and-page-images";

fn not_the_model(file: &ModelFile, dir: &Path) -> String {
    format!(
        "{} in {} is not the published model: its SHA-256 differs",
        file.name,
        dir.display()
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A partial file never takes the output's name.
fn write_atomically(output: &Path, doc: &Document) -> io::Result<()> {
    let name = output
        .file_name()
        .ok_or_else(|| io::Error::other("the output has no file name"))?;
    let temp = output.with_file_name(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let written = write_zip(File::create(&temp)?, doc).and_then(|()| fs::rename(&temp, output));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

fn write_zip(file: File, doc: &Document) -> io::Result<()> {
    let names = figure_names(doc);
    let mut zip = ZipWriter::new(file);
    let json = serde_json::to_vec_pretty(&FileDocument::new(doc, &names))?;
    let large = |len: usize| u32::try_from(len).is_err();
    zip.start_file(
        "document.json",
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(large(json.len())),
    )?;
    zip.write_all(&json)?;
    // A JPEG or PNG is compressed already.
    let figures = doc.blocks.iter().filter_map(|b| match &b.kind {
        BlockKind::Figure(f) => Some(f),
        _ => None,
    });
    let cover = doc.cover.as_ref().map(|f| (f, cover_name(f)));
    for (figure, name) in figures.zip(names.iter().cloned()).chain(cover) {
        zip.start_file(
            name.as_str(),
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .large_file(large(figure.data.len())),
        )?;
        zip.write_all(&figure.data)?;
    }
    zip.finish()?;
    Ok(())
}

/// `figures/fig001.jpg`, `figures/fig002.png`, … in reading order.
fn figure_names(doc: &Document) -> Vec<String> {
    let figures = doc.blocks.iter().filter_map(|b| match &b.kind {
        BlockKind::Figure(f) => Some(f),
        _ => None,
    });
    figures
        .enumerate()
        .map(|(i, f)| format!("figures/fig{:03}.{}", i + 1, f.ext))
        .collect()
}

// ---------------------------------------------------------------------------
// The report

/// A document that reads by OCR has lost text on a page it could not read; one
/// that reads from a text layer was never going to read that image.
fn page_items(doc: &Document) -> Vec<Item> {
    let by_ocr = doc
        .pages
        .iter()
        .any(|p| p.text_source == TextSource::Ocr || p.problems.contains(&PageProblem::OcrFailed));
    let mut items = Vec::new();
    for page in &doc.pages {
        for problem in &page.problems {
            items.push(problem_item(page, *problem, by_ocr));
        }
    }
    items
}

fn problem_item(page: &Page, problem: PageProblem, by_ocr: bool) -> Item {
    let lost = if by_ocr { "error" } else { "warning" };
    let (code, severity, message) = match problem {
        PageProblem::OcrFailed => (
            "PV-OCR-FAILED",
            "error",
            "OCR failed on the page's image".into(),
        ),
        PageProblem::UndecodableImage(Some(format)) => (
            "PV-IMAGE-UNDECODABLE",
            lost,
            format!("a {format} image cannot be decoded"),
        ),
        PageProblem::UndecodableImage(None) => (
            "PV-IMAGE-UNDECODABLE",
            lost,
            "the page's image cannot be decoded".into(),
        ),
        PageProblem::ImageNotRead => (
            "PV-IMAGE-NOT-READ",
            "warning",
            "only an image here, and OCR does not run on a PDF with text".into(),
        ),
        // A new kind of problem may be worth more, but it is at least this.
        _ => ("PV-PAGE-PROBLEM", "warning", problem.name().to_string()),
    };
    Item {
        kind: "finding",
        code,
        severity,
        location: format!("page {}", page.number),
        message,
        data: serde_json::json!({ "page": page.number, "problem": problem.name() }),
    }
}

fn print_human(job: &Job, output: &Path, doc: &Document, items: &[Item]) {
    let plural = |n: usize, one: &'static str, many: &'static str| {
        format!("{n} {}", if n == 1 { one } else { many })
    };
    println!(
        "{} -> {}: {}, {}",
        job.input,
        output.display(),
        plural(doc.pages.len(), "page", "pages"),
        plural(doc.chapters().len(), "chapter", "chapters"),
    );
    // One line per kind of problem, not one per page.
    let mut seen: Vec<(&str, &str, &str)> = items
        .iter()
        .map(|i| (i.severity, i.code, i.message.as_str()))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    for (severity, code, message) in seen {
        let pages: Vec<u32> = items
            .iter()
            .filter(|i| i.code == code && i.message == message)
            .filter_map(|i| i.data["page"].as_u64().map(|n| n as u32))
            .collect();
        eprintln!("{severity}: {}: {message}", numbers(&pages));
    }
}

// ---------------------------------------------------------------------------
// The file: this shape is the interface, kept apart from the Rust types.

#[derive(Serialize)]
struct FileDocument<'a> {
    schema_version: u32,
    producer: Producer,
    language: Option<&'a str>,
    metadata: FileMetadata<'a>,
    pages: Vec<FilePage<'a>>,
    blocks: Vec<FileBlock<'a>>,
    /// Page 1's picture when it is a cover, else `null`.
    cover: Option<FileCover>,
}

#[derive(Serialize)]
struct FileCover {
    file: String,
    media_type: &'static str,
}

/// The cover's file in the zip.
fn cover_name(f: &paginaverto::Figure) -> String {
    format!("figures/cover.{}", f.ext)
}

#[derive(Serialize)]
struct Producer {
    tool: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct FileMetadata<'a> {
    info: &'a std::collections::BTreeMap<String, String>,
    xmp: Option<&'a str>,
    catalog_lang: Option<&'a str>,
    outline: Vec<FileEntry<'a>>,
}

#[derive(Serialize)]
struct FileEntry<'a> {
    level: u32,
    title: &'a str,
    page: Option<u32>,
}

#[derive(Serialize)]
struct FilePage<'a> {
    number: u32,
    width: f64,
    height: f64,
    text_source: &'static str,
    problems: Vec<&'a str>,
}

#[derive(Serialize)]
struct FileBlock<'a> {
    id: &'a str,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runs: Option<Vec<FileRun<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_type: Option<&'static str>,
    boxes: Vec<FileBox>,
    /// Absent on a figure, `null` on text until there is a figure to give.
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<Option<f32>>,
}

#[derive(Serialize)]
struct FileRun<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    italic: bool,
}

#[derive(Serialize)]
struct FileBox {
    page: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Points to two decimals: a hundredth of a point is far below what a box means.
fn points(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

impl<'a> FileDocument<'a> {
    fn new(doc: &'a Document, figures: &'a [String]) -> FileDocument<'a> {
        let mut figures = figures.iter();
        FileDocument {
            schema_version: SCHEMA_VERSION,
            producer: Producer {
                tool: "paginaverto",
                version: VERSION,
            },
            language: doc.language.as_deref(),
            metadata: FileMetadata {
                info: &doc.metadata.info,
                xmp: doc.metadata.xmp.as_deref(),
                catalog_lang: doc.metadata.catalog_lang.as_deref(),
                outline: doc.metadata.outline.iter().map(file_entry).collect(),
            },
            pages: doc.pages.iter().map(file_page).collect(),
            blocks: doc
                .blocks
                .iter()
                .map(|b| file_block(b, &mut figures))
                .collect(),
            cover: doc.cover.as_ref().map(|f| FileCover {
                file: cover_name(f),
                media_type: f.media_type,
            }),
        }
    }
}

fn file_entry(e: &OutlineEntry) -> FileEntry<'_> {
    FileEntry {
        level: e.level,
        title: &e.title,
        page: e.page,
    }
}

fn file_page(p: &Page) -> FilePage<'_> {
    FilePage {
        number: p.number,
        width: points(p.width),
        height: points(p.height),
        text_source: p.text_source.name(),
        problems: p.problems.iter().map(|x| x.name()).collect(),
    }
}

fn file_block<'a>(b: &'a Block, figures: &mut std::slice::Iter<'a, String>) -> FileBlock<'a> {
    let boxes = b.boxes.iter().map(file_box).collect();
    let mut out = FileBlock {
        id: &b.id,
        kind: "paragraph",
        level: b.level(),
        runs: Some(b.runs().iter().map(file_run).collect()),
        file: None,
        media_type: None,
        boxes,
        confidence: Some(b.confidence),
    };
    match &b.kind {
        BlockKind::Heading { .. } => out.kind = "heading",
        BlockKind::Figure(f) => {
            out.kind = "figure";
            out.runs = None;
            out.file = figures.next().map(String::as_str);
            out.media_type = Some(f.media_type);
            out.confidence = None;
        }
        _ => {}
    }
    out
}

fn file_run(r: &Run) -> FileRun<'_> {
    FileRun {
        text: &r.text,
        italic: r.italic,
    }
}

fn file_box(b: &BlockBox) -> FileBox {
    FileBox {
        page: b.page,
        x: points(b.x),
        y: points(b.y),
        width: points(b.width),
        height: points(b.height),
    }
}
