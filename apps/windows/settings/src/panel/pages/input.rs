//! 「输入」页：`[input]` 分节的八项（简拼、模糊音、简繁、中英混输、标点全半角、符号映射、成对补全、数字后半角）。

use cloudime_platform::{
    FullHalfPunctuation, MO_HU_YIN_BITS, PAIRWISE_COMPLETION_BITS, PUNCTUATION_MAPPING_BITS,
    SimpTrad,
};
use windows_reactor::*;

use crate::panel::controls::{field, field_top, page, radio_row, scroll_list};
use crate::panel::{Message, Settings};

/// 一个「复选框（自带文本）」的列表项。
///
/// windows-reactor 没有 XAML 那种 `DataTemplate` / `ItemsSource`，所谓「项目模板」就是在 Rust 里给
/// 每一项建一份 `ListViewItem`；文本直接给 `CheckBox`（它是 `ContentControl`，自带内容区），
/// 不用再套 `TextBlock`。子项带 key，勾选状态变化后重渲染靠 key 找回来原地改，不会丢焦点。
fn check_item(
    key: String,
    label: &str,
    on: bool,
    message: impl Fn(bool) -> Message + 'static,
    context: &mut ViewContext<Settings>,
) -> KeyedView {
    KeyedView::new(
        key,
        ListViewItem::new().content(
            CheckBox::new()
                .is_checked(on)
                .on_is_checked_changed(context.callback(message))
                .content(label),
        ),
    )
}

/// 符号映射的界面写法：`/ → 、`、`小键盘 * → ×`。
fn mapping_label(notation: &str, text: &str) -> String {
    let (keypad, key) = match notation.strip_prefix("{kp}") {
        Some(rest) => (true, rest),
        None => (false, notation),
    };
    let name = if keypad {
        format!("小键盘 {key}")
    } else {
        key.to_owned()
    };
    format!("{name} → {text}")
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let input = &settings.config.input;
    let fuzzy: Vec<KeyedView> = MO_HU_YIN_BITS
        .iter()
        .enumerate()
        .map(|(index, (bit, label))| {
            check_item(
                format!("fuzzy-{index}"),
                label,
                input.mo_hu_yin_list & bit != 0,
                move |value| Message::MoHuYin(index, value),
                context,
            )
        })
        .collect();
    let pairwise: Vec<KeyedView> = PAIRWISE_COMPLETION_BITS
        .iter()
        .enumerate()
        .map(|(index, (bit, label, ..))| {
            check_item(
                format!("pair-{index}"),
                label,
                input.punctuation_marks_pairwise_completion & bit != 0,
                move |value| Message::PairwiseCompletion(index, value),
                context,
            )
        })
        .collect();
    let mapping: Vec<KeyedView> = PUNCTUATION_MAPPING_BITS
        .iter()
        .enumerate()
        .map(|(index, (bit, notation, text))| {
            let label = mapping_label(notation, text);
            check_item(
                format!("map-{index}"),
                &label,
                input.punctuation_marks_mapping & bit != 0,
                move |value| Message::PunctuationMapping(index, value),
                context,
            )
        })
        .collect();
    let body = StackPanel::new().spacing(16.0).children([
        field(
            "使用简拼",
            "中文模式下：打出「YD」即可得到「云朵」，无需打出「YUNDUO」。",
            ToggleSwitch::new()
                .is_on(input.use_jian_pin)
                .on_toggled(context.callback(Message::UseJianPin)),
        ),
        field_top(
            "模糊音",
            "勾选项的两种读音互用（如「ZE」可以同时匹配「泽」和「折」）。使用模糊音匹配到的候选项排在完全匹配的候选项之后。\
             如果全部没有勾选，表示关闭模糊音。",
            scroll_list(fuzzy),
        ),
        field(
            "简体中文 / 繁體中文切换",
            "",
            radio_row(
                "simp-trad",
                SimpTrad::ALL
                    .iter()
                    .map(|mode| (mode.label(), *mode == input.simp_trad_chinese_chars_toggle)),
                Message::SimpTrad,
                context,
            ),
        ),
        field(
            "中英文混合输入",
            "中文模式下：可以输入单词，出现英文的候选项后，按下反引号（`）可以在全部小写 - 全部大写 - 首字母大写之间循环切换。",
            ToggleSwitch::new()
                .is_on(input.mixture_input)
                .on_toggled(context.callback(Message::MixtureInput)),
        ),
        field(
            "全角 / 半角标点符号",
            "",
            ComboBox::new()
                .items_source(
                    FullHalfPunctuation::ALL
                        .iter()
                        .map(|mode| mode.label()),
                )
                .selected_index(
                    FullHalfPunctuation::ALL
                        .iter()
                        .position(|mode| *mode == input.full_half_punctuation_marks_toggle),
                )
                .on_selection_changed(context.callback(Message::FullHalfPunctuation)),
        ),
        field_top(
            "中文模式下符号映射",
            "选中项会进行映射：按按键出现的是对应映射的符号。英文模式不受影响。",
            scroll_list(mapping),
        ),
        field_top(
            "符号成对补全",
            "选中项左边符号输入时会补上右半边，光标停在中间；再敲一次右半边就跳过去。",
            scroll_list(pairwise),
        ),
        field(
            "数字后标点符号使用半角",
            "开启后，数字后面的标点一律为半角，例如「23:06」「2.36」和「25+9」。",
            ToggleSwitch::new()
                .is_on(input.use_half_wide_punctuation_marks_after_digital)
                .on_toggled(context.callback(Message::HalfWideAfterDigit)),
        ),
        field(
            "状态切换提示",
            "中 / 英、大写锁定、全 / 半角、简 / 繁、中文 / 西文标点变化时，在输入光标附近显示一个停留 1 秒的提示条。只在处于输入状态（焦点在可输入的文本框里）时显示。",
            ToggleSwitch::new()
                .is_on(input.show_status_change_tip)
                .on_toggled(context.callback(Message::ShowStatusChangeTip)),
        ),
        crate::panel::controls::feedback(&settings.notice),
    ]);
    page("输入", body)
}
