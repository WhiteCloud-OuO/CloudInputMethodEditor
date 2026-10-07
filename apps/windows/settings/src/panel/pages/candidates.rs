//! 「候选」页：`[candidate]` 分节（排布、个数、三个字体、序号样式、最小宽度、按程序隐藏）。

use cloudime_platform::{
    CandidateConfig, FontChoice, ItemNumberStyle, LayoutMode, MAX_ASSOCIATION_COUNTS,
    MAX_CANDIDATE_COUNT, MIN_ASSOCIATION_COUNTS, MIN_CANDIDATE_COUNT, PreeditMode,
};
use windows_reactor::*;

use crate::panel::controls::{
    field, field_top, labeled, note, page, radio_row, scroll_list, slider_field,
};
use crate::panel::{Message, Settings};

/// 本地整句模型文件在不在这里：用户目录 `%APPDATA%\CloudIME\local_models\` 优先，其次随包 `data\local_models\`。
/// 判定与 Server 的 `rescore::find_model` 对齐（`.qjm` 单文件，或三件套里的 `model.safetensors`）。
fn local_model_present(settings: &Settings) -> bool {
    let data_dir = settings.data_dir();
    let root =
        cloudime_platform::resources::bundled_root().unwrap_or_else(|| data_dir.to_path_buf());
    [
        data_dir.join("local_models"),
        root.join("data/local_models"),
    ]
    .into_iter()
    .any(|dir| dir_has_model(&dir))
}

/// 目录里能加载的模型：一个 `.qjm` 单文件，或三件套里的 `model.safetensors`。
fn dir_has_model(dir: &std::path::Path) -> bool {
    if dir.join("model.safetensors").is_file() {
        return true;
    }
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("qjm"))
        })
    })
}

/// 模型开关那一项：常规说明小字下面，模型文件缺失时再补一行红字，免得以为是开关坏了。
fn model_field(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let toggle = ToggleSwitch::new()
        .is_on(
            settings
                .config
                .candidate
                .use_local_sentence_organization_model,
        )
        .on_toggled(context.callback(Message::LocalModel));
    let row = labeled("使用本地整句模型（输入法内置）", toggle);
    let hint = note(
        "开启后，消耗一部分处理器和内存资源以获得更精准的整句输入；关闭后，仅使用词库和短语匹配。",
    );
    if local_model_present(settings) {
        return StackPanel::new().spacing(4.0).children([row, hint]);
    }
    let warning: View = TextBlock::new()
        .text(
            "未找到本地模型文件，开启也不生效：把 .qjm 模型放进 %APPDATA%\\CloudIME\\local_models\\，或让安装包带上 data\\local_models\\。",
        )
        .text_wrapping(TextWrapping::Wrap)
        .font_size(12.0)
        .foreground(ThemeBrush::SystemCritical)
        .into();
    StackPanel::new()
        .spacing(4.0)
        .children([row, hint, warning])
}

/// 三个字体项：点按钮弹系统字体对话框时用它区分改哪一个。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontRole {
    /// 拼音串（候选窗口顶部那一行）。
    Pinyin,

    /// 候选项。
    Candidate,

    /// 候选项序号。
    ItemNumber,

    /// 翻译 Tip（候选窗口底部那一行左侧）。
    Translate,
}

impl FontRole {
    /// 这个角色当前的字体。
    pub(crate) fn current(self, config: &CandidateConfig) -> &FontChoice {
        match self {
            Self::Pinyin => &config.pinyin_font,
            Self::Candidate => &config.candidate_font,
            Self::ItemNumber => &config.item_number_font,
            Self::Translate => &config.translate_font,
        }
    }
}

/// 一个「滑轨 + 右侧数字」的整数值项：拖动即时更新，数字定宽、垂直居中，不把滑轨顶来顶去。
/// 「字体…」按钮：上面显示当前的字族与字号。
fn font_button(
    role: FontRole,
    config: &CandidateConfig,
    context: &mut ViewContext<Settings>,
) -> View {
    Button::new()
        .on_click(context.callback(move |_| Message::PickFont(role)))
        .content(role.current(config).label())
}

/// 枚举下拉：按 `label()` 列项，选中 `current`（找不到取 0）。
fn mode_combo<T: PartialEq + Copy>(
    all: &'static [T],
    current: T,
    label: fn(T) -> &'static str,
    callback: Callback<Option<usize>>,
) -> ComboBox {
    ComboBox::new()
        .items_source(all.iter().map(|mode| label(*mode)))
        .selected_index(all.iter().position(|mode| *mode == current).unwrap_or(0))
        .on_selection_changed(callback)
}

/// 「不显示候选框」的程序名单：一个输入框 + 添加按钮；下面每行是一个 2 列 `Grid`（程序名 / 删除），
/// 最多同时显示 5 行，多的靠列表内滚动。
fn program_list(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let query = settings.program_query.clone().unwrap_or_default();
    let add = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children((
            TextBox::new()
                .width(260.0)
                .text(query)
                .placeholder_text("例如 notepad.exe")
                .on_text_changed(context.callback(Message::ProgramQuery)),
            Button::new()
                .on_click(context.message(Message::ProgramAdd))
                .content("添加"),
        ));
    let mut items: Vec<KeyedView> = Vec::new();
    for program in &settings.config.candidate.program_list_of_hiding_candidate {
        items.push(KeyedView::new(
            program.clone(),
            ListViewItem::new().content(
                // 第一列吃剩余宽度放程序名，第二列按内容宽放按钮。
                Grid::new()
                    .columns([GridLength::STAR, GridLength::Auto])
                    .column_spacing(12.0)
                    .keyed_children([
                        KeyedView::new(
                            "name",
                            TextBlock::new()
                                .text(program.clone())
                                .vertical_alignment(VerticalAlignment::Center)
                                .grid_column(0),
                        ),
                        KeyedView::new(
                            "remove",
                            Button::new()
                                .on_click(context.message(Message::ProgramRemove(program.clone())))
                                .grid_column(1)
                                .content("删除"),
                        ),
                    ]),
            ),
        ));
    }
    if items.is_empty() {
        return add;
    }
    StackPanel::new()
        .spacing(8.0)
        .children((add, scroll_list(items)))
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let c = &settings.config.candidate;
    let rows = [
        model_field(settings, context),
        field(
            "候选项排布方向",
            "横排时只给高亮的候选单独一行。",
            radio_row(
                "arrangement",
                LayoutMode::ALL
                    .iter()
                    .map(|mode| (mode.label(), *mode == c.candidate_arrangement_direction)),
                Message::Arrangement,
                context,
            ),
        ),
        slider_field(
            "候选项个数",
            "",
            c.candidate_count(),
            MIN_CANDIDATE_COUNT,
            MAX_CANDIDATE_COUNT,
            Message::CandidateCount,
            context,
        ),
        slider_field(
            "联想候选项目上限",
            "候选里「比读法更长的词」（联想）最多留几条。打得短（尤其单个字母）时联想候选会很多，调小能让要选的字 / 词留在候选窗口里；0 表示不显示联想候选。",
            c.association_counts(),
            MIN_ASSOCIATION_COUNTS,
            MAX_ASSOCIATION_COUNTS,
            Message::AssociationCounts,
            context,
        ),
        field(
            "拼音串字体",
            "单击后选择字体与字号；没装的字体自动回到系统字体。",
            font_button(FontRole::Pinyin, c, context),
        ),
        field(
            "候选项字体",
            "",
            font_button(FontRole::Candidate, c, context),
        ),
        field(
            "候选项序号字体",
            "",
            font_button(FontRole::ItemNumber, c, context),
        ),
        field("翻译字体", "", font_button(FontRole::Translate, c, context)),
        field(
            "候选项序号样式",
            "",
            mode_combo(
                &ItemNumberStyle::ALL,
                c.item_number_style,
                ItemNumberStyle::label,
                context.callback(Message::ItemNumberStyle),
            ),
        ),
        field(
            "候选框最小宽度",
            "仅在候选项排布方向为垂直时有效。单位为像素。",
            NumberBox::new()
                .minimum(0.0)
                .maximum(2000.0)
                .value(c.candidate_box_minimum_width as f64)
                .on_value_changed(context.callback(Message::CandidateBoxMinimumWidth)),
        ),
        field(
            "展示更多候选项",
            "在打字时按下 Tab 键启用。常规：显示更多的候选项；U 模式：显示符号输入面板；I 模式：显示高级编辑面板；V 模式：显示表达式计算器面板。（功能暂未实现）",
            ToggleSwitch::new()
                .is_on(c.show_more_candidate_items)
                .on_toggled(context.callback(Message::ShowMoreCandidates)),
        ),
        field_top(
            "在下列程序中不显示候选框（使用原始输入）",
            "开启后，在这些程序里输入法完全不接管、按键原样交给应用，候选框与拼音行都不出现，避免遮挡它们自己的补全列表。写 exe 文件名，不区分大小写。",
            program_list(settings, context),
        ),
        field(
            "拼音显示位置",
            "「只在候选窗口」时正在敲的拼音不显示在应用里，终端或行内拼音不正常的应用可以选它。",
            mode_combo(
                &PreeditMode::ALL,
                c.preedit,
                PreeditMode::label,
                context.callback(Message::Preedit),
            ),
        ),
        crate::panel::controls::feedback(&settings.notice),
    ];
    page("候选", StackPanel::new().spacing(16.0).children(rows))
}
