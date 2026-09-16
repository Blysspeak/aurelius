//! Unix-socket protocol for handing out a query embedding without every
//! process loading its own copy of bge-m3 (спека `011-dense-retrieval`,
//! `data-model.md` §6). The daemon is the single owner of the weights: it
//! loads them once at startup and serves this socket; `au search` and
//! anything else that needs a query vector asks over the socket instead of
//! loading the model itself.
//!
//! Exchange is one request, one response, no long-lived connection: the
//! client writes a JSON object, half-closes its write side, the server
//! reads to EOF, writes one JSON object back, and closes. No framing byte
//! or length prefix is needed because each direction only ever carries
//! exactly one JSON value before the writer is done.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fastembed::TextEmbedding;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

/// File name of the socket inside the aurelius home directory — the same
/// directory that already holds the database and the daemon's own lock
/// file (`daemon_lock_path`, `crates/au/src/commands.rs`), by the same
/// principle: derived from wherever the active home is, not a separately
/// hard-coded path.
const SOCKET_FILE: &str = "embed.sock";

/// `$AURELIUS_HOME/embed.sock`. `home_dir` is the directory the database
/// lives in (`db_path().parent()`).
pub fn socket_path(home_dir: &Path) -> PathBuf {
    home_dir.join(SOCKET_FILE)
}

/// A client → daemon request: the raw search query text, no Contextual
/// Prepending (`embed::format_for_embedding`) — that header is for notes
/// being written, not for the text a query is embedded with.
#[derive(Serialize, Deserialize)]
struct Request {
    query: String,
}

/// A daemon → client response. Untagged: the two shapes (`{"vector": [...]}`
/// / `{"error": "..."}`) are already mutually exclusive by field name alone,
/// so a wrapper discriminant would only add a byte no reader needs.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Response {
    Vector { vector: Vec<f32> },
    Error { error: String },
}

// ---------------------------------------------------------------------------
// Client side — used by `au search` (and anything else short-lived that
// wants a query vector without paying for the weights).
// ---------------------------------------------------------------------------

/// Ask the daemon at `socket` for the embedding of `query`, or give up with
/// a reason after `timeout`.
///
/// Every failure — no socket file, connection refused, a timeout, or the
/// daemon's own `{"error": ...}` — comes back as `Err(reason)`, never a
/// hang and never a panic. The caller's contract (spec 011 §6): on `Err`,
/// answer from full-text alone and say so in the output, don't retry and
/// don't block the search on it.
///
/// # Errors
/// A human-readable reason the vector could not be obtained.
pub async fn request_vector(
    socket: &Path,
    query: &str,
    timeout: Duration,
) -> Result<Vec<f32>, String> {
    match tokio::time::timeout(timeout, request_vector_inner(socket, query)).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "нет ответа от embed-сокета {} за {timeout:?}",
            socket.display()
        )),
    }
}

async fn request_vector_inner(socket: &Path, query: &str) -> Result<Vec<f32>, String> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("сокет {} недоступен: {e}", socket.display()))?;

    let request = Request {
        query: query.to_owned(),
    };
    let bytes =
        serde_json::to_vec(&request).map_err(|e| format!("не удалось собрать запрос: {e}"))?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|e| format!("не удалось отправить запрос: {e}"))?;
    // Half-close the write side so the server's `read_to_end` sees EOF and
    // knows the request is complete — this is a one-shot exchange, not a
    // stream of messages.
    stream
        .shutdown()
        .await
        .map_err(|e| format!("не удалось закрыть запись: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("не удалось прочитать ответ: {e}"))?;

    match serde_json::from_slice::<Response>(&buf) {
        Ok(Response::Vector { vector }) => Ok(vector),
        Ok(Response::Error { error }) => Err(error),
        Err(e) => Err(format!("демон ответил не тем, что ожидалось: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Server side — run by `au daemon` once bge-m3 is loaded. Never called if
// the model failed to load; the daemon's tick loop keeps running either
// way (спека 011 §6, ограничение №1 — model loading is an addition to the
// daemon's real job, never a precondition for it).
// ---------------------------------------------------------------------------

/// A handle to the daemon's single in-memory copy of bge-m3, shared between
/// this socket's connection loop and `au daemon`'s embedding-queue drain
/// step (`drain_embedding_queue`, `crates/au/src/commands.rs`) — both lock
/// the same weights rather than either holding, let alone loading, its own.
pub type SharedModel = Arc<Mutex<TextEmbedding>>;

/// Accepts connections on `listener` forever, each handled with the shared
/// `model`. Runs until the daemon's caller aborts the task (on `SIGTERM`) —
/// there is no other exit path here, by design: the socket lives exactly as
/// long as the daemon does.
pub async fn serve(listener: UnixListener, model: SharedModel) {
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => {
                // Transient accept failure (e.g. too many open files) — try
                // again rather than taking the whole socket down over one
                // connection, but don't busy-loop on a persistent one.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let model = Arc::clone(&model);
        tokio::spawn(async move {
            let _ = handle_connection(stream, &model).await;
        });
    }
}

async fn handle_connection(mut stream: UnixStream, model: &SharedModel) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

    let response = match serde_json::from_slice::<Request>(&buf) {
        Ok(request) => {
            let model = Arc::clone(model);
            match tokio::task::spawn_blocking(move || embed_locked(&model, request.query)).await {
                Ok(Ok(vector)) => Response::Vector { vector },
                Ok(Err(e)) => Response::Error {
                    error: e.to_string(),
                },
                Err(e) => Response::Error {
                    error: format!("инференс упал: {e}"),
                },
            }
        }
        Err(e) => Response::Error {
            error: format!("плохой запрос: {e}"),
        },
    };

    let bytes = serde_json::to_vec(&response)?;
    stream.write_all(&bytes).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Runs on a blocking-pool thread (`spawn_blocking`): `TextEmbedding::embed`
/// is synchronous CPU/GPU work, not something to hold the async executor's
/// thread hostage for. A poisoned mutex (a previous request panicked mid
/// inference) still holds a perfectly usable model — recovered with
/// `into_inner` rather than propagated, since there is no cleanup a caller
/// could meaningfully do about it and no runtime path here may panic itself.
fn embed_locked(model: &SharedModel, text: String) -> anyhow::Result<Vec<f32>> {
    let mut guard = match model.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    crate::embed::embed_single(&mut guard, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_lives_under_the_home_directory() {
        let home = Path::new("/tmp/aurelius-home");
        assert_eq!(
            socket_path(home),
            PathBuf::from("/tmp/aurelius-home/embed.sock")
        );
    }

    #[test]
    fn response_vector_round_trips_through_json() {
        let resp = Response::Vector {
            vector: vec![0.1, 0.2, 0.3],
        };
        let bytes = serde_json::to_vec(&resp).expect("serialize");
        match serde_json::from_slice::<Response>(&bytes).expect("deserialize") {
            Response::Vector { vector } => assert_eq!(vector, vec![0.1, 0.2, 0.3]),
            Response::Error { .. } => panic!("expected Vector"),
        }
    }

    #[test]
    fn response_error_round_trips_through_json() {
        let resp = Response::Error {
            error: "boom".to_owned(),
        };
        let bytes = serde_json::to_vec(&resp).expect("serialize");
        match serde_json::from_slice::<Response>(&bytes).expect("deserialize") {
            Response::Error { error } => assert_eq!(error, "boom"),
            Response::Vector { .. } => panic!("expected Error"),
        }
    }

    #[tokio::test]
    async fn request_vector_on_missing_socket_is_a_clean_err_not_a_hang() {
        let dir = std::env::temp_dir().join(format!("au-embed-sock-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket = dir.join("does-not-exist.sock");
        let result = request_vector(&socket, "hello", Duration::from_millis(500)).await;
        assert!(result.is_err());
    }
}
