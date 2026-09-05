//! End-to-end conversion over real files on disk, one per route.
//!
//! The `.docx` is assembled here rather than checked in: a binary fixture in
//! the repository is a fixture nobody can review in a diff.

// Integration test — the whole file is test code, not a runtime path;
// unwrap/expect here are the assertion mechanism itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};

use aurelius::doc::convert::{self, DocError};

/// A temp directory that removes itself.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("aurelius-doc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("write fixture");
        path
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The smallest WordprocessingML package Word and anydoc both accept.
fn write_docx(dir: &TmpDir, name: &str, paragraphs: &[&str]) -> PathBuf {
    let body: String = paragraphs
        .iter()
        .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
        .collect();

    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>{body}</w:body></w:document>"#
    );

    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;

    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

    let path = dir.0.join(name);
    let file = std::fs::File::create(&path).expect("create docx");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    for (entry, contents) in [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("word/document.xml", document.as_str()),
    ] {
        zip.start_file(entry, options).expect("start zip entry");
        zip.write_all(contents.as_bytes()).expect("write zip entry");
    }
    zip.finish().expect("finish docx");
    path
}

#[test]
fn docx_converts_to_markdown() {
    let dir = TmpDir::new();
    let path = write_docx(&dir, "brief.docx", &["Project brief", "Ship by November."]);

    let converted = convert::convert_file(&path).expect("convert docx");

    assert_eq!(converted.format, "docx");
    assert!(
        converted.markdown.contains("Project brief"),
        "got: {}",
        converted.markdown
    );
    assert!(
        converted.markdown.contains("Ship by November."),
        "got: {}",
        converted.markdown
    );
}

/// Detection reads the signature first, so the extension being wrong is not
/// the caller's problem.
#[test]
fn a_docx_named_txt_still_converts_as_a_document() {
    let dir = TmpDir::new();
    let path = write_docx(&dir, "mislabelled.txt", &["Real content"]);

    let converted = convert::convert_file(&path).expect("convert mislabelled docx");

    assert_eq!(converted.format, "docx");
    assert!(converted.markdown.contains("Real content"));
}

#[test]
fn csv_converts_to_a_markdown_table() {
    let dir = TmpDir::new();
    let path = dir.write("prices.csv", "item,price\nlamp,42\ndesk,180\n");

    let converted = convert::convert_file(&path).expect("convert csv");

    assert_eq!(converted.format, "csv");
    assert!(
        converted.markdown.contains("item"),
        "{}",
        converted.markdown
    );
    assert!(converted.markdown.contains("180"), "{}", converted.markdown);
    assert!(
        converted.markdown.contains('|'),
        "expected a table: {}",
        converted.markdown
    );
}

#[test]
fn html_converts_to_markdown() {
    let dir = TmpDir::new();
    let path = dir.write(
        "page.html",
        "<html><body><h1>Title</h1><p>Some <strong>bold</strong> text.</p></body></html>",
    );

    let converted = convert::convert_file(&path).expect("convert html");

    assert_eq!(converted.format, "html");
    assert!(
        converted.markdown.contains("# Title"),
        "{}",
        converted.markdown
    );
    assert!(
        converted.markdown.contains("**bold**"),
        "{}",
        converted.markdown
    );
}

#[test]
fn prose_passes_through_and_source_gets_fenced() {
    let dir = TmpDir::new();

    let notes = dir.write("notes.md", "# Heading\n\nbody\n");
    let converted = convert::convert_file(&notes).expect("convert md");
    assert_eq!(converted.format, "text");
    assert_eq!(converted.markdown, "# Heading\n\nbody\n");

    let source = dir.write("main.rs", "fn main() {}\n");
    let converted = convert::convert_file(&source).expect("convert rs");
    assert!(
        converted.markdown.starts_with("```rs\n"),
        "{}",
        converted.markdown
    );
}

#[test]
fn identical_content_hashes_identically_under_different_names() {
    let dir = TmpDir::new();
    let first = dir.write("a.csv", "x,y\n1,2\n");
    let second = dir.write("b.csv", "x,y\n1,2\n");

    let a = convert::convert_file(&first).expect("convert a");
    let b = convert::convert_file(&second).expect("convert b");

    assert_eq!(a.sha256, b.sha256);
}

#[test]
fn audio_is_refused_by_name_with_a_reason() {
    let dir = TmpDir::new();
    let path = dir.write("talk.mp3", [0u8; 64]);

    let error = convert::convert_file(&path).expect_err("mp3 must be refused");

    assert!(matches!(error, DocError::Media(_)), "got: {error:?}");
    let message = error.to_string();
    assert!(message.contains("transcription"), "got: {message}");
}

#[test]
fn images_are_refused_with_the_ocr_reason() {
    let dir = TmpDir::new();
    let path = dir.write("scan.png", [0u8; 64]);

    let error = convert::convert_file(&path).expect_err("png must be refused");

    assert!(matches!(error, DocError::Image(_)), "got: {error:?}");
    assert!(error.to_string().contains("OCR"), "got: {error}");
}

#[test]
fn unknown_binary_is_refused_rather_than_mangled() {
    let dir = TmpDir::new();
    let path = dir.write("blob.bin", [0x00, 0xff, 0x00, 0xfe, 0x00]);

    let error = convert::convert_file(&path).expect_err("binary must be refused");

    assert!(matches!(error, DocError::Binary(_)), "got: {error:?}");
}

#[test]
fn a_missing_file_says_so() {
    let error = convert::convert_file(Path::new("A:/definitely/not/here.docx"))
        .expect_err("missing file must fail");

    assert!(matches!(error, DocError::NotFound(_)), "got: {error:?}");
}
