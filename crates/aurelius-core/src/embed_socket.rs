//! Unix-socket protocol for handing out a query embedding without every
//! process loading its own copy of bge-m3 (спека `011-dense-retrieval`,
//! `data-model.md` §6). The daemon is the single owner of the weights: it
//! serves this socket from startup, loads the weights on the first embed
//! request and drops them again after `AURELIUS_EMBED_IDLE_SECS` without
//! one ([`Lazy`]); `au search` and anything else that needs a query vector
//! asks over the socket instead of loading the model itself.
//!
//! Exchange is one request, one response, no long-lived connection: the
//! client writes a JSON object, half-closes its write side, the server
//! reads to EOF, writes one JSON object back, and closes. No framing byte
//! or length prefix is needed because each direction only ever carries
//! exactly one JSON value before the writer is done.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use fastembed::{TextEmbedding, TextRerank};
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
    Scores { scores: Vec<f32> },
    Error { error: String },
}

/// What the server accepts: today's `{"query": ...}` (no `op`) for a query
/// vector, or `{"op": "rerank", "query": ..., "docs": [...]}` for
/// cross-encoder scores in the order of `docs`. A daemon older than `op`
/// ignores the unknown fields and answers the rerank request with a vector,
/// so the rerank client accepts nothing but `{"scores": [...]}`.
#[derive(Deserialize)]
struct Incoming {
    #[serde(default)]
    op: Option<String>,
    query: String,
    #[serde(default)]
    docs: Vec<String>,
}

/// A cross-encoder behind the socket; a trait so tests serve a fake one.
pub trait Reranker: Send + 'static {
    /// Scores of `docs` against `query`, in the order of `docs`.
    ///
    /// # Errors
    /// Inference failed.
    fn scores(&mut self, query: &str, docs: &[String]) -> anyhow::Result<Vec<f32>>;
}

impl Reranker for TextRerank {
    fn scores(&mut self, query: &str, docs: &[String]) -> anyhow::Result<Vec<f32>> {
        crate::embed::rerank_scores(self, query, docs)
    }
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
        // The daemon loads the weights lazily and drops them after idle
        // time (`Lazy`), so a cold load outlasting `timeout` is the usual
        // reason here, not a dead daemon — the load goes on after the
        // client gives up, and the next request finds the model up.
        Err(_) => Err(format!(
            "нет ответа от embed-сокета {} за {timeout:?} — вероятно, демон \
             поднимает модель после простоя, следующий запрос её застанет",
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
        Ok(Response::Scores { .. }) => Err("демон ответил оценками вместо вектора".to_owned()),
        Err(e) => Err(format!("демон ответил не тем, что ожидалось: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Server side — run by `au daemon` from startup, before any weights exist.
// A failed load answers each request with `{"error": ...}` and the daemon's
// tick loop keeps running either way (спека 011 §6, ограничение №1 — model
// loading is an addition to the daemon's real job, never a precondition).
// ---------------------------------------------------------------------------

/// How long the weights stay loaded with no embed call when
/// `AURELIUS_EMBED_IDLE_SECS` is unset. Zero — never unload.
///
/// Решение владельца 19.09.2026: ответ памяти обязан укладываться в секунду, а
/// холодная загрузка — это 1.4-1.6 с на CPU (и ~2 с на CUDA). Поэтому по
/// умолчанию модель висит резидентно: замер 19.09 — RSS демона 1722 МБ, тёплый
/// запрос 65-80 мс. Это системная память, а не видеопамять: карта остаётся
/// свободной. Ленивое поведение целиком возвращается двумя переменными:
/// `AURELIUS_EMBED_IDLE_SECS=<секунды>` и `AURELIUS_EMBED_PRELOAD=0`.
pub const DEFAULT_IDLE_SECS: u64 = 0;

/// `AURELIUS_EMBED_IDLE_SECS`: whole seconds, `0` = never unload (`None`).
///
/// # Errors
/// A value that is not a whole number of seconds.
pub fn parse_idle_secs(raw: Option<&str>) -> Result<Option<Duration>, String> {
    let secs = match raw.map(str::trim) {
        None | Some("") => DEFAULT_IDLE_SECS,
        Some(v) => v.parse::<u64>().map_err(|_| {
            format!("AURELIUS_EMBED_IDLE_SECS={v}: expected whole seconds, 0 disables unloading")
        })?,
    };
    Ok(if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    })
}

/// `AURELIUS_EMBED_PRELOAD`: `0` возвращает ленивую загрузку — модель встаёт
/// первым запросом, а не за стартом демона. Всё остальное (и отсутствие
/// переменной) — предзагрузка.
pub fn preload_from_env() -> bool {
    match std::env::var("AURELIUS_EMBED_PRELOAD") {
        Ok(v) => !matches!(v.trim(), "0" | "false" | "no"),
        Err(_) => true,
    }
}

/// The daemon's idle timeout. A bad value keeps the default and says so
/// once, instead of failing the daemon: the model is an addition to its
/// real job, never a precondition for it.
pub fn idle_from_env() -> Option<Duration> {
    let raw = std::env::var("AURELIUS_EMBED_IDLE_SECS").ok();
    parse_idle_secs(raw.as_deref()).unwrap_or_else(|reason| {
        eprintln!("embed: {reason}; using {DEFAULT_IDLE_SECS}s");
        Some(Duration::from_secs(DEFAULT_IDLE_SECS))
    })
}

enum State<M> {
    Unloaded,
    Loaded {
        model: M,
        last_used: Instant,
    },
    /// Sticky until the daemon restarts, as a startup failure was before
    /// lazy loading: retrying would re-pay the attempt on every queue tick.
    Failed(String),
}

/// One lazily loaded model behind one lock: the load happens under the same
/// mutex as inference, so concurrent first requests wait for a single copy
/// of the weights instead of each loading its own.
pub struct Lazy<M> {
    state: Mutex<State<M>>,
    load: Box<dyn Fn() -> anyhow::Result<M> + Send + Sync>,
    idle: Option<Duration>,
}

impl<M> Lazy<M> {
    /// Loads nothing yet. `idle: None` never unloads.
    pub fn new(
        load: Box<dyn Fn() -> anyhow::Result<M> + Send + Sync>,
        idle: Option<Duration>,
    ) -> Self {
        Self {
            state: Mutex::new(State::Unloaded),
            load,
            idle,
        }
    }

    /// Runs `f` on the model, loading it first if needed. Blocks for the
    /// whole load and inference, so callers run it on `spawn_blocking`.
    ///
    /// # Errors
    /// The load failed (now or earlier), or `f` itself failed.
    pub fn with<R>(&self, f: impl FnOnce(&mut M) -> anyhow::Result<R>) -> anyhow::Result<R> {
        // A poisoned lock (a previous call panicked mid inference) still
        // holds a usable model; no runtime path here may panic itself.
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if matches!(*state, State::Unloaded) {
            *state = match (self.load)() {
                Ok(model) => State::Loaded {
                    model,
                    last_used: Instant::now(),
                },
                Err(e) => {
                    eprintln!("embed: load failed, vector search off until restart — {e}");
                    State::Failed(e.to_string())
                }
            };
        }
        match &mut *state {
            State::Loaded { model, last_used } => {
                let out = f(model);
                *last_used = Instant::now();
                out
            }
            State::Failed(reason) => Err(anyhow::anyhow!("bge-m3 не поднялась — {reason}")),
            State::Unloaded => Err(anyhow::anyhow!("bge-m3 не загружена")),
        }
    }

    /// Загружает модель заранее, чтобы первый запрос не платил за холодный
    /// старт: тот же путь, что у `with`, но без инференса. Ответ памяти обязан
    /// укладываться в секунду, а холодная загрузка — это 1.4-1.6 с (решение
    /// владельца 19.09.2026). Ошибка ничего не роняет: она оседает в
    /// `State::Failed` ровно так же, как осела бы при ленивой загрузке.
    pub fn preload(&self) -> anyhow::Result<()> {
        self.with(|_| Ok(()))
    }

    /// Takes the model out when it has sat unused for the idle timeout, so
    /// the caller can drop it off the lock. Never waits: a held lock means a
    /// request is running, which is not idle.
    pub fn take_idle(&self) -> Option<(M, Duration)> {
        let idle = self.idle?;
        let mut state = match self.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        match std::mem::replace(&mut *state, State::Unloaded) {
            State::Loaded { model, last_used } if last_used.elapsed() >= idle => {
                Some((model, last_used.elapsed()))
            }
            other => {
                *state = other;
                None
            }
        }
    }
}

/// The daemon's single lazily loaded bge-m3, shared between this socket's
/// connection loop and `au daemon`'s embedding-queue drain step
/// (`drain_embedding_queue`, `crates/au/src/commands.rs`) — both go through
/// the same holder rather than either holding, let alone loading, its own.
pub type SharedModel = Arc<Lazy<TextEmbedding>>;

/// The production holder: bge-m3 via `embed::init_bge_m3`, unloaded after
/// `idle` without a call.
pub fn shared_bge_m3(idle: Option<Duration>) -> SharedModel {
    Arc::new(Lazy::new(Box::new(crate::embed::init_bge_m3), idle))
}

/// The daemon's single lazily loaded bge-reranker-v2-m3. Process-wide rather
/// than a `serve` argument so the daemon's call site stays as it is; created
/// on first use with the same idle timeout as bge-m3.
fn shared_reranker() -> Arc<Lazy<TextRerank>> {
    static RERANKER: OnceLock<Arc<Lazy<TextRerank>>> = OnceLock::new();
    Arc::clone(RERANKER.get_or_init(|| {
        Arc::new(Lazy::new(
            Box::new(crate::embed::init_bge_reranker),
            idle_from_env(),
        ))
    }))
}

/// Accepts connections on `listener` forever, each handled with the shared
/// `model`. Runs until the daemon's caller aborts the task (on `SIGTERM`) —
/// there is no other exit path here, by design: the socket lives exactly as
/// long as the daemon does.
pub async fn serve(listener: UnixListener, model: SharedModel) {
    let reranker = shared_reranker();
    if let Some(idle) = reranker.idle {
        // The daemon's tick loop only unloads bge-m3; the reranker is
        // unloaded here, checked a few times per idle period.
        let watched = Arc::clone(&reranker);
        let every = (idle / 4).max(Duration::from_secs(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                if let Some((model, quiet)) = watched.take_idle() {
                    drop(model);
                    eprintln!("rerank: unloaded after {}s idle", quiet.as_secs());
                }
            }
        });
    }
    serve_with(listener, model, reranker).await;
}

async fn serve_with<R: Reranker>(
    listener: UnixListener,
    model: SharedModel,
    reranker: Arc<Lazy<R>>,
) {
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
        let reranker = Arc::clone(&reranker);
        tokio::spawn(async move {
            let _ = handle_connection(stream, &model, &reranker).await;
        });
    }
}

async fn handle_connection<R: Reranker>(
    mut stream: UnixStream,
    model: &SharedModel,
    reranker: &Arc<Lazy<R>>,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

    let response = match serde_json::from_slice::<Incoming>(&buf) {
        Ok(Incoming {
            op: Some(op),
            query,
            docs,
        }) if op == "rerank" => {
            let reranker = Arc::clone(reranker);
            match tokio::task::spawn_blocking(move || reranker.with(|m| m.scores(&query, &docs)))
                .await
            {
                Ok(Ok(scores)) => Response::Scores { scores },
                Ok(Err(e)) => Response::Error {
                    error: e.to_string(),
                },
                Err(e) => Response::Error {
                    error: format!("инференс упал: {e}"),
                },
            }
        }
        Ok(Incoming { op: Some(op), .. }) => Response::Error {
            error: format!("неизвестная операция {op}"),
        },
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
/// — and on the first request after startup or an idle unload, the load
/// itself — is synchronous CPU/GPU work, not something to hold the async
/// executor's thread hostage for.
fn embed_locked(model: &SharedModel, text: String) -> anyhow::Result<Vec<f32>> {
    model.with(|m| crate::embed::embed_single(m, text))
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
            _ => panic!("expected Vector"),
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
            _ => panic!("expected Error"),
        }
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stand-in model: the value is the load ordinal, `loads` counts calls.
    fn counting(loads: &Arc<AtomicUsize>, idle: Option<Duration>) -> Lazy<usize> {
        let loads = Arc::clone(loads);
        Lazy::new(
            Box::new(move || Ok(loads.fetch_add(1, Ordering::SeqCst) + 1)),
            idle,
        )
    }

    #[test]
    fn nothing_loads_until_the_first_call_and_then_only_once() {
        let loads = Arc::new(AtomicUsize::new(0));
        let lazy = counting(&loads, None);
        assert_eq!(loads.load(Ordering::SeqCst), 0);
        assert_eq!(lazy.with(|m| Ok(*m)).unwrap(), 1);
        assert_eq!(lazy.with(|m| Ok(*m)).unwrap(), 1);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn idle_model_is_taken_out_and_reloaded_on_the_next_call() {
        let loads = Arc::new(AtomicUsize::new(0));
        let lazy = counting(&loads, Some(Duration::from_millis(1)));
        lazy.with(|_| Ok(())).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let (model, quiet) = lazy.take_idle().expect("idle model is taken");
        assert_eq!(model, 1);
        assert!(quiet >= Duration::from_millis(1));
        assert!(lazy.take_idle().is_none(), "nothing left to take");
        assert_eq!(lazy.with(|m| Ok(*m)).unwrap(), 2);
    }

    #[test]
    fn fresh_busy_or_never_unloading_model_is_not_taken() {
        let loads = Arc::new(AtomicUsize::new(0));
        let fresh = counting(&loads, Some(Duration::from_secs(3600)));
        fresh.with(|_| Ok(())).unwrap();
        assert!(fresh.take_idle().is_none());

        let never = counting(&loads, None);
        never.with(|_| Ok(())).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(never.take_idle().is_none());

        // Mid-call the lock is held: `take_idle` returns at once instead of
        // waiting, and the running call keeps its model.
        let busy = counting(&loads, Some(Duration::from_millis(1)));
        busy.with(|_| Ok(())).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        busy.with(|_| {
            assert!(busy.take_idle().is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn failed_load_is_reported_and_not_retried() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let lazy: Lazy<usize> = Lazy::new(
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("no weights"))
            }),
            Some(Duration::from_millis(1)),
        );
        let first = lazy.with(|m| Ok(*m)).unwrap_err().to_string();
        assert!(first.contains("no weights"), "{first}");
        assert!(lazy.with(|m| Ok(*m)).is_err());
        assert!(lazy.take_idle().is_none());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn idle_secs_default_is_never_unload_and_garbage_is_refused() {
        // По умолчанию простоев нет: ответ памяти обязан укладываться в
        // секунду, а холодная загрузка — это 1.4-1.6 с (решение владельца
        // 19.09.2026). Ленивое поведение включается числом секунд явно.
        assert_eq!(parse_idle_secs(None).unwrap(), None);
        assert_eq!(parse_idle_secs(Some("")).unwrap(), None);
        assert_eq!(
            parse_idle_secs(Some("45")).unwrap(),
            Some(Duration::from_secs(45))
        );
        assert_eq!(parse_idle_secs(Some("0")).unwrap(), None);
        assert!(parse_idle_secs(Some("5m")).is_err());
    }

    #[tokio::test]
    async fn request_vector_on_missing_socket_is_a_clean_err_not_a_hang() {
        let dir = std::env::temp_dir().join(format!("au-embed-sock-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket = dir.join("does-not-exist.sock");
        let result = request_vector(&socket, "hello", Duration::from_millis(500)).await;
        assert!(result.is_err());
    }

    /// Scores each document by its length, so the order is predictable.
    struct FakeReranker;

    impl Reranker for FakeReranker {
        fn scores(&mut self, query: &str, docs: &[String]) -> anyhow::Result<Vec<f32>> {
            assert_eq!(query, "q");
            Ok(docs.iter().map(|d| d.len() as f32).collect())
        }
    }

    #[tokio::test]
    async fn rerank_round_trips_through_the_socket_on_a_fake_model() {
        let socket =
            std::env::temp_dir().join(format!("au-embed-rerank-{}.sock", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&socket).expect("bind");
        let model: SharedModel = Arc::new(Lazy::new(
            Box::new(|| Err(anyhow::anyhow!("not in this test"))),
            None,
        ));
        let reranker = Arc::new(Lazy::new(Box::new(|| Ok(FakeReranker)), None));
        let server = tokio::spawn(serve_with(listener, model, reranker));

        let path = socket.clone();
        let scores = tokio::task::spawn_blocking(move || {
            let docs = vec!["aaa".to_owned(), "a".to_owned(), "aa".to_owned()];
            crate::graph::rerank_scores_at(&path, "q", &docs, Duration::from_secs(5))
        })
        .await
        .expect("join");
        assert_eq!(scores, Some(vec![3.0, 1.0, 2.0]));

        // Today's vector request still answers as before (here: the error
        // of a model that cannot load), not with scores.
        let err = request_vector(&socket, "q", Duration::from_secs(5)).await;
        assert!(err.unwrap_err().contains("not in this test"));

        server.abort();
        let _ = std::fs::remove_file(&socket);
    }
}
