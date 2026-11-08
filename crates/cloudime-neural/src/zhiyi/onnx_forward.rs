//! int8 ONNX 版，ONNX Runtime 跑（Windows 用）：运行库在运行时按路径加载，只用 CPU。
//! 试过 DirectML：每次序列长度都不同，集显（Radeon 880M）上一次 200 ms，比 CPU 两线程（30 ms）慢得多，见 docs/notes/crate-notes.md。
//!
//! 建 Session 时 ONNX Runtime 会对 80 MB 的 int8 图做一遍图优化（约 1 秒）。优化后的图与输入格式无关，
//! 这里把它存到用户缓存目录，下次直接读优化图（关掉图优化），省掉每次启动的那一秒。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;

use super::forward::Forward;
use super::input::Sequence;
use crate::NeuralError;

const MODEL_FILE: &str = "laya.int8g.onnx";

/// 优化后的图与它的指纹（模型 / 运行库的大小 + 修改时间）。
const CACHE_FILE: &str = "laya.int8g.optimized.onnx";
const CACHE_STAMP: &str = "laya.int8g.optimized.stamp";

pub(super) struct OnnxForward {
    /// `Session::run` 要 `&mut`；打分本来就一次一条。
    session: Mutex<Session>,
}

impl OnnxForward {
    pub fn load(
        dir: &Path,
        runtime: &Path,
        threads: usize,
        cache_dir: Option<&Path>,
    ) -> Result<Self, NeuralError> {
        load_runtime(runtime)?;
        let model = dir.join(MODEL_FILE);
        let session = match cache_dir {
            Some(cache) => load_cached(&model, runtime, threads, cache),
            None => build(&model, threads, GraphOptimizationLevel::Level3, None),
        }?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }
}

/// 有可用的优化缓存就直接读；否则正常建 Session，并把优化图写进缓存。
fn load_cached(
    model: &Path,
    runtime: &Path,
    threads: usize,
    cache_dir: &Path,
) -> Result<Session, NeuralError> {
    let stamp = stamp(model, runtime);
    let cache = cache_dir.join(CACHE_FILE);
    let stamp_file = cache_dir.join(CACHE_STAMP);
    let fresh = std::fs::read_to_string(&stamp_file)
        .ok()
        .is_some_and(|saved| saved.trim() == stamp);
    if fresh && cache.is_file() {
        match build(&cache, threads, GraphOptimizationLevel::Disable, None) {
            Ok(session) => {
                tracing::info!(cache = %cache.display(), "知意模型已从优化缓存加载");
                return Ok(session);
            }
            Err(error) => {
                tracing::warn!(%error, "优化缓存读不出来，重建");
                let _ = std::fs::remove_file(&cache);
            }
        }
    }
    // 首次 / 模型或运行库换了：正常建一次，让 ORT 顺手把优化图写出来。
    // 缓存目录不可写时不能让整个模型加载失败：退回「只建、不缓存」。
    if std::fs::create_dir_all(cache_dir).is_ok() {
        let temp = cache_dir.join(format!("{CACHE_FILE}.{}.tmp", std::process::id()));
        match build(
            model,
            threads,
            GraphOptimizationLevel::Level3,
            Some(temp.as_path()),
        ) {
            Ok(session) => {
                if temp.is_file() {
                    let _ = std::fs::remove_file(&cache);
                    match std::fs::rename(&temp, &cache) {
                        Ok(()) => {
                            let _ = std::fs::write(&stamp_file, &stamp);
                        }
                        Err(error) => {
                            tracing::warn!(%error, "优化缓存写入失败，下次仍从原模型加载");
                            let _ = std::fs::remove_file(&temp);
                        }
                    }
                }
                return Ok(session);
            }
            Err(error) => {
                tracing::warn!(%error, "优化缓存写不出来，改从原模型加载");
                let _ = std::fs::remove_file(&temp);
            }
        }
    }
    build(model, threads, GraphOptimizationLevel::Level3, None)
}

/// 模型或运行库一变，缓存就作废。
fn stamp(model: &Path, runtime: &Path) -> String {
    let part = |path: &Path| {
        std::fs::metadata(path)
            .ok()
            .and_then(|meta| {
                let modified = meta
                    .modified()
                    .ok()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?;
                Some(format!("{}:{}", meta.len(), modified.as_secs()))
            })
            .unwrap_or_else(|| "missing".to_owned())
    };
    format!("{}|{}|{}", model.display(), part(model), part(runtime))
}

fn build(
    model: &Path,
    threads: usize,
    level: GraphOptimizationLevel,
    save: Option<&Path>,
) -> Result<Session, NeuralError> {
    let started = std::time::Instant::now();
    let mut builder = Session::builder().map_err(onnx_error)?;
    builder = builder.with_optimization_level(level).map_err(onnx_error)?;
    builder = builder.with_intra_threads(threads).map_err(onnx_error)?;
    if let Some(path) = save {
        builder = builder
            .with_optimized_model_path(path)
            .map_err(onnx_error)?;
    }
    let session = builder.commit_from_file(model).map_err(onnx_error)?;
    tracing::info!(
        model = %model.display(),
        level = ?level,
        total_ms = started.elapsed().as_millis(),
        "知意 ONNX 会话已建立"
    );
    Ok(session)
}

impl Forward for OnnxForward {
    fn logits(&self, sequence: &Sequence) -> Result<Vec<f32>, NeuralError> {
        self.run(sequence, "logits").map(|(_, values)| values)
    }

    fn char_logits(&self, sequence: &Sequence) -> Result<Option<Vec<Vec<f32>>>, NeuralError> {
        let (shape, values) = self.run(sequence, "char_logits")?;
        let columns = shape.last().copied().unwrap_or(0).max(1);
        Ok(Some(values.chunks(columns).map(<[f32]>::to_vec).collect()))
    }
}

impl OnnxForward {
    /// 跑一次前向，取名为 `output` 的输出：(形状, 展平的值)。
    fn run(
        &self,
        sequence: &Sequence,
        output: &str,
    ) -> Result<(Vec<usize>, Vec<f32>), NeuralError> {
        let row = |values: Vec<i64>| Tensor::from_array(([1, values.len()], values));
        let ints = |values: &[u32]| values.iter().map(|&v| i64::from(v)).collect::<Vec<_>>();
        let slots: Vec<i64> = (0..sequence.units.len() as i64)
            .filter(|&i| sequence.units[i as usize] != 0)
            .collect();
        let markers = sequence.markers.len();
        let inputs = ort::inputs![
            "input_ids" => row(ints(&sequence.ids)).map_err(onnx_error)?,
            "attention_mask" => row(vec![1; sequence.ids.len()]).map_err(onnx_error)?,
            "unit_ids" => row(ints(&sequence.units)).map_err(onnx_error)?,
            "marker_pos" => row(ints(&sequence.markers)).map_err(onnx_error)?,
            "marker_mask" => Tensor::from_array(([1, markers], vec![true; markers])).map_err(onnx_error)?,
            "slot_pos" => row(slots).map_err(onnx_error)?,
        ];
        let mut session = self
            .session
            .lock()
            .map_err(|_| NeuralError::Corrupt("ONNX session poisoned"))?;
        let outputs = session.run(inputs).map_err(onnx_error)?;
        let (shape, values) = outputs[output]
            .try_extract_tensor::<f32>()
            .map_err(onnx_error)?;
        let shape = shape.iter().map(|&d| d.max(0) as usize).collect();
        Ok((shape, values.to_vec()))
    }
}

/// 整个进程只加载一次运行库；第二次起用第一次的结果（换路径也不重载）。
fn load_runtime(path: &Path) -> Result<(), NeuralError> {
    static LOADED: OnceLock<Result<(), String>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            ort::init_from(path)
                .map(|builder| {
                    // ONNX Runtime 的 Windows 版缺省开着遥测（ETW），关掉
                    builder.with_name("qingjian").with_telemetry(false).commit();
                })
                .map_err(|error| format!("{}: {error}", path.display()))
        })
        .clone()
        .map_err(NeuralError::Onnx)
}

fn onnx_error(error: impl std::fmt::Display) -> NeuralError {
    NeuralError::Onnx(error.to_string())
}
