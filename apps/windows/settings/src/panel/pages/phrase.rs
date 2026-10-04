//! 「短语」页：数据目录下短语库（缺省 `Phrase.db`）里的用户短语（内容 + 输入码 + 固定候选位置）。
//!
//! 配置文件只记短语库的位置（`[phrase] file`）；这一页的增删改都写那个 SQLite 文件，
//! Server 每秒看一次它的 mtime，改完自动生效。表单在列表上方：点某一行的「编辑」把它填进表单。

use cloudime_core::CustomPhrase;
use cloudime_core::custom_phrase::{DEFAULT_POSITION, MAX_POSITION, MIN_POSITION};
use cloudime_platform::PhraseStore;
use windows_reactor::*;

use crate::panel::controls::{field, note, page};
use crate::panel::{Message, Settings};

/// 列表三列的宽度（与表头一致），单位 DIP。
const CONTENT_WIDTH: f64 = 320.0;
const CODE_WIDTH: f64 = 110.0;
const POSITION_WIDTH: f64 = 70.0;

/// 列表里预览文本保留几个字符。
const PREVIEW_CHARS: usize = 24;

/// 表单里正在编辑的一条短语。
#[derive(Clone)]
pub(crate) struct PhraseForm {
    /// 触发字母串。
    pub(crate) code: String,

    /// 原样上屏的文本。
    pub(crate) text: String,

    /// 候选位置（界面上是 `f64`，落盘取整）。
    pub(crate) position: f64,
}

impl Default for PhraseForm {
    fn default() -> Self {
        Self {
            code: String::new(),
            text: String::new(),
            position: f64::from(DEFAULT_POSITION),
        }
    }
}

impl PhraseForm {
    /// 把一条现有短语填进表单。
    pub(crate) fn from_phrase(phrase: &CustomPhrase) -> Self {
        Self {
            code: phrase.code.clone(),
            text: phrase.text.clone(),
            position: f64::from(phrase.position),
        }
    }
}

/// 短语库的位置：数据目录（`%APPDATA%\CloudIME`）加 `[phrase] file`。
pub(crate) fn store(settings: &Settings) -> PhraseStore {
    PhraseStore::locate(settings.data_dir(), &settings.config.phrase)
}

/// 读全部短语；读不出来按空表，并在页面上给出原因。
pub(crate) fn load(store: &PhraseStore) -> (Vec<CustomPhrase>, String) {
    match store.load() {
        Ok(phrases) => (phrases, String::new()),
        Err(error) => (Vec::new(), format!("短语库读不出来：{error}")),
    }
}

/// 保存表单里的这条：编辑中替换原来那条，否则追加；成功后清空表单。
pub(crate) fn save(settings: &mut Settings) {
    let form = settings.phrase_form.clone();
    let candidate = CustomPhrase {
        code: form.code.trim().to_ascii_lowercase(),
        text: form.text.clone(),
        position: form
            .position
            .round()
            .clamp(f64::from(MIN_POSITION), f64::from(MAX_POSITION)) as u32,
    };
    let mut phrases = settings.phrases.clone();
    match settings.phrase_edit {
        Some(index) if index < phrases.len() => phrases[index] = candidate,
        _ => phrases.push(candidate),
    }
    persist(settings, phrases, "已保存");
}

/// 删除第 `index` 条。
pub(crate) fn remove(settings: &mut Settings, index: usize) {
    let mut phrases = settings.phrases.clone();
    let Some(removed) = phrases.get(index).cloned() else {
        return;
    };
    phrases.remove(index);
    let message = format!(
        "已删除「{}」",
        CustomPhrase::preview(&removed.text, PREVIEW_CHARS)
    );
    persist(settings, phrases, &message);
}

/// 写短语库；成功才更新界面状态，失败保留表单内容好让用户改。
fn persist(settings: &mut Settings, phrases: Vec<CustomPhrase>, ok: &str) {
    match store(settings).save(&phrases) {
        Ok(()) => {
            settings.phrases = phrases;
            settings.phrase_form = PhraseForm::default();
            settings.phrase_edit = None;
            settings.phrase_status = format!("{ok}，输入法将自动更新。");
        }
        Err(error) => settings.phrase_status = format!("保存失败：{error}"),
    }
}

/// 表头 / 单元格：固定宽度对齐三列。
fn cell(text: &str, width: f64, bold: bool) -> View {
    let block = TextBlock::new()
        .text(text)
        .width(width)
        .text_wrapping(TextWrapping::NoWrap);
    if bold {
        block.font_weight(FontWeight::SEMI_BOLD).into()
    } else {
        block.into()
    }
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let store = store(settings);
    let mut rows: Vec<KeyedView> = vec![KeyedView::new(
        "header",
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .children((
                cell("短语内容", CONTENT_WIDTH, true),
                cell("触发字母串", CODE_WIDTH, true),
                cell("位置", POSITION_WIDTH, true),
            )),
    )];
    if settings.phrases.is_empty() {
        rows.push(KeyedView::new(
            "empty",
            note("还没有短语，用下面的表单加一条。"),
        ));
    }
    for (index, phrase) in settings.phrases.iter().enumerate() {
        rows.push(KeyedView::new(
            format!("phrase-{index}"),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(12.0)
                .children((
                    cell(
                        &CustomPhrase::preview(&phrase.text, PREVIEW_CHARS),
                        CONTENT_WIDTH,
                        false,
                    ),
                    cell(&phrase.code, CODE_WIDTH, false),
                    cell(&phrase.position.to_string(), POSITION_WIDTH, false),
                    Button::new()
                        .on_click(context.message(Message::PhraseEdit(index)))
                        .content("编辑"),
                    Button::new()
                        .on_click(context.message(Message::PhraseRemove(index)))
                        .content("删除"),
                )),
        ));
    }

    let editing = settings.phrase_edit.is_some();
    let mut buttons: Vec<KeyedView> = vec![KeyedView::new(
        "phrase-save",
        Button::new()
            .on_click(context.message(Message::PhraseSave))
            .content(if editing {
                "保存修改"
            } else {
                "添加短语"
            }),
    )];
    if editing {
        buttons.push(KeyedView::new(
            "phrase-cancel",
            Button::new()
                .on_click(context.message(Message::PhraseCancel))
                .content("取消"),
        ));
    }

    let form = settings.phrase_form.clone();
    let body = StackPanel::new().spacing(12.0).children([
        note(&format!(
            "短语单独存在 {}，配置文件只记它的位置。输入码敲全时短语出现在你指定的候选位置（0 第一位、1 第二位……），同一位置的多条按保存顺序排；保存后输入法自动更新。",
            store.path.display()
        )),
        StackPanel::new().spacing(4.0).keyed_children(rows),
        field(
            "短语内容",
            "原样上屏的文本，可以有多行。",
            TextBox::new()
                .width(CONTENT_WIDTH + 100.0)
                .accepts_return(true)
                .text_wrapping(TextWrapping::Wrap)
                .text(form.text.clone())
                .on_text_changed(context.callback(Message::PhraseText)),
        ),
        field(
            "触发字母串",
            "1–32 个小写英文字母；输入码敲全时才出这条短语。",
            TextBox::new()
                .width(200.0)
                .text(form.code.clone())
                .placeholder_text("例如 ww")
                .on_text_changed(context.callback(Message::PhraseCode)),
        ),
        field(
            "候选位置",
            &format!("固定在候选窗口的第几位：{MIN_POSITION} 是第一位、{DEFAULT_POSITION} 是第二位……；超出候选数就排到最后。"),
            NumberBox::new()
                .minimum(f64::from(MIN_POSITION))
                .maximum(f64::from(MAX_POSITION))
                .value(form.position)
                .on_value_changed(context.callback(Message::PhrasePosition)),
        ),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .keyed_children(buttons),
        note(&settings.phrase_status),
    ]);
    page("短语", body)
}
