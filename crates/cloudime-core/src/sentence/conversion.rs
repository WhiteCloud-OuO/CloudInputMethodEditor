use super::SentenceWord;

/// 一次整句转换的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    /// 整句文本。
    pub text: String,

    /// 覆盖的全拼音节，与 `text` 逐字对应。
    pub syllables: Vec<String>,

    /// 路径上的词，按顺序。只有一个说明整段本来就是一个词，不必当整句显示。
    pub words: Vec<SentenceWord>,

    /// 路径得分（log 概率之和），越大越好。
    pub score: f64,

    /// `score` 里静态语言模型那部分（每步只问静态模型、不认识就兜底的 log 概率之和），不含个人 n-gram 插值、
    /// 用户加分与代价。神经重打分用它：神经分替换的是静态模型的判断，个人的那些原样保留。
    pub static_score: f64,

    /// 路径上模糊音 / 敲错变体的代价之和（已含在 `score` 里）：大于 0 说明这条路径不是按敲的原样读的。
    pub penalty: f64,

    /// 路径词权重的对数之和：每个词贡献 `ln(词频 × (1+用户次数) × 联想折扣)`，词权重下限 1.0。
    /// 整句权重取它的几何平均。
    pub log_weight: f64,

    /// 神经重打分的模型系数 `clamp(exp(λ·(神经分 − 静态分)), e^-4, e^4)`；没拿到神经分时 1.0。
    pub neural_factor: f64,
}

impl Conversion {
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// 整句权重：路径词权重的几何平均 × 模型系数。
    pub fn weight(&self) -> f64 {
        if self.words.is_empty() {
            return self.neural_factor;
        }
        (self.log_weight / self.words.len() as f64).exp() * self.neural_factor
    }

    /// 路径里有音节不是按敲的原样读的（模糊音或敲错变体）。
    pub fn altered(&self) -> bool {
        self.penalty > 0.0
    }

    /// 路径里有占位音节（没转成字的拼音）。
    pub fn has_placeholder(&self) -> bool {
        self.words.iter().any(|w| w.placeholder)
    }
}
