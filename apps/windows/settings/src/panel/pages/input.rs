//! 「输入」页：`[input]` 分节。最上面是输入方案；**选中的方案会在下拉下面生成它自己的一块面板**，
//! 使用简拼 / 模糊音 / 中英文混合输入都放在「全键拼音」那块面板里（别的方案下压根不生成，不用置灰）；
//! 其余各项与方案无关，直接排在后面。

use cloudime_platform::input_schemes::{KeyMapKey, KeyMapModifiers, KeyMapReference, SchemeEntry};
use cloudime_platform::{
    FullHalfPunctuation, InputScheme, MO_HU_YIN_BITS, PAIRWISE_COMPLETION_BITS,
    PUNCTUATION_MAPPING_BITS, SimpTrad,
};
use windows_reactor::*;

use crate::panel::controls::{error_note, field, field_top, note, page, radio_row, scroll_list};
use crate::panel::{Message, Settings};

/// 「辅助码方案」下拉里「不使用」那一项（写进 `use_input_scheme.cfg` 时就是留空）。
const NO_AUXILIARY: &str = "（不使用）";

/// 下拉选中的下标 → 枚举值：越界 / 还没选中时退回缺省（第一项）。
fn pick<T: Copy + Default>(options: &[T], index: Option<usize>) -> T {
    index
        .and_then(|index| options.get(index).copied())
        .unwrap_or_default()
}

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

/// 方案专属面板的卡片外观：卡片底色 + 圆角 + 1 DIP 边框 + 16 DIP 内边距。
///
/// 面板里要排成表格就在 `content` 里用 `Grid` + 子项自己的 `grid_row(i)`；普通竖排 `StackPanel` 就够。
fn scheme_card(content: impl Into<View>) -> View {
    Border::new()
        .background(ThemeBrush::CardBackground)
        .border_brush(ThemeBrush::CardStroke)
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(6.0))
        .padding(Thickness::uniform(16.0))
        .content(content)
}

/// 方案面板的内容：粗体标题 + 若干行（每行自己带 key，跟着方案走）。
fn scheme_body(title: &str, rows: Vec<KeyedView>) -> View {
    let mut children: Vec<KeyedView> = vec![KeyedView::new(
        "scheme-title",
        TextBlock::new()
            .text(title)
            .font_size(14.0)
            .font_weight(FontWeight::SEMI_BOLD),
    )];
    children.extend(rows);
    StackPanel::new().spacing(12.0).keyed_children(children)
}

/// 还没实现的方案：面板里先放一句「待定」。
fn scheme_placeholder(title: &str) -> View {
    scheme_body(
        title,
        vec![KeyedView::new(
            "scheme-todo",
            note("这个方案的选项放在这里（待定）。"),
        )],
    )
}

/// 「双键拼音」面板：选哪一份方案（读 / 写 `use_input_scheme.cfg`）、读不出来的红字、以及这份方案的只读信息。
fn dual_key_panel(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let entries = &settings.scheme_entries;
    let current = settings
        .scheme_current
        .as_ref()
        .and_then(|current| entries.iter().position(|entry| entry.id == *current));
    let mut rows: Vec<KeyedView> = vec![KeyedView::new(
        "scheme-pick",
        field(
            "方案",
            "方案文件放安装目录的 InputSchemes\\DoubleKeysPinyin（.toml 或 .ini；同名只认 toml）。\n\
             改完方案文件、或换了方案，点「应用方案」让它生效。",
            ComboBox::new()
                .items_source(entries.iter().map(SchemeEntry::label))
                .selected_index(current)
                .on_selection_changed(context.callback(Message::DualKeyScheme)),
        ),
    )];
    if entries.is_empty() {
        rows.push(KeyedView::new(
            "scheme-empty",
            error_note("方案目录里还没有 .toml / .ini —— 放一份进去再点「刷新方案」。"),
        ));
    }
    // 读不出来的：选中的那份把原因列全，别的那几份报个名字就够了。
    if let Some(entry) = current.and_then(|index| entries.get(index))
        && !entry.problems().is_empty()
    {
        rows.push(KeyedView::new(
            "scheme-broken",
            error_note(&format!("「{}」读不出来：{}", entry.id, entry.problems())),
        ));
    }
    let broken: Vec<&SchemeEntry> = entries
        .iter()
        .filter(|entry| entry.scheme.is_err())
        .filter(|entry| Some(entry.id.as_str()) != settings.scheme_current.as_deref())
        .collect();
    if !broken.is_empty() {
        let ids: Vec<&str> = broken.iter().map(|entry| entry.id.as_str()).collect();
        rows.push(KeyedView::new(
            "scheme-broken-others",
            error_note(&format!(
                "另有 {} 份读不出来：{}",
                ids.len(),
                ids.join("、")
            )),
        ));
    }
    // 选中那份的只读信息（版本 / 作者 / 零声母 / 原始按键 / 词库 / 辅助码）
    if let Some(scheme) = current
        .and_then(|index| entries.get(index))
        .and_then(|entry| entry.scheme.as_ref().ok())
    {
        rows.push(KeyedView::new(
            "scheme-info",
            note(&format!(
                "版本 {} · 作者 {} · 零声母 {} 组",
                scheme.version,
                scheme.author.as_deref().unwrap_or("未署名"),
                scheme.zero_initials.len(),
            )),
        ));
        rows.push(KeyedView::new(
            "scheme-word-bank",
            note(&format!(
                "词库：{}",
                scheme.word_bank.as_ref().map_or_else(
                    || "软件默认词库".to_owned(),
                    |path| path.display().to_string()
                ),
            )),
        ));
        // 「显示原始输入按键」是配置项（`[input] show_raw_input`），与方案文件无关
        rows.push(KeyedView::new(
            "show-raw-input",
            field(
                "显示原始输入按键",
                "双键拼音下：拼音行显示敲的按键（`ypdo`），没选候选回车也上屏原始按键；\
                 关掉则显示或上屏展开后的完整拼音（`yun'duo`）。",
                ToggleSwitch::new()
                    .is_on(settings.config.input.show_raw_input)
                    .on_toggled(context.callback(Message::ShowRawInput)),
            ),
        ));
        // 「中英文混合输入」：双键拼音自己那一份，写 `use_input_scheme.cfg`（`config.toml` 那份只管全键拼音）
        rows.push(KeyedView::new(
            "scheme-mixture-input",
            field(
                "中英文混合输入",
                "开着才按敲的按键匹配英文词（`hello` → hello；整段英文词 + 前缀补全，不做拼错纠正），\
                 中文候选照旧一起出。关掉一律不匹配。这一项记在方案目录的 use_input_scheme.cfg，不进 config.toml。",
                ToggleSwitch::new()
                    .is_on(settings.scheme_mixture_input)
                    .on_toggled(context.callback(Message::DualKeyMixtureInput)),
            ),
        ));
        // 开关直接改写方案文件里的 `[Auxiliary] UsingAuxiliaryCodes`（方案文件不是 config.toml）
        rows.push(KeyedView::new(
            "scheme-auxiliary-using",
            field(
                "使用辅助码",
                "打满两键后紧接着敲大写字母（最多 4 位）就是那个字的辅助码，逐字对应、只筛选不排序。\
                 第一个键敲了大写（Caps Lock / Shift 卡着）时只把那一个键压成小写，后面的辅助码照旧，当正常双拼打。",
                ToggleSwitch::new()
                    .is_on(scheme.using_auxiliary_codes)
                    .on_toggled(context.callback(Message::AuxiliaryCodes)),
            ),
        ));
        // 用哪份辅助码表是**用户级**的：记进 use_input_scheme.cfg，不写在方案文件里
        let mut auxiliary_items: Vec<&str> = vec![NO_AUXILIARY];
        auxiliary_items.extend(settings.auxiliary_files.iter().map(String::as_str));
        let auxiliary_selected = settings
            .auxiliary_selected
            .as_deref()
            .and_then(|selected| {
                settings
                    .auxiliary_files
                    .iter()
                    .position(|name| name == selected)
            })
            .map_or(0, |index| index + 1);
        let mut auxiliary_hint = String::from(
            "辅助码表统一放 AuxiliaryCodes\\ 下（.tsv / .txt，一行一个「字 + 制表符 + 码」，码里的字母大小写都行，\
             读进来按大写算）；选中的会记进 use_input_scheme.cfg。",
        );
        if let Some(selected) = settings
            .auxiliary_selected
            .as_deref()
            .filter(|_| auxiliary_selected == 0)
        {
            auxiliary_hint.push_str(&format!(
                "（现在找不到「{selected}」—— 把文件放进 AuxiliaryCodes\\，或在这里重选一个。）"
            ));
        }
        rows.push(KeyedView::new(
            "scheme-auxiliary-file",
            field(
                "辅助码方案",
                &auxiliary_hint,
                ComboBox::new()
                    .items_source(auxiliary_items)
                    .selected_index(Some(auxiliary_selected))
                    .on_selection_changed(context.callback(Message::AuxiliaryScheme)),
            ),
        ));
        // 「在候选项显示辅助码」：用户级设置（写 `use_input_scheme.cfg` 的 `candidate_auxiliary`，缺省关）；
        // 只在「使用辅助码」开着、而且选到的那份表读得出来时才生效 —— 否则置灰。
        let auxiliary_effective = scheme.using_auxiliary_codes && auxiliary_selected > 0;
        rows.push(KeyedView::new(
            "scheme-candidate-auxiliary",
            field(
                "在候选项显示辅助码",
                "候选词右边隔一个空格显示辅助码（`爱情 [XH]`），显示的是光标前那个音节对应的那个字的码：\
                 打 `ai|qg` 看「爱」的码，打 `aiqg|` 看「情」的码；候选里对不上那个位置的字（比如单字的「爱」）就不显示。\
                 只在开着辅助码、而且选的那份辅助码表读得出来时有效。这一项记在方案目录的 use_input_scheme.cfg。",
                ToggleSwitch::new()
                    .is_on(settings.candidate_auxiliary)
                    .is_enabled(auxiliary_effective)
                    .on_toggled(context.callback(Message::CandidateAuxiliary)),
            ),
        ));
        // 值写在方案目录的 `use_input_scheme.cfg`（与方案、辅助码方案同一份），不是 config.toml
        let mode_items: Vec<&str> = KeyMapReference::ALL
            .iter()
            .map(|mode| mode.label())
            .collect();
        let mode_selected = KeyMapReference::ALL
            .iter()
            .position(|mode| *mode == settings.key_map_reference)
            .unwrap_or(0);
        rows.push(KeyedView::new(
            "scheme-key-map",
            field(
                "启用键位映射参考",
                "「按下组合键时」：打双拼的过程中按下面那套组合键弹出当前方案的键位映射参考\
                 （哪个键是声母 / 韵母 / 零声母），新手可以对着敲，再按一次收起；\
                 「始终跟随候选窗口」：每回组句都跟候选窗一起出现，不用按键。",
                ComboBox::new()
                    .items_source(mode_items)
                    .selected_index(Some(mode_selected))
                    .on_selection_changed(context.callback(|index: Option<usize>| {
                        Message::KeyMapReference(pick(&KeyMapReference::ALL, index))
                    })),
            ),
        ));
        let modifier_items: Vec<&str> = KeyMapModifiers::ALL
            .iter()
            .map(|modifiers| modifiers.label())
            .collect();
        let key_items: Vec<&str> = KeyMapKey::ALL.iter().map(|key| key.label()).collect();
        rows.push(KeyedView::new(
            "scheme-key-map-hotkey",
            field(
                "键位映射参考组合键",
                "注：部分组合键可能被应用程序或系统占用，导致无法生效。",
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(8.0)
                    .children((
                        ComboBox::new()
                            .items_source(modifier_items)
                            .selected_index(Some(
                                KeyMapModifiers::ALL
                                    .iter()
                                    .position(|modifiers| {
                                        *modifiers == settings.key_map_hotkey.modifiers
                                    })
                                    .unwrap_or(0),
                            ))
                            .on_selection_changed(context.callback(|index: Option<usize>| {
                                Message::KeyMapHotkeyModifiers(pick(&KeyMapModifiers::ALL, index))
                            })),
                        TextBlock::new()
                            .text("+")
                            .vertical_alignment(VerticalAlignment::Center),
                        ComboBox::new()
                            .items_source(key_items)
                            .selected_index(Some(
                                KeyMapKey::ALL
                                    .iter()
                                    .position(|key| *key == settings.key_map_hotkey.key)
                                    .unwrap_or(0),
                            ))
                            .on_selection_changed(context.callback(|index: Option<usize>| {
                                Message::KeyMapHotkeyKey(pick(&KeyMapKey::ALL, index))
                            })),
                    )),
            ),
        ));
    }
    rows.push(KeyedView::new(
        "scheme-buttons",
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .children((
                Button::new()
                    .on_click(context.message(Message::RefreshDualKeySchemes))
                    .content("刷新方案"),
                Button::new()
                    .on_click(context.message(Message::ApplyDualKeyScheme))
                    .content("应用方案"),
                Button::new()
                    .on_click(context.message(Message::OpenDualKeySchemeDir))
                    .content("打开方案目录"),
            )),
    ));
    scheme_body("双键拼音", rows)
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
    // 每个方案各有一块自己的面板，紧跟在下拉后面；只有选中的那块会被生成 —— 框架没导出
    // `Visibility`，「看不见」就是不把它放进 children（`controls::feedback` 同一条路）。
    // 使用简拼 / 模糊音 / 中英文混合输入都是全键拼音才有的东西，就放在它这块面板里：
    // 别的方案下它们压根不存在，不用再逐项置灰。
    let scheme_panel = match input.input_schemes {
        InputScheme::FullPinyin => scheme_card(scheme_body(
            "全键拼音",
            vec![
                KeyedView::new(
                    "use-jian-pin",
                    field(
                        "使用简拼",
                        "中文模式下：打出「YD」即可得到「云朵」，无需打出「YUNDUO」。",
                        ToggleSwitch::new()
                            .is_on(input.use_jian_pin)
                            .on_toggled(context.callback(Message::UseJianPin)),
                    ),
                ),
                KeyedView::new(
                    "fuzzy",
                    field_top(
                        "模糊音",
                        "勾选项的两种读音互用（如「ZE」可以同时匹配「泽」和「折」）。使用模糊音匹配到的候选项排在完全匹配的候选项之后。\
                         如果全部没有勾选，表示关闭模糊音。",
                        scroll_list(fuzzy),
                    ),
                ),
                KeyedView::new(
                    "mixture-input",
                    field(
                        "中英文混合输入",
                        "中文模式下：可以输入单词，出现英文的候选项后，按下反引号（`）可以在全部小写 - 全部大写 - 首字母大写之间循环切换。",
                        ToggleSwitch::new()
                            .is_on(input.mixture_input)
                            .on_toggled(context.callback(Message::MixtureInput)),
                    ),
                ),
            ],
        )),
        InputScheme::DualKeyPinyin => scheme_card(dual_key_panel(settings, context)),
        InputScheme::ShapeCode => scheme_card(scheme_placeholder("形码输入")),
    };
    // 每一项都带 key：面板出现 / 消失会改变子项的个数，而 `children` 是**按位置认身份**的
    // （框架文档：动态集合用 `keyed_children`）—— 按位置的话，面板一插进来，后面每一项都换了
    // 身份、会被重建（丢焦点 / 选中态）。这一页有会变的子项，所以整页都用 `keyed_children`。
    let mut body: Vec<KeyedView> = vec![KeyedView::new(
        "input-scheme",
        field(
            "输入方案",
            "全键拼音按完整拼音打（`yunduo` → 云朵）；双键拼音每个音节固定两键；形码输入按字形编码。\n\
             选中的方案，它自己的设置会在下面出现。",
            ComboBox::new()
                .items_source(InputScheme::ALL.iter().map(|scheme| scheme.label()))
                .selected_index(
                    InputScheme::ALL
                        .iter()
                        .position(|scheme| *scheme == input.input_schemes),
                )
                .on_selection_changed(context.callback(Message::InputScheme)),
        ),
    )];
    // 选中的方案，它自己的面板紧跟在下拉后面。
    body.push(KeyedView::new("input-scheme-panel", scheme_panel));
    body.extend([
        KeyedView::new(
            "simp-trad-row",
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
        ),
        KeyedView::new(
            "full-half-punctuation",
            field(
                "全角 / 半角标点符号",
                "",
                ComboBox::new()
                    .items_source(FullHalfPunctuation::ALL.iter().map(|mode| mode.label()))
                    .selected_index(
                        FullHalfPunctuation::ALL
                            .iter()
                            .position(|mode| *mode == input.full_half_punctuation_marks_toggle),
                    )
                    .on_selection_changed(context.callback(Message::FullHalfPunctuation)),
            ),
        ),
        KeyedView::new(
            "punctuation-mapping",
            field_top(
                "中文模式下符号映射",
                "选中项会进行映射：按按键出现的是对应映射的符号。英文模式不受影响。",
                scroll_list(mapping),
            ),
        ),
        KeyedView::new(
            "pairwise-completion",
            field_top(
                "符号成对补全",
                "选中项左边符号输入时会补上右半边，光标停在中间；再敲一次右半边就跳过去。",
                scroll_list(pairwise),
            ),
        ),
        KeyedView::new(
            "half-wide-after-digit",
            field(
                "数字后标点符号使用半角",
                "开启后，数字后面的标点一律为半角，例如「23:06」「2.36」和「25+9」。",
                ToggleSwitch::new()
                    .is_on(input.use_half_wide_punctuation_marks_after_digital)
                    .on_toggled(context.callback(Message::HalfWideAfterDigit)),
            ),
        ),
        KeyedView::new(
            "status-change-tip",
            field(
                "状态切换提示",
                "中 / 英、大写锁定、全 / 半角、简 / 繁、中文 / 西文标点变化时，在输入光标附近显示一个停留 1 秒的提示条。只在处于输入状态（焦点在可输入的文本框里）时显示。",
                ToggleSwitch::new()
                    .is_on(input.show_status_change_tip)
                    .on_toggled(context.callback(Message::ShowStatusChangeTip)),
            ),
        ),
        KeyedView::new(
            "feedback",
            crate::panel::controls::feedback(&settings.notice),
        ),
    ]);
    let body = StackPanel::new().spacing(16.0).keyed_children(body);
    page("输入", body)
}
