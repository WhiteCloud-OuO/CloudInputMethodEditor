//! 「输入」页：`[input]` 分节的八项（简拼、模糊音、简繁、中英混输、标点全半角、符号映射、成对补全、数字后半角）。

use cloudime_platform::{
    FullHalfPunctuation, MO_HU_YIN_BITS, PAIRWISE_COMPLETION_BITS, PUNCTUATION_MAPPING_BITS,
    SimpTrad,
};
use windows_reactor::*;

use crate::panel::controls::{field, page};
use crate::panel::{Message, Settings};

/// 勾选一个位（`width` 按这一组一行放几个来定）。
fn check_cell(
    stem: &str,
    label: &str,
    index: usize,
    on: bool,
    width: f64,
    message: impl Fn(usize, bool) -> Message + 'static,
    context: &mut ViewContext<Settings>,
) -> KeyedView {
    KeyedView::new(
        format!("{stem}-{index}"),
        CheckBox::new()
            .is_checked(on)
            .on_is_checked_changed(context.callback(move |value| message(index, value)))
            .width(width)
            .content(label),
    )
}

/// 每行 `per_row` 个地把勾选排开。
fn check_grid(mut cells: Vec<KeyedView>, per_row: usize) -> View {
    let mut rows: Vec<KeyedView> = Vec::new();
    while !cells.is_empty() {
        let take = cells.len().min(per_row);
        let row: Vec<KeyedView> = cells.drain(..take).collect();
        let stem = format!("row-{}", rows.len());
        rows.push(KeyedView::new(
            stem,
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(12.0)
                .keyed_children(row),
        ));
    }
    StackPanel::new().spacing(8.0).keyed_children(rows)
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
            check_cell(
                "fuzzy",
                label,
                index,
                input.mo_hu_yin_list & bit != 0,
                190.0,
                Message::MoHuYin,
                context,
            )
        })
        .collect();
    let pairwise: Vec<KeyedView> = PAIRWISE_COMPLETION_BITS
        .iter()
        .enumerate()
        .map(|(index, (bit, label, ..))| {
            check_cell(
                "pair",
                label,
                index,
                input.punctuation_marks_pairwise_completion & bit != 0,
                120.0,
                Message::PairwiseCompletion,
                context,
            )
        })
        .collect();
    let mapping: Vec<KeyedView> = PUNCTUATION_MAPPING_BITS
        .iter()
        .enumerate()
        .map(|(index, (bit, notation, text))| {
            let label = mapping_label(notation, text);
            check_cell(
                "map",
                &label,
                index,
                input.punctuation_marks_mapping & bit != 0,
                130.0,
                Message::PunctuationMapping,
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
        field(
            "模糊音",
            "勾选项的两种读音互用（如「ZE」可以同时匹配「泽」和「折」）。使用模糊音匹配到的候选项排在完全匹配的候选项之后。\
             如果全部没有勾选，表示关闭模糊音。",
            check_grid(fuzzy, 3),
        ),
        field(
            "简体中文 / 繁體中文切换",
            "",
            RadioButtons::new()
                .items_source(SimpTrad::ALL.iter().map(|mode| mode.label()))
                .selected_index(
                    SimpTrad::ALL
                        .iter()
                        .position(|mode| *mode == input.simp_trad_chinese_chars_toggle),
                )
                .on_selection_changed(context.callback(Message::SimpTrad)),
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
        field(
            "中文模式下符号映射",
            "选中项会进行映射：按按键出现的是对应映射的符号。英文模式不受影响。",
            check_grid(mapping, 5),
        ),
        field(
            "符号成对补全",
            "选中项左边符号输入时会补上右半边，光标停在中间；再敲一次右半边就跳过去。",
            check_grid(pairwise, 5),
        ),
        field(
            "数字后标点符号使用半角",
            "开启后，数字后面的标点一律为半角，例如「23:06」「2.36」和「25+9」。",
            ToggleSwitch::new()
                .is_on(input.use_half_wide_punctuation_marks_after_digital)
                .on_toggled(context.callback(Message::HalfWideAfterDigit)),
        ),
        crate::panel::controls::feedback(&settings.notice),
    ]);
    page("输入", body)
}
