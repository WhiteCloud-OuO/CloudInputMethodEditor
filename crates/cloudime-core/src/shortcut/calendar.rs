//! V 模式里的日期、时间、星期候选写法。

use jiff::Zoned;
use jiff::civil::Weekday;

/// `rq`：年月日的 12 种常用写法。
pub fn date_forms(now: &Zoned) -> Vec<String> {
    let (year, month, day) = (now.year(), now.month(), now.day());
    vec![
        format!("{year}年{month}月{day}日"),
        format!("{year}年{month:02}月{day:02}日"),
        format!("{year}-{month}-{day}"),
        format!("{year}-{month:02}-{day:02}"),
        format!("{year}/{month}/{day}"),
        format!("{year}/{month:02}/{day:02}"),
        format!("{month}月{day}日"),
        format!("{month:02}月{day:02}日"),
        format!("{month}-{day}"),
        format!("{month:02}-{day:02}"),
        format!("{month}/{day}"),
        format!("{month:02}/{day:02}"),
    ]
}

/// `sj`：当前时间的 8 种写法。`seconds` 打开时每种都带上秒。
pub fn time_forms(now: &Zoned, seconds: bool) -> Vec<String> {
    let hour = now.hour();
    let minute = now.minute();
    let second = now.second();
    let hour24 = format!("{hour:02}");
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let chinese_period = if hour < 12 { "上午" } else { "下午" };
    let (am_pm, am_pm_compact) = if hour < 12 {
        ("a.m.", "am")
    } else {
        ("p.m.", "pm")
    };
    let second_cn = if seconds {
        format!("{second:02}秒")
    } else {
        String::new()
    };
    let second_colon = if seconds {
        format!(":{second:02}")
    } else {
        String::new()
    };
    let second_dash = if seconds {
        format!("-{second:02}")
    } else {
        String::new()
    };
    vec![
        format!("{hour24}时{minute:02}分{second_cn}"),
        format!("{chinese_period}{hour12}时{minute:02}分{second_cn}"),
        format!("{hour24}:{minute:02}{second_colon}"),
        format!("{hour24}-{minute:02}{second_dash}"),
        format!("{hour12}:{minute:02}{second_colon} {am_pm}"),
        format!("{hour12}-{minute:02}{second_dash} {am_pm}"),
        format!("{hour12}:{minute:02}{second_colon} {am_pm_compact}"),
        format!("{hour12}-{minute:02}{second_dash} {am_pm_compact}"),
    ]
}

/// `xq`：`星期一` / `周一` / `礼拜一`；星期日用「礼拜天」。
pub fn weekday_forms(now: &Zoned) -> Vec<String> {
    let name = match now.weekday() {
        Weekday::Monday => "一",
        Weekday::Tuesday => "二",
        Weekday::Wednesday => "三",
        Weekday::Thursday => "四",
        Weekday::Friday => "五",
        Weekday::Saturday => "六",
        Weekday::Sunday => "日",
    };
    let worship = if now.weekday() == Weekday::Sunday {
        "天"
    } else {
        name
    };
    vec![
        format!("星期{name}"),
        format!("周{name}"),
        format!("礼拜{worship}"),
    ]
}

/// `xz`：现在（日期 + 时间）的 14 种组合写法。
pub fn now_forms(now: &Zoned, seconds: bool) -> Vec<String> {
    let (year, month, day) = (now.year(), now.month(), now.day());
    let hour = now.hour();
    let minute = now.minute();
    let second = now.second();
    let hour24 = format!("{hour:02}");
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let (am_pm, am_pm_compact) = if hour < 12 {
        ("a.m.", "am")
    } else {
        ("p.m.", "pm")
    };
    let second_cn = if seconds {
        format!("{second:02}秒")
    } else {
        String::new()
    };
    let second_colon = if seconds {
        format!(":{second:02}")
    } else {
        String::new()
    };

    let chinese_dates = [
        format!("{year}年{month}月{day}日"),
        format!("{year}年{month:02}月{day:02}日"),
    ];
    let numeric_dates = [
        format!("{year}-{month}-{day}"),
        format!("{year}-{month:02}-{day:02}"),
        format!("{year}/{month}/{day}"),
        format!("{year}/{month:02}/{day:02}"),
    ];

    let mut forms = Vec::with_capacity(14);
    for date in chinese_dates {
        forms.push(format!("{date}{hour24}时{minute:02}分{second_cn}"));
    }
    for date in &numeric_dates {
        forms.push(format!("{date} {hour24}:{minute:02}{second_colon}"));
    }
    for date in &numeric_dates {
        forms.push(format!("{date} {hour12}:{minute:02}{second_colon} {am_pm}"));
    }
    for date in &numeric_dates {
        forms.push(format!(
            "{date} {hour12}:{minute:02}{second_colon}{am_pm_compact}"
        ));
    }
    forms
}
