//! int8 ONNX 版，ONNX Runtime 跑（Windows 用）：运行库在运行时按路径加载。
//!
//! 默认 CPU；开了 `[debugging] prefer_gpu_model` 且构建带 `directml` feature 时优先 DirectML。
//! DML 用原模型的**动态维度**：固定维度需要 padding 感知的量化图，而那份图在 DirectML 上数值
//! 不稳定（见 `docs/notes/performance.md`），所以不按桶 padding。短输入 DML 可能比 CPU 慢，
//! 这是用户显式打开实验开关后的取舍；GPU 不可用、初始化失败或推理失败时回退 CPU。
//!
//! 建 Session 时 ONNX Runtime 会对 80 MB 的 int8 图做一遍图优化（约 1 秒）。CPU 的优化图与输入格式无关，
//! 这里把它存到用户缓存目录，下次直接读优化图（关掉图优化），省掉每次启动的那一秒。

use std::path::{Path, PathBuf};
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

/// 一次前向的输出：(形状, 展平的值)。
type RunOutput = (Vec<usize>, Vec<f32>);

pub(super) struct OnnxForward {
    /// 原模型：CPU 路径与 DML 用。
    original: PathBuf,

    /// ONNX Runtime 动态库路径（缓存指纹用）。
    runtime: PathBuf,

    /// ORT 线程数。
    threads: usize,

    /// CPU 优化图缓存目录。
    cache_dir: Option<PathBuf>,

    /// CPU 会话；GPU 正常时按需懒加载（只在回退时建）。
    cpu: Mutex<Option<Session>>,

    /// DirectML 会话；`None` 表示用 CPU。
    #[cfg(feature = "directml")]
    dml: Mutex<Option<Dml>>,
}

impl OnnxForward {
    pub fn load(
        dir: &Path,
        runtime: &Path,
        threads: usize,
        cache_dir: Option<&Path>,
        prefer_gpu: bool,
    ) -> Result<Self, NeuralError> {
        load_runtime(runtime)?;
        let original = dir.join(MODEL_FILE);

        #[cfg(feature = "directml")]
        let mut dml = if prefer_gpu {
            match Dml::load(&original, threads) {
                Ok(dml) => {
                    tracing::info!(model = %original.display(), "知意模型优先使用 DirectML");
                    Some(dml)
                }
                Err(error) => {
                    tracing::warn!(%error, "DirectML 初始化失败，回退 CPU");
                    None
                }
            }
        } else {
            None
        };

        #[cfg(not(feature = "directml"))]
        let dml: Option<()> = {
            if prefer_gpu {
                tracing::warn!("构建时没有启用 directml feature，模型仍用 CPU");
            }
            None
        };

        // CPU 在 GPU 探测 / 回退时都要用；先建好，GPU 可用也留作回退。
        let mut cpu = Some(load_cpu(&original, runtime, threads, cache_dir)?);

        #[cfg(feature = "directml")]
        if let (Some(dml_state), Some(cpu_session)) = (dml.as_mut(), cpu.as_mut()) {
            let forced = std::env::var_os("CLOUDIME_ZHIYI_DML_FORCE").is_some();
            if forced {
                tracing::warn!("CLOUDIME_ZHIYI_DML_FORCE 已设置：跳过 DirectML 数值/速度探测");
            } else {
                match probe_dml(dml_state, cpu_session) {
                    Ok(()) => tracing::info!(model = %original.display(), "知意模型使用 DirectML"),
                    Err(error) => {
                        tracing::warn!(%error, "DirectML 探测不达标，回退 CPU");
                        dml = None;
                    }
                }
            }
        }

        Ok(Self {
            original,
            runtime: runtime.to_owned(),
            threads,
            cache_dir: cache_dir.map(Path::to_owned),
            cpu: Mutex::new(cpu),
            #[cfg(feature = "directml")]
            dml: Mutex::new(dml),
        })
    }

    /// 跑一次前向，取名为 `output` 的输出：(形状, 展平的值)。
    fn run(&self, sequence: &Sequence, output: &str) -> Result<RunOutput, NeuralError> {
        #[cfg(feature = "directml")]
        if let Some(result) = self.try_dml(sequence, output) {
            return result;
        }
        self.run_cpu(sequence, output)
    }

    /// 先试 DirectML；返回 `None` 表示这次走 CPU。
    #[cfg(feature = "directml")]
    fn try_dml(&self, sequence: &Sequence, output: &str) -> Option<Result<RunOutput, NeuralError>> {
        let mut guard = self.dml.lock().ok()?;
        let dml = guard.as_mut()?;
        match dml.run(sequence, output) {
            Ok(value) => Some(Ok(value)),
            Err(error) => {
                tracing::warn!(%error, "DirectML 推理失败，回退 CPU");
                *guard = None;
                None
            }
        }
    }

    fn run_cpu(&self, sequence: &Sequence, output: &str) -> Result<RunOutput, NeuralError> {
        let mut guard = self
            .cpu
            .lock()
            .map_err(|_| NeuralError::Corrupt("ONNX session poisoned"))?;
        if guard.is_none() {
            *guard = Some(load_cpu(
                &self.original,
                &self.runtime,
                self.threads,
                self.cache_dir.as_deref(),
            )?);
        }
        let session = guard.as_mut().expect("just filled");
        run_session(session, sequence, output)
    }
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

/// DirectML 会话（原模型，动态维度）。
#[cfg(feature = "directml")]
struct Dml {
    session: Session,
}

#[cfg(feature = "directml")]
impl Dml {
    fn load(original: &Path, threads: usize) -> Result<Self, NeuralError> {
        Ok(Self {
            session: build_dml(original, threads)?,
        })
    }

    fn run(&mut self, sequence: &Sequence, output: &str) -> Result<RunOutput, NeuralError> {
        run_session(&mut self.session, sequence, output)
    }
}

/// 用一个固定序列比对 DML 与 CPU：数值差太大或明显更慢就不用 DML。
#[cfg(feature = "directml")]
fn probe_dml(dml: &mut Dml, cpu: &mut Session) -> Result<(), NeuralError> {
    let sequence = probe_sequence();
    let (_, cpu_logits) = run_session(cpu, &sequence, "logits")?;
    let (_, dml_logits) = dml.run(&sequence, "logits")?;
    let logits_diff = max_diff(&cpu_logits, &dml_logits);
    let (_, cpu_chars) = run_session(cpu, &sequence, "char_logits")?;
    let (_, dml_chars) = dml.run(&sequence, "char_logits")?;
    let chars_diff = max_diff(&cpu_chars, &dml_chars);
    if logits_diff > 5e-3 || chars_diff > 5e-3 {
        return Err(NeuralError::Onnx(format!(
            "DirectML 数值差过大：logits {logits_diff:.3e} / char {chars_diff:.3e}"
        )));
    }

    let cpu_time = measure_cpu(cpu, &sequence)?;
    let dml_time = measure_dml(dml, &sequence)?;
    if dml_time > cpu_time * 2.0 {
        return Err(NeuralError::Onnx(format!(
            "DirectML 比 CPU 慢：GPU {dml_time:.1}ms / CPU {cpu_time:.1}ms"
        )));
    }
    tracing::info!(
        logits_diff,
        chars_diff,
        cpu_ms = cpu_time,
        dml_ms = dml_time,
        "DirectML 探测通过"
    );
    Ok(())
}

#[cfg(feature = "directml")]
fn probe_sequence() -> Sequence {
    let ids: Vec<u32> = (0..128).map(|index| 10 + (index % 100) as u32).collect();
    let units: Vec<u32> = (0..ids.len())
        .map(|index| {
            if index % 16 == 0 {
                (index / 16 + 1) as u32
            } else {
                0
            }
        })
        .collect();
    let markers = vec![1, 20, 40, 60, 80, 100, 120, 127];
    Sequence {
        ids,
        units,
        markers,
    }
}

#[cfg(feature = "directml")]
fn max_diff(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f32::max)
}

#[cfg(feature = "directml")]
fn measure_cpu(session: &mut Session, sequence: &Sequence) -> Result<f64, NeuralError> {
    let started = std::time::Instant::now();
    for _ in 0..3 {
        run_session(session, sequence, "logits")?;
    }
    Ok(started.elapsed().as_secs_f64() * 1000.0 / 3.0)
}

#[cfg(feature = "directml")]
fn measure_dml(dml: &mut Dml, sequence: &Sequence) -> Result<f64, NeuralError> {
    let started = std::time::Instant::now();
    for _ in 0..3 {
        dml.run(sequence, "logits")?;
    }
    Ok(started.elapsed().as_secs_f64() * 1000.0 / 3.0)
}

#[cfg(feature = "directml")]
fn build_dml(model: &Path, threads: usize) -> Result<Session, NeuralError> {
    use ort::ep::DirectML;
    use ort::ep::ExecutionProvider;
    if !DirectML::default().is_available().map_err(onnx_error)? {
        return Err(NeuralError::Onnx(
            "当前 onnxruntime.dll 不支持 DirectML".to_owned(),
        ));
    }
    let started = std::time::Instant::now();
    let provider = DirectML::default().build().error_on_failure();
    let session = Session::builder()
        .map_err(onnx_error)?
        // DirectML EP 不支持内存模式和并行执行，开着会报错 / 反复准备。
        .with_memory_pattern(false)
        .map_err(onnx_error)?
        .with_parallel_execution(false)
        .map_err(onnx_error)?
        .with_execution_providers([provider])
        .map_err(onnx_error)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(onnx_error)?
        .with_intra_threads(threads)
        .map_err(onnx_error)?
        .with_inter_threads(1)
        .map_err(onnx_error)?
        .commit_from_file(model)
        .map_err(onnx_error)?;
    tracing::info!(
        model = %model.display(),
        total_ms = started.elapsed().as_millis(),
        "DirectML 会话已建立"
    );
    Ok(session)
}

/// 跑一次前向。
fn run_session(
    session: &mut Session,
    sequence: &Sequence,
    output: &str,
) -> Result<RunOutput, NeuralError> {
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
    let outputs = session.run(inputs).map_err(onnx_error)?;
    let (shape, values) = outputs[output]
        .try_extract_tensor::<f32>()
        .map_err(onnx_error)?;
    let shape = shape.iter().map(|&d| d.max(0) as usize).collect();
    Ok((shape, values.to_vec()))
}

/// CPU：有可用的优化缓存就直接读；否则正常建 Session，并把优化图写进缓存。
fn load_cpu(
    model: &Path,
    runtime: &Path,
    threads: usize,
    cache_dir: Option<&Path>,
) -> Result<Session, NeuralError> {
    match cache_dir {
        Some(cache) => load_cached(model, runtime, threads, cache),
        None => build(model, threads, GraphOptimizationLevel::Level3, None),
    }
}

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
