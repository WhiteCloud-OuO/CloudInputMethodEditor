//! 快捷候选（搜狗「v 模式」的那一套）：不查词库、由输入本身直接算出来的候选。
//!
//! - `rq` / `sj` / `xq`：今天的日期、现在的时间、星期几，插在本地候选第二位起。
//! - 表达式键 `v`（[`EXPRESSION_PREFIX`]）开头进表达式模式：`v1+2` 出 `3` 与 `1+2=3`，
//!   `v123` 出中文数字与金额（小写与大写），`v123.5` 出小数读法与金额，同样各有小写与大写
//!   （一百二十三元五角 / 壹佰贰拾叁元伍角）。表达式模式下缓冲区允许数字与运算符，候选不走拼音解析。

mod calendar;
mod evaluator;
mod numeral;

use jiff::Zoned;

use crate::candidate::{Candidate, CandidateKind};

pub use calendar::{date_forms, time_forms, weekday_forms};
pub use evaluator::evaluate;
pub use numeral::{
    amount_lower, amount_upper, chinese_decimal_lower, chinese_decimal_upper, chinese_lower,
    chinese_upper,
};

/// 表达式模式的固定前缀。`v` 不是任何拼音音节的开头，用它不会和拼音冲突。
pub const EXPRESSION_PREFIX: char = 'v';

/// 表达式模式下允许敲进缓冲区的非字母字符：数字与四则运算符号。
/// 字母（`x` 当乘号）本来就能进缓冲区，不在此列。
pub fn is_expression_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '+' | '-' | '*' | '/' | '(' | ')' | '.' | '^')
}

/// 按输入算快捷候选；不是快捷输入时为空。表达式固定用 [`EXPRESSION_PREFIX`]；`now` 由调用方给，测试可固定时间。
pub fn candidates(input: &str, now: &Zoned) -> Vec<Candidate> {
    let texts: Vec<String> = match input {
        "rq" => date_forms(now),
        "sj" => time_forms(now),
        "xq" => weekday_forms(now),
        _ => match input.strip_prefix(EXPRESSION_PREFIX) {
            Some(body) => expression_forms(body),
            None => Vec::new(),
        },
    };
    texts
        .into_iter()
        .map(|text| Candidate {
            text,
            kind: CandidateKind::Shortcut,
            syllables: Vec::new(),
            reading: None,
        })
        .collect()
}

/// 金额最多几位小数（角、分）。
const AMOUNT_FRACTION_DIGITS: usize = 2;

/// 表达式键之后的部分：一个数出中文数字与金额，四则运算出结果与「算式=结果」。
fn expression_forms(body: &str) -> Vec<String> {
    if let Some(forms) = number_forms(body) {
        return forms;
    }
    match evaluate(body) {
        Some(result) => vec![result.clone(), format!("{body}={result}")],
        None => Vec::new(),
    }
}

/// 单个非负数（`123`、`123.5`）：中文数字在前，金额在后；小数超过两位没有金额写法。
/// 末尾刚敲了 `.`（`123.`）按整数给，候选不闪。
fn number_forms(body: &str) -> Option<Vec<String>> {
    let (integer, fraction) = body.split_once('.').unwrap_or((body, ""));
    let digits = |text: &str| text.bytes().all(|b| b.is_ascii_digit());
    if integer.is_empty() || !digits(integer) || !digits(fraction) {
        return None;
    }
    let mut forms = if fraction.is_empty() {
        vec![chinese_lower(integer), chinese_upper(integer)]
    } else {
        vec![
            chinese_decimal_lower(integer, fraction),
            chinese_decimal_upper(integer, fraction),
        ]
    };
    if fraction.len() <= AMOUNT_FRACTION_DIGITS {
        forms.push(amount_lower(integer, fraction));
        forms.push(amount_upper(integer, fraction));
    }
    Some(forms)
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use jiff::tz::TimeZone;

    use super::*;

    fn now() -> Zoned {
        date(2026, 9, 3)
            .at(19, 6, 23, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
    }

    fn texts(input: &str) -> Vec<String> {
        candidates(input, &now())
            .into_iter()
            .map(|c| c.text)
            .collect()
    }

    #[test]
    fn calendar_shortcuts() {
        assert_eq!(texts("rq"), ["2026年9月3日", "2026-09-03", "2026/09/03"]);
        assert_eq!(texts("sj"), ["19:06", "19:06:23", "19点06分"]);
        assert_eq!(texts("xq"), ["星期四", "周四"]);
        assert!(texts("rqi").is_empty());
    }

    #[test]
    fn expression_shortcuts() {
        assert_eq!(texts("v1+2"), ["3", "1+2=3"]);
        assert_eq!(
            texts("v123"),
            [
                "一百二十三",
                "壹佰贰拾叁",
                "一百二十三元整",
                "壹佰贰拾叁元整"
            ]
        );
        assert_eq!(
            texts("v123.5"),
            [
                "一百二十三点五",
                "壹佰贰拾叁点伍",
                "一百二十三元五角",
                "壹佰贰拾叁元伍角"
            ]
        );
        assert_eq!(texts("v123."), texts("v123"));
        // 超过角分的小数只有读法；带运算符的照常算
        assert_eq!(texts("v3.14159"), ["三点一四一五九", "叁点壹肆壹伍玖"]);
        assert_eq!(texts("v1.5*2"), ["3", "1.5*2=3"]);
        assert!(texts("v").is_empty());
        assert!(texts("v1+").is_empty());
        assert!(texts("very").is_empty());
    }

    #[test]
    fn expression_chars() {
        assert!(is_expression_char('7'));
        assert!(is_expression_char('('));
        assert!(!is_expression_char('='));
        assert!(!is_expression_char('x'));
    }
}
