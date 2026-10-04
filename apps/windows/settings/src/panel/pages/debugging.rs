//! 「调试」页：实验性开关、输入统计面板（原「统计」页）、文件 / 日志 / 学习设置（原「高级」页），
//! 以及日志 / 软件官网 / 组件入口。
//!
//! 统计面板读数据目录里的 `usage.tsv` / `user-vocab.tsv`；文件与日志入口都直通本机路径，不经 Server。

use cloudime_core::{Usage, UsageSummary, VocabularySummary};
use cloudime_learning::{UsageStats, VocabularyBook};
use cloudime_platform::{LogLevel, PhraseStore, WordBank};
use jiff::Zoned;
use windows_reactor::*;

use crate::panel::controls::{field, note, page};
use crate::panel::{Message, Settings};

const COLUMNS: [&str; 4] = ["汉字", "中文词", "英文词", "上屏次数"];

/// 小节标题。
fn heading(text: &str) -> View {
    TextBlock::new()
        .text(text)
        .font_weight(FontWeight::SEMI_BOLD)
        .into()
}

fn cell(text: impl Into<String>, width: f64) -> View {
    TextBlock::new().text(text).width(width).into()
}

/// 行首标签 + 四个数字。
fn table_row(label: &str, values: [String; 4], strong: bool) -> View {
    let label_block = TextBlock::new()
        .text(label)
        .width(110.0)
        .font_weight(if strong {
            FontWeight::SEMI_BOLD
        } else {
            FontWeight::NORMAL
        });
    let cells = [
        label_block.into(),
        cell(values[0].clone(), 90.0),
        cell(values[1].clone(), 90.0),
        cell(values[2].clone(), 90.0),
        cell(values[3].clone(), 90.0),
    ];
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children(cells)
}

fn columns(usage: &Usage) -> [String; 4] {
    [
        group_digits(usage.hanzi),
        group_digits(usage.words),
        group_digits(usage.english_words),
        group_digits(usage.commits),
    ]
}

fn since_line(summary: &UsageSummary) -> String {
    match &summary.since {
        Some(date) => format!("自 {date} 起，有输入的天数 {}。", summary.days),
        None => "还没有记录，打几个字再来看。".to_owned(),
    }
}

fn vocabulary_line(summary: &VocabularySummary) -> String {
    if summary.seen == 0 {
        return "还没记录词汇：打中文时候选窗口里的词就是词汇的来源。".to_owned();
    }
    format!(
        "见过 {} 个词，上屏过 {} 个；本周新见 {} 个。",
        group_digits(summary.seen),
        group_digits(summary.committed),
        group_digits(summary.new_this_week),
    )
}

/// 「有 / 无」。
fn yes_no(present: bool) -> &'static str {
    if present { "有" } else { "无" }
}

/// 目录里有文件。
fn has_files(path: &std::path::Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}

/// 数据与组件的一行行说明。
fn data_lines(settings: &Settings) -> Vec<String> {
    let data_dir = settings.data_dir();
    let root =
        cloudime_platform::resources::bundled_root().unwrap_or_else(|| data_dir.to_path_buf());
    let bank = WordBank::locate(&root);
    let files = bank.files();
    let main = bank
        .main()
        .map(|(stem, _)| stem)
        .unwrap_or_else(|| "无（回落样例）".to_owned());
    let phrases = PhraseStore::locate(data_dir, &settings.config.phrase)
        .load()
        .map_or(0, |phrases| phrases.len());
    vec![
        format!(
            "词库目录 {}：{} 个文件，主词库 {}。",
            bank.dir.display(),
            files.len(),
            main
        ),
        format!("用户短语：{phrases} 条。"),
        format!(
            "随包数据：主词库{} · 语言模型{} · 本地整句模型{}。",
            yes_no(bank.path().is_file()),
            yes_no(root.join("data/generated/lm.qj").is_file()),
            yes_no(
                has_files(&root.join("data/local_models"))
                    || has_files(&data_dir.join("local_models"))
            ),
        ),
        format!(
            "版本：{}（{}）。",
            super::about::VERSION,
            option_env!("CLOUDIME_BUILD").unwrap_or("本地构建")
        ),
    ]
}

/// 输入统计面板：今天 / 最近 7 天 / 累计的输入量与词汇记录。
fn usage_panel(settings: &Settings) -> Vec<KeyedView> {
    let data_dir = settings.data_dir();
    let today = Zoned::now().date();
    let usage = UsageStats::open(data_dir.join("usage.tsv")).summary_on(today);
    let vocabulary = VocabularyBook::open(data_dir.join("user-vocab.tsv")).summary_on(today);
    let header = table_row("", COLUMNS.map(str::to_owned), true);
    let rows = [
        header,
        table_row("今天", columns(&usage.today), false),
        table_row("最近 7 天", columns(&usage.week), false),
        table_row("累计", columns(&usage.total), false),
    ];
    let mut panel: Vec<KeyedView> = vec![
        KeyedView::new("panel-usage-title", heading("输入统计面板")),
        KeyedView::new(
            "panel-usage-table",
            StackPanel::new().spacing(6.0).children(rows),
        ),
        KeyedView::new("panel-usage-since", note(&since_line(&usage))),
        KeyedView::new(
            "panel-usage-note",
            note(
                "数的是上屏的文字：选一个词算一个中文词，整句按词切开数；英文候选、回车原样上屏的英文词算英文词。只在这台电脑上数，与输入日志无关。",
            ),
        ),
        KeyedView::new("panel-vocab-title", heading("词汇")),
        KeyedView::new("panel-vocab-note", note(&vocabulary_line(&vocabulary))),
    ];
    panel.push(KeyedView::new("panel-data-title", heading("数据与组件")));
    panel.extend(
        data_lines(settings)
            .into_iter()
            .enumerate()
            .map(|(index, line)| KeyedView::new(format!("panel-data-{index}"), note(&line))),
    );
    panel
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let g = &settings.config.general;
    let d = &settings.config.debugging;
    let rows = [
        field(
            "自动隐藏悬浮工具栏（实验性功能）",
            "启用后，当处于全屏幕状态，或者用户切换输入法为其他输入法，或者禁用输入法时自动隐藏。",
            ToggleSwitch::new()
                .is_on(d.auto_hide_float_tool_bar)
                .on_toggled(context.callback(Message::AutoHideFloatToolBar)),
        ),
        StackPanel::new()
            .spacing(6.0)
            .keyed_children(usage_panel(settings)),
        field(
            "配置文件",
            "",
            Button::new()
                .on_click(context.message(Message::OpenConfigFile))
                .content("在记事本中打开"),
        ),
        field(
            "数据目录",
            "配置、短语、学习数据与统计都在这里。",
            Button::new()
                .on_click(context.message(Message::OpenDataDir))
                .content("打开数据目录"),
        ),
        field(
            "日志",
            "输入法、引擎与设置程序的日志都在这一个目录（%LOCALAPPDATA%\\CloudIME\\logs），按天分文件，保留 7 天。",
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    Button::new()
                        .on_click(context.message(Message::OpenLogDir))
                        .content("打开日志目录"),
                    Button::new()
                        .on_click(context.message(Message::ExportLogs))
                        .content("打包日志到桌面"),
                )),
        ),
        field(
            "详细日志",
            "排查问题时临时打开，会记下敲的拼音与上屏文字。",
            ToggleSwitch::new()
                .is_on(g.log_level == LogLevel::Debug)
                .on_toggled(context.callback(Message::VerboseLog)),
        ),
        field(
            "学习输入习惯",
            "按你的选择调整候选顺序、记新词与敲错纠正。关掉后不再学，已学的仍参与排序。",
            ToggleSwitch::new()
                .is_on(g.learning)
                .on_toggled(context.callback(Message::Learning)),
        ),
        field(
            "记录输入日志",
            "每次上屏记一行，只写本机、不上传，用于离线评测与个人模型。",
            ToggleSwitch::new()
                .is_on(g.input_log)
                .on_toggled(context.callback(Message::InputLog)),
        ),
        field(
            "清空输入日志",
            "",
            Button::new()
                .on_click(context.message(Message::ClearInputLog))
                .content("清空输入日志"),
        ),
        field(
            "软件官网",
            "",
            Button::new()
                .on_click(context.message(Message::OpenWebsite))
                .content("打开官网"),
        ),
        field(
            "组件（实验性功能）",
            "还没做，先留一个入口。",
            Button::new()
                .on_click(context.message(Message::OpenComponents))
                .content("打开组件"),
        ),
    ];
    page("调试", StackPanel::new().spacing(16.0).children(rows))
}

/// 千位分隔。
fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
