//! 模型的后台加载：加载并预热要几百毫秒到几秒，不能挡住按键；加载完由 Router 在下一次消息时接上。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use cloudime_core::ChoiceModel;
use cloudime_neural::zhiyi::ChoiceScorer;
use cloudime_neural::{CharScorer, NeuralError};

/// 这次要加载哪个模型。
pub(super) enum ModelFiles {
    /// 基本：含章·知微整句模型。
    Sentence(PathBuf),

    /// 增强：知意的 int8 ONNX 模型目录、运行库、ORT 线程数与是否优先 GPU。
    Zhiyi {
        dir: PathBuf,
        runtime: PathBuf,
        threads: usize,
        prefer_gpu: bool,
    },
}

/// 加载的是哪一档，失败时据此决定往哪一档回退。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelKind {
    Sentence,
    Zhiyi,
}

/// 加载好的模型。
pub(super) enum LoadedModel {
    Sentence(Box<CharScorer>),
    Choice(Arc<dyn ChoiceModel>),
}

/// 一次进行中的加载。
pub(crate) struct ModelLoader {
    result: Receiver<Result<LoadedModel, NeuralError>>,
    kind: ModelKind,
}

/// 一次加载的结局。
pub(super) enum Loaded {
    /// 还在加载。
    Pending,

    /// 加载好了（或失败了）。
    Done(Box<Result<LoadedModel, NeuralError>>),

    /// 加载线程没了（起不来 / panic）。
    Gone,
}

impl ModelLoader {
    /// 起线程加载并预热一次；线程起不来返回 `None`。
    pub(super) fn spawn(files: ModelFiles) -> Option<Self> {
        let kind = match &files {
            ModelFiles::Sentence(_) => ModelKind::Sentence,
            ModelFiles::Zhiyi { .. } => ModelKind::Zhiyi,
        };
        let (tx, result) = channel();
        let spawned = std::thread::Builder::new()
            .name("cloudime-model-load".to_owned())
            .spawn(move || {
                let started = std::time::Instant::now();
                let loaded = match &files {
                    ModelFiles::Sentence(path) => load_sentence(path).map(LoadedModel::Sentence),
                    ModelFiles::Zhiyi {
                        dir,
                        runtime,
                        threads,
                        prefer_gpu,
                    } => load_zhiyi(dir, runtime, *threads, *prefer_gpu).map(LoadedModel::Choice),
                };
                if loaded.is_ok() {
                    let total_ms = started.elapsed().as_millis();
                    match &files {
                        ModelFiles::Sentence(path) => tracing::info!(
                            path = %path.display(),
                            total_ms,
                            "本地模型已加载并预热"
                        ),
                        ModelFiles::Zhiyi {
                            dir,
                            threads,
                            prefer_gpu,
                            ..
                        } => tracing::info!(
                            path = %dir.display(),
                            threads,
                            prefer_gpu,
                            total_ms,
                            "本地模型已加载并预热"
                        ),
                    }
                }
                let _ = tx.send(loaded);
            });
        match spawned {
            Ok(_) => Some(Self { result, kind }),
            Err(error) => {
                tracing::warn!(%error, "起不了模型加载线程，本地模型不用");
                None
            }
        }
    }

    pub(super) fn kind(&self) -> ModelKind {
        self.kind
    }

    /// 看一眼有没有结果，不阻塞。
    pub(super) fn poll(&self) -> Loaded {
        match self.result.try_recv() {
            Ok(result) => Loaded::Done(Box::new(result)),
            Err(TryRecvError::Empty) => Loaded::Pending,
            Err(TryRecvError::Disconnected) => Loaded::Gone,
        }
    }
}

fn load_sentence(path: &Path) -> Result<Box<CharScorer>, NeuralError> {
    let scorer = CharScorer::load(path)?;
    scorer.score("", &["的"])?;
    Ok(Box::new(scorer))
}

fn load_zhiyi(
    dir: &Path,
    runtime: &Path,
    threads: usize,
    prefer_gpu: bool,
) -> Result<Arc<dyn ChoiceModel>, NeuralError> {
    let cache = cloudime_platform::dirs::cache_dir().map(|dir| dir.join("onnx"));
    let scorer = ChoiceScorer::load_onnx(dir, runtime, threads, cache.as_deref(), prefer_gpu)?;
    // 预热：第一次前向要做图优化。
    let typed = scorer.letter_units("ni");
    scorer.score("", &typed, &["ni", "nin"])?;
    Ok(Arc::new(scorer))
}
