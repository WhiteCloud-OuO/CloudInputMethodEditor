//! 设置窗口的消息类型：导航切换与各页的「改动」，根组件的 `update` 据此落盘。

/// 设置窗口的消息；「改动」消息带控件新值，`update` 据此落盘。
#[derive(Clone)]
pub(crate) enum Message {
    /// 导航切换分节（`None` 是取消选中，忽略）。
    Navigate(Option<String>),

    // 输入页
    UseJianPin(bool),
    /// 模糊音位下标（见 `MO_HU_YIN_BITS`）+ 新值。
    MoHuYin(usize, bool),
    SimpTrad(Option<usize>),
    MixtureInput(bool),
    FullHalfPunctuation(Option<usize>),
    /// 成对补全的位下标（见 `PAIRWISE_COMPLETION_BITS`）+ 新值。
    PairwiseCompletion(usize, bool),
    /// 符号映射的位下标（见 `PUNCTUATION_MAPPING_BITS`）+ 新值。
    PunctuationMapping(usize, bool),
    HalfWideAfterDigit(bool),
    /// 状态切换提示（`[input] show_status_change_tip`）。
    ShowStatusChangeTip(bool),

    // 候选页
    /// 本地整句模型开关（`[candidate] use_local_sentence_organization_model`）。
    LocalModel(bool),
    /// 候选项排布方向（`LayoutMode::ALL` 的下标）。
    Arrangement(Option<usize>),
    /// 候选项个数（滑轨）。
    CandidateCount(f64),
    /// 联想候选项目上限（滑轨）。
    AssociationCounts(f64),
    /// 点了某个字体的「字体…」按钮：弹系统字体对话框。
    PickFont(super::pages::candidates::FontRole),
    /// 候选项序号样式（`ItemNumberStyle::ALL` 的下标）。
    ItemNumberStyle(Option<usize>),
    /// 候选框最小宽度（物理像素）。
    CandidateBoxMinimumWidth(Option<f64>),
    /// 展示更多候选项（功能暂未实现，只落配置）。
    ShowMoreCandidates(bool),
    /// 使用鼠标选词（`MouseWordSelection::ALL` 的下标）。
    MouseWordSelection(Option<usize>),
    /// 展开后每个候选项的最大宽度（物理像素，0 表示不限）。
    CandidateItemMaximumWidth(Option<f64>),
    /// 请正在跑的输入法服务重启（脚本改动生效用，与任务栏右键「重启输入法服务」同一条路）。
    RestartServer,
    /// 「不显示候选框」程序名单的输入框。
    ProgramQuery(String),
    /// 把输入框里的程序名加进名单。
    ProgramAdd,
    /// 从名单里移除一个程序。
    ProgramRemove(String),
    /// 拼音显示位置。
    Preedit(Option<usize>),

    // 词库页
    /// 把一个词库挪进 `removed\`。
    RemoveWordBank(String),
    /// 从词库中查询生僻项条目（`[word_bank] rare_items`）。
    RareItems(bool),
    ImportDictionary,

    // 短语页
    /// 启用软件自带短语（`[phrase] use_default_phrases`）。
    UseDefaultPhrases(bool),
    /// 表单里触发字母串的输入。
    PhraseCode(String),
    /// 表单里短语内容的输入。
    PhraseText(String),
    /// 表单里候选显示内容（title）的输入。
    PhraseTitle(String),
    /// 表单里候选位置的输入。
    PhrasePosition(Option<f64>),
    /// 添加一条 / 保存修改中的那条。
    PhraseSave,
    /// 取消编辑，清空表单。
    PhraseCancel,
    /// 把第 `index` 条填进表单。
    PhraseEdit(usize),
    /// 删除第 `index` 条。
    PhraseRemove(usize),

    // 翻译页
    /// 启用翻译 Tip（`[translate] enabled`）。
    TranslateEnabled(bool),
    /// 选中的本地词典（清单里的下标）。
    TranslateDictionary(Option<usize>),
    /// 学会所需上屏次数（滑轨）。
    TranslateNeedTimes(f64),
    /// 重置这份词典的学习内容（`[translate] reset_counter` 加 1，Server 见到就清）。
    ResetTranslateLearning,

    // 脚本页
    /// 启用 / 禁用某个脚本（文件名，写 `[script] disabled`）。
    ScriptToggle(String, bool),
    /// 删除某个脚本文件。
    ScriptRemove(String),
    /// 用记事本打开某个脚本。
    ScriptEdit(String),
    /// 新建一个脚本：建文件、写模板、用记事本打开。
    ScriptNew,

    // 主题页
    /// 选中的主题（下拉下标；`None` 是取消选中，忽略）。
    ThemeSelect(Option<usize>),
    /// 重新扫描 `Themes\` 两个目录。
    ThemeRefresh,
    /// 从默认主题复制一份到草稿。
    ThemeNew,
    /// 「新建主题」名字框的输入。
    ThemeNewName(String),
    /// 把草稿写到用户目录（只存主题，不换到它）。
    ThemeSave,
    /// 把选中的主题写进 `[theme] curr_theme`（触发热加载，换到它）。
    ThemeApply,
    /// 挑一个 `.json` 主题文件导入到用户主题目录，成功后刷新列表。
    ThemeImport,
    /// 把当前主题导出到用户挑的位置。
    ThemeExport,
    /// 点了某个颜色的 `#aarrggbb`：弹颜色对话框改它。
    ThemeColorOpen(super::pages::theme::Slot),
    /// 颜色对话框里改了色。
    ThemeColorChanged(windows_reactor::Color),
    /// 颜色对话框关了（`Primary` 是「确定」）。
    ThemeColorClosed(windows_reactor::ContentDialogResult),

    // 调试页：文件 / 日志 / 学习（原「高级」页）
    VerboseLog(bool),
    InputLog(bool),
    /// 学习输入习惯开关。
    Learning(bool),
    OpenDataDir,
    OpenLogDir,
    /// 日志目录 + config.toml 打成 zip 放桌面。
    ExportLogs,
    ClearInputLog,
    /// 打开项目 GitHub 页面。
    OpenRepository,
    /// 用系统默认程序打开随包的使用手册（`tutorial.md`）。
    OpenTutorial,

    // 调试页
    /// 在屏幕上显示悬浮工具栏（`[status_bar] show_status_bar`）。
    ShowStatusBar(bool),
    /// 自动隐藏悬浮工具栏（`[debugging] auto_hide_float_tool_bar`）。
    AutoHideFloatToolBar(bool),
    /// 不处于输入状态时自动禁用输入法（`[debugging] auto_disable_without_text_input`）。
    AutoDisableWithoutTextInput(bool),
    /// 「组件」入口：功能还没做，只记一条日志。设置里已不显示这个入口、代码留着，所以允许未构造。
    #[allow(dead_code)]
    OpenComponents,
}
