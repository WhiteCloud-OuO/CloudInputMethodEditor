//! 根组件的 Reactor 生命周期：建状态、按消息落盘、画左侧导航 + 当前页。

use std::path::{Path, PathBuf};

use cloudime_platform::{
    Config, FullHalfPunctuation, ItemNumberStyle, LayoutMode, LogLevel, MAX_ASSOCIATION_COUNTS,
    MAX_CANDIDATE_COUNT, MIN_ASSOCIATION_COUNTS, MIN_CANDIDATE_COUNT, MO_HU_YIN_BITS,
    PAIRWISE_COMPLETION_BITS, PUNCTUATION_MAPPING_BITS, PreeditMode, SimpTrad, UpdateChannel,
};
use windows_reactor::*;

use super::controls::{export_logs, log_dir, open_in_editor, open_with_explorer};
use super::font_dialog;
use super::notice::Notice;
use super::pages::phrase::PhraseForm;
use super::pages::{about, dictionaries, phrase};
use super::{Message, Settings};

impl Component for Settings {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), _context: &ComponentContext<Self>) -> Self {
        let path = Self::config_path();
        Self::ensure_config_file(&path);
        // 旧版配置先迁一遍（幂等）；设置程序也可能在 Server 之前启动
        let migration = cloudime_platform::migrate::migrate(&path);
        if !migration.is_empty() {
            crate::log::info(format!(
                "旧版配置与短语已迁移：配置 {}，短语 {}",
                migration.config, migration.phrases
            ));
        }
        let config = Config::load(&path).unwrap_or_default();
        let data_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let (phrases, phrase_status) = phrase::load(&cloudime_platform::PhraseStore::locate(
            &data_dir,
            &config.phrase,
        ));
        Self {
            config,
            path,
            page: "input".to_string(),
            notice: Notice::default(),
            update_state: Self::update_state_path()
                .map(|path| cloudime_update::UpdateState::load(&path))
                .unwrap_or_default(),
            update_checking: false,
            update_error: None,
            dictionary_status: String::new(),
            program_query: None,
            phrases,
            phrase_form: PhraseForm::default(),
            phrase_edit: None,
            phrase_status,
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Navigate(Some(tag)) => {
                self.page = tag;
                // 上一页的导入提示不跟着过来
                self.notice.clear();
            }
            Message::Navigate(None) => {}

            // 输入页
            Message::UseJianPin(on) => self.save("input", "use_jian_pin", on),
            Message::MoHuYin(index, on) => {
                if let Some((bit, _)) = MO_HU_YIN_BITS.get(index) {
                    let mut list = self.config.input.mo_hu_yin_list;
                    if on {
                        list |= bit;
                    } else {
                        list &= !bit;
                    }
                    self.save("input", "mo_hu_yin_list", i64::from(list));
                }
            }
            Message::SimpTrad(Some(i)) if i < SimpTrad::ALL.len() => {
                self.save(
                    "input",
                    "simp_trad_chinese_chars_toggle",
                    SimpTrad::ALL[i].key(),
                );
            }
            Message::MixtureInput(on) => self.save("input", "mixture_input", on),
            Message::FullHalfPunctuation(Some(i)) if i < FullHalfPunctuation::ALL.len() => {
                self.save(
                    "input",
                    "full_half_punctuation_marks_toggle",
                    FullHalfPunctuation::ALL[i].key(),
                );
            }
            Message::PairwiseCompletion(index, on) => {
                if let Some((bit, ..)) = PAIRWISE_COMPLETION_BITS.get(index) {
                    let mut mask = self.config.input.punctuation_marks_pairwise_completion;
                    if on {
                        mask |= bit;
                    } else {
                        mask &= !bit;
                    }
                    self.save(
                        "input",
                        "punctuation_marks_pairwise_completion",
                        i64::from(mask),
                    );
                }
            }
            Message::PunctuationMapping(index, on) => {
                if let Some((bit, ..)) = PUNCTUATION_MAPPING_BITS.get(index) {
                    let mut mask = self.config.input.punctuation_marks_mapping;
                    if on {
                        mask |= bit;
                    } else {
                        mask &= !bit;
                    }
                    self.save("input", "punctuation_marks_mapping", i64::from(mask));
                }
            }
            Message::HalfWideAfterDigit(on) => {
                self.save("input", "use_half_wide_punctuation_marks_after_digital", on)
            }

            // 候选页
            Message::LocalModel(on) => {
                self.save("candidate", "use_local_sentence_organization_model", on);
            }
            Message::Arrangement(Some(i)) if i < LayoutMode::ALL.len() => {
                self.save(
                    "candidate",
                    "candidate_arrangement_direction",
                    LayoutMode::ALL[i].key(),
                );
            }
            Message::CandidateCount(value) => {
                let count = (value.round() as i64)
                    .clamp(MIN_CANDIDATE_COUNT as i64, MAX_CANDIDATE_COUNT as i64);
                self.save("candidate", "candidate_count", count);
            }
            Message::AssociationCounts(value) => {
                let count = (value.round() as i64)
                    .clamp(MIN_ASSOCIATION_COUNTS as i64, MAX_ASSOCIATION_COUNTS as i64);
                self.save("candidate", "candidate_association_counts", count);
            }
            Message::PickFont(role) => {
                let current = role.current(&self.config.candidate).clone();
                if let Some(choice) = font_dialog::pick_font(&current) {
                    self.save_font(role, &choice);
                }
            }
            Message::ItemNumberStyle(Some(i)) if i < ItemNumberStyle::ALL.len() => {
                self.save(
                    "candidate",
                    "item_number_style",
                    ItemNumberStyle::ALL[i].key(),
                );
            }
            Message::CandidateBoxMinimumWidth(Some(value)) => {
                let width = (value.round() as i64).clamp(0, 2000);
                self.save("candidate", "candidate_box_minimum_width", width);
            }
            Message::ShowMoreCandidates(on) => {
                self.save("candidate", "show_more_candidate_items", on);
            }
            Message::ProgramQuery(text) => self.program_query = Some(text),
            Message::ProgramAdd => {
                let name = self
                    .program_query
                    .take()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if !name.is_empty() {
                    let mut programs = self
                        .config
                        .candidate
                        .program_list_of_hiding_candidate
                        .clone();
                    if !self.config.candidate.hides_candidate_for(&name) {
                        programs.push(name);
                        self.save_array("candidate", "program_list_of_hiding_candidate", &programs);
                    }
                }
            }
            Message::ProgramRemove(name) => {
                let mut programs = self
                    .config
                    .candidate
                    .program_list_of_hiding_candidate
                    .clone();
                programs.retain(|program| !program.eq_ignore_ascii_case(&name));
                self.save_array("candidate", "program_list_of_hiding_candidate", &programs);
            }
            Message::Preedit(Some(i)) if i < PreeditMode::ALL.len() => {
                self.save("candidate", "preedit", PreeditMode::ALL[i].key());
            }

            // 词库页
            Message::RemoveWordBank(file) => dictionaries::remove(self, &file),
            Message::RareItems(on) => self.save("word_bank", "rare_items", on),
            Message::ImportDictionary => {
                dictionaries::import(self);
                self.reload();
            }

            // 短语页
            Message::PhraseCode(text) => self.phrase_form.code = text,
            Message::PhraseText(text) => self.phrase_form.text = text,
            Message::PhrasePosition(Some(value)) => self.phrase_form.position = value,
            Message::PhraseSave => phrase::save(self),
            Message::PhraseCancel => {
                self.phrase_form = PhraseForm::default();
                self.phrase_edit = None;
            }
            Message::PhraseEdit(index) => {
                if let Some(selected) = self.phrases.get(index) {
                    self.phrase_form = PhraseForm::from_phrase(selected);
                    self.phrase_edit = Some(index);
                    self.phrase_status.clear();
                }
            }
            Message::PhraseRemove(index) => phrase::remove(self, index),

            // 调试页（文件 / 日志 / 学习那几项，原「高级」页）
            Message::VerboseLog(on) => {
                let level = if on { LogLevel::Debug } else { LogLevel::Info };
                self.save("general", "log_level", level.key());
            }
            Message::InputLog(on) => self.save("general", "input_log", on),
            Message::Learning(on) => self.save("general", "learning", on),
            Message::OpenConfigFile => {
                Self::ensure_config_file(&self.path);
                open_in_editor(&self.path);
            }
            Message::OpenDataDir => {
                Self::ensure_config_file(&self.path);
                open_with_explorer(&self.data_dir().to_string_lossy());
            }
            Message::OpenLogDir => {
                if let Some(logs) = log_dir() {
                    open_with_explorer(&logs.to_string_lossy());
                }
            }
            Message::ExportLogs => export_logs(),
            Message::ClearInputLog => {
                let log = self.data_dir().join("input-log.jsonl");
                if let Err(error) = std::fs::remove_file(&log)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    crate::log::warn(format!("清空输入日志失败: {error}"));
                }
            }

            // 关于页
            Message::OpenWebsite => open_with_explorer(about::WEBSITE_URL),
            Message::OpenDownload => open_with_explorer(cloudime_update::DOWNLOAD_URL),

            // 关于页：检查更新
            Message::UpdateCheck(on) => self.save("update", "check", on),
            Message::UpdateChannel(Some(i)) if i < UpdateChannel::ALL.len() => {
                self.save("update", "channel", UpdateChannel::ALL[i].key());
            }
            Message::CheckUpdateNow => {
                let Some(path) = Self::update_state_path() else {
                    return;
                };
                if self.update_checking {
                    return;
                }
                self.update_checking = true;
                self.update_error = None;
                let config = self.config.update.clone();
                context.spawn_background(move |_cancel| {
                    let result =
                        cloudime_update::Checker::check_blocking(&path, about::VERSION, &config);
                    Message::UpdateChecked(result.map(|r| r.map_err(|error| error.to_string())))
                });
            }
            Message::UpdateChecked(result) => {
                self.update_checking = false;
                match result {
                    Some(Ok(state)) => self.update_state = state,
                    Some(Err(error)) => self.update_error = Some(error),
                    None => {}
                }
            }
            Message::OpenRepository => open_with_explorer(about::REPOSITORY_URL),

            // 调试页
            Message::AutoHideFloatToolBar(on) => {
                self.save("debugging", "auto_hide_float_tool_bar", on);
            }
            Message::OpenComponents => {
                crate::log::info("「组件」页还没有做，点了只记一条日志");
            }

            // 下拉被清空 / 越界：不改
            _ => {}
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title("云朵设置");
        let item = |tag: &str, label: &str, symbol| {
            KeyedView::new(
                tag,
                NavigationViewItem::new()
                    .tag(tag)
                    .is_selected(self.page == tag)
                    .slots([
                        SlotView::new(
                            NavigationViewItemSlot::Icon,
                            SymbolIcon::new().symbol(symbol),
                        ),
                        SlotView::new(NavigationViewItemSlot::Content, label),
                    ]),
            )
        };
        let items = [
            item("input", "输入", Symbol::Keyboard),
            item("candidates", "候选", Symbol::View),
            item("dictionaries", "词库", Symbol::Library),
            item("phrase", "短语", Symbol::Comment),
            item("debugging", "调试", Symbol::Repair),
            item("about", "关于", Symbol::Help),
        ];
        NavigationView::new()
            .pane_display_mode(NavigationViewPaneDisplayMode::Left)
            .pane_title("云朵输入法")
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_pane_toggle_button_visible(false)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .is_settings_visible(false)
            .on_selected_tag_changed(context.callback(Message::Navigate))
            .slots([
                SlotView::collection(NavigationViewSlot::MenuItems, items),
                SlotView::new(NavigationViewSlot::Content, self.page_content(context)),
            ])
    }
}
