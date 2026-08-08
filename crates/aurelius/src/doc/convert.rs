//! Local file → Markdown conversion.
//!
//! Three routes, tried in that order: `anydoc` for the office formats and PDF,
//! `htmd` for HTML, and a passthrough for anything that is already text. Every
//! route runs in-process — no network, no API key, no temporary files.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Ceiling on input size. Conversion reads the whole file into memory, and an
/// agent pointing this at a disk image should get a sentence back rather than
/// an out-of-memory kill.
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

/// Extensions we refuse by name rather than by failing to parse: the caller
/// gets told *why* the format is out of scope instead of "unrecognized".
const AUDIO_VIDEO_EXT: &[&str] = &[
    "mp3", "wav", "m4a", "flac", "ogg", "opus", "aac", "wma", "aiff", "mp4", "mov", "avi", "mkv",
    "webm", "wmv", "flv", "m4v", "mpg", "mpeg",
];

const IMAGE_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "ico", "avif",
];

/// Text formats that are already prose: handed through untouched. Everything
/// else textual is fenced, so that a `.rs` file stays valid Markdown.
const PROSE_EXT: &[&str] = &["md", "markdown", "mdx", "txt", "text", "log", "rst", "adoc"];

const HTML_EXT: &[&str] = &["html", "htm", "xhtml"];

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("no such file: {0}")]
    NotFound(String),

    #[error("not a file: {0}")]
    NotAFile(String),

    #[error("{path} is {size} bytes, over the {MAX_FILE_BYTES} byte limit")]
    TooLarge { path: String, size: u64 },

    #[error(
        "{0} is audio or video. This converts documents, not speech — \
         transcription needs an external service and is not part of this tool"
    )]
    Media(String),

    #[error(
        "{0} is an image. Reading text off an image needs OCR, which is not \
         part of this tool; a text-layer PDF converts fine"
    )]
    Image(String),

    #[error("{0} is binary and matches no supported document format")]
    Binary(String),

    #[error("could not convert {path}: {source} [{code}]")]
    Document {
        path: String,
        code: &'static str,
        #[source]
        source: anydoc::ConvertError,
    },

    #[error("could not convert HTML in {path}: {source}")]
    Html {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// A converted document, before anything decides where to put it.
#[derive(Debug)]
pub struct Converted {
    pub markdown: String,
    /// Which route produced it: an `anydoc` format name, `html`, or `text`.
    pub format: String,
    /// SHA-256 of the *source bytes* — the cache key. A copy of a document
    /// under a different name hashes the same and is not converted twice.
    pub sha256: String,
    pub byte_size: u64,
}

/// A file read once: its bytes, its identity, and the facts routing needs.
///
/// Reading and converting are separate steps so that a caller holding a cache
/// can hash the file, find it already converted, and stop — without reading it
/// a second time to convert it.
#[derive(Debug)]
pub struct Source {
    pub bytes: Vec<u8>,
    pub sha256: String,
    ext: String,
    display: String,
}

/// Read a file and rule out what is out of scope, before any parsing happens.
pub fn read_source(path: &Path) -> Result<Source, DocError> {
    let display = path.display().to_string();

    let meta = std::fs::metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            DocError::NotFound(display.clone())
        } else {
            DocError::Io {
                path: display.clone(),
                source,
            }
        }
    })?;

    if !meta.is_file() {
        return Err(DocError::NotAFile(display));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(DocError::TooLarge {
            path: display,
            size: meta.len(),
        });
    }

    let ext = extension_of(path);
    if AUDIO_VIDEO_EXT.contains(&ext.as_str()) {
        return Err(DocError::Media(display));
    }
    if IMAGE_EXT.contains(&ext.as_str()) {
        return Err(DocError::Image(display));
    }

    let bytes = std::fs::read(path).map_err(|source| DocError::Io {
        path: display.clone(),
        source,
    })?;
    let sha256 = hex_digest(&bytes);

    Ok(Source {
        bytes,
        sha256,
        ext,
        display,
    })
}

/// Convert bytes already read. The format comes from the content signature
/// first and the extension only as a fallback, so a mislabelled `.txt` that is
/// really a `.docx` still converts.
pub fn convert_source(source: Source) -> Result<Converted, DocError> {
    let Source {
        bytes,
        sha256,
        ext,
        display,
    } = source;
    let byte_size = bytes.len() as u64;

    // Route 1: a real document format, detected by signature.
    let by_extension = anydoc::Format::from_extension(&ext);
    if let Some(format) = anydoc::Format::from_bytes(&bytes).or(by_extension) {
        let markdown =
            anydoc::to_markdown_bytes(&bytes, format).map_err(|source| DocError::Document {
                path: display,
                code: source.code(),
                source,
            })?;
        return Ok(Converted {
            markdown,
            format: format_name(format).to_owned(),
            sha256,
            byte_size,
        });
    }

    // Route 2: HTML. Lossy decoding is deliberate — a stray invalid byte in a
    // scraped page should not cost the caller the whole document.
    if HTML_EXT.contains(&ext.as_str()) {
        let html = String::from_utf8_lossy(&bytes);
        let markdown = htmd::convert(&html).map_err(|source| DocError::Html {
            path: display,
            source,
        })?;
        return Ok(Converted {
            markdown,
            format: "html".to_owned(),
            sha256,
            byte_size,
        });
    }

    // Route 3: anything already textual.
    let Some(text) = decode_text(&bytes) else {
        return Err(DocError::Binary(display));
    };
    Ok(Converted {
        markdown: wrap_text(&text, &ext),
        format: "text".to_owned(),
        sha256,
        byte_size,
    })
}

/// Read and convert in one step, for callers with no cache to consult.
pub fn convert_file(path: &Path) -> Result<Converted, DocError> {
    convert_source(read_source(path)?)
}

/// UTF-8 text, BOM stripped. `None` for anything that is not valid UTF-8 or
/// carries NUL bytes — the reliable tell that a file is binary.
fn decode_text(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text).to_owned())
}

/// Prose passes through; source and data files get fenced so the result is
/// valid Markdown rather than text that happens to be in a `.md` response.
fn wrap_text(text: &str, ext: &str) -> String {
    if ext.is_empty() || PROSE_EXT.contains(&ext) {
        return text.to_owned();
    }
    // A fence has to be longer than the longest run of backticks inside it.
    let longest_run = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    format!("{fence}{ext}\n{text}\n{fence}\n")
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Stable lowercase names for the graph and the cache. `Format` is
/// `#[non_exhaustive]`-adjacent in spirit; an explicit match means a new
/// variant upstream shows up as a compile error here rather than as a
/// silently renamed format in old cache rows.
fn format_name(format: anydoc::Format) -> &'static str {
    match format {
        anydoc::Format::Doc => "doc",
        anydoc::Format::Docx => "docx",
        anydoc::Format::Odt => "odt",
        anydoc::Format::Pdf => "pdf",
        anydoc::Format::Ppt => "ppt",
        anydoc::Format::Pptx => "pptx",
        anydoc::Format::Rtf => "rtf",
        anydoc::Format::Epub => "epub",
        anydoc::Format::Excel => "excel",
        anydoc::Format::Ods => "ods",
        anydoc::Format::Odp => "odp",
        anydoc::Format::Csv => "csv",
    }
}

/// Heading lines, for the outline a spilled document returns instead of its
/// body. Fenced code is skipped so that a `# comment` in a shell script does
/// not turn into a chapter.
pub fn outline(markdown: &str, max: usize) -> Vec<String> {
    let mut in_fence = false;
    let mut headings = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if trimmed.starts_with('#') && trimmed.contains(' ') {
            headings.push(trimmed.to_owned());
            if headings.len() >= max {
                break;
            }
        }
    }
    headings
}

/// Character-addressed slice. Offsets are in characters, not bytes, because
/// they cross a JSON boundary and a byte offset can land mid-codepoint.
pub fn slice_chars(markdown: &str, offset: usize, limit: usize) -> String {
    markdown.chars().skip(offset).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_passes_through_unfenced() {
        assert_eq!(wrap_text("hello", "txt"), "hello");
        assert_eq!(wrap_text("# Title", "md"), "# Title");
    }

    #[test]
    fn source_files_get_fenced_with_their_language() {
        assert_eq!(
            wrap_text("fn main() {}", "rs"),
            "```rs\nfn main() {}\n```\n"
        );
    }

    #[test]
    fn fence_outgrows_backticks_in_the_content() {
        let fenced = wrap_text("let s = \"```\";", "rs");
        assert!(fenced.starts_with("````rs\n"), "got: {fenced}");
        assert!(fenced.ends_with("\n````\n"), "got: {fenced}");
    }

    #[test]
    fn binary_content_is_not_text() {
        assert!(decode_text(&[0x00, 0x01, 0x02]).is_none());
        assert!(decode_text(&[0xff, 0xfe, 0xfd]).is_none());
    }

    #[test]
    fn bom_is_stripped() {
        let with_bom = [0xef, 0xbb, 0xbf, b'h', b'i'];
        assert_eq!(decode_text(&with_bom).as_deref(), Some("hi"));
    }

    #[test]
    fn outline_skips_headings_inside_code_fences() {
        let md = "# Real\n\n```sh\n# not a heading\n```\n\n## Also real";
        assert_eq!(outline(md, 10), vec!["# Real", "## Also real"]);
    }

    #[test]
    fn outline_respects_its_cap() {
        let md = "# a\n# b\n# c";
        assert_eq!(outline(md, 2), vec!["# a", "# b"]);
    }

    #[test]
    fn slice_counts_characters_not_bytes() {
        assert_eq!(slice_chars("привет", 2, 3), "иве");
    }

    #[test]
    fn digest_is_stable_lowercase_hex() {
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
