# 003 — Documents to Markdown

Status: implementing
Date: 2026-08-08

## Problem

An agent working through Aurelius can read source files and search the web, but a
`.docx` brief, a `.pdf` contract or an `.xlsx` price list is opaque to it. The user
has to convert those by hand and paste the text in. Every such document is knowledge
that never reaches the graph.

## Approach

Convert locally, in-process, with no network and no API keys.
[`anydoc`](https://github.com/firecrawl/anydoc) (MIT, Rust) covers the office
formats and PDF; `htmd` covers HTML; plain-text files pass through untouched.

Conversion is cheap (single-digit milliseconds), so the cache exists for a different
reason than speed: it is what makes converted documents *searchable later*. A
document read once in July is findable in September without the original file.

## Scope

Converted:

| Route | Formats |
|---|---|
| `anydoc` | doc, docx, docm, ppt, pptx, pptm, ppsx, ppsm, pps, pot, xls, xlsx, xlsm, xlsb, odt, ods, odp, rtf, epub, csv, pdf |
| `htmd` | html, htm, xhtml |
| passthrough | md, txt, text, log, json, yaml, yml, toml, xml, and source files (rs, py, ts, js, go, java, c, h, cpp, sh, sql, …) |

Not converted, and refused with a message that says so: audio and video
(transcription), images and scanned pages (OCR). These need external paid services;
they are a separate decision, not a silent gap.

## Tools

### `doc_convert`

```
path            required  file or directory
recursive       bool      descend into subdirectories (default false)
max_files       int       cap for directory mode (default 200)
project         string    attach to this project when saving to the graph
save_to_graph   bool      create a `document` node (default false)
max_inline_chars int      inline/spill threshold (default 40000)
force           bool      re-convert even on a cache hit
```

Response is hybrid. Markdown at or under `max_inline_chars` comes back whole. Larger
output is written to `<data_dir>/aurelius/docs/<sha8>-<stem>.md` and the response
carries the path, the heading outline, the first 2000 characters, `total_chars`, and
a pointer to `doc_read`. A 200-page PDF must not be able to burn an agent's context
in one call.

Directory mode converts every supported file it finds and returns one summary row
per file; unsupported files are listed as skipped rather than failing the batch.
Non-recursive by default so that a tool pointed at a project root does not walk into
`node_modules`.

### `doc_read`

```
ref     required  file path or sha256 of a previously converted document
offset  int       character offset (default 0)
limit   int       characters to return (default 40000)
```

Paginated read from the cache. Claude Code has `Read`, but other MCP clients have no
filesystem access at all, and the cache — not the spilled `.md` — is the source of
truth.

### `doc_recall`

```
query   required  FTS5 query
limit   int       max results (default 10)
```

Full-text search across everything ever converted. The web-search analogue,
`search_recall`, already establishes this shape.

## Storage

Migration V8 adds `doc_cache` keyed by the SHA-256 of the *file contents* (not the
path — a renamed or copied file is the same document), plus a `doc_fts` FTS5 mirror
with the same trigger set `search_cache`/`search_fts` uses.

`save_to_graph` creates a `document` node holding metadata and a note excerpt, linked
`belongs_to` its project. The full markdown stays in the cache: node payloads travel
over sync, and a synced graph must not start shipping megabytes of document text.

## Limits

Files over 100 MB are refused before reading. Embedded images render as alt text;
binary assets are not extracted.

## Verification

Unit tests: format routing, cache hit by content hash, pagination bounds, directory
walking with the file cap, refusal of unsupported formats. Fixtures for csv, html,
txt and a docx assembled in-test.

End-to-end: build, install, convert a real PDF and a real DOCX through `au doc`, then
call all three tools from a live MCP session.
