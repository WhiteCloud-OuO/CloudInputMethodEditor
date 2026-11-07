//! 知意输入法的选词模型（Zhiyi 1.4V2-Long-L12）：给候选打分，或按音节给出字分布。
//!
//! 这里只接 int8 ONNX 版（Windows 用，ONNX Runtime 的 `onnxruntime.dll` 运行时按路径加载）。
//! 权重与 `tokenizer.json`、`unified_vocab.json` 放模型目录；模型不随包，设置页下载或用户手动放。

mod bpe;
mod choice_model;
mod forward;
mod input;
#[cfg(feature = "zhiyi")]
mod onnx_forward;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::NeuralError;
use bpe::Bpe;
use forward::Forward;

/// `unified_vocab.json` 里用得上的部分。
#[derive(Deserialize)]
struct UnifiedVocab {
    units: Vec<String>,
    letters: String,

    /// composition 头的字表（`char_logits` 的列）；旧模型包没有。
    #[serde(default)]
    chars: Vec<String>,
}

/// 加载好的选词模型。
pub struct ChoiceScorer {
    forward: Box<dyn Forward>,
    tokenizer: Bpe,
    /// 拼音单元名（`f:zhong` / `p:g`）→ 单元号。
    units: HashMap<String, u32>,
    /// 英文字母表，单元号是 `units.len()` + 下标。
    letters: Vec<char>,

    /// composition 头的字 → 列号。
    chars: Arc<HashMap<char, u32>>,
}

impl ChoiceScorer {
    /// int8 ONNX 版：`runtime` 是 ONNX Runtime 动态库，CPU 跑 `threads` 个线程。
    #[cfg(feature = "zhiyi")]
    pub fn load_onnx(dir: &Path, runtime: &Path, threads: usize) -> Result<Self, NeuralError> {
        let forward = onnx_forward::OnnxForward::load(dir, runtime, threads)?;
        Self::with_forward(dir, Box::new(forward))
    }

    fn with_forward(dir: &Path, forward: Box<dyn Forward>) -> Result<Self, NeuralError> {
        let vocab: UnifiedVocab = read_json(&dir.join("unified_vocab.json"))?;
        let tokenizer = Bpe::load(&dir.join("tokenizer.json"))?;
        Ok(Self {
            forward,
            tokenizer,
            units: vocab
                .units
                .iter()
                .enumerate()
                .map(|(i, name)| (name.clone(), i as u32))
                .collect(),
            letters: vocab.letters.chars().collect(),
            chars: Arc::new(
                vocab
                    .chars
                    .iter()
                    .enumerate()
                    .filter_map(|(i, text)| Some((text.chars().next()?, i as u32)))
                    .collect(),
            ),
        })
    }

    /// 给 `candidates` 各打一个分（越大越合适）。
    pub fn score(
        &self,
        context: &str,
        typed: &[u32],
        candidates: &[&str],
    ) -> Result<Vec<f32>, NeuralError> {
        let sequence = self.sequence(context, typed, candidates)?;
        self.forward.logits(&sequence)
    }

    /// composition 头的字表：字 → [`Self::compose`] 每行里的列号。
    pub fn char_index(&self) -> Arc<HashMap<char, u32>> {
        Arc::clone(&self.chars)
    }

    /// 按音节出字：每个音节一行、各字的 log 概率。
    pub fn compose(
        &self,
        context: &str,
        syllables: &[(&str, bool)],
    ) -> Result<Option<Vec<Vec<f32>>>, NeuralError> {
        if self.chars.is_empty() || syllables.is_empty() {
            return Ok(None);
        }
        let mut typed = Vec::with_capacity(syllables.len());
        for (text, complete) in syllables {
            let name = format!("{}:{text}", if *complete { 'f' } else { 'p' });
            let Some(&unit) = self.units.get(&name) else {
                return Ok(None);
            };
            typed.push(unit);
        }
        // 选词头要至少一个候选标记，给一个空候选占位；composition 头只看拼音槽位。
        let sequence = self.sequence(context, &typed, &[""])?;
        let Some(rows) = self.forward.char_logits(&sequence)? else {
            return Ok(None);
        };
        if rows.len() != syllables.len() {
            return Ok(None);
        }
        Ok(Some(
            rows.into_iter().map(|row| log_softmax(&row)).collect(),
        ))
    }

    fn encode(&self, text: &str) -> Vec<u32> {
        self.tokenizer.encode(text)
    }
}

fn log_softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = logits.iter().map(|&l| (l - max).exp()).sum();
    let log_sum = sum.ln();
    logits.iter().map(|&l| l - max - log_sum).collect()
}

#[cfg(all(test, feature = "zhiyi"))]
mod tests {
    use super::*;

    /// 本地放了知意模型与 onnxruntime.dll 时才跑：`CLOUDIME_ZHIYI_DIR` / `CLOUDIME_ONNXRUNTIME`。
    #[test]
    fn scores_and_composes_with_a_local_model() {
        let (Some(dir), Some(runtime)) = (
            std::env::var_os("CLOUDIME_ZHIYI_DIR").map(std::path::PathBuf::from),
            std::env::var_os("CLOUDIME_ONNXRUNTIME").map(std::path::PathBuf::from),
        ) else {
            return;
        };
        let scorer = ChoiceScorer::load_onnx(&dir, &runtime, 2).unwrap();
        let typed = scorer.letter_units("nihao");
        let scores = scorer.score("今天", &typed, &["你好", "拟好"]).unwrap();
        assert_eq!(scores.len(), 2);
        let rows = scorer
            .compose("今天", &[("ni", true), ("hao", true)])
            .unwrap()
            .expect("模型带 composition 头");
        assert_eq!(rows.len(), 2);
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, NeuralError> {
    let text = std::fs::read_to_string(path).map_err(|source| NeuralError::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| NeuralError::Json {
        path: path.to_owned(),
        source,
    })
}
