//! 双键拼音：键位表与解码器。
//!
//! 表由外部给（方案文件在 `cloudime-platform` 里读，这里不碰 IO）；规范见
//! `docs/design/input-schemes.md`。
//!
//! 解码器干的事：把「两键一个音节」的按键串还原成完整拼音 —— `ypdo` → `yun'duo`。
//! 同键多义（`t` = `ve`/`ue`）两端都试，所以一次可能给出几种串（封顶 [`MAX_ALTERNATIVES`]）。

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::parser::Segmentation;

/// 一次组句最多给几个候选串。
pub const MAX_ALTERNATIVES: usize = 9;

/// 一个音节最多几位的辅助码。
pub const MAX_AUXILIARY: usize = 4;

/// 单字母声母：键位表里没写声母的键，是这些字母时按字母本身当声母（`zh`/`ch`/`sh` 由表里给）。
const SINGLE_INITIALS: &str = "bpmfdtnlgkhjqxrzcsyw";

/// 一个键的映射：声母与韵母分开收（同一个键按位置分别当声母与韵母）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMapping {
    /// 声母（`zh` / `ch` / `sh`）；通常为空。
    pub initials: Vec<String>,

    /// 韵母，按方案文件里的顺序。
    pub finals: Vec<String>,
}

/// 双键拼音键位表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// 零声母：`[Shengmu]` 里写的「两键 → 零声母韵母」（键已归一成小写）。
    /// 例：`('a','a') → a`、`('a','y') → ang`。没写的两键组合就不出零声母。
    pub zero_initials: BTreeMap<(char, char), String>,

    /// 键 → 声母 / 韵母。
    pub remapping: BTreeMap<char, KeyMapping>,

    /// 物理键互换（`(y, z)` = y 与 z 互换）。
    pub exchange: Vec<(char, char)>,

    /// 这个方案开不开辅助码（`[Auxiliary] UsingAuxiliaryCodes`）：开了才把大写字母当辅助码。
    /// 辅助码**逐字对应**（每个字一位、最多 [`MAX_AUXILIARY`] 位），没有「位置」可选。
    pub auxiliary: bool,
}

impl Table {
    /// `Remapping` 里有没有用到 `;` —— 有才把它当拼音键吃进组句（微软 / 搜狗的 `ing`）。
    pub fn uses_semicolon(&self) -> bool {
        self.remapping.contains_key(&';')
    }
}

/// 解出来的一个音节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syllable {
    /// 完整拼音（如 `yun`）。
    pub pinyin: String,

    /// 辅助码（大写，可能空着）。
    pub auxiliary: String,
}

/// 一种解法：整串音节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// 按顺序解出来的音节。
    pub syllables: Vec<Syllable>,

    /// 尾巴上没解完的按键（还在打的那半个音节），原样带着。
    pub trailing: String,

    /// 尾巴上那半个音节后面紧跟的辅助码（大写，可能空着）。
    /// 例：`yH`（`y` 是「要」的第一键、第二键还没打直接敲辅助码）→ `trailing = "y"`、`trailing_auxiliary = "H"`。
    pub trailing_auxiliary: String,
}

impl Expansion {
    /// 拼回完整拼音串：音节之间用 `'` 隔开（现有 parser 把 `'` 当硬边界，正好锁住分词）；
    /// 没解完的尾巴原样接在后面，parser 会把它当「半个音节」。
    pub fn to_pinyin(&self) -> String {
        let mut text = String::new();
        for syllable in &self.syllables {
            if !text.is_empty() {
                text.push('\'');
            }
            text.push_str(&syllable.pinyin);
        }
        text.push_str(&self.trailing);
        text
    }
}

impl Table {
    /// 按下这些键能解出哪些串：`'` 强制分词；**开了辅助码时大小写有意义** —— 小写字母是拼音键、
    /// 大写字母是辅助码（跟在**完整音节**或**末尾那半个音节**后面，最多 [`MAX_AUXILIARY`] 位）；
    /// 别的位置出现大写就解不出来。
    /// 解不出来（半截音节、键位表里没有的键、大小写混用）就是空表 —— 调用方按原样放行。
    ///
    /// 开了辅助码、且**第一个键是大写**（Caps Lock / Shift 卡着）时先过 [`Self::lowered_leading_key`]，
    /// 不然整段都解不出来。
    pub fn expand(&self, keys: &str) -> Vec<Expansion> {
        let keys: Vec<char> = keys.chars().map(|key| self.exchange_key(key)).collect();
        let mut out = Vec::new();
        self.walk(&keys, 0, Vec::new(), &mut out);
        out
    }

    /// 开了辅助码时**第一个键是大写**（Caps Lock / Shift 卡着）就把它压成小写：大写只能当辅助码，
    /// 开头一个大写本来整段都切不出来。**只动第一个键** —— 后面还可能有辅助码（`YpdoYP` → `ypdoYP`）。
    /// 候选窗里的拼音行与回车原样上屏也跟着用这一份。没开辅助码、或不是大写开头 → 原样返回。
    pub fn lowered_leading_key<'a>(&self, keys: &'a str) -> Cow<'a, str> {
        let mut chars = keys.chars();
        match chars.next().filter(|key| key.is_ascii_uppercase()) {
            Some(first) if self.auxiliary => {
                let rest = chars.as_str();
                let mut out = String::with_capacity(keys.len());
                out.push(first.to_ascii_lowercase());
                out.push_str(rest);
                Cow::Owned(out)
            }
            _ => Cow::Borrowed(keys),
        }
    }

    /// 把一种解法转成**切分**：每个音节都按方案定死，之后**不再交给全键那套切分器**自由切一遍。
    ///
    /// 这一点是硬要求：按键只有方案那一种读法。交给全键的切分器，它还会认**简拼**（`kai` 切成 `k` + `ai`）、
    /// 还会把末尾音节放宽成前缀 —— `ceshi` 解出的 `ce` + `song` + 半个音节会被读成 `ce'shi`，
    /// 于是「测试」混进候选。
    ///
    /// 尾巴上那半个音节（只按了一个键）还不知道是什么音节，按它**可能接出**的音节**一族一份切分**
    /// （调用方按并集查）：声母那一族收成一条前缀（`g` → `g…`），零声母那几族一个音节一份 ——
    /// 首道 `u` 那几族是 `e` / `ei` / `eng`，它们互不为前缀，收成公共前缀 `e…` 会把同前缀的 `en` / `er`
    /// 也放进来。一个都接不出来时尾巴不参与，只按已经完整的音节读；连一个完整音节都没有就是空表。
    pub fn segmentations(&self, expansion: &Expansion) -> Vec<Segmentation> {
        let head: Vec<crate::parser::Syllable> = expansion
            .syllables
            .iter()
            .map(|syllable| crate::parser::Syllable::complete(&syllable.pinyin))
            .collect();
        let mut out: Vec<Segmentation> = Vec::new();
        if let Some(first) = expansion
            .trailing
            .chars()
            .next()
            .map(|first| first.to_ascii_lowercase())
        {
            if let Some(initial) = self.initial(first)
                && self
                    .pair_syllables(first)
                    .any(|syllable| syllable.starts_with(&initial))
            {
                let mut syllables = head.clone();
                syllables.push(crate::parser::Syllable::partial(&initial));
                out.push(Segmentation { syllables });
            }
            for ((key, _), final_) in &self.zero_initials {
                if *key == first {
                    let mut syllables = head.clone();
                    syllables.push(crate::parser::Syllable::complete(final_));
                    out.push(Segmentation { syllables });
                }
            }
        }
        if out.is_empty() && !head.is_empty() {
            out.push(Segmentation { syllables: head });
        }
        out
    }

    /// 这个键当头键、第二个键还不知道时，后面可能接出的音节（第二个键取遍所有键）。
    fn pair_syllables(&self, first: char) -> impl Iterator<Item = String> {
        self.remapping
            .keys()
            .flat_map(move |second| self.syllable(first, *second))
    }

    /// 前 `count` 个音节在按键串里占几个键（`'` 也算，辅助码算它那个音节的）。
    ///
    /// 第 `count` 个音节只有半个（尾巴上那一个键）时按 1 个键补上 —— 双拼查候选时的「覆盖多少」用
    /// **按键数**，不用读法里的字母数：同一段按键的几种切法得用同一把尺子量，不然尾巴那半个音节的
    /// 几种猜法会互相压（`a` 的 `ang` 比 `a` 长，候选就全被 `ang` 那一族占了）。
    pub fn typed_covered(&self, keys: &str, count: usize) -> usize {
        self.typed_span(keys, count)
            .or_else(|| {
                self.typed_span(keys, count.saturating_sub(1))
                    .map(|keys| keys + 1)
            })
            .unwrap_or(0)
    }

    /// 物理键互换：在查键位表**之前**做（`(y, z)` = 打 z 出 y、打 y 出 z），大小写跟着走。
    fn exchange_key(&self, key: char) -> char {
        for (left, right) in &self.exchange {
            if key.eq_ignore_ascii_case(left) {
                return if key.is_ascii_uppercase() {
                    right.to_ascii_uppercase()
                } else {
                    *right
                };
            }
            if key.eq_ignore_ascii_case(right) {
                return if key.is_ascii_uppercase() {
                    left.to_ascii_uppercase()
                } else {
                    *left
                };
            }
        }
        key
    }

    /// 前 `count` 个音节在**按键串**里占多少字节（一个音节两键 + 紧跟的辅助码，中间的 `'` 也算上）。
    ///
    /// 双拼敲的不是拼音本身（`vhgo` → `zhong'guo`），选中候选时靠它把「候选吃了几个音节」换算回
    /// 缓冲区要吃掉多少键 —— 不然选完候选拼音串里还留着原来的按键。键不够返回 `None`。
    pub fn typed_span(&self, keys: &str, count: usize) -> Option<usize> {
        let chars: Vec<char> = keys.chars().collect();
        let (mut index, mut syllables, mut bytes) = (0, 0, 0);
        while syllables < count {
            // `'` 只是强制分词，跟着吃掉、不算一个音节
            if chars.get(index) == Some(&'\'') {
                bytes += 1;
                index += 1;
                continue;
            }
            // 一个音节两键
            let first = chars.get(index)?;
            let second = chars.get(index + 1)?;
            bytes += first.len_utf8() + second.len_utf8();
            index += 2;
            // 开了辅助码：紧跟音节的大写字母（最多 [`MAX_AUXILIARY`] 位）也算这个音节的键
            let mut auxiliary = 0;
            while self.auxiliary
                && auxiliary < MAX_AUXILIARY
                && chars.get(index).is_some_and(char::is_ascii_uppercase)
            {
                bytes += chars[index].len_utf8();
                index += 1;
                auxiliary += 1;
            }
            syllables += 1;
        }
        Some(bytes)
    }

    /// 原始按键的分词显示：按键按音节用半角 `'` 隔开（`vhgo` → `vh'go`，辅助码贴着它那个音节）。
    ///
    /// `show_raw_input` 开着时拼音行走它 —— 光给一串按键看不出打了几段。
    pub fn typed_marked(&self, keys: &str) -> String {
        let chars: Vec<char> = keys.chars().collect();
        let mut groups: Vec<String> = Vec::new();
        let mut index = 0;
        while index < chars.len() {
            // 用户自己敲的 `'` 只是分词，跳过（分隔符由下面统一补）
            if chars[index] == '\'' {
                index += 1;
                continue;
            }
            // 一个音节两键（末尾那半个音节就按剩下的算）
            let end = (index + 2).min(chars.len());
            let mut group: String = chars[index..end].iter().collect();
            index = end;
            // 开了辅助码：紧跟音节的大写字母贴着这个音节
            let mut auxiliary = 0;
            while self.auxiliary
                && auxiliary < MAX_AUXILIARY
                && chars.get(index).is_some_and(char::is_ascii_uppercase)
            {
                group.push(chars[index]);
                index += 1;
                auxiliary += 1;
            }
            groups.push(group);
        }
        groups.join("'")
    }

    fn walk(&self, keys: &[char], index: usize, done: Vec<Syllable>, out: &mut Vec<Expansion>) {
        if out.len() >= MAX_ALTERNATIVES {
            return;
        }
        if index >= keys.len() {
            out.push(Expansion {
                syllables: done,
                trailing: String::new(),
                trailing_auxiliary: String::new(),
            });
            return;
        }
        // `'` 只是强制分词，本身不上屏
        if keys[index] == '\'' {
            self.walk(keys, index + 1, done, out);
            return;
        }
        // 一个音节两键；只剩一个键就是「还在打的那半个音节」，原样带着（不是字母就不管，调用方放行）
        if index + 1 >= keys.len() {
            // 开了辅助码：大写是辅助码，落单的大写没有音节可挂 → 这一支什么都不出
            let lone = keys[index];
            let is_auxiliary = self.auxiliary && lone.is_ascii_uppercase();
            if lone.is_ascii_alphabetic() && !is_auxiliary {
                out.push(Expansion {
                    syllables: done,
                    trailing: lone.to_string(),
                    trailing_auxiliary: String::new(),
                });
            }
            return;
        }
        let (first, second) = (keys[index], keys[index + 1]);
        // 开了辅助码：**半音节后面直接跟辅助码**（`yH` = 「要」只按了第一键 `y` 就敲 `H`，第二键跳过）——
        // 第二键位置上是**大写**就是这种写法；那串大写必须**一直吃到末尾**才算半个音节（后面还有键
        // 就说明这不是尾巴，交给下面「音节位上有大写」那一支按打错处理）。
        if self.auxiliary && second.is_ascii_uppercase() {
            let auxiliary = self.auxiliary_run(keys, index + 1);
            if index + 1 + auxiliary.chars().count() >= keys.len()
                && first.is_ascii_alphabetic()
                && !first.is_ascii_uppercase()
            {
                out.push(Expansion {
                    syllables: done,
                    trailing: first.to_string(),
                    trailing_auxiliary: auxiliary,
                });
            }
            return;
        }
        // 开了辅助码：**小写才是拼音键**（大写是辅助码，只能跟在完整音节 / 末尾那半个音节后面 ——
        // 上面两步已经收过）
        // —— 音节位上出现大写就说明打错了，这一支解不出东西（`RFvf` 这种混用不合法）
        if self.auxiliary && first.is_ascii_uppercase() {
            return;
        }
        // 开了辅助码：紧跟音节的大写字母（最多 4 位）属于这个音节，不算分词
        let auxiliary = self.auxiliary_run(keys, index + 2);
        let consumed = 2 + auxiliary.len();
        for pinyin in self.syllable(first, second) {
            let mut next = done.clone();
            next.push(Syllable {
                pinyin,
                auxiliary: auxiliary.clone(),
            });
            self.walk(keys, index + consumed, next, out);
        }
    }

    /// 从 `from` 开始那串辅助码（大写字母，最多 [`MAX_AUXILIARY`] 位）；没开辅助码就是空的。
    fn auxiliary_run(&self, keys: &[char], from: usize) -> String {
        let mut run = String::new();
        if self.auxiliary {
            let mut look = from;
            while look < keys.len()
                && run.chars().count() < MAX_AUXILIARY
                && keys[look].is_ascii_uppercase()
            {
                run.push(keys[look]);
                look += 1;
            }
        }
        run
    }

    /// 两键能拼出哪些音节（可能多个：同键多义，以及「声母 + 韵母」与「零声母」两条路）。
    fn syllable(&self, first: char, second: char) -> Vec<String> {
        let first = first.to_ascii_lowercase();
        let second = second.to_ascii_lowercase();
        let mut out = Vec::new();
        if let Some(initial) = self.initial(first)
            && let Some(finals) = self.finals(second)
        {
            for final_ in finals {
                out.push(spelled(&initial, final_));
            }
        }
        if let Some(zero) = self.zero_initials.get(&(first, second)) {
            out.push(zero.clone());
        }
        // 只留**合法音节**：`sh` + `iang`（`l` 键两义）这种组合不是拼音，丢掉才不会挡住同键另一义的 `shuang`。
        // 上游 `Scheme::syllable` 就是拿 `parser::is_syllable` 这么筛的。
        out.retain(|pinyin| crate::parser::is_syllable(pinyin));
        out
    }

    /// 这个键当声母时是什么：键位表写了声母就用它，否则是单字母声母时用字母本身。
    fn initial(&self, key: char) -> Option<String> {
        if let Some(initial) = self
            .remapping
            .get(&key)
            .and_then(|mapping| mapping.initials.first())
        {
            return Some(initial.clone());
        }
        SINGLE_INITIALS.contains(key).then(|| key.to_string())
    }

    /// 这个键的韵母（没写韵母就是没有）。
    fn finals(&self, key: char) -> Option<&[String]> {
        let mapping = self.remapping.get(&key)?;
        (!mapping.finals.is_empty()).then_some(mapping.finals.as_slice())
    }
}

/// 声母 + 韵母 → 音节，按普通话音节表的**标准写法**归一化：`j` / `q` / `x` / `y` 后面的 `v`（ü）一律写 `u`。
///
/// 方案里把 ü 那一类韵母写成 `ve` / `v` 时（首道 `b = "ve"`），`j` + `b` 拼出来的是 `jve` ——
/// 那是同一个音（觉 = jüe），但音节表与词库里只有标准写法 `jue`，不归一化就会被「只留合法音节」筛掉，
/// 表现为「这个键按下去什么也没有」。`l` / `n` 后面的 `v` 不用动（`lve` / `nve` 本身就是标准写法）。
fn spelled(initial: &str, final_: &str) -> String {
    match initial {
        "j" | "q" | "x" | "y" => format!("{initial}{}", final_.replace('v', "u")),
        _ => format!("{initial}{final_}"),
    }
}

/// 辅助码表：Core 不读文件，表由 platform 读出来后注入（与重打分那套「注入 trait」一个路子）。
pub trait AuxiliaryTable {
    /// 这个词的码（都按大写）；没有这个词就是 `None`。
    fn codes(&self, word: &str) -> Option<&[String]>;

    /// 这个字有没有**以敲的这几位开头**的码（大小写不敏感；一个词多个码时任一个以它开头就算有）。
    ///
    /// 是**前缀**不是全等：`侧` 的码是 `RS`，只敲了 `R` 也算命中（那一位还没敲完，候选照旧在里面，
    /// 候选右侧那段码会把已经敲上的 `R` 画下划线）。
    fn matches(&self, word: &str, typed: &str) -> bool {
        self.codes(word)
            .is_some_and(|codes| codes.iter().any(|code| is_prefix(typed, code)))
    }
}

/// 敲的这几位是不是表里那条码的**前缀**（大小写不敏感）。空串不算（那是「没敲码」）。
fn is_prefix(typed: &str, code: &str) -> bool {
    !typed.is_empty()
        && code
            .get(..typed.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(typed))
}

/// 这个候选能不能过辅助码这关：**逐字对应**，敲的码是表里那条码的**前缀**就算命中。
///
/// `words` 是候选的词：词文本 + **这个词每个字敲的辅助码**（空串 = 那个字没敲码；长度 = 词的字数）。
/// 判定顺序：**先找词**（把敲的码按字连起来，是词表里那条整码的前缀就算命中）；**对不上再逐字查表**
/// （敲了码的那个字，它的码得以敲的码开头；没敲码的字不参与）。一个字都没敲码的词不参与筛选；
/// 敲了码、对不上 → 这个候选丢掉（辅助码**只筛选**，不排序）。
pub fn keeps(words: &[(String, Vec<String>)], table: &dyn AuxiliaryTable) -> bool {
    words
        .iter()
        .all(|(text, typed)| keeps_word(text, typed, table))
}

fn keeps_word(text: &str, typed: &[String], table: &dyn AuxiliaryTable) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let count = chars.len();
    if count == 0 {
        return false;
    }
    // 这个词一个字都没敲码：不参与筛选（辅助码只筛「敲了码、但对不上」的那些）
    if typed.iter().all(|code| code.is_empty()) {
        return true;
    }
    // 先找词：词表里的整条码
    let joined: String = typed.concat();
    if let Some(codes) = table.codes(text) {
        for code in codes {
            // 整条码：把敲的码按字连起来，是它的**前缀**就算命中（`PD` 对「爱情」的 `PDXH`，敲一半也算）
            if is_prefix(&joined, code) {
                return true;
            }
            // 或者一格一位切开比（传统单字母码：`K` + `F` 对「开发」的 `KF`）
            let letters: Vec<char> = code.chars().collect();
            if letters.len() == count
                && (0..count).all(|index| {
                    typed.get(index).is_none_or(|typed| {
                        typed.is_empty() || is_prefix(typed, &letters[index].to_string())
                    })
                })
            {
                return true;
            }
        }
    }
    // 再逐字查表：敲了码的那个字，它的码得以敲的码开头；没敲码的字不参与
    (0..count).all(|index| {
        typed
            .get(index)
            .is_none_or(|code| code.is_empty() || table.matches(&chars[index].to_string(), code))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自然码（与内置样例同一张表）。
    fn natural() -> Table {
        let mut remapping: BTreeMap<char, KeyMapping> = BTreeMap::new();
        let mut put = |key: &str, initials: &str, finals: &str| {
            let character = key.chars().next().unwrap();
            remapping.insert(
                character,
                KeyMapping {
                    initials: initials
                        .split(',')
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    finals: finals
                        .split(',')
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned)
                        .collect(),
                },
            );
        };
        put("v", "zh", "ui,v");
        put("i", "ch", "i");
        put("u", "sh", "u");
        put("a", "", "a");
        put("b", "", "ou");
        put("c", "", "iao");
        put("d", "", "iang,uang");
        put("e", "", "e");
        put("f", "", "en");
        put("g", "", "eng");
        put("h", "", "ang");
        put("j", "", "an");
        put("k", "", "ao");
        put("l", "", "ai");
        put("m", "", "ian");
        put("n", "", "in");
        put("o", "", "uo,o");
        put("p", "", "un");
        put("q", "", "iu");
        put("r", "", "uan");
        put("s", "", "iong,ong");
        put("t", "", "ve,ue");
        put("w", "", "ia,ua");
        put("x", "", "ie");
        put("y", "", "uai,ing");
        put("z", "", "ei");
        // 自然码的零声母（与内置样例的 [Shengmu] 一致）
        let zero_initials = [
            ("aa", "a"),
            ("oo", "o"),
            ("ee", "e"),
            ("ai", "ai"),
            ("ei", "ei"),
            ("ao", "ao"),
            ("ou", "ou"),
            ("an", "an"),
            ("en", "en"),
            ("ah", "ang"),
            ("eg", "eng"),
            ("er", "er"),
        ]
        .into_iter()
        .map(|(keys, final_)| {
            let mut chars = keys.chars();
            (
                (chars.next().unwrap(), chars.next().unwrap()),
                final_.to_owned(),
            )
        })
        .collect();
        Table {
            zero_initials,
            remapping,
            exchange: Vec::new(),
            auxiliary: false,
        }
    }

    /// 两键一个音节，拼成完整拼音（`ypdo` → `yun'duo`）。
    #[test]
    fn two_keys_per_syllable() {
        let expansions = natural().expand("ypdo");
        let pinyin: Vec<String> = expansions.iter().map(Expansion::to_pinyin).collect();
        assert!(pinyin.contains(&"yun'duo".to_owned()), "{pinyin:?}");
        // `o` 键两义（uo / o），但 `do` 不是合法音节 → 不会被翻出来
        assert!(!pinyin.contains(&"yun'do".to_owned()), "{pinyin:?}");
        assert!(
            expansions
                .iter()
                .all(|expansion| expansion.syllables.len() == 2)
        );
        // 声母是翘舌时按表里的来：`vi` → zhi
        let pinyin: Vec<String> = natural()
            .expand("vi")
            .iter()
            .map(Expansion::to_pinyin)
            .collect();
        assert!(pinyin.contains(&"zhi".to_owned()), "{pinyin:?}");
    }

    /// 解法直接变成切分：音节按方案定死（之后不再交给全键那套切）；尾巴上那半个音节按它
    /// **可能接出**的音节一族一份切分。
    #[test]
    fn segmentations_follow_the_scheme() {
        let join = |table: &Table, keys: &str| -> Vec<String> {
            table
                .expand(keys)
                .into_iter()
                .next()
                .map(|expansion| {
                    table
                        .segmentations(&expansion)
                        .iter()
                        .map(|segmentation| segmentation.joined("'"))
                        .collect()
                })
                .unwrap_or_default()
        };

        let table = natural();
        // `ypdo` → `yun'duo`，没有尾巴
        let readings = join(&table, "ypdo");
        assert_eq!(readings, vec!["yun'duo".to_owned()], "{readings:?}");

        // 尾巴上那半个音节（`ypd` 的 `d`）：声母那一族收成一条前缀（`d` 开头那些音节）
        let readings = join(&table, "ypd");
        assert_eq!(readings, vec!["yun'd".to_owned()], "{readings:?}");
        let expansion = table.expand("ypd").into_iter().next().unwrap();
        let readings = table.segmentations(&expansion);
        assert!(!readings[0].syllables[1].complete, "半个音节按前缀算");

        // 只有零声母那几组的键（`o` → `o` / `ou`）：一个音节一份，不收成公共前缀 `o…`
        let readings = join(&table, "o");
        assert_eq!(
            readings,
            vec!["o".to_owned(), "ou".to_owned()],
            "{readings:?}"
        );

        // 两族都收：`e` 既接 `sh…`（一条前缀），又接零声母的 `e` / `ei` / `eng`（各一份）
        let mut table = natural();
        table.remapping.insert(
            'e',
            KeyMapping {
                initials: vec!["sh".to_owned()],
                finals: vec!["e".to_owned()],
            },
        );
        let readings = join(&table, "e");
        assert_eq!(
            readings,
            vec![
                "sh".to_owned(),
                "e".to_owned(),
                "eng".to_owned(),
                "ei".to_owned(),
                "en".to_owned(),
                "er".to_owned()
            ],
            "{readings:?}"
        );

        // 这个键接不出任何音节（清掉零声母表后的 `a`）：尾巴不参与，只按完整的音节读
        let mut table = natural();
        table.zero_initials.clear();
        assert_eq!(join(&table, "ypa"), vec!["yun".to_owned()]);
        // 连一个完整音节都没有：这个解法没有读法
        assert!(join(&table, "a").is_empty(), "{:?}", join(&table, "a"));
    }

    /// `j` / `q` / `x` / `y` 后面的 `v`（ü）按标准写法归一成 `u`：方案里的 `ve` 键（首道 `b = "ve"`）
    /// 按下去也能出「觉」（`jve` → `jue`），不然它会被「只留合法音节」筛掉、什么也没有。
    #[test]
    fn u_umlaut_is_written_as_u_after_jqxy() {
        let mut table = natural();
        table.remapping.insert(
            'b',
            KeyMapping {
                initials: Vec::new(),
                finals: vec!["ve".to_owned()],
            },
        );
        let pinyin = |keys: &str| -> Vec<String> {
            table
                .expand(keys)
                .iter()
                .map(Expansion::to_pinyin)
                .collect()
        };
        // `lb` = l + ve → lve（本身就是标准写法，不动）
        assert!(
            pinyin("lb").contains(&"lve".to_owned()),
            "{:?}",
            pinyin("lb")
        );
        // `jb` = j + ve → jue（归一化）
        assert!(
            pinyin("jb").contains(&"jue".to_owned()),
            "{:?}",
            pinyin("jb")
        );
        assert!(
            pinyin("qb").contains(&"que".to_owned()),
            "{:?}",
            pinyin("qb")
        );
        assert!(
            pinyin("xb").contains(&"xue".to_owned()),
            "{:?}",
            pinyin("xb")
        );
        assert!(
            pinyin("yb").contains(&"yue".to_owned()),
            "{:?}",
            pinyin("yb")
        );
    }

    /// 同键多义里**不合法的组合直接丢掉**（`sh` + `iang` 不是拼音），免得挡住同键另一条义（`shuang`）。
    #[test]
    fn illegal_combinations_are_dropped() {
        let mut table = natural();
        table.remapping.insert(
            'l',
            KeyMapping {
                initials: Vec::new(),
                finals: vec!["iang".to_owned(), "uang".to_owned()],
            },
        );
        let pinyin: Vec<String> = table
            .expand("ul")
            .iter()
            .map(Expansion::to_pinyin)
            .collect();
        assert!(pinyin.contains(&"shuang".to_owned()), "{pinyin:?}");
        assert!(!pinyin.contains(&"shiang".to_owned()), "{pinyin:?}");
    }

    /// 同键多义两端都试：`lt` → lve / lue。
    #[test]
    fn ambiguous_finals_give_both() {
        let pinyin: Vec<String> = natural()
            .expand("lt")
            .iter()
            .map(Expansion::to_pinyin)
            .collect();
        assert!(pinyin.contains(&"lve".to_owned()), "{pinyin:?}");
        assert!(pinyin.contains(&"lue".to_owned()), "{pinyin:?}");
    }

    /// 零声母完全按 `[Shengmu]` 的表来：写了哪两键就是哪个音节，没写的解不出来。
    #[test]
    fn zero_initials_come_from_the_table() {
        let pinyin = |keys: &str| -> Vec<String> {
            natural()
                .expand(keys)
                .iter()
                .map(Expansion::to_pinyin)
                .collect()
        };
        // 特例：`aa` 是 a、`oo` 是 o、`ee` 是 e
        assert!(pinyin("aa").contains(&"a".to_owned()), "{:?}", pinyin("aa"));
        assert!(pinyin("oo").contains(&"o".to_owned()), "{:?}", pinyin("oo"));
        // 双韵母照原样：爱 = `ai`；昂 = `ah`（ang 的键是 h）
        assert!(
            pinyin("ai").contains(&"ai".to_owned()),
            "{:?}",
            pinyin("ai")
        );
        assert!(
            pinyin("ah").contains(&"ang".to_owned()),
            "{:?}",
            pinyin("ah")
        );
        assert!(
            pinyin("er").contains(&"er".to_owned()),
            "{:?}",
            pinyin("er")
        );
        // 表里没写就解不出来
        let mut table = natural();
        table.zero_initials.clear();
        assert!(table.expand("aa").is_empty(), "没写零声母就解不出来");
    }

    /// 开了辅助码：**小写才是拼音键，大写是辅助码**。
    ///
    /// 大写只能跟在**完整音节**后面（当那个音节的辅助码），或者跟在**末尾那半个音节**后面
    /// （第二键跳过、直接敲辅助码，`vI` 这种）；别的位置出现大写、以及落单的大写，这一支
    /// 什么都解不出来（`RFvf` 这种大小写混用不合法）。**没开辅助码时大写仍当拼音**
    /// （Shift 打字照旧，大小写不敏感）。
    #[test]
    fn uppercase_is_only_an_auxiliary_code() {
        let mut table = natural();
        table.auxiliary = true;
        assert!(table.expand("Vi").is_empty(), "第一个键大写 → 解不出来");
        assert!(table.expand("uCu").is_empty(), "中间出现大写 → 解不出来");
        assert!(table.expand("V").is_empty(), "落单的大写没有音节可挂");
        // 第二键位置是大写、而且一直吃到末尾：那是「半音节 + 辅助码」（`vI` = 只按了 `v` 就敲 `I`）
        let expansions = table.expand("vI");
        assert_eq!(expansions.len(), 1, "{expansions:?}");
        assert_eq!(expansions[0].trailing, "v");
        assert_eq!(expansions[0].trailing_auxiliary, "I");

        // 跟在完整音节后面才是辅助码
        let expansions = table.expand("uuC");
        assert_eq!(expansions.len(), 1, "{expansions:?}");
        assert_eq!(expansions[0].syllables[0].pinyin, "shu");
        assert_eq!(expansions[0].syllables[0].auxiliary, "C");

        // 没开辅助码时大写仍当拼音
        let pinyin: Vec<String> = natural()
            .expand("Vi")
            .iter()
            .map(Expansion::to_pinyin)
            .collect();
        assert!(pinyin.contains(&"zhi".to_owned()), "{pinyin:?}");
    }

    /// `'` 强制分词，而且不上屏（`'` 只用于分词）。
    #[test]
    fn apostrophe_forces_a_boundary_and_is_not_typed() {
        let expansions = natural().expand("yp'do");
        let pinyin: Vec<String> = expansions.iter().map(Expansion::to_pinyin).collect();
        assert!(pinyin.contains(&"yun'duo".to_owned()), "{pinyin:?}");
        assert!(
            expansions
                .iter()
                .all(|expansion| !expansion.to_pinyin().contains("''")),
            "连着两个 `'` 不该出现"
        );
    }

    /// 开了辅助码：音节后跟的大写字母是辅助码（不分词），小写字母就是下一个音节。
    #[test]
    fn uppercase_after_a_syllable_is_an_auxiliary_code() {
        let mut table = natural();
        table.auxiliary = true;
        // uuC：不分词，C 是 `shu`（sh + u）的辅助码
        let expansions = table.expand("uuC");
        assert_eq!(expansions.len(), 1, "{expansions:?}");
        assert_eq!(expansions[0].syllables.len(), 1);
        assert_eq!(expansions[0].syllables[0].pinyin, "shu");
        assert_eq!(expansions[0].syllables[0].auxiliary, "C");
        // uu'c：强制分词 → `uu` 一个音节，`c` 是还在打的半个音节
        let expansions = table.expand("uu'c");
        assert_eq!(expansions[0].syllables.len(), 1, "{expansions:?}");
        assert_eq!(expansions[0].trailing, "c");
        assert!(
            expansions[0]
                .syllables
                .iter()
                .all(|syllable| syllable.auxiliary.is_empty())
        );
        // 没开辅助码时大写不算辅助码（按同一个键解）
        let expansions = natural().expand("uuC");
        assert!(
            expansions[0]
                .syllables
                .iter()
                .all(|syllable| syllable.auxiliary.is_empty())
        );
        // 辅助码最多 4 位：`uuABCDuu` 里只有前 4 个大写算辅助码，后面 `uu` 是下一个音节
        let expansions = table.expand("uuABCDuu");
        assert_eq!(expansions[0].syllables.len(), 2, "{expansions:?}");
        assert_eq!(expansions[0].syllables[0].auxiliary, "ABCD");
        assert_eq!(expansions[0].syllables[1].auxiliary, "");
    }

    /// Exchange 作用在物理键上，且发生在查表之前。
    #[test]
    fn exchange_swaps_the_physical_keys() {
        let mut table = natural();
        table.exchange = vec![('y', 'z')];
        // 打 `zpdo`：物理 z 被换成 y → yp = yun；do 照旧
        let pinyin: Vec<String> = table
            .expand("zpdo")
            .iter()
            .map(Expansion::to_pinyin)
            .collect();
        assert!(pinyin.contains(&"yun'duo".to_owned()), "{pinyin:?}");
    }

    /// 解不出来就返回空；只剩一个键是「还在打的半个音节」，原样带着。
    #[test]
    fn unparsable_input_is_left_alone() {
        let expansions = natural().expand("y");
        assert_eq!(expansions.len(), 1);
        assert!(expansions[0].syllables.is_empty());
        assert_eq!(expansions[0].trailing, "y");
        assert!(natural().expand("y9").is_empty(), "9 不是键");
        assert!(natural().expand("|").is_empty());
    }

    /// 候选封顶。
    #[test]
    fn alternatives_are_capped() {
        // 每个 `o`（uo/o）都两义，串长了会爆；封顶 9
        let expansions = natural().expand("oooooooo");
        assert!(expansions.len() <= MAX_ALTERNATIVES, "{}", expansions.len());
    }
}

/// 辅助码筛选的判定（`keeps`）单独一套：表用一个小实现顶替。
#[cfg(test)]
mod auxiliary_tests {
    use super::*;
    use std::collections::BTreeMap;

    /// 只够测试用的辅助码表。
    struct Table(BTreeMap<String, Vec<String>>);

    impl Table {
        fn new(entries: &[(&str, &str)]) -> Self {
            let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (word, code) in entries {
                map.entry((*word).to_owned())
                    .or_default()
                    .push(code.to_uppercase());
            }
            Self(map)
        }
    }

    impl AuxiliaryTable for Table {
        fn codes(&self, word: &str) -> Option<&[String]> {
            self.0.get(word).map(Vec::as_slice)
        }
    }

    /// 一个候选词：词文本 + 这个词每个字敲的辅助码（空串 = 没敲）。
    fn word(text: &str, typed: &[&str]) -> (String, Vec<String>) {
        (
            text.to_owned(),
            typed.iter().map(|code| (*code).to_owned()).collect(),
        )
    }

    /// **逐字对应**：敲了码的每个字都要对上，一个对不上就丢；没敲码的字不参与。
    #[test]
    fn every_typed_code_must_match() {
        let table = Table::new(&[("爱", "PD"), ("情", "XH")]);
        assert!(
            keeps(&[word("爱情", &["PD", "XH"])], &table),
            "两个字都对得上"
        );
        assert!(
            keeps(&[word("爱情", &["PD", ""])], &table),
            "只敲了第一个字，第二个字不参与"
        );
        assert!(
            !keeps(&[word("爱情", &["PD", "QH"])], &table),
            "第二个字对不上 → 丢"
        );
        assert!(
            !keeps(&[word("爱情", &["XH", "XH"])], &table),
            "第一个字对不上 → 丢"
        );
    }

    /// 字号大小写不敏感（用户敲的是大写键，表里可能写成小写）。
    #[test]
    fn codes_are_case_insensitive() {
        let table = Table::new(&[("爱", "pd")]);
        assert!(keeps(&[word("爱", &["PD"])], &table));
    }

    /// 一个字都没敲码的词不参与筛选（敲了别的词也照旧过）。
    #[test]
    fn words_without_any_typed_code_pass() {
        let table = Table::new(&[("爱情", "PDXH")]);
        assert!(keeps(
            &[word("爱情", &["", ""]), word("别的", &["", ""])],
            &table
        ));
        // 敲了码的词照旧按码判：对得上就过
        assert!(keeps(&[word("爱情", &["PD", "XH"])], &table));
    }

    /// 词表里的整条码：敲的码按字连起来是它的**前缀**就过（表里没有这个词就逐字查）。
    #[test]
    fn the_whole_word_code_is_checked_first() {
        let table = Table::new(&[("爱情", "PDXH")]);
        assert!(keeps(&[word("爱情", &["PD", "XH"])], &table), "整条码对上");
        // **敲了一半也算**：`PD` 是 `PDXH` 的前缀，那一位还没敲完，候选照旧在
        assert!(keeps(&[word("爱情", &["PD", ""])], &table), "前半个字的码");
        assert!(keeps(&[word("爱情", &["P", ""])], &table), "只敲了一位");
        // 连起来对不上 → 落到逐字查表；字表里没有这两个字 → 丢
        assert!(!keeps(&[word("爱情", &["PD", "QQ"])], &table));
        assert!(!keeps(&[word("爱情", &["PX", ""])], &table), "敲错的码不认");
    }

    /// 词表里没这个词（或者只写了单字）就逐字查表。
    #[test]
    fn falls_back_to_single_char_entries() {
        let table = Table::new(&[("词", "C"), ("典", "D")]);
        assert!(keeps(&[word("词典", &["C", ""])], &table), "第一个字对上");
        assert!(keeps(&[word("词典", &["", "D"])], &table), "第二个字对上");
        assert!(
            !keeps(&[word("词典", &["D", ""])], &table),
            "字表里对不上就丢"
        );
        assert!(!keeps(&[word("别的", &["C", ""])], &table), "表里没这个字");
    }

    /// 整句（多词）要逐词都过。
    #[test]
    fn every_word_must_pass() {
        let table = Table::new(&[("词", "C"), ("典", "D")]);
        assert!(keeps(&[word("词", &["C"]), word("典", &["D"])], &table));
        assert!(!keeps(&[word("词", &["C"]), word("典", &["X"])], &table));
    }
}
