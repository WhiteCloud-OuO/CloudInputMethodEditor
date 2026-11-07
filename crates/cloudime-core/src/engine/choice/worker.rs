//! 选词模型的后台线程。

use std::cell::Cell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use super::{CharTable, ChoiceModel, ChoiceRequest, ComposeRequest};

/// 后台任务。
pub(crate) enum Job {
    Score(ChoiceRequest),
    Compose(Vec<ComposeRequest>),
}

/// 后台结果。
pub(crate) enum Finished {
    Scored {
        request: ChoiceRequest,
        logits: Vec<f32>,
    },
    Composed(Vec<(String, Option<Arc<CharTable>>)>),
}

/// 选词模型的后台线程：一页候选要算几十毫秒，不能放在按键回调里。
pub(crate) struct ChoiceWorker {
    jobs: Sender<(u64, Job)>,
    results: Receiver<(u64, Finished)>,
    handle: Option<JoinHandle<()>>,

    /// 最近一次请求的序号；0 是还没请求过。
    latest: Cell<u64>,
}

impl ChoiceWorker {
    pub fn spawn(model: Arc<dyn ChoiceModel>) -> Self {
        let (jobs, job_rx) = channel::<(u64, Job)>();
        let (result_tx, results) = channel::<(u64, Finished)>();
        let handle = std::thread::Builder::new()
            .name("cloudime-choice".to_owned())
            .spawn(move || {
                while let Ok(first) = job_rx.recv() {
                    let mut score = None;
                    let mut compose = None;
                    for (serial, job) in std::iter::once(first).chain(job_rx.try_iter()) {
                        match job {
                            Job::Score(request) => score = Some((serial, request)),
                            Job::Compose(requests) => compose = Some((serial, requests)),
                        }
                    }
                    let mut done = Vec::with_capacity(2);
                    if let Some((serial, request)) = score {
                        done.push((serial, run_score(&*model, request)));
                    }
                    if let Some((serial, requests)) = compose {
                        done.push((serial, run_compose(&*model, &requests)));
                    }
                    done.sort_by_key(|(serial, _)| *serial);
                    if done
                        .into_iter()
                        .any(|result| result_tx.send(result).is_err())
                    {
                        break;
                    }
                }
            })
            .ok();
        if handle.is_none() {
            tracing::warn!("起不了选词模型线程，本次不用模型");
        }
        Self {
            jobs,
            results,
            handle,
            latest: Cell::new(0),
        }
    }

    pub fn latest(&self) -> u64 {
        self.latest.get()
    }

    pub fn submit(&self, job: Job) {
        let serial = self.latest.get() + 1;
        self.latest.set(serial);
        if self.jobs.send((serial, job)).is_err() {
            tracing::warn!("选词模型线程已退出");
        }
    }

    pub fn poll(&self) -> Option<(u64, Finished)> {
        self.results.try_recv().ok()
    }
}

impl Drop for ChoiceWorker {
    fn drop(&mut self) {
        let _ = self.handle.take();
    }
}

fn run_score(model: &dyn ChoiceModel, request: ChoiceRequest) -> Finished {
    let texts: Vec<&str> = request.texts.iter().map(String::as_str).collect();
    let started = std::time::Instant::now();
    let logits = model.score(&request.context, &request.typed, &texts);
    tracing::debug!(
        texts = texts.len(),
        ms = started.elapsed().as_millis(),
        "选词模型打分完成"
    );
    Finished::Scored { request, logits }
}

fn run_compose(model: &dyn ChoiceModel, requests: &[ComposeRequest]) -> Finished {
    let started = std::time::Instant::now();
    let tables = requests
        .iter()
        .map(|request| {
            let syllables: Vec<(&str, bool)> = request
                .syllables
                .iter()
                .map(|(text, complete)| (text.as_str(), *complete))
                .collect();
            let table = model.compose(&request.context, &syllables).map(Arc::new);
            (request.key.clone(), table)
        })
        .collect();
    tracing::debug!(
        requests = requests.len(),
        ms = started.elapsed().as_millis(),
        "选词模型出字完成"
    );
    Finished::Composed(tables)
}
