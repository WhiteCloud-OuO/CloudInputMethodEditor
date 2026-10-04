use cloudime_dictionary::Match;

use super::association_factor;

/// 一条待排序的词库命中。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scored<'a> {
    /// 词库命中，含词、拼音、词频与是否精确。
    pub hit: Match<'a>,

    /// 词覆盖了输入开头多少个字母（不含 `'`）。按字母而不是音节算，不同切分之间才可比：
    /// `xian` 的 先（1 音节）和 西安（2 音节）覆盖同样 4 个字母。前缀词（`kaifazhe` → 开发）覆盖得少。
    /// 用来截出「同输入串选择次数」的键。
    pub coverage: usize,

    /// 输入最后一个音节是完整的、且与命中的对应音节相同（`xian` 的 先 为真；
    /// 被当成「还没打完」的前缀扩展来的 想 = `xiang` 为假）。排序时真在前。
    /// 覆盖满 + 末音节对上，等价于「整段输入被完整读出来」，不必再单列一条「音节数正好等于输入」。
    pub full_last: bool,

    /// 切分里非末尾的简拼音节数（`kai f a` 是 1，`kai fa` 是 0）：越少越像用户敲的原话，
    /// 否则 `kaifa` 会因 `kai f a` 这种切法让 开放啊 排到 开放 前面。排序时少者在前。
    pub abbreviated: usize,

    /// 产生这条命中的读法有几个音节（简拼 / 前缀按模式数算）：联想折扣 `0.8^k` 里的 `k` 用它。
    pub reading_syllables: usize,

    /// 读法的层级：0 原样、1 模糊音、2 一处编辑纠错。排序时原样优先于模糊优先于纠错，
    /// 同一层内才比权重——高频单字与模糊命中不能靠词频压过原样读法（`xian` 的 先 不能被 想 顶掉）。
    pub tier: u8,

    /// 用户权重因子，来自 Learner 的 `rank_weight`：乘在词频上，替代原来的「1 + 全局选过次数」。
    /// 自造词就是它的权重（写在用户词库的词频里），其余词按重复输入次数缓升。
    pub weight: f64,

    /// 权重折扣：原样读法 1.0，模糊音命中 0.5，一处编辑的纠错读法见 Engine。
    /// 词频、用户次数与上下文都是乘法因子，这个折扣也乘进去。
    pub correction: f64,
}

impl<'a> Scored<'a> {
    /// 联想折扣：词比产生它的读法多出的字数每字乘 0.8。
    pub fn association(&self) -> f64 {
        association_factor(self.hit.text, self.reading_syllables)
    }

    /// 预选用的便宜键：不查上下文与同输入串选择次数，命中太多时先按它砍到够排的量。
    pub(super) fn preselect_weight(&self) -> f64 {
        f64::from(self.hit.frequency) * self.weight * self.correction * self.association()
    }
    /// 候选项权重（越大越靠前）：词频 × 用户权重 × 同输入串选择次数 × 上下文系数 × 纠错系数 × 联想折扣。
    pub(super) fn weight(&self, choice: u32, context_factor: f64) -> f64 {
        f64::from(self.hit.frequency)
            * self.weight
            * (1.0 + f64::from(choice))
            * context_factor
            * self.correction
            * self.association()
    }
}
