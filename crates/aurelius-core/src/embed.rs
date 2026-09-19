use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use ort::execution_providers::{CPU, CUDA};
use std::path::PathBuf;

fn models_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AURELIUS_MODELS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = crate::home::resolve().unwrap_or_else(|| {
        dirs_next::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("aurelius")
    });
    home.join("models")
}

fn runtime_dylib_path() -> PathBuf {
    if let Ok(path) = std::env::var("ORT_DYLIB_PATH") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let home = crate::home::resolve().unwrap_or_else(|| {
        dirs_next::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("aurelius")
    });
    home.join("onnxruntime").join("libonnxruntime.so")
}

// Must run before any other `ort` API call: the first call into `ort`
// otherwise falls back to its own default dylib lookup, which `.expect()`s
// on failure instead of returning a Result.
fn init_ort_runtime() -> anyhow::Result<()> {
    let path = runtime_dylib_path();
    if !path.is_file() {
        anyhow::bail!(
            "ONNX Runtime library not found at {}: set ORT_DYLIB_PATH or install it there",
            path.display()
        );
    }
    ort::init_from(&path)
        .map_err(|e| anyhow::anyhow!("failed to load ONNX Runtime from {}: {e}", path.display()))?
        .commit();
    Ok(())
}

/// Which execution providers `init_bge_m3` may try, from
/// `AURELIUS_EMBED_DEVICE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// CUDA first, CPU when CUDA can't run a kernel (unset or `auto`).
    Auto,
    /// CPU only, the CUDA attempt is skipped entirely (`cpu`).
    Cpu,
}

/// An unknown value is an error, not a silent `Auto`: a typo like `gpu`
/// would otherwise quietly load onto the device the owner tried to avoid.
///
/// # Errors
/// Any value other than empty, `auto` or `cpu` (case-insensitive).
pub fn parse_device(raw: Option<&str>) -> anyhow::Result<Device> {
    match raw.map(str::trim) {
        None | Some("") => Ok(Device::Auto),
        Some(v) if v.eq_ignore_ascii_case("auto") => Ok(Device::Auto),
        Some(v) if v.eq_ignore_ascii_case("cpu") => Ok(Device::Cpu),
        Some(other) => anyhow::bail!("AURELIUS_EMBED_DEVICE={other}: expected auto or cpu"),
    }
}

/// Loads bge-m3 and prints one `embed: loaded on ...` line to stderr —
/// the daemon's journal is where lazy loads and idle unloads are seen.
pub fn init_bge_m3() -> anyhow::Result<TextEmbedding> {
    let device = parse_device(std::env::var("AURELIUS_EMBED_DEVICE").ok().as_deref())?;
    init_ort_runtime()?;
    let cache_dir = models_dir();
    let has_weights = std::fs::read_dir(&cache_dir).is_ok_and(|mut entries| {
        entries.any(|entry| {
            entry.is_ok_and(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("models--BAAI--bge-m3")
            })
        })
    });
    if !has_weights {
        anyhow::bail!(
            "bge-m3 weights not found at {}: expected a models--BAAI--bge-m3 directory there",
            cache_dir.display()
        );
    }

    let started = std::time::Instant::now();
    let load_cpu = |cache_dir: PathBuf| {
        let cpu_opts = InitOptions::new(EmbeddingModel::BGEM3)
            .with_show_download_progress(true)
            .with_cache_dir(cache_dir)
            .with_execution_providers(vec![CPU::default().build()]);
        TextEmbedding::try_new(cpu_opts)
    };

    if device == Device::Cpu {
        let model = load_cpu(cache_dir)?;
        eprintln!(
            "embed: loaded on CPU in {:.1}s (AURELIUS_EMBED_DEVICE=cpu)",
            started.elapsed().as_secs_f64()
        );
        return Ok(model);
    }

    let cuda_opts = InitOptions::new(EmbeddingModel::BGEM3)
        .with_show_download_progress(true)
        .with_cache_dir(cache_dir.clone())
        .with_execution_providers(vec![CUDA::default().build().error_on_failure()]);

    // Registering the CUDA provider can succeed while the device still can't
    // run a single kernel (e.g. no prebuilt kernels for this GPU's compute
    // capability). Only an actual run proves the provider works, so probe it
    // before trusting it.
    let cuda_attempt: anyhow::Result<TextEmbedding> = (|| {
        let mut model = TextEmbedding::try_new(cuda_opts)?;
        embed_batch(&mut model, vec!["cuda smoke test".to_string()])?;
        Ok(model)
    })();

    match cuda_attempt {
        Ok(model) => {
            eprintln!(
                "embed: loaded on CUDA in {:.1}s",
                started.elapsed().as_secs_f64()
            );
            Ok(model)
        }
        Err(cuda_err) => {
            let model = load_cpu(cache_dir)?;
            eprintln!(
                "embed: loaded on CPU in {:.1}s, CUDA failed — {cuda_err}",
                started.elapsed().as_secs_f64()
            );
            Ok(model)
        }
    }
}

pub fn format_for_embedding(
    project: &str,
    node_type: &str,
    subject: &str,
    created_at: &str,
    text: &str,
) -> String {
    format!(
        "[Project: {} | Type: {} | Subject: {} | {}]\n{}",
        project, node_type, subject, created_at, text
    )
}

pub fn embed_batch(model: &mut TextEmbedding, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
    let embeddings = model.embed(texts, None)?;
    Ok(embeddings)
}

pub fn embed_single(model: &mut TextEmbedding, text: String) -> anyhow::Result<Vec<f32>> {
    let mut batch = embed_batch(model, vec![text])?;
    Ok(batch.pop().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_defaults_to_auto_and_accepts_cpu() {
        assert_eq!(parse_device(None).unwrap(), Device::Auto);
        assert_eq!(parse_device(Some("")).unwrap(), Device::Auto);
        assert_eq!(parse_device(Some("auto")).unwrap(), Device::Auto);
        assert_eq!(parse_device(Some(" CPU ")).unwrap(), Device::Cpu);
    }

    #[test]
    fn unknown_device_is_an_error_naming_the_variable() {
        let err = parse_device(Some("gpu")).unwrap_err().to_string();
        assert!(err.contains("AURELIUS_EMBED_DEVICE=gpu"), "{err}");
    }
}
