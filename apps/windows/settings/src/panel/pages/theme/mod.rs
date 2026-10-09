//! 「主题」页：选 / 新建 / 编辑 `Themes\*.json`（三个窗口的 21 个颜色），顶部一块**真渲染**预览
//! （与真实窗口同一套渲染器，见 [`preview`]）。
//!
//! 改动先进「草稿」：点「确认保存」只写到 `%APPDATA%\CloudIME\Themes\<名字>.json`；
//! 点「应用主题」把 `curr_theme` 写进配置**并立刻重启输入法服务**（重启才盖得过脚本临时主题）。
//! 颜色点一下弹 `ColorDialog` 选。

mod preview;

use cloudime_platform::{THEMES_DIR, ThemeColor, ThemeFile};
use windows_reactor::*;

use crate::panel::controls::{field, note, page};
use crate::panel::{Message, Settings, theme_stem};

/// 两行控件的首列宽度：让「选择主题」那一行的 `ComboBox` 与上面一行的名字框左边对齐。
/// 取 120 是因为 WinUI 的 `Button` 最小宽度就是 120（再小会被顶回去），「新建主题」本来就至少这么宽。
const PICKER_LABEL_WIDTH: f64 = 120.0;

/// 主题文件里那 21 个颜色槽。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    CandidateBackground,
    CandidateHighlight,
    CandidateShadow,
    CandidatePinyin,
    CandidatePinyinCaret,
    CandidateHighlightText,
    CandidateText,
    CandidateHighlightIndex,
    CandidateIndex,
    CandidatePageNumber,
    CandidateBadge,
    CandidateTranslateMeta,
    CandidateTranslateLearned,
    CandidateTranslateFresh,
    CandidateExtra,
    BarBackground,
    BarIcon,
    BarShadow,
    TipBackground,
    TipIcon,
    TipShadow,
}

impl Slot {
    /// 全部槽，页里按这个顺序列。
    pub(crate) const ALL: [Self; 21] = [
        Self::CandidateBackground,
        Self::CandidateHighlight,
        Self::CandidateShadow,
        Self::CandidatePinyin,
        Self::CandidatePinyinCaret,
        Self::CandidateHighlightText,
        Self::CandidateText,
        Self::CandidateHighlightIndex,
        Self::CandidateIndex,
        Self::CandidatePageNumber,
        Self::CandidateBadge,
        Self::CandidateTranslateMeta,
        Self::CandidateTranslateLearned,
        Self::CandidateTranslateFresh,
        Self::CandidateExtra,
        Self::BarBackground,
        Self::BarIcon,
        Self::BarShadow,
        Self::TipBackground,
        Self::TipIcon,
        Self::TipShadow,
    ];

    /// 界面上的名字。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::CandidateBackground => "候选窗口背景颜色",
            Self::CandidateHighlight => "候选窗口高亮条颜色",
            Self::CandidateShadow => "候选窗口阴影颜色",
            Self::CandidatePinyin => "候选窗口拼音串颜色",
            Self::CandidatePinyinCaret => "候选窗口拼音串光标颜色",
            Self::CandidateHighlightText => "候选窗口高亮候选项颜色",
            Self::CandidateText => "候选窗口普通候选项颜色",
            Self::CandidateHighlightIndex => "候选窗口高亮候选项序号颜色",
            Self::CandidateIndex => "候选窗口普通候选项序号颜色",
            Self::CandidatePageNumber => "候选窗口页码颜色",
            Self::CandidateBadge => "候选窗口候选项角标颜色",
            Self::CandidateTranslateMeta => "候选窗口候选项翻译Tip词性和分号颜色",
            Self::CandidateTranslateLearned => "候选窗口候选项翻译Tip已学习释义颜色",
            Self::CandidateTranslateFresh => "候选窗口候选项翻译Tip未学习释义颜色",
            Self::CandidateExtra => "候选窗口候选项额外内容颜色",
            Self::BarBackground => "悬浮工具栏背景颜色",
            Self::BarIcon => "悬浮工具栏图标颜色",
            Self::BarShadow => "悬浮工具栏阴影颜色",
            Self::TipBackground => "状态切换提示背景颜色",
            Self::TipIcon => "状态切换提示图标颜色",
            Self::TipShadow => "状态切换提示阴影颜色",
        }
    }

    /// 这份主题里这个槽当前的颜色。
    pub(crate) fn get(self, theme: &ThemeFile) -> ThemeColor {
        let candidate = theme.candidate;
        match self {
            Self::CandidateBackground => candidate.background,
            Self::CandidateHighlight => candidate.highlight,
            Self::CandidateShadow => candidate.shadow,
            Self::CandidatePinyin => candidate.pinyin,
            Self::CandidatePinyinCaret => candidate.pinyin_caret,
            Self::CandidateHighlightText => candidate.highlight_text,
            Self::CandidateText => candidate.text,
            Self::CandidateHighlightIndex => candidate.highlight_index,
            Self::CandidateIndex => candidate.index,
            Self::CandidatePageNumber => candidate.page_number,
            Self::CandidateBadge => candidate.badge,
            Self::CandidateTranslateMeta => candidate.translate_meta,
            Self::CandidateTranslateLearned => candidate.translate_learned,
            Self::CandidateTranslateFresh => candidate.translate_fresh,
            Self::CandidateExtra => candidate.extra,
            Self::BarBackground => theme.bar.background,
            Self::BarIcon => theme.bar.icon,
            Self::BarShadow => theme.bar.shadow,
            Self::TipBackground => theme.tip.background,
            Self::TipIcon => theme.tip.icon,
            Self::TipShadow => theme.tip.shadow,
        }
    }

    /// 改这份主题里这个槽的颜色。
    pub(crate) fn set(self, theme: &mut ThemeFile, color: ThemeColor) {
        match self {
            Self::CandidateBackground => theme.candidate.background = color,
            Self::CandidateHighlight => theme.candidate.highlight = color,
            Self::CandidateShadow => theme.candidate.shadow = color,
            Self::CandidatePinyin => theme.candidate.pinyin = color,
            Self::CandidatePinyinCaret => theme.candidate.pinyin_caret = color,
            Self::CandidateHighlightText => theme.candidate.highlight_text = color,
            Self::CandidateText => theme.candidate.text = color,
            Self::CandidateHighlightIndex => theme.candidate.highlight_index = color,
            Self::CandidateIndex => theme.candidate.index = color,
            Self::CandidatePageNumber => theme.candidate.page_number = color,
            Self::CandidateBadge => theme.candidate.badge = color,
            Self::CandidateTranslateMeta => theme.candidate.translate_meta = color,
            Self::CandidateTranslateLearned => theme.candidate.translate_learned = color,
            Self::CandidateTranslateFresh => theme.candidate.translate_fresh = color,
            Self::CandidateExtra => theme.candidate.extra = color,
            Self::BarBackground => theme.bar.background = color,
            Self::BarIcon => theme.bar.icon = color,
            Self::BarShadow => theme.bar.shadow = color,
            Self::TipBackground => theme.tip.background = color,
            Self::TipIcon => theme.tip.icon = color,
            Self::TipShadow => theme.tip.shadow = color,
        }
    }
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let mut rows: Vec<KeyedView> = Vec::new();
    rows.push(KeyedView::new(
        "preview",
        preview::view(&settings.theme_draft, &settings.config.candidate),
    ));
    rows.push(KeyedView::new(
        "new",
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                Button::new()
                    .on_click(context.message(Message::ThemeNew))
                    .width(PICKER_LABEL_WIDTH)
                    .content("新建主题"),
                TextBox::new()
                    .width(200.0)
                    .placeholder_text("新主题名字")
                    .text(settings.theme_new_name.clone())
                    .on_text_changed(context.callback(Message::ThemeNewName)),
                Button::new()
                    .on_click(context.message(Message::ThemeSave))
                    .content("确认保存"),
            )),
    ));
    let selected = settings
        .theme_selected
        .as_ref()
        .and_then(|name| settings.theme_names.iter().position(|item| item == name));
    rows.push(KeyedView::new(
        "pick",
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                TextBlock::new()
                    .text("选择主题")
                    .width(PICKER_LABEL_WIDTH)
                    .vertical_alignment(VerticalAlignment::Center),
                ComboBox::new()
                    .width(200.0)
                    .items_source(settings.theme_names.clone())
                    .selected_index(selected)
                    .on_selection_changed(context.callback(Message::ThemeSelect)),
                Button::new()
                    .on_click(context.message(Message::ThemeRefresh))
                    .content("刷新主题"),
                Button::new()
                    .on_click(context.message(Message::ThemeApply))
                    .content("应用主题"),
                Button::new()
                    .on_click(context.message(Message::ThemeImport))
                    .content("导入主题"),
                Button::new()
                    .on_click(context.message(Message::ThemeExport))
                    .content("导出当前主题"),
            )),
    ));
    if !settings.theme_status.is_empty() {
        rows.push(KeyedView::new("status", note(&settings.theme_status)));
    }
    for slot in Slot::ALL {
        rows.push(KeyedView::new(
            slot.label(),
            field(
                slot.label(),
                "",
                Button::new()
                    .on_click(context.callback(move |_| Message::ThemeColorOpen(slot)))
                    .content(slot.get(&settings.theme_draft).hex()),
            ),
        ));
    }
    rows.push(KeyedView::new(
        "color-dialog",
        settings.color_dialog.view(
            "选择颜色",
            context.callback(Message::ThemeColorChanged),
            context.callback(Message::ThemeColorClosed),
        ),
    ));
    page("主题", StackPanel::new().spacing(16.0).keyed_children(rows))
}

/// 「导入主题」：挑一个 `.json`（**先解析一遍确认是主题**，不是就直接报错不拷），
/// 复制进用户主题目录，成功后刷新列表并选中它。
pub(crate) fn import(settings: &mut Settings) {
    let Some(source) = rfd::FileDialog::new()
        .add_filter("主题文件（.json）", &["json"])
        .set_title("导入主题")
        .pick_file()
    else {
        return;
    };
    let theme = match ThemeFile::load(&source) {
        Ok(theme) => theme,
        Err(error) => {
            settings.theme_status = format!("「{}」不是主题文件：{error}", source.display());
            return;
        }
    };
    let raw = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let stem = match theme_stem(raw, None) {
        Ok(stem) => stem,
        Err(reason) => {
            settings.theme_status = format!("导入失败：{reason}");
            return;
        }
    };
    let Some(dir) = cloudime_platform::dirs::user_dir().map(|dir| dir.join(THEMES_DIR)) else {
        settings.theme_status = "找不到用户目录（%APPDATA%），导入不了。".to_owned();
        return;
    };
    let target = dir.join(format!("{stem}.json"));
    if let Err(error) = theme.save(&target) {
        settings.theme_status = format!("导入失败：{error}");
        return;
    }
    // 成功导入后刷新主题目录（新名字立刻出现在下拉里），并把它读进草稿。
    settings.refresh_theme_names();
    settings.select_theme(&stem);
    settings.theme_status = format!("已导入「{stem}」；点「应用主题」让三个窗口换上。");
}

/// 「导出当前主题」：把**当前草稿**另存到用户挑的位置；用户主题目录里那一份不动。
pub(crate) fn export(settings: &mut Settings) {
    let name = settings
        .theme_selected
        .clone()
        .unwrap_or_else(|| "theme".to_owned());
    let Some(target) = rfd::FileDialog::new()
        .add_filter("主题文件（.json）", &["json"])
        .set_file_name(format!("{name}.json"))
        .set_title("导出当前主题")
        .save_file()
    else {
        return;
    };
    match settings.theme_draft.save(&target) {
        Ok(()) => settings.theme_status = format!("已导出到 {}。", target.display()),
        Err(error) => settings.theme_status = format!("导出失败：{error}"),
    }
}
