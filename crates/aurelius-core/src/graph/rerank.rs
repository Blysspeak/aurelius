//! Cross-encoder rerank of the fused hybrid candidates (bge-reranker-v2-m3,
//! served by the daemon over the same `embed.sock` as query vectors).
//!
//! Strictly optional: no socket, an old daemon, an error or a slow answer all
//! come back as `None`, and the caller keeps the fused order without a word.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::Deserialize;

use crate::models::Node;

/// How many of the fused candidates the cross-encoder re-orders.
pub const RERANK_TOP: usize = 30;

/// How long a search waits for the scores before keeping the fused order.
/// Short on purpose: a cold reranker load takes seconds and must not stall a
/// search; the load goes on and the next search finds the model up.
pub const RERANK_TIMEOUT: Duration = Duration::from_millis(1500);

/// How much of the note goes into a candidate's document text.
const NOTE_CHARS: usize = 600;

/// The text the cross-encoder reads for `node`: label, claim and the first
/// 600 characters of the note, one per line, empty parts left out.
#[must_use]
pub fn rerank_doc(node: &Node) -> String {
    let claim = node
        .data
        .get(crate::provenance::CLAIM_KEY)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let note: String = node
        .note
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(NOTE_CHARS)
        .collect();
    [node.label.as_str(), claim, note.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The daemon socket beside the database `conn` is open on: the same
/// `$AURELIUS_HOME/embed.sock` the query vector comes from. `None` for an
/// in-memory database.
#[must_use]
pub fn rerank_socket_for(conn: &Connection) -> Option<PathBuf> {
    let db = conn.path().filter(|p| !p.is_empty())?;
    Path::new(db).parent().map(crate::embed_socket::socket_path)
}

#[derive(Deserialize)]
struct Scores {
    scores: Vec<f32>,
}

/// Asks the daemon at `socket` for cross-encoder scores of `docs` against
/// `query`, in the order of `docs`. Anything but exactly one score per
/// document within `timeout` is `None`: no socket, an `{"error": ...}` (an
/// unknown op included), an old daemon's `{"vector": ...}`, a timeout.
#[must_use]
pub fn rerank_scores_at(
    socket: &Path,
    query: &str,
    docs: &[String],
    timeout: Duration,
) -> Option<Vec<f32>> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let request = serde_json::json!({ "op": "rerank", "query": query, "docs": docs });
    let bytes = serde_json::to_vec(&request).ok()?;
    stream.write_all(&bytes).ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let Scores { scores } = serde_json::from_slice(&buf).ok()?;
    (scores.len() == docs.len()).then_some(scores)
}

/// Re-orders the first [`RERANK_TOP`] of `nodes` by cross-encoder score,
/// highest first; ties keep their fused order (stable sort). The tail past
/// the top stays where it was. Returns whether a rerank happened.
pub fn rerank_in_place(socket: &Path, query: &str, nodes: &mut [Node]) -> bool {
    let top = nodes.len().min(RERANK_TOP);
    if top < 2 {
        return false;
    }
    let docs: Vec<String> = nodes[..top].iter().map(rerank_doc).collect();
    let Some(scores) = rerank_scores_at(socket, query, &docs, RERANK_TIMEOUT) else {
        return false;
    };
    let mut order: Vec<usize> = (0..top).collect();
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    let reordered: Vec<Node> = order.iter().map(|&i| nodes[i].clone()).collect();
    for (slot, node) in nodes.iter_mut().zip(reordered) {
        *slot = node;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn temp_socket() -> PathBuf {
        std::env::temp_dir().join(format!("au-rerank-{}.sock", uuid::Uuid::new_v4()))
    }

    /// A one-shot server that answers the first connection with `reply`.
    fn answer_once(socket: &Path, reply: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = String::new();
            stream.read_to_string(&mut req).unwrap();
            stream.write_all(reply.as_bytes()).unwrap();
            req
        })
    }

    #[test]
    fn unknown_op_error_is_none() {
        let socket = temp_socket();
        let server = answer_once(&socket, r#"{"error":"неизвестная операция rerank"}"#);
        let docs = vec!["a".to_owned(), "b".to_owned()];
        let got = rerank_scores_at(&socket, "q", &docs, Duration::from_secs(2));
        let req = server.join().unwrap();
        assert!(got.is_none());
        assert!(req.contains(r#""op":"rerank""#), "{req}");
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn old_daemon_vector_answer_is_none() {
        let socket = temp_socket();
        let server = answer_once(&socket, r#"{"vector":[0.1,0.2]}"#);
        let docs = vec!["a".to_owned(), "b".to_owned()];
        assert!(rerank_scores_at(&socket, "q", &docs, Duration::from_secs(2)).is_none());
        server.join().unwrap();
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn missing_socket_is_none() {
        let docs = vec!["a".to_owned()];
        assert!(rerank_scores_at(&temp_socket(), "q", &docs, Duration::from_millis(200)).is_none());
    }
}
