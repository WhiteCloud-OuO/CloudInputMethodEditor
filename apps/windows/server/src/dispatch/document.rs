//! 输入框文本快照：脚本的 `cloudime.text.*` 用的那份、按「显示宽度」切片。
//!
//! 为什么绕这一圈：整篇只有 TSF DLL 读得到（它在应用进程里、手里有编辑上下文），Server 只能等它送
//! （`ClientMessage::Surrounding.document`）。DLL 什么时候送由 `ServerMessage::ModeSync` 的
//! `want_document` 决定 —— 只在**有脚本登记**时才要，没脚本零读取、零开销。
//!
//! 因为这一拍给不出文本，脚本在拿到第一份快照之前调 `cloudime.text.*` 会是 `nil`，这就是返回值要有
//! `nil` 这一态的原因。快照只在**换会话（焦点 / 输入框变了）与转私密**时清掉：DLL 每段组句起始都会送
//! 一份新的（Server 只要还有脚本就一直请它读），所以快照跟着文档走；组句结束不清，下一段组句的第一键
//! 也还有上一份兜底（DLL 的读发生在组句起始的编辑会话里，晚于那一键）。

use std::cell::RefCell;
use std::rc::Rc;

use cloudime_platform::protocol::DocumentText;
use cloudime_script::TextRange;

/// 一份文本快照的句柄（`Rc` 里包着，给脚本那边的接口闭包共用同一个）。
#[derive(Clone, Default)]
pub(crate) struct Document {
    /// 最近一次 DLL 送来的快照；`None` = 还没拿到（或私密 / 读不到）。
    text: Rc<RefCell<Option<DocumentText>>>,
}

impl Document {
    /// 存一份 DLL 送来的快照：**只认 `Some`**。`None` 表示「这段组句没请 DLL 读 / 读不到 / 私密」，
    /// 不能拿它把上一次读到的那份抹掉 —— DLL 每段组句都送一条 `Surrounding`，没请它读的那段自然
    /// 带 `None`，若照单覆盖，快照就会在「读到」与「抹掉」之间来回抖，脚本时灵时不灵。
    pub(crate) fn store(&self, document: Option<DocumentText>) {
        if let Some(document) = document {
            *self.text.borrow_mut() = Some(document);
        }
    }

    /// 清掉（换会话 / 转私密）：下一段组句重读。
    pub(crate) fn clear(&self) {
        *self.text.borrow_mut() = None;
    }

    /// 脚本的 `cloudime.text.all` / `before` / `after`：按**显示宽度**上限切（中文 / 全角算 2）。
    /// 还没有快照就返回 `None`（`want_document` 那一拍会请 DLL 下一段组句带上）。
    pub(crate) fn slice(&self, range: TextRange, limit: u64) -> Option<(String, bool)> {
        let document = self.text.borrow();
        let document = document.as_ref()?;
        Some(slice_text(&document.text, document.caret, range, limit))
    }
}

/// 按**显示宽度**上限切一段外部读来的整篇文本（`caret` 是光标在里面的字符下标，未知就传字数）。
/// 三个入口与 [`Document::slice`] 同一套：超上限时 `before` 丢头、`after` 丢尾、`all` 留光标附近。
pub(crate) fn slice_text(text: &str, caret: usize, range: TextRange, limit: u64) -> (String, bool) {
    let before = before_caret(text, caret);
    let after = after_caret(text, caret);
    match range {
        // 整篇：超上限就留**光标附近**那一段（前一半 + 后一半），别把光标甩在外面
        TextRange::All => keep_around(&before, &after, limit),
        // 光标前：超上限从**头部**丢（留下的靠光标最近）
        TextRange::Before => take_last(&before, limit),
        // 光标后：超上限从**尾部**丢（同样留下靠光标最近的）
        TextRange::After => take_first(&after, limit),
    }
}

/// 光标之前那一段（`caret` 是字符下标，超界就取到头）。
fn before_caret(text: &str, caret: usize) -> String {
    text.chars().take(caret).collect()
}

/// 光标之后那一段（从光标那个字符起）。
fn after_caret(text: &str, caret: usize) -> String {
    text.chars().skip(caret).collect()
}

/// 一段文本的显示宽度。
fn weighted_len(text: &str) -> u64 {
    text.chars().map(weight).sum()
}

/// 整篇超上限时留「光标附近」：前一半尽量留、后面补足（哪边不够就多留另一边）。
fn keep_around(before: &str, after: &str, limit: u64) -> (String, bool) {
    if weighted_len(before) + weighted_len(after) <= limit {
        return (format!("{before}{after}"), false);
    }
    let (head, _) = take_last(before, limit.div_ceil(2));
    let room = limit.saturating_sub(weighted_len(&head));
    let (tail, _) = take_first(after, room);
    (format!("{head}{tail}"), true)
}

/// 从尾部留：把开头丢掉，直到显示宽度不超过 `limit`。返回 `(留下的文本, 有没有丢过)`。
fn take_last(text: &str, limit: u64) -> (String, bool) {
    let mut total = 0u64;
    for (index, c) in text.char_indices().rev() {
        total += weight(c);
        if total > limit {
            return (text[index + c.len_utf8()..].to_owned(), true);
        }
    }
    (text.to_owned(), false)
}

/// 从头部留：把尾巴丢掉，直到显示宽度不超过 `limit`。返回 `(留下的文本, 有没有丢过)`。
fn take_first(text: &str, limit: u64) -> (String, bool) {
    let mut total = 0u64;
    for (index, c) in text.char_indices() {
        total += weight(c);
        if total > limit {
            return (text[..index].to_owned(), true);
        }
    }
    (text.to_owned(), false)
}

/// 一个字符的显示宽度：西文 / 半角 **1**、中文 / 全角 **2**（近似 East Asian Wide）。
pub(crate) fn weight(c: char) -> u64 {
    let wide = matches!(c as u32,
        0x1100..=0x115F      // 韩文字母
        | 0x2460..=0x24FF    // ① ⑴ 这类带圈字符（候选项序号就用它们）
        | 0x2700..=0x27BF    // ❶ ✔ 这类符号
        | 0x2E80..=0xA4CF    // CJK 部首、假名、注音、汉字……
        | 0xAC00..=0xD7A3    // 韩文音节
        | 0xF900..=0xFAFF    // CJK 兼容汉字
        | 0xFE30..=0xFE6F    // CJK 兼容形式
        | 0xFF00..=0xFF60    // 全角 ASCII
        | 0xFFE0..=0xFFE6    // 全角符号
        | 0x1F300..=0x1FAFF  // emoji（当全角）
        | 0x20000..=0x3FFFD  // 扩展汉字
    );
    if wide { 2 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_count_chinese_as_two() {
        assert_eq!(weight('a'), 1);
        assert_eq!(weight(' '), 1);
        assert_eq!(weight('中'), 2);
        assert_eq!(weight('，'), 2);
        assert_eq!(weight('①'), 2);
    }

    /// 超上限时从**开头**丢，留下光标附近那一段（`abcdef中文` 丢到只剩尾部）。
    #[test]
    fn take_last_keeps_the_tail() {
        assert_eq!(take_last("abc", 10), ("abc".to_owned(), false));
        // 上限 6：`e`(1) + `f`(1) + `中`(2) + `文`(2) = 6
        assert_eq!(take_last("abcdef中文", 6), ("ef中文".to_owned(), true));
        // 上限 5：`f`(1) + `中`(2) + `文`(2) = 5
        assert_eq!(take_last("abcdef中文", 5), ("f中文".to_owned(), true));
        assert_eq!(take_last("", 3), (String::new(), false));
    }

    /// `after` 反过来：超上限从**尾部**丢（同样留下靠光标最近的）。
    #[test]
    fn take_first_keeps_the_head() {
        assert_eq!(take_first("abc", 10), ("abc".to_owned(), false));
        assert_eq!(take_first("abc中文", 4), ("abc".to_owned(), true));
        assert_eq!(take_first("中文abc", 4), ("中文".to_owned(), true));
        assert_eq!(take_first("", 3), (String::new(), false));
    }

    /// 整篇超上限：前一半 + 后一半，光标留在中间那一带。
    #[test]
    fn all_with_a_limit_keeps_the_text_around_the_caret() {
        assert_eq!(keep_around("abc", "def", 100), ("abcdef".to_owned(), false));
        // 上限 4：前面留 2（`bc`），后面补 2（`de`）
        assert_eq!(keep_around("abc", "def", 4), ("bcde".to_owned(), true));
        // 前面不够就从后面多补
        assert_eq!(keep_around("a", "defg", 4), ("adef".to_owned(), true));
    }

    /// `before` 只切光标之前那一段。
    #[test]
    fn before_stops_at_the_caret() {
        assert_eq!(before_caret("abcd", 2), "ab");
        assert_eq!(before_caret("abcd", 99), "abcd");
        assert_eq!(before_caret("abcd", 0), "");
    }

    /// 快照没有时给脚本 `None`；拿到之后 `store(None)` 不能把它抹掉（DLL 没被请求的那段组句就带 `None`）。
    #[test]
    fn a_missing_snapshot_gives_none_and_none_never_wipes_it() {
        let document = Document::default();
        assert_eq!(document.slice(TextRange::All, u64::MAX), None);

        document.store(Some(DocumentText {
            text: "ab中文cd".to_owned(),
            caret: 2,
        }));
        // 没请 DLL 读的那段组句送 `None`：留着上一次那份，别抹
        document.store(None);
        assert_eq!(
            document.slice(TextRange::All, u64::MAX),
            Some(("ab中文cd".to_owned(), false))
        );
        assert_eq!(
            document.slice(TextRange::Before, u64::MAX),
            Some(("ab".to_owned(), false))
        );
        assert_eq!(
            document.slice(TextRange::After, u64::MAX),
            Some(("中文cd".to_owned(), false))
        );
        // 上限 2：`after` 只留最靠光标的头一个字（`中` 宽 2）
        assert_eq!(
            document.slice(TextRange::After, 2),
            Some(("中".to_owned(), true))
        );
        // 整篇上限 4：前面 2（`ab`）+ 后面 2（`中`）刚好占满
        assert_eq!(
            document.slice(TextRange::All, 4),
            Some(("ab中".to_owned(), true))
        );
        // 清了（换会话 / 转私密）之后又是「没拿到」
        document.clear();
        assert_eq!(document.slice(TextRange::All, u64::MAX), None);
    }
}
