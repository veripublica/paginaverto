//! The command line, end to end, on generated PDFs.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

/// How a page is made.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    /// A composite font with no `/ToUnicode`.
    OpaqueFont,
    /// A whole-page JBIG2 image and no text.
    Jbig2,
    /// Nothing on the page.
    Blank,
}

fn write_pdf(name: &str, pages: &[Kind]) -> PathBuf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids: Vec<Object> = Vec::new();
    for kind in pages {
        let simple = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let opaque = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Opaque",
            "Encoding" => "Identity-H",
        });
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 2000, "Height" => 3000,
                "Filter" => "JBIG2Decode",
            },
            vec![0; 8],
        ));
        let font = if matches!(kind, Kind::OpaqueFont) {
            "F2"
        } else {
            "F1"
        };
        let operations = match kind {
            Kind::Jbig2 | Kind::Blank => vec![],
            _ => vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![font.into(), 12.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::string_literal("Some words shown here. ".repeat(10))],
                ),
                Operation::new("ET", vec![]),
            ],
        };
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let mut resources = dictionary! {
            "Font" => dictionary! { "F1" => simple, "F2" => opaque },
        };
        if matches!(kind, Kind::Jbig2) {
            resources.set("XObject", dictionary! { "Im1" => image });
        }
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 400.into(), 600.into()],
            "Rotate" => 90,
            "Contents" => content, "Resources" => resources,
        });
        kids.push(page.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let path =
        std::env::temp_dir().join(format!("paginaverto-cli-{}-{name}.pdf", std::process::id()));
    doc.save(&path).unwrap();
    path
}

fn tool(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_paginaverto"))
        .args(args)
        .output()
        .unwrap()
}

fn inspect(paths: &[&Path], extra: &[&str]) -> Output {
    let mut args = vec!["inspect".to_string()];
    for path in paths {
        args.push("-i".into());
        args.push(path.to_str().unwrap().into());
    }
    args.extend(extra.iter().map(|s| s.to_string()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    tool(&args)
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn version_and_help_go_to_stdout() {
    let out = tool(&["-V"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("paginaverto {}", env!("CARGO_PKG_VERSION"))
    );
    let out = tool(&["inspect", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("EXIT CODES"));
}

#[test]
fn a_usage_error_is_short_and_on_stderr() {
    let out = tool(&["inspect", "book.pdf"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.starts_with("error: ") && text.contains("--help"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn it_tells_the_pages_apart() {
    let path = write_pdf("mixed", &[Kind::Text, Kind::Text, Kind::Blank]);
    let out = inspect(&[&path], &[]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("3 pages"), "{text}");
    assert!(text.contains("pages 1-2: text layer"), "{text}");
    assert!(text.contains("page 3: nothing to read"), "{text}");

    let report = json(&inspect(&[&path], &["--format", "json"]));
    assert_eq!(report["tool"], "paginaverto");
    assert_eq!(report["convention"], "0.7");
    assert_eq!(report["status"], "ok");
    let input = &report["inputs"][0];
    assert_eq!(input["summary"]["text_layer"], 2);
    assert_eq!(input["summary"]["empty"], 1);
    assert_eq!(input["items"][0]["data"]["source"], "text_layer");
    // Rotated a quarter turn, so width and height swap.
    assert_eq!(input["items"][0]["data"]["width"], 600.0);
    assert_eq!(input["items"][0]["data"]["height"], 400.0);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_font_that_cannot_be_decoded_is_a_problem() {
    let path = write_pdf("opaque", &[Kind::OpaqueFont, Kind::Text]);
    let out = inspect(&[&path], &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("error: Opaque (Type0)"), "{text}");
    assert!(text.contains("page 1"), "{text}");

    let out = inspect(&[&path], &["--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let report = json(&out);
    assert_eq!(report["status"], "problems");
    let items = report["inputs"][0]["items"].as_array().unwrap();
    let font = items
        .iter()
        .find(|i| i["code"] == "PV-FONT-UNREADABLE")
        .unwrap();
    assert_eq!(font["location"], "Opaque");
    assert_eq!(font["data"]["pages"], serde_json::json!([1]));
    // The simple font on page 2 has an encoding, so it is only a note.
    let simple = items
        .iter()
        .find(|i| i["code"] == "PV-FONT-NO-TOUNICODE")
        .unwrap();
    assert_eq!(simple["severity"], "info");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn an_undecodable_page_image_is_a_problem() {
    let path = write_pdf("jbig2", &[Kind::Jbig2, Kind::Jbig2]);
    let out = inspect(&[&path], &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a scan"), "{text}");
    assert!(text.contains("pages 1-2: scan, needs OCR"), "{text}");
    assert!(text.contains("a JBIG2 image cannot be decoded"), "{text}");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn an_undecodable_image_in_a_pdf_with_text_is_only_a_warning() {
    let path = write_pdf("figure", &[Kind::Text, Kind::Text, Kind::Jbig2]);
    let out = inspect(&[&path], &[]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("warning: page 3: a JBIG2 image"), "{text}");

    let report = json(&inspect(&[&path], &["--format", "json"]));
    assert_eq!(report["status"], "ok");
    assert_eq!(report["inputs"][0]["summary"]["warnings"], 1);
    assert_eq!(report["inputs"][0]["summary"]["errors"], 0);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn an_unreadable_input_does_not_stop_the_others() {
    let good = write_pdf("good", &[Kind::Text]);
    let missing = std::env::temp_dir().join("paginaverto-cli-no-such-file.pdf");
    let out = inspect(&[&missing, &good], &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let report = json(&out);
    assert_eq!(report["status"], "error");
    assert_eq!(report["inputs"][0]["status"], "error");
    assert!(report["inputs"][0]["error"].is_string());
    assert!(report["inputs"][0].get("summary").is_none());
    assert_eq!(report["inputs"][1]["status"], "ok");

    let out = inspect(&[&missing, &good], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: "));
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 page,"));
    std::fs::remove_file(good).unwrap();
}

fn convert_paths(name: &str, pages: &[Kind]) -> (PathBuf, PathBuf) {
    let input = write_pdf(name, pages);
    let output = input.with_extension("zip");
    let _ = std::fs::remove_file(&output);
    (input, output)
}

fn document_of(zip_path: &Path) -> serde_json::Value {
    let file = std::fs::File::open(zip_path).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    let entry = zip.by_name("document.json").unwrap();
    serde_json::from_reader(entry).unwrap()
}

#[test]
fn convert_writes_the_document_beside_the_input() {
    let (input, output) = convert_paths("convert", &[Kind::Text, Kind::Text]);
    let out = tool(&["-i", input.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let doc = document_of(&output);
    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["producer"]["tool"], "paginaverto");
    assert_eq!(doc["language"], serde_json::Value::Null);
    assert_eq!(doc["pages"].as_array().unwrap().len(), 2);
    let page = &doc["pages"][0];
    assert_eq!(page["number"], 1);
    assert_eq!(page["width"], 600.0, "the page is turned");
    assert_eq!(page["text_source"], "text_layer");
    let block = &doc["blocks"][0];
    assert_eq!(block["id"], "b1");
    assert_eq!(block["kind"], "paragraph");
    assert_eq!(block["confidence"], serde_json::Value::Null);
    assert!(
        block["runs"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Some words")
    );
    assert!(block["boxes"][0]["page"].is_number());
    let _ = std::fs::remove_file(&output);
}

#[test]
fn convert_records_the_language_and_reports_in_json() {
    let (input, output) = convert_paths("convert-json", &[Kind::Text]);
    let out = tool(&[
        "convert",
        "-i",
        input.to_str().unwrap(),
        "--language",
        "tr",
        "--threads",
        "2",
        "--format",
        "json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report = json(&out);
    assert_eq!(report["status"], "ok");
    let object = &report["inputs"][0];
    assert_eq!(object["output"], output.to_str().unwrap());
    assert_eq!(object["summary"]["text_layer"], 1);
    assert_eq!(document_of(&output)["language"], "tr");
    let _ = std::fs::remove_file(&output);
}

#[test]
fn convert_will_not_replace_a_file_without_force() {
    let (input, output) = convert_paths("convert-force", &[Kind::Text]);
    std::fs::write(&output, b"mine").unwrap();
    let out = tool(&["-i", input.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(message.contains("use -f to replace it"), "{message}");
    assert_eq!(std::fs::read(&output).unwrap(), b"mine");

    let out = tool(&["-f", "-i", input.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(document_of(&output)["schema_version"], 1);
    let _ = std::fs::remove_file(&output);
}

#[test]
fn convert_never_writes_over_its_input() {
    let (input, _) = convert_paths("convert-self", &[Kind::Text]);
    let before = std::fs::read(&input).unwrap();
    let path = input.to_str().unwrap();
    let out = tool(&["-f", "-i", path, "-o", path]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is the input"));
    assert_eq!(std::fs::read(&input).unwrap(), before);
}

#[test]
fn convert_refuses_a_scan_without_models_and_writes_nothing() {
    let (input, output) = convert_paths("convert-scan", &[Kind::Jbig2]);
    let out = tool(&["-i", input.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!output.exists());
}

#[test]
fn convert_names_a_folder_without_models() {
    let (input, output) = convert_paths("convert-models", &[Kind::Text]);
    let empty = std::env::temp_dir();
    let out = tool(&[
        "-i",
        input.to_str().unwrap(),
        "--ocr-models",
        empty.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("text-detection"));
    assert!(!output.exists());
}

#[test]
fn convert_refuses_a_model_file_that_is_not_the_published_one() {
    let (input, output) = convert_paths("convert-sha", &[Kind::Text]);
    let dir = input.with_extension("models");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("text-detection.onnx"), b"not a model").unwrap();
    std::fs::write(dir.join("text-recognition.onnx"), b"not a model").unwrap();
    let out = tool(&[
        "-i",
        input.to_str().unwrap(),
        "--ocr-models",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(message.contains("SHA-256"), "{message}");
    assert!(!output.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_counter_goes_to_a_pipe() {
    let (input, output) = convert_paths("convert-quiet", &[Kind::Text, Kind::Text]);
    let out = tool(&["-i", input.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(out.stderr, b"", "stderr is not a terminal here");
    let _ = std::fs::remove_file(&output);
}

#[test]
fn a_page_image_in_a_pdf_with_text_is_a_warning_with_the_document_written() {
    let (input, output) = convert_paths(
        "convert-image",
        &[Kind::Text, Kind::Text, Kind::Text, Kind::Jbig2],
    );
    let out = tool(&["-i", input.to_str().unwrap(), "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report = json(&out);
    let item = &report["inputs"][0]["items"][0];
    assert_eq!(item["code"], "PV-IMAGE-UNDECODABLE");
    assert_eq!(item["severity"], "warning");
    let doc = document_of(&output);
    assert_eq!(doc["pages"][3]["problems"][0], "undecodable_images");
    assert_eq!(doc["pages"][3]["text_source"], "none");
    let _ = std::fs::remove_file(&output);
}
