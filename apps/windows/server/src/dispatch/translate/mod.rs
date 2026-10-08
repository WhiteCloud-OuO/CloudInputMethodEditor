//! 本地词典的翻译 Tip：打开词典、查高亮候选的释义、记住学没学会。
//!
//! 词典在安装目录的 `LocalDictionary\`（清单 `dictionaries.list` 决定设置页里能选哪些），
//! 学习状态在用户数据目录的 `translate.db`——词典文件只读，学没学会得另存一处。
//! 换词典与「重置学习内容」都由 [`Translate::configure`] 跟着配置热加载走。
//!
//! 另外还有一份**在线**翻译（[`online`]，候选窗按 `Ctrl+T`）：那一条与本地词典无关，
//! 结果画在候选窗底部本地 Tip 的下面单独一行。

pub(crate) mod online;

use std::path::PathBuf;

use cloudime_translate::{Glossary, Learning, Manifest, Sense, Tip};

use super::Router;
use crate::speech;

/// 本地词典与它那份学习状态。
pub(crate) struct Translate {
    /// 安装目录 `LocalDictionary\` 的清单。
    manifest: Manifest,

    /// 打开着的词典；没选 / 打不开为 `None`。
    glossary: Option<Glossary>,

    /// `glossary` 是哪个文件（配置里选的那个）。
    dictionary: String,

    /// 学习状态；打不开为 `None`（那时 Tip 一律按「没学会」上色）。
    learning: Option<Learning>,

    /// 已应用的 `[translate] reset_counter`：配置里变了就把这份词典的学习记录清空。
    reset_counter: u64,

    /// 多释义选择（Ctrl + 反引号 / 鼠标右键）：`Some` 时自绘帧整屏换成「词条 + 各条释义」。
    choices: Option<Choices>,
}

/// 正在做的多释义选择。
pub(crate) struct Choices {
    /// 候选在布局里的下标：上屏时要按它消耗拼音。
    pub(crate) index: usize,

    /// 被翻译的词条（当标题画在顶部那一行）。
    pub(crate) word: String,

    /// 各条释义。
    pub(crate) senses: Vec<Sense>,

    /// 高亮在第几条释义上（从 0 起）：鼠标悬停挪它，圆角矩形跟着滑。
    pub(crate) highlight: usize,
}

/// 一次「翻译 Tip 动作」（`Ctrl + 反引号`，以及开着 Tip 时的鼠标右键）的结果。
pub(crate) enum TranslateAction {
    /// 上屏：`Some(text)` 是要落进文档的文本，`None` 表示只并进组句（还有没选完的拼音）。
    Commit(Option<String>),

    /// 有多条释义：已经进了选择界面（自绘帧跟着变了）。
    Choosing,

    /// 这个词条在这份词典里没有译文（或这一格没有候选）。
    Nothing,
}

/// 在线翻译那一行当成「一条释义」时的词性标记：释义列表里能一眼看出它来自网上（渲染成 `(在线)`），
/// 也用来判断「这一次上屏的是在线译文」—— 那种**不**记本地词典的「学会」次数（那是本地释义的次数）。
pub(super) const ONLINE_POS: &str = "在线";

/// 把「在线翻译那一行」并进释义列表的**最前面**（`None` 就原样返回）。
///
/// 与本地词典的释义并列、排第一：用户按 `Ctrl + 反引号` 时，最想要的通常就是刚翻出来的那条。
fn senses_with_online(senses: Vec<Sense>, online: Option<String>) -> Vec<Sense> {
    let Some(text) = online else {
        return senses;
    };
    let mut merged = Vec::with_capacity(senses.len() + 1);
    merged.push(Sense {
        pos: Some(ONLINE_POS.to_owned()),
        text,
        reading: None,
    });
    merged.extend(senses);
    merged
}

/// 「本地释义 + 在线那一行」的合并顺序（纯函数，不需要真实词典）。
impl Router {
    /// 对第 `index` 个候选做「翻译 Tip 动作」：**本地词典的释义 + 在线翻译那一行**（脚本给这一格翻的、
    /// 且已经翻好的那条）一起列出来，在线那条排最前面 —— 一条就直接上屏译文，多条进选择界面，
    /// 都没有就什么都不做。上屏文本由调用方处理——按键直接回给 DLL，鼠标得攒进 `pending_commit`
    /// 等下一拍 `Poll` 带走（候选窗在 Server 手里，收不到按键）。
    pub(crate) fn translate_action(&mut self, index: usize) -> TranslateAction {
        let Some(word) = self.layout_candidate(index).map(|candidate| candidate.text) else {
            return TranslateAction::Nothing;
        };
        let online = self.online.translation_for(&word);
        let senses = senses_with_online(self.translate.senses(&word).unwrap_or_default(), online);
        if senses.is_empty() {
            return TranslateAction::Nothing;
        }
        if let [sense] = senses.as_slice() {
            let text = sense.text.clone();
            let from_online = sense.pos.as_deref() == Some(ONLINE_POS);
            // `None` 是「只并进组句」：整段还没选完，等后面选完 / 回车再一起交给应用
            return TranslateAction::Commit(self.commit_sense(index, &word, &text, !from_online));
        }
        self.translate.begin_choices(index, word, senses);
        TranslateAction::Choosing
    }

    /// 在多释义选择里选中第 `index` 条释义（数字键与鼠标单击共用）：退出选择界面，上屏这条译文。
    /// 返回要落进文档的文本（`None` = 只并进组句，还没整段上屏完）；下标越界返回 `None` 且不动状态。
    pub(crate) fn choose_sense(&mut self, index: usize) -> Option<String> {
        let choices = self.translate.choices()?;
        let sense = choices.senses.get(index)?;
        let (candidate, word, text) = (choices.index, choices.word.clone(), sense.text.clone());
        let from_online = sense.pos.as_deref() == Some(ONLINE_POS);
        self.translate.end_choices();
        self.commit_sense(candidate, &word, &text, !from_online)
    }

    /// 上屏第 `index` 个候选的第 `text` 条译文：拼音消耗按这个候选走（`Engine::commit_translation`），
    /// 落进文档的是译文；`record` 为真（译文来自**本地词典**）时顺带给这个词条记一次「译文上屏」。
    pub(crate) fn commit_sense(
        &mut self,
        index: usize,
        word: &str,
        text: &str,
        record: bool,
    ) -> Option<String> {
        let candidate = self.layout_candidate(index)?;
        let commit = self.engine.commit_translation(&candidate, text);
        if record {
            self.translate
                .record(word, self.config.translate_need_times);
        }
        commit
    }

    /// 发音（`Shift + 反引号`、候选窗鼠标中键）：念高亮候选的译文。只有一条释义才念；
    /// 多条要进选择界面（那一屏里念高亮那条）；没有译文、没开翻译 Tip 就不念。
    pub(crate) fn speak(&mut self) {
        if !self.config.translate_enabled {
            return;
        }
        if let Some(choices) = self.translate.choices() {
            if let Some(sense) = choices.senses.get(choices.highlight) {
                speech::speak(&sense.text);
            }
            return;
        }
        let word = self.highlighted_text();
        let senses = self.translate.senses(&word);
        if let Some(text) = single_sense(senses.as_deref()) {
            speech::speak(text);
        }
    }
}

/// 这一键该念哪条译文：**只有一条**释义才念它（多条要进选择界面一条条念，没有就什么也不念）。
pub(super) fn single_sense(senses: Option<&[Sense]>) -> Option<&str> {
    match senses {
        Some([sense]) => Some(&sense.text),
        _ => None,
    }
}

impl Translate {
    pub(crate) fn new() -> Self {
        Self {
            learning: open_learning(),
            ..Self::empty()
        }
    }

    /// 测试用的一份空白状态：学习库不开（Tip 一律按「没学会」上色，也不会往用户目录的
    /// `translate.db` 里写东西）。真实的 Server 走 [`Translate::new`]。
    pub(crate) fn without_learning() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        Self {
            manifest: Manifest::default(),
            glossary: None,
            dictionary: String::new(),
            learning: None,
            reset_counter: 0,
            choices: None,
        }
    }

    /// 配置（重新）应用：重读清单、选了别的词典就换掉、`reset_counter` 变了就清空学习记录。
    /// 任何配置改动都会走到这里，所以这几步都要便宜（清单是个几行的小文件）。
    pub(crate) fn configure(&mut self, dictionary: &str, reset_counter: u64) {
        self.manifest = Manifest::load(local_dictionary_dir());
        if self.dictionary != dictionary {
            tracing::info!(dictionary, "换本地词典");
            self.dictionary = dictionary.to_owned();
            self.glossary = if dictionary.is_empty() {
                None
            } else {
                Self::open_dictionary(&self.manifest, dictionary)
            };
            self.choices = None;
        }
        if reset_counter != self.reset_counter {
            self.reset_counter = reset_counter;
            let Some(learning) = &mut self.learning else {
                return;
            };
            match learning.reset(&self.dictionary) {
                Ok(removed) => tracing::info!(
                    dictionary = %self.dictionary,
                    removed,
                    "重置翻译学习内容"
                ),
                Err(error) => tracing::warn!(%error, "重置翻译学习内容失败"),
            }
        }
    }

    fn open_dictionary(manifest: &Manifest, dictionary: &str) -> Option<Glossary> {
        let path = manifest.path_of(dictionary);
        match Glossary::open(&path) {
            Ok(glossary) => {
                tracing::info!(path = %path.display(), entries = glossary.len(), "本地词典已加载");
                Some(glossary)
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "本地词典打不开，翻译 Tip 停用");
                None
            }
        }
    }

    /// 一个词条在词典里的释义；没词典 / 查不到为 `None`。
    pub(crate) fn senses(&self, word: &str) -> Option<Vec<Sense>> {
        if word.is_empty() {
            return None;
        }
        self.glossary.as_ref()?.lookup(word)
    }

    /// 一个词条的翻译 Tip（释义 + 学没学会）。
    pub(crate) fn tip(&self, word: &str) -> Option<Tip> {
        let senses = self.senses(word)?;
        Some(Tip {
            word: word.to_owned(),
            learned: self.is_learned(word),
            senses,
        })
    }

    /// 这个词条学会没有（没开学习库一律按没学会）。
    fn is_learned(&self, word: &str) -> bool {
        self.learning
            .as_ref()
            .is_some_and(|learning| learning.entry(&self.dictionary, word).learned)
    }

    /// 译文上屏一次：`input_times` 加 1，到「学会所需上屏次数」就算学会（Tip 换颜色）。
    pub(crate) fn record(&mut self, word: &str, need_times: u32) {
        let Some(learning) = &mut self.learning else {
            return;
        };
        if let Err(error) = learning.record_commit(&self.dictionary, word, need_times) {
            tracing::warn!(%error, "记翻译学习状态失败");
        }
    }

    /// 进入多释义选择（高亮落在第一条上）。
    pub(crate) fn begin_choices(&mut self, index: usize, word: String, senses: Vec<Sense>) {
        self.choices = Some(Choices {
            index,
            word,
            senses,
            highlight: 0,
        });
    }

    /// 正在做的多释义选择（词条 + 各条释义 + 它属于哪个候选）。
    pub(crate) fn choices(&self) -> Option<&Choices> {
        self.choices.as_ref()
    }

    /// 选择界面里把高亮挪到第 `index` 条释义（鼠标悬停）：真挪动了返回 `true`。
    pub(crate) fn set_choice_highlight(&mut self, index: usize) -> bool {
        let Some(choices) = self.choices.as_mut() else {
            return false;
        };
        if index >= choices.senses.len() || choices.highlight == index {
            return false;
        }
        choices.highlight = index;
        true
    }

    /// 选择界面里把高亮挪 `delta` 条（方向键）：夹在 `[0, 末尾]`，真挪动了返回 `true`。
    pub(crate) fn move_choice_highlight(&mut self, delta: isize) -> bool {
        let Some(choices) = self.choices.as_mut() else {
            return false;
        };
        let last = choices.senses.len().saturating_sub(1) as isize;
        let next = (choices.highlight as isize + delta).clamp(0, last) as usize;
        if next == choices.highlight {
            return false;
        }
        choices.highlight = next;
        true
    }

    /// 退出多释义选择。
    pub(crate) fn end_choices(&mut self) {
        self.choices = None;
    }
}

/// 安装目录下的 `LocalDictionary\`；开发时是仓库根的那一份。
fn local_dictionary_dir() -> PathBuf {
    cloudime_platform::resources::bundled_root()
        .map(|root| root.join("LocalDictionary"))
        .unwrap_or_default()
}

/// 学习状态库：用户数据目录的 `translate.db`。
fn open_learning() -> Option<Learning> {
    let path = cloudime_platform::dirs::user_dir()?.join("translate.db");
    match Learning::open(&path) {
        Ok(learning) => Some(learning),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "翻译学习状态库打不开，翻译 Tip 一律按没学会上色");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use cloudime_translate::Sense;

    use super::{ONLINE_POS, senses_with_online};

    fn sense(text: &str) -> Sense {
        Sense {
            pos: None,
            text: text.to_owned(),
            reading: None,
        }
    }

    /// 在线那条排最前面，并且带一个 `(在线)` 的词性标记（列表里能看出它从哪来）。
    #[test]
    fn the_online_translation_goes_first() {
        let merged =
            senses_with_online(vec![sense("sad"), sense("sorrow")], Some("悲伤".to_owned()));
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].text, "悲伤");
        assert_eq!(merged[0].pos.as_deref(), Some(ONLINE_POS));
        assert_eq!(merged[1].text, "sad");
        assert_eq!(merged[2].text, "sorrow");

        // 没有在线译文就原样返回
        let local = vec![sense("sad")];
        assert_eq!(senses_with_online(local.clone(), None), local);
    }
}
