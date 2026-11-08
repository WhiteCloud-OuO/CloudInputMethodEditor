//! 本地整句模型：后台加载、停键后请求重排、结果到了重画当前页。
//!
//! 按键回调里永远只跑词级模型；模型的意见在停键 80 毫秒后请求、几十毫秒后到，只换候选窗口里的整句候选，
//! 用户翻过页或动过高亮就不打扰。前文优先用应用里光标前的文字（DLL 起组句时随 `ClientMessage::Surrounding` 送来），没有退回本会话历史。
//! 节拍由工人循环驱动：[`Router::next_tick`] 说下次多久来一次 [`Router::tick`]；DLL 组句期间每 80 ms 的 `Poll` 也顺带 tick。
//! 加载在 [`loader`]，进行态在 [`state`]。

mod loader;
mod state;

use std::path::{Path, PathBuf};
use std::time::Duration;

use cloudime_core::CandidateLayout;
use cloudime_platform::LocalModelMode;
use cloudime_platform::protocol::SessionId;

pub(crate) use self::loader::ModelLoader;
use self::loader::{Loaded, LoadedModel, ModelFiles, ModelKind};
pub(crate) use self::state::RescoreState;
use super::Router;
use super::composed::{Composed, marked_parts};

/// 混合档：这么多音节以上才让含章参与整句重打分；更短只走知意。
const HYBRID_SENTENCE_MIN_SYLLABLES: usize = 9;

/// 混合档输入到这么多音节就先补装含章，给 9 音节正式使用留出加载时间。
const HYBRID_PREFETCH_SYLLABLES: usize = 6;

/// 启动时先装哪个模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartModel {
    /// 只装含章。
    Sentence,

    /// 只装知意。
    Zhiyi,

    /// 混合档：先装知意，含章等输入变长再补。
    ZhiyiThenSentence,

    /// 一个可用模型都没有。
    None,
}

/// 按请求档位与现有模型文件，决定先装哪个、实际能生效到哪一档。
///
/// 缺文件的降级顺序：混合 → 增强 / 基本 → 关闭；增强 → 基本 → 关闭。
/// 混合档两个文件都在时先报「增强」——含章还没装，补装成功后才真正按混合工作。
fn start_plan(
    mode: LocalModelMode,
    has_sentence: bool,
    has_zhiyi: bool,
) -> (StartModel, LocalModelMode) {
    let off = (StartModel::None, LocalModelMode::Off);
    let basic = |has: bool| {
        if has {
            (StartModel::Sentence, LocalModelMode::Basic)
        } else {
            off
        }
    };
    let enhanced = || {
        if has_zhiyi {
            (StartModel::Zhiyi, LocalModelMode::Enhanced)
        } else {
            basic(has_sentence)
        }
    };
    match mode {
        LocalModelMode::Off => off,
        LocalModelMode::Basic => basic(has_sentence),
        LocalModelMode::Enhanced => enhanced(),
        LocalModelMode::Hybrid => match (has_sentence, has_zhiyi) {
            (true, true) => (StartModel::ZhiyiThenSentence, LocalModelMode::Enhanced),
            (false, true) => (StartModel::Zhiyi, LocalModelMode::Enhanced),
            (true, false) => (StartModel::Sentence, LocalModelMode::Basic),
            (false, false) => off,
        },
    }
}

/// 找模型（`.qjm` 单文件，或开发时的三件套目录）：用户目录 `local_models/` 优先（用户自己的模型），
/// 否则随包 `Models/`（旧的 `data/local_models/` 只作开发兜底）；都没有为 `None`。
/// 目录里有多份时按 [`cloudime_neural::find_model`] 优先词表含汉字的字级模型。
pub fn find_model(user_dir: Option<&Path>, bundled_root: &Path) -> Option<PathBuf> {
    let candidates = [
        user_dir.map(|dir| dir.join("local_models")),
        Some(bundled_root.join("Models")),
        Some(bundled_root.join("data/local_models")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|dir| cloudime_neural::find_model(&dir))
}

/// 找知意模型与 ONNX Runtime：模型优先随包的 `Models/laya-zhen-long-l12-int8g/`，
/// 其次用户目录 `models/zhiyi-onnx/`（开发时也认 `target/installer/onnxruntime/` 里的运行库）。
pub fn find_zhiyi(user_dir: Option<&Path>, bundled_root: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut candidates = vec![bundled_root.join("Models/laya-zhen-long-l12-int8g")];
    if let Some(user) = user_dir {
        candidates.push(user.join("models/zhiyi-onnx"));
    }
    let dir = candidates
        .into_iter()
        .find(|dir| dir.join("laya.int8g.onnx").is_file())?;
    let runtime = [
        bundled_root.join("onnxruntime.dll"),
        bundled_root.join("target/installer/onnxruntime/onnxruntime.dll"),
    ]
    .into_iter()
    .find(|path| path.is_file())?;
    Some((dir, runtime))
}

impl Router {
    /// 启动时：记下模型文件，按 `[candidate] local_model` 决定要不要加载。
    pub fn configure_local_model(
        &mut self,
        model_path: Option<PathBuf>,
        zhiyi: Option<(PathBuf, PathBuf)>,
        mode: LocalModelMode,
        zhiyi_threads: usize,
    ) {
        self.model_path = model_path;
        self.zhiyi_model = zhiyi;
        self.applied_model = mode;
        self.zhiyi_threads = zhiyi_threads;
        self.effective_model = LocalModelMode::Off;
        self.lazy_sentence = None;
        self.sentence_ready = false;
        self.choice_ready = false;
        if mode != LocalModelMode::Off {
            self.load_local_model();
        }
    }

    /// 模型优先使用 GPU（实验性）：当前还没有 GPU 推理路径，只记录偏好；接上 DML 后由这里触发重载。
    pub fn set_prefer_gpu_model(&mut self, prefer: bool) {
        if prefer == self.prefer_gpu_model {
            return;
        }
        self.prefer_gpu_model = prefer;
        tracing::info!(prefer, "模型优先使用图形处理器（实验性）：当前版本仍用 CPU");
    }

    /// 热加载：模式或知意线程数变了才重载 / 卸载。
    pub(super) fn apply_model_config(&mut self, mode: LocalModelMode, zhiyi_threads: usize) {
        if mode == self.applied_model && zhiyi_threads == self.zhiyi_threads {
            return;
        }
        self.applied_model = mode;
        self.zhiyi_threads = zhiyi_threads;
        self.unload_local_model();
        if mode != LocalModelMode::Off {
            self.load_local_model();
        }
    }

    /// 在后台线程加载模型；没有模型文件就按可用档位回退。
    ///
    /// 混合档先只装知意：含章只在输入达到 [`HYBRID_PREFETCH_SYLLABLES`] 个音节时才在后台补装，
    /// 平时不占那份内存。知意缺失 / 加载失败时退到基本，基本也没有才退到关闭。
    fn load_local_model(&mut self) {
        if self.model_loader.is_some() {
            return;
        }
        let (start, effective) = start_plan(
            self.applied_model,
            self.model_path.is_some(),
            self.zhiyi_model.is_some(),
        );
        if effective != self.applied_model {
            tracing::warn!(
                requested = ?self.applied_model,
                effective = ?effective,
                "本地模型按可用文件降级"
            );
        }
        let started = match start {
            StartModel::None => false,
            StartModel::Sentence => self
                .model_path
                .clone()
                .is_some_and(|path| self.spawn_sentence(path)),
            StartModel::Zhiyi => self
                .zhiyi_model
                .clone()
                .is_some_and(|(dir, runtime)| self.spawn_zhiyi(dir, runtime)),
            StartModel::ZhiyiThenSentence => {
                let spawned = self
                    .zhiyi_model
                    .clone()
                    .is_some_and(|(dir, runtime)| self.spawn_zhiyi(dir, runtime));
                if spawned {
                    // 含章等第一次长输入再补，平时不占那份内存。
                    self.lazy_sentence = self.model_path.clone();
                }
                spawned
            }
        };
        self.effective_model = if started {
            effective
        } else {
            LocalModelMode::Off
        };
    }

    /// 起后台加载线程；起不来返回 `false`（调用方据此别把档位说成已生效）。
    fn spawn_sentence(&mut self, path: PathBuf) -> bool {
        self.model_loader = ModelLoader::spawn(ModelFiles::Sentence(path));
        self.model_loader.is_some()
    }

    fn spawn_zhiyi(&mut self, dir: PathBuf, runtime: PathBuf) -> bool {
        self.model_loader = ModelLoader::spawn(ModelFiles::Zhiyi {
            dir,
            runtime,
            threads: self.zhiyi_threads,
        });
        self.model_loader.is_some()
    }

    /// 混合档输入变长时补装含章；只起一次，失败就继续用知意。
    pub(super) fn ensure_lazy_sentence(&mut self) {
        if self.applied_model != LocalModelMode::Hybrid
            || self.sentence_ready
            || self.model_loader.is_some()
            || self.engine.composition_syllables() < HYBRID_PREFETCH_SYLLABLES
        {
            return;
        }
        let Some(path) = self.lazy_sentence.take() else {
            return;
        };
        tracing::info!("混合模式输入变长，后台补装含章模型");
        self.spawn_sentence(path);
    }

    /// 卸掉模型（配置关掉或换模式）。
    fn unload_local_model(&mut self) {
        self.model_loader = None;
        self.lazy_sentence = None;
        self.sentence_ready = false;
        self.choice_ready = false;
        self.effective_model = LocalModelMode::Off;
        self.engine.set_async_sentence_scorer(None);
        self.engine.set_sentence_scorer_min_syllables(0);
        self.engine.set_choice_model(None);
        self.rescore.stop();
        tracing::info!("本地模型已卸载");
    }

    /// 加载线程有结果了就接到 Engine 上；每次按键 / tick 顺手看一眼，不阻塞。
    pub(super) fn attach_loaded_model(&mut self) {
        let Some(loader) = &self.model_loader else {
            return;
        };
        let kind = loader.kind();
        match loader.poll() {
            Loaded::Pending => {}
            Loaded::Done(result) => {
                self.model_loader = None;
                match *result {
                    Ok(LoadedModel::Sentence(scorer)) => {
                        self.sentence_ready = true;
                        self.engine.set_async_sentence_scorer(Some(scorer));
                        if self.applied_model == LocalModelMode::Hybrid && self.choice_ready {
                            // 混合档的含章补装好了：9 音节以上两者协作。
                            self.engine
                                .set_sentence_scorer_min_syllables(HYBRID_SENTENCE_MIN_SYLLABLES);
                            self.effective_model = LocalModelMode::Hybrid;
                        } else {
                            self.engine.set_sentence_scorer_min_syllables(0);
                            self.effective_model = LocalModelMode::Basic;
                        }
                        // 模型上线了：日志里补一条会话信息，之后的条目知道重排开着
                        self.log_model_ready();
                    }
                    Ok(LoadedModel::Choice(model)) => {
                        self.choice_ready = true;
                        self.engine.set_sentence_scorer_min_syllables(0);
                        self.engine.set_choice_model(Some(model));
                        // 混合档的含章还没补装：先按增强档工作。
                        self.effective_model = if self.applied_model == LocalModelMode::Hybrid
                            && self.sentence_ready
                        {
                            LocalModelMode::Hybrid
                        } else {
                            LocalModelMode::Enhanced
                        };
                        self.log_model_ready();
                    }
                    Err(error) => {
                        tracing::warn!(%error, ?kind, "本地模型加载失败");
                        self.fallback_after_load_failure(kind);
                    }
                }
            }
            Loaded::Gone => {
                self.model_loader = None;
                self.fallback_after_load_failure(kind);
            }
        }
    }

    /// 模型上线了：日志里补一条会话信息，之后的条目知道重排开着。
    fn log_model_ready(&mut self) {
        tracing::info!(
            requested = ?self.applied_model,
            effective = ?self.effective_model,
            "本地模型已就绪"
        );
        self.engine
            .log_session(env!("CARGO_PKG_VERSION"), "windows");
    }

    /// 一档模型加载失败：按可用文件退到更低一档，别让设置里选着「混合」却完全没模型。
    fn fallback_after_load_failure(&mut self, kind: ModelKind) {
        match kind {
            ModelKind::Zhiyi => {
                self.choice_ready = false;
                self.lazy_sentence = None;
                if let Some(path) = &self.model_path {
                    tracing::warn!("知意加载失败，回退到基本模型");
                    self.effective_model = if self.spawn_sentence(path.clone()) {
                        LocalModelMode::Basic
                    } else {
                        LocalModelMode::Off
                    };
                } else {
                    self.effective_model = LocalModelMode::Off;
                }
            }
            ModelKind::Sentence => {
                self.sentence_ready = false;
                if self.choice_ready {
                    // 混合档的含章补装失败：知意还在，继续按增强档工作。
                    tracing::warn!("含章加载失败，混合档先按知意工作");
                    self.engine.set_sentence_scorer_min_syllables(0);
                    self.effective_model = LocalModelMode::Enhanced;
                } else {
                    self.effective_model = LocalModelMode::Off;
                }
            }
        }
    }

    /// 组句结束：什么都不等了；应用前文也作废（下一段组句 DLL 会再送）。
    pub(super) fn stop_rescoring(&mut self) {
        self.rescore.stop();
        self.engine.set_rescoring_context(None);
        // 组句结束了：脚本也不该再从 `cloudime.context` 里读到上一段的前文
        self.surrounding.clear();
        // 文本快照**不**在这里清：清了的话每段组句的第一次 `cloudime.text.*` 都是 `nil`
        //（DLL 那份快照要到组句起始的编辑会话才读得到，晚于第一键）。留着上一段组句的快照当底，
        // 焦点 / 会话真变了才清（见 `ensure_focus` / `OpenSession` / `CloseSession`）。
    }

    /// DLL 送来聚焦会话的光标前文：刷给脚本（`cloudime.context`），组句里再给 Engine 当前文。
    /// 不是聚焦会话的丢掉；没组句时只留给脚本 —— `key` 触发脚本没组句也要读光标前文（见 `script_wants_text`）。
    pub(super) fn set_surrounding(&mut self, session: SessionId, text: String, context: u64) {
        if self.focused != Some(session) {
            return;
        }
        // 换了输入框（TSF 上下文变了）：这个应用里上一框的历史不再当前文。
        if context != 0 && self.input_box != Some(context) {
            self.input_box = Some(context);
            self.engine.enter_input_box();
        }
        // 脚本的 `cloudime.context`：`key` 触发脚本每个按键前现读的那份前文走这里
        self.surrounding = text.clone();
        // 智能符号：DLL 前文是光标前最可靠的来源；空串不覆盖本进程刚上屏的文字
        self.note_text_before_caret(text.chars().next_back());
        // 本地整句模型的前文只在组句里有意义：没组句就到这儿（引擎那边保持无前文）
        if self.engine.composition().is_empty() {
            self.engine.set_rescoring_context(None);
            return;
        }
        // 缓存里按旧前文记的「要打分的」作废，重新攒一次并重新计时
        self.engine
            .set_rescoring_context((!text.is_empty()).then_some(text));
        if matches!(self.composed, Some(Composed::Candidates { .. })) {
            // 查一次只为按新前文重新记下要打分的文本，候选顺序此刻不变
            let _ = self.engine.query();
            self.schedule_rescoring();
        }
    }

    /// 缓冲变化之后：有整句路径等着打分就起防抖计时，否则停下。
    pub(super) fn schedule_rescoring(&mut self) {
        if self.engine.rescoring_pending() {
            self.rescore.schedule();
        } else {
            self.rescore.stop();
        }
    }

    /// 工人循环下次该多久后来一次 [`Self::tick`]：在等重排就按它的节拍，否则按配置热加载的一秒。
    pub fn next_tick(&self) -> Duration {
        // 脚本有挂着的 HTTP / MCP 请求时收快一点：结果早到就早回调，空转只多一次 mtime 检查。
        if self.scripts.has_pending_requests() {
            return Duration::from_millis(20);
        }
        self.rescore
            .next_deadline()
            .map_or(super::reload::CONFIG_POLL_INTERVAL, |deadline| {
                deadline.min(super::reload::CONFIG_POLL_INTERVAL)
            })
    }

    /// 到点了：接上加载好的模型、推进重排、看一眼配置文件。工人循环超时与 DLL 的 `Poll` 都会调。
    pub fn tick(&mut self) {
        self.ensure_lazy_sentence();
        self.attach_loaded_model();
        self.advance_rescoring();
        self.poll_config_reload();
        // 脚本的 HTTP 结果也借这一拍收：到了就调回调、按回调要的改候选显示再重画
        self.poll_scripts_requests();
        // 链尾清理：脚本这一轮 MCP 任务结束后，关掉它启动的 Server / sidecar
        self.scripts.finish_mcp_scopes();
        // i 模式 Shift+数字 起的后台脚本结果
        self.poll_background_scripts();
        // `time_range` 脚本到点了（本地时间落在它声明的时间段里）也借这一拍派一次 `time`
        self.poll_script_time();
    }

    /// 防抖到点就发请求；在等结果就收一次，收到了重查并重画当前页。
    fn advance_rescoring(&mut self) {
        if self.engine.composition().is_empty() {
            self.rescore.stop();
            return;
        }
        if self.rescore.debounce_elapsed() {
            if self.engine.request_rescoring() {
                self.rescore.start_polling();
            } else {
                self.rescore.stop();
            }
        }
        if !self.rescore.polling() {
            return;
        }
        if !self.engine.poll_rescoring() {
            // 等太久多半是前文变了（上屏后接着打下一段）、结果作废；真卡住也只是这轮不重排
            if self.rescore.expired() {
                tracing::debug!("等本地整句模型超时，本轮不重排");
                self.rescore.stop();
            }
            return;
        }
        // 多释义选择开着时不重排：`Choices.index` 是布局下标，重排后会对到别的候选上
        // （选中那条释义时就会消耗错的拼音）。选完退出后组句再变一次，那时才轮到重排。
        if self.highlight >= self.page_size()
            || self.navigated
            || self.translate.choices().is_some()
        {
            self.rescore.stop();
            return;
        }
        self.requery_rescored();
        // 混合档两个线程可能先后回来：还有一边在算就继续轮询，别只收一边就停。
        if self.engine.rescoring_pending() {
            self.engine.request_rescoring();
        }
        if self.engine.rescoring_pending() || self.engine.rescoring_in_flight() {
            self.rescore.start_polling();
        } else {
            self.rescore.stop();
        }
    }

    /// 分回来了：按重排后的顺序重建候选布局，重画当前页。
    fn requery_rescored(&mut self) {
        let Ok(query) = self.engine.query() else {
            return;
        };
        let page_size = self.page_size();
        let Some(Composed::Candidates {
            preedit,
            cursor,
            layout,
        }) = self.composed.as_mut()
        else {
            return;
        };
        *layout = CandidateLayout::new(query.candidates.items.clone(), page_size);
        (*preedit, *cursor) = marked_parts(&query, self.engine.special_input());
        let frame = self.self_drawn_frame();
        self.reconcile_candidates(&frame);
        // 前文在结果回来之前换过（Surrounding 晚到）：这次查询又记下了一批要打分的，再来一轮
        self.schedule_rescoring();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有模型文件时：任何档位都退到关闭，不赌「加载了但其实是空的」。
    #[test]
    fn missing_models_fall_back_to_off() {
        for mode in LocalModelMode::ALL {
            assert_eq!(
                start_plan(mode, false, false),
                (StartModel::None, LocalModelMode::Off),
                "{mode:?}"
            );
        }
    }

    /// 只有含章：基本照用，增强 / 混合退到基本。
    #[test]
    fn sentence_only_falls_back_to_basic() {
        assert_eq!(
            start_plan(LocalModelMode::Basic, true, false),
            (StartModel::Sentence, LocalModelMode::Basic)
        );
        for mode in [LocalModelMode::Enhanced, LocalModelMode::Hybrid] {
            assert_eq!(
                start_plan(mode, true, false),
                (StartModel::Sentence, LocalModelMode::Basic),
                "{mode:?}"
            );
        }
    }

    /// 只有知意：增强照用，混合退到增强，基本仍要求含章（没有就关闭）。
    #[test]
    fn zhiyi_only_falls_back_to_enhanced() {
        assert_eq!(
            start_plan(LocalModelMode::Enhanced, false, true),
            (StartModel::Zhiyi, LocalModelMode::Enhanced)
        );
        assert_eq!(
            start_plan(LocalModelMode::Hybrid, false, true),
            (StartModel::Zhiyi, LocalModelMode::Enhanced)
        );
        assert_eq!(
            start_plan(LocalModelMode::Basic, false, true),
            (StartModel::None, LocalModelMode::Off)
        );
    }

    /// 两个都在：混合先装知意（先报增强），含章等输入变长再补。
    #[test]
    fn hybrid_starts_with_zhiyi_and_lazy_sentence() {
        assert_eq!(
            start_plan(LocalModelMode::Hybrid, true, true),
            (StartModel::ZhiyiThenSentence, LocalModelMode::Enhanced)
        );
    }
}
