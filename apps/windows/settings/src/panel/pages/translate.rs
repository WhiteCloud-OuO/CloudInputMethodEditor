//! 「翻译」页：本地词典的翻译 Tip（开关、选词典、学会所需上屏次数、重置学习内容、使用说明）。

use cloudime_platform::{MAX_NEED_TIMES, MIN_NEED_TIMES};
use cloudime_translate::Manifest;
use windows_reactor::*;

use crate::panel::controls::{field, field_top, note, page, slider_field};
use crate::panel::{Message, Settings};

/// 使用说明：按定稿的措辞照抄。
const USAGE: &str = "\
翻译功能开启时，输入法会从对应词典中匹配项目，并在候选窗口左下角的位置输出。
按下Ctrl+反引号（`）可选中翻译Tip的内容。如果一个候选项对应多个翻译，则还会要求选择释义项。例如：
候选项为“悲伤的”，对应释义为
悲伤的，adj. sad; n. sorrow
此时按下Ctrl+反引号（`）会让候选窗口暂时变为
悲伤的
① sad (adj.)
② sorrow (n.)
此时再按对应数字键方可选中候选。";

/// 安装目录 `LocalDictionary\` 的清单（与 Server 同一份来源；开发时是仓库根那份）。
pub(crate) fn manifest(settings: &Settings) -> Manifest {
    let root = cloudime_platform::resources::bundled_root()
        .unwrap_or_else(|| settings.data_dir().to_path_buf());
    Manifest::load(root.join("LocalDictionary"))
}

/// 「本地词典」下拉：选项来自清单里的显示名，存的是文件名。
fn dictionary_field(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let manifest = manifest(settings);
    if manifest.is_empty() {
        return field(
            "本地词典",
            "没有找到本地词典：安装目录 `LocalDictionary\\` 下要有词典文件与清单 `dictionaries.list`（一行 `显示名=文件名`）。",
            note("（没有可选的词典）"),
        );
    }
    let selected = manifest
        .items()
        .iter()
        .position(|item| item.file == settings.config.translate.dictionary);
    let names: Vec<&str> = manifest
        .items()
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    let combo = ComboBox::new()
        .items_source(names)
        .selected_index(selected.unwrap_or(0))
        .on_selection_changed(context.callback(Message::TranslateDictionary));
    field(
        "本地词典",
        "选项来自安装目录 `LocalDictionary\\dictionaries.list`；单个词典文件可以是 `.qj` 或 `.db`。",
        combo,
    )
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let translate = &settings.config.translate;
    let need_times = translate.need_times();
    let rows = [
        field(
            "启用翻译 Tip",
            "候选窗口底部那一行的左侧显示高亮候选在本地词典里的释义；按 Ctrl + 反引号 把译文上屏。",
            ToggleSwitch::new()
                .is_on(translate.enabled)
                .on_toggled(context.callback(Message::TranslateEnabled)),
        ),
        dictionary_field(settings, context),
        slider_field(
            "学会所需上屏次数",
            "一个词条的译文上屏这么多次之后算「学会」，Tip 的颜色跟着变；缺省 3。",
            need_times as usize,
            MIN_NEED_TIMES as usize,
            MAX_NEED_TIMES as usize,
            Message::TranslateNeedTimes,
            context,
        ),
        field(
            "重置学习内容",
            "把这份词典里记过的词条全部清掉：上屏次数回到 0、重新按「没学会」上色。",
            Button::new()
                .on_click(context.callback(|_| Message::ResetTranslateLearning))
                .content("重置"),
        ),
        field_top("使用说明", "", note(USAGE)),
        crate::panel::controls::feedback(&settings.notice),
    ];
    page("翻译", StackPanel::new().spacing(16.0).children(rows))
}
