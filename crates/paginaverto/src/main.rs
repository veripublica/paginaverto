//! paginaverto's CLI, following the **veripublica CLI convention v0.7**
//! (<https://github.com/veripublica/conventions>).
//!
//! `convert`, the default, reads one PDF and writes the document to a zip.
//! `inspect` reads and reports, for one input or more. Nothing here asks
//! anything, so there are no prompts. The argument grammar is the family's
//! (kepubverto's, imagoverto's), ported deliberately.

#![forbid(unsafe_code)]

mod convert;

use std::process::ExitCode;

use paginaverto::{FontInfo, Input, Inspection, Options, PageInfo, PageSource};
use serde::Serialize;

use convert::Job;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
paginaverto — read a PDF into a structured document

USAGE:
    paginaverto -i <PATH> [OPTIONS]
    paginaverto <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    convert     Read a PDF and write the document to a zip: the default. See
                paginaverto convert --help.
    inspect     Report, page by page, where the text comes from. See
                paginaverto inspect --help.

OPTIONS:
    -V, --version           Print paginaverto <version> to stdout and exit 0.
    -h, --help              Print this help to stdout and exit 0.

EXAMPLES:
    paginaverto -i book.pdf
    paginaverto inspect -i book.pdf

Conforms to veripublica conventions v0.7.";

const CONVERT_HELP: &str = "\
paginaverto convert — read a PDF and write the document to a zip

USAGE:
    paginaverto convert -i <PATH> [OPTIONS]
    paginaverto -i <PATH> [OPTIONS]

OPTIONS:
    -i, --input <PATH>      The input. The only input form; positional paths are
                            not accepted.
    -o, --output <PATH>     Where to write the output. Default: the input's name
                            with .zip, beside it.
    -f, --force             Permit replacing existing output files. Never lifts
                            the output-equals-input refusal.
        --language <TAG>    The language of the content (BCP-47, such as tr),
                            recorded as the document's language. Required for a
                            scan. Without it, the PDF's own /Lang is recorded.
        --ocr-models <DIR>  The folder holding the OCR model files; the tool
                            never downloads them. Without it, a scan is refused.
        --threads <N>       How many threads do the work, 1 or more. The output
                            never depends on it. Default: every core but two, at
                            most four for OCR.
        --format <FORMAT>   Report format: human (the default), or json: the
                            shared machine envelope (one JSON object on stdout,
                            see the veripublica FORMATS spec).
    -V, --version           Print paginaverto <version> to stdout and exit 0.
    -h, --help              Print this help to stdout and exit 0.

The zip holds document.json and, if the PDF has any, figures/. A page that cannot
be read does not stop the rest: it is reported, and the file records it.

EXAMPLES:
    paginaverto -i book.pdf
    paginaverto convert -i scan.pdf --language tr --ocr-models models/ -o scan.zip

EXIT CODES:
    0   the document was written.
    1   the document was written, but a scan lost text: a page's image could not
        be decoded, or OCR failed on it.
    2   paginaverto could not run, or the PDF was refused: nothing was written.

Conforms to veripublica conventions v0.7.";

const INSPECT_HELP: &str = "\
paginaverto inspect — report, page by page, where a PDF's text comes from

USAGE:
    paginaverto inspect -i <PATH> [-i <PATH>...] [OPTIONS]

OPTIONS:
    -i, --input <PATH>      The input. The only input form; positional paths are
                            not accepted. Repeat it for more than one.
        --format <FORMAT>   Report format: human (the default), or json: the
                            shared machine envelope (one JSON object on stdout,
                            see the veripublica FORMATS spec).
    -V, --version           Print paginaverto <version> to stdout and exit 0.
    -h, --help              Print this help to stdout and exit 0.

Each page's text comes from a text layer, an OCR layer embedded over a page
image, or a scan only OCR can read. Fonts the pages show text in are listed when
they have no usable /ToUnicode map. It reads no text out and runs no OCR.

EXAMPLES:
    paginaverto inspect -i book.pdf
    paginaverto inspect -i a.pdf -i b.pdf --format json

EXIT CODES:
    0   every input was read, and nothing makes its text unreadable.
    1   every input was read, and at least one has a font whose text cannot be
        decoded, or is a scan with a page image that cannot be decoded. In a PDF
        with text, such an image is only a warning: OCR would not read it anyway.
    2   paginaverto could not run, or could not read at least one input. The
        others are still reported.

Conforms to veripublica conventions v0.7.";

/// The convention version this CLI and its envelope implement (FORMATS §1.1):
/// asserted here, never inherited.
const CONVENTION: &str = "0.7";

/// Which help text.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Topic {
    Tool,
    Convert,
    Inspect,
}

#[derive(Debug, PartialEq)]
enum Cli {
    Help(Topic),
    Version,
    /// The short problem message, without the `error:` prefix or the
    /// `--help` pointer main adds.
    Usage(String),
    Convert(Job),
    Inspect {
        inputs: Vec<String>,
        json: bool,
    },
}

/// Parse the arguments after the program name, deciding everything before any
/// work is done. `--name value` and `--name=value`; `-i value` and `-ivalue`;
/// boolean short flags bundle; a value-taking short flag consumes the rest of
/// its token or the next one (POSIX: `-iv` means `-i v`); and the token after a
/// value-taking option is always its value (`-i -q.pdf` names `-q.pdf`). The
/// only bare word is a subcommand, and only as the first argument.
fn parse(args: &[String]) -> Cli {
    let (topic, args) = match args.first().map(String::as_str) {
        Some("inspect") => (Topic::Inspect, &args[1..]),
        Some("convert") => (Topic::Convert, &args[1..]),
        _ => (Topic::Tool, args),
    };
    let inspecting = topic == Topic::Inspect;
    let mut inputs: Vec<String> = Vec::new();
    let mut format: Option<String> = None;
    let mut output: Option<String> = None;
    let mut language: Option<String> = None;
    let mut models: Option<String> = None;
    let mut threads: Option<String> = None;
    let (mut help, mut version, mut force) = (false, false, false);
    let mut error: Option<String> = None;

    // Record the first usage error but keep scanning, so that a later `-h` can
    // still short-circuit a malformed line (§5).
    macro_rules! fail {
        ($($a:tt)*) => {{ if error.is_none() { error = Some(format!($($a)*)); } }};
    }
    // A single-valued option given twice is two answers to one question (§3.4).
    macro_rules! once {
        ($slot:ident, $name:expr, $value:expr) => {{
            if $slot.is_some() {
                fail!("option '{}' given more than once", $name);
            } else {
                $slot = Some($value);
            }
        }};
    }

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Accepted and ignored; the convention gives it no other meaning.
        } else if let Some(long) = arg.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let takes_value = match name {
                "input" | "format" => true,
                "output" | "language" | "ocr-models" | "threads" => !inspecting,
                _ => false,
            };
            match name {
                "help" => help = true,
                "version" => version = true,
                "force" if !inspecting => force = true,
                _ if takes_value => {
                    let value = match attached {
                        Some(v) => v,
                        None => {
                            i += 1;
                            match args.get(i) {
                                Some(v) => v.clone(),
                                None => {
                                    fail!("option '--{name}' needs a value");
                                    break;
                                }
                            }
                        }
                    };
                    match name {
                        "input" => inputs.push(value),
                        "format" => once!(format, "--format", value),
                        "output" => once!(output, "--output", value),
                        "language" => once!(language, "--language", value),
                        "ocr-models" => once!(models, "--ocr-models", value),
                        _ => once!(threads, "--threads", value),
                    }
                }
                _ => fail!("unexpected option '--{name}'"),
            }
        } else if arg.len() > 1 && arg.starts_with('-') {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'h' => help = true,
                    'V' => version = true,
                    'f' if !inspecting => force = true,
                    c @ 'i' | c @ 'o' if c == 'i' || !inspecting => {
                        let rest: String = chars[j + 1..].iter().collect();
                        let value = if !rest.is_empty() {
                            Some(rest)
                        } else {
                            i += 1;
                            args.get(i).cloned()
                        };
                        match value {
                            Some(v) if c == 'i' => inputs.push(v),
                            Some(v) => once!(output, "-o", v),
                            None => fail!("option '-{c}' needs a value"),
                        }
                        break;
                    }
                    c => {
                        fail!("unexpected option '-{c}'");
                        break;
                    }
                }
                j += 1;
            }
        } else if arg == "inspect" || arg == "convert" {
            fail!("the subcommand '{arg}' must come first");
        } else {
            fail!("unexpected argument '{arg}'; use -i {arg}");
        }
        i += 1;
    }

    let json = match format.as_deref() {
        None | Some("human") => false,
        Some("json") => true,
        Some(f) => {
            fail!("invalid value '{f}' for --format; supported values: human, json");
            false
        }
    };
    if let Some(tag) = &language
        && !is_language_tag(tag)
    {
        fail!("invalid value '{tag}' for --language; a BCP-47 tag, such as tr");
    }
    let threads = threads.and_then(|n| match n.parse::<usize>() {
        Ok(n) if n >= 1 => Some(n),
        _ => {
            fail!("invalid value '{n}' for --threads; a number, 1 or more");
            None
        }
    });

    if help {
        return Cli::Help(topic);
    }
    if let Some(message) = error {
        return Cli::Usage(message);
    }
    if version {
        return Cli::Version;
    }
    if inputs.is_empty() {
        return Cli::Usage("missing required -i".into());
    }
    if inspecting {
        return Cli::Inspect { inputs, json };
    }
    if inputs.len() > 1 {
        let n = inputs.len();
        return Cli::Usage(format!("convert takes 1 input, {n} given"));
    }
    Cli::Convert(Job {
        input: inputs.remove(0),
        output,
        force,
        language,
        models,
        threads,
        json,
    })
}

/// The shape of a BCP-47 tag: subtags of letters and digits, the first letters
/// only. Whether it names a language is the caller's business.
fn is_language_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=8).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Cli::Help(topic) => {
            println!(
                "{}",
                match topic {
                    Topic::Tool => HELP,
                    Topic::Convert => CONVERT_HELP,
                    Topic::Inspect => INSPECT_HELP,
                }
            );
            ExitCode::SUCCESS
        }
        Cli::Version => {
            println!("paginaverto {VERSION}");
            ExitCode::SUCCESS
        }
        Cli::Usage(message) => {
            eprintln!("error: {message} (see --help)");
            ExitCode::from(2)
        }
        Cli::Convert(job) => convert::run(&job),
        Cli::Inspect { inputs, json } => inspect(&inputs, json),
    }
}

fn inspect(inputs: &[String], json: bool) -> ExitCode {
    let options = Options::default();
    let mut reports = Vec::with_capacity(inputs.len());
    for path in inputs {
        let result = paginaverto::inspect(Input::Path(path.as_ref()), &options);
        if !json {
            print_human(path, &result);
        }
        reports.push(input_object(path, result));
    }
    let status = worst(&reports);
    if json && let Err(e) = Envelope::new(status, reports).print() {
        eprintln!("error: could not write the report: {e}");
        return ExitCode::from(2);
    }
    report_status(status)
}

/// The exit code a report's status means (§6).
fn report_status(status: &str) -> ExitCode {
    ExitCode::from(match status {
        "ok" => 0,
        "problems" => 1,
        _ => 2,
    })
}

fn worst(reports: &[InputObject<'_>]) -> &'static str {
    let has = |status: &str| reports.iter().any(|r| r.status == status);
    if has("error") {
        "error"
    } else if has("problems") {
        "problems"
    } else {
        "ok"
    }
}

// ---------------------------------------------------------------------------
// The human report

fn print_human(path: &str, result: &Result<Inspection, paginaverto::Error>) {
    let found = match result {
        Ok(found) => found,
        Err(e) => {
            eprintln!("error: {path}: {e}");
            return;
        }
    };
    println!(
        "{path}: {} {}, reads as {}",
        found.pages.len(),
        if found.pages.len() == 1 {
            "page"
        } else {
            "pages"
        },
        if found.reads_as_scan {
            "a scan: every page needs OCR"
        } else {
            "a PDF with text"
        }
    );
    for (first, last, source) in stretches(&found.pages) {
        println!("  {}: {}", page_span(first, last), label(source));
    }
    for page in &found.pages {
        if let Some(format) = page.undecodable_image {
            println!(
                "  {}: page {}: a {format} image cannot be decoded",
                image_severity(found),
                page.number
            );
        }
    }
    for font in &found.fonts {
        println!(
            "  {}: {} ({}), no usable /ToUnicode, {}: {}",
            if font.readable { "note" } else { "error" },
            font_name(font),
            font.subtype,
            if font.readable {
                "read through its encoding"
            } else {
                "its text cannot be decoded"
            },
            numbers(&font.pages),
        );
    }
}

/// A page image that cannot be decoded costs a scan its text. In a PDF with
/// text it goes through no OCR anyway, and costs a figure at most.
fn image_severity(found: &Inspection) -> &'static str {
    if found.reads_as_scan {
        "error"
    } else {
        "warning"
    }
}

fn font_name(font: &FontInfo) -> &str {
    if font.name.is_empty() {
        "unnamed font"
    } else {
        &font.name
    }
}

fn label(source: PageSource) -> &'static str {
    match source {
        PageSource::TextLayer => "text layer",
        PageSource::EmbeddedOcr => "embedded OCR layer",
        PageSource::Scan => "scan, needs OCR",
        PageSource::Empty => "nothing to read",
        _ => "unknown",
    }
}

/// Runs of neighbouring pages from the same source.
fn stretches(pages: &[PageInfo]) -> Vec<(u32, u32, PageSource)> {
    let mut out: Vec<(u32, u32, PageSource)> = Vec::new();
    for page in pages {
        match out.last_mut() {
            Some((_, last, source)) if *source == page.source => *last = page.number,
            _ => out.push((page.number, page.number, page.source)),
        }
    }
    out
}

fn page_span(first: u32, last: u32) -> String {
    if first == last {
        format!("page {first}")
    } else {
        format!("pages {first}-{last}")
    }
}

/// `1-3, 7`: page numbers, in order, as ranges.
fn numbers(pages: &[u32]) -> String {
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for &n in pages {
        match spans.last_mut() {
            Some((_, last)) if *last + 1 == n => *last = n,
            _ => spans.push((n, n)),
        }
    }
    let text: Vec<String> = spans
        .iter()
        .map(|&(a, b)| {
            if a == b {
                a.to_string()
            } else {
                format!("{a}-{b}")
            }
        })
        .collect();
    let word = if pages.len() == 1 { "page" } else { "pages" };
    format!("{word} {}", text.join(", "))
}

// ---------------------------------------------------------------------------
// The machine report (FORMATS.md)

#[derive(Serialize)]
struct Envelope<'a> {
    tool: &'static str,
    tool_version: &'static str,
    convention: &'static str,
    status: &'static str,
    inputs: Vec<InputObject<'a>>,
}

impl<'a> Envelope<'a> {
    fn new(status: &'static str, inputs: Vec<InputObject<'a>>) -> Envelope<'a> {
        Envelope {
            tool: "paginaverto",
            tool_version: VERSION,
            convention: CONVENTION,
            status,
            inputs,
        }
    }

    fn print(&self) -> Result<(), serde_json::Error> {
        println!("{}", serde_json::to_string(self)?);
        Ok(())
    }
}

#[derive(Serialize)]
struct InputObject<'a> {
    path: &'a str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<serde_json::Value>,
    items: Vec<Item>,
}

#[derive(Serialize)]
struct Item {
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'static str,
    severity: &'static str,
    location: String,
    message: String,
    data: serde_json::Value,
}

fn input_object(path: &str, result: Result<Inspection, paginaverto::Error>) -> InputObject<'_> {
    let found = match result {
        Ok(found) => found,
        Err(e) => {
            return InputObject {
                path,
                status: "error",
                error: Some(e.to_string()),
                output: None,
                summary: None,
                items: Vec::new(),
            };
        }
    };
    let count = |source| found.pages.iter().filter(|p| p.source == source).count();
    let severity = image_severity(&found);
    let mut items: Vec<Item> = found
        .pages
        .iter()
        .flat_map(|p| page_items(p, severity))
        .collect();
    items.extend(found.fonts.iter().map(font_item));
    let counted = |severity| items.iter().filter(|i| i.severity == severity).count();
    let (errors, warnings, infos) = (counted("error"), counted("warning"), counted("info"));
    InputObject {
        path,
        status: if errors > 0 { "problems" } else { "ok" },
        error: None,
        output: None,
        summary: Some(serde_json::json!({
            "pages": found.pages.len(),
            "text_layer": count(PageSource::TextLayer),
            "embedded_ocr": count(PageSource::EmbeddedOcr),
            "scan": count(PageSource::Scan),
            "empty": count(PageSource::Empty),
            "reads_as_scan": found.reads_as_scan,
            "errors": errors,
            "warnings": warnings,
            "infos": infos,
        })),
        items,
    }
}

fn page_items(page: &PageInfo, image_severity: &'static str) -> Vec<Item> {
    let location = format!("page {}", page.number);
    let mut items = vec![Item {
        kind: "finding",
        code: "PV-PAGE",
        severity: "info",
        location: location.clone(),
        message: label(page.source).to_string(),
        data: serde_json::json!({
            "page": page.number,
            "source": page.source.name(),
            "width": page.width,
            "height": page.height,
        }),
    }];
    if let Some(format) = page.undecodable_image {
        items.push(Item {
            kind: "finding",
            code: "PV-IMAGE-UNDECODABLE",
            severity: image_severity,
            location,
            message: format!("a {format} image cannot be decoded"),
            data: serde_json::json!({ "page": page.number, "format": format }),
        });
    }
    items
}

fn font_item(font: &FontInfo) -> Item {
    let (code, severity, what) = if font.readable {
        ("PV-FONT-NO-TOUNICODE", "info", "read through its encoding")
    } else {
        ("PV-FONT-UNREADABLE", "error", "its text cannot be decoded")
    };
    Item {
        kind: "finding",
        code,
        severity,
        location: font_name(font).to_string(),
        message: format!("no usable /ToUnicode, {what}"),
        data: serde_json::json!({ "subtype": font.subtype, "pages": font.pages }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Cli {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse(&args)
    }

    fn inputs(paths: &[&str], json: bool) -> Cli {
        Cli::Inspect {
            inputs: paths.iter().map(|s| s.to_string()).collect(),
            json,
        }
    }

    #[test]
    fn the_accepted_syntaxes() {
        for line in [
            &["inspect", "-i", "a.pdf"][..],
            &["inspect", "-ia.pdf"],
            &["inspect", "--input", "a.pdf"],
            &["inspect", "--input=a.pdf"],
            &["inspect", "--", "-i", "a.pdf"],
        ] {
            assert_eq!(run(line), inputs(&["a.pdf"], false), "{line:?}");
        }
    }

    #[test]
    fn the_token_after_a_value_option_is_its_value() {
        assert_eq!(
            run(&["inspect", "-i", "-h.pdf"]),
            inputs(&["-h.pdf"], false)
        );
        assert_eq!(run(&["inspect", "-iV"]), inputs(&["V"], false));
    }

    #[test]
    fn inspect_takes_several_inputs() {
        assert_eq!(
            run(&["inspect", "-i", "a", "-i", "b", "--format", "json"]),
            inputs(&["a", "b"], true)
        );
    }

    #[test]
    fn help_short_circuits_a_malformed_line() {
        assert_eq!(run(&["--nope", "-h"]), Cli::Help(Topic::Tool));
        assert_eq!(
            run(&["inspect", "stray", "--help"]),
            Cli::Help(Topic::Inspect)
        );
        assert_eq!(
            run(&["convert", "stray", "--help"]),
            Cli::Help(Topic::Convert)
        );
        assert_eq!(run(&["-Vh"]), Cli::Help(Topic::Tool));
    }

    #[test]
    fn version() {
        assert_eq!(run(&["-V"]), Cli::Version);
        assert_eq!(run(&["--version"]), Cli::Version);
    }

    #[track_caller]
    fn usage(args: &[&str], part: &str) {
        match run(args) {
            Cli::Usage(message) => assert!(message.contains(part), "{message}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn usage_errors() {
        usage(&[], "missing required -i");
        usage(&["-i", "a", "-i", "b"], "1 input, 2 given");
        usage(
            &["convert", "-i", "a", "-o", "x", "--output", "y"],
            "more than once",
        );
        usage(&["-i", "a", "--threads", "0"], "--threads");
        usage(&["-i", "a", "--threads", "many"], "--threads");
        usage(&["-i", "a", "--language", "not a tag"], "--language");
        usage(&["-i", "a", "--language", ""], "--language");
        usage(&["inspect", "-i", "a", "-o", "x"], "'-o'");
        usage(&["inspect", "-i", "a", "-f"], "'-f'");
        usage(&["inspect", "-i", "a", "--threads", "2"], "'--threads'");
        usage(&["inspect"], "missing required -i");
        usage(&["inspect", "a.pdf"], "use -i a.pdf");
        usage(&["-i", "a.pdf", "inspect"], "must come first");
        usage(&["inspect", "-i"], "needs a value");
        usage(&["inspect", "--input"], "needs a value");
        usage(&["inspect", "-x"], "'-x'");
        usage(&["inspect", "--nope"], "'--nope'");
        usage(&["inspect", "-i", "a", "--format", "xml"], "human, json");
        usage(
            &[
                "inspect", "-i", "a", "--format", "json", "--format", "human",
            ],
            "more than once",
        );
    }

    #[test]
    fn convert_is_the_default() {
        let want = Cli::Convert(Job {
            input: "a.pdf".into(),
            output: Some("b.zip".into()),
            force: true,
            language: Some("pt-BR".into()),
            models: Some("m".into()),
            threads: Some(3),
            json: true,
        });
        let line = [
            "-i",
            "a.pdf",
            "-fob.zip",
            "--language=pt-BR",
            "--ocr-models",
            "m",
            "--threads",
            "3",
            "--format",
            "json",
        ];
        assert_eq!(run(&line), want);
        let mut named = vec!["convert"];
        named.extend(line);
        assert_eq!(run(&named), want);
    }

    #[test]
    fn page_numbers_read_as_ranges() {
        assert_eq!(numbers(&[1, 2, 3, 7]), "pages 1-3, 7");
        assert_eq!(numbers(&[4]), "page 4");
    }
}
