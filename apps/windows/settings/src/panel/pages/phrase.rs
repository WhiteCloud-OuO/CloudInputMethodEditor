//! 「短语」页：安装目录 `Phrases\Phrase.db` 里用户自己的短语（内容 + 候选显示 + 输入码 + 候选位置）。
//!
//! 页最上方是「启用软件自带短语」开关（写 `[phrase] use_default_phrases`）；这一页的增删改都写短语库的 `user` 表，
//! Server 每秒看一次文件的 mtime，改完自动生效。表单在列表上方：点某一行的「编辑」把它填进表单。

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

    /// 候选里显示的内容；留空时候选显示短语内容。
    pub(crate) title: String,

    /// 候选位置（界面上是 `f64`，落盘取整）。
    pub(crate) position: f64,
}

impl Default for PhraseForm {
    fn default() -> Self {
        Self {
            code: String::new(),
            text: String::new(),
            title: String::new(),
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
            title: phrase.title.clone().unwrap_or_default(),
            position: f64::from(phrase.position),
        }
    }
}

/// 短语库的安装根：随包根；拿不到退回数据目录（只可能出现在开发或异常环境）。
pub(crate) fn root(settings: &Settings) -> std::path::PathBuf {
    cloudime_platform::resources::bundled_root()
        .unwrap_or_else(|| settings.data_dir().to_path_buf())
}

/// 短语库的位置：安装目录下的 `Phrases\Phrase.db`。
pub(crate) fn store(settings: &Settings) -> PhraseStore {
    PhraseStore::locate(&root(settings))
}

/// 读用户短语（不含软件自带那份）；读不出来按空表，并在页面上给出原因。
pub(crate) fn load(store: &PhraseStore) -> (Vec<CustomPhrase>, String) {
    match store.load(false) {
        Ok(phrases) => (phrases, String::new()),
        Err(error) => (Vec::new(), format!("短语库读不出来：{error}")),
    }
}

/// 保存表单里的这条：编辑中替换原来那条，否则追加；成功后清空表单。
pub(crate) fn save(settings: &mut Settings) {
    let form = settings.phrase_form.clone();
    let title = form.title.trim();
    let candidate = CustomPhrase {
        code: form.code.trim().to_ascii_lowercase(),
        text: form.text.clone(),
        title: (!title.is_empty()).then(|| title.to_owned()),
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

/// 写短语库的 `user` 表；成功才更新界面状态，失败保留表单内容好让用户改。
fn persist(settings: &mut Settings, phrases: Vec<CustomPhrase>, ok: &str) {
    match store(settings).save_user(&phrases) {
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
        let content = match &phrase.title {
            Some(title) => format!(
                "{}（上屏：{}）",
                CustomPhrase::preview(title, PREVIEW_CHARS),
                CustomPhrase::preview(&phrase.text, PREVIEW_CHARS)
            ),
            None => CustomPhrase::preview(&phrase.text, PREVIEW_CHARS),
        };
        rows.push(KeyedView::new(
            format!("phrase-{index}"),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(12.0)
                .children((
                    cell(&content, CONTENT_WIDTH, false),
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
        field(
            "启用软件自带短语",
            "随安装包带的一份常用短语参与出候选；关掉只用自己的短语。自带的始终在库里，不占下面的列表。",
            ToggleSwitch::new()
                .is_on(settings.config.phrase.use_default_phrases)
                .on_toggled(context.callback(Message::UseDefaultPhrases)),
        ),
        note(&format!(
            "短语库固定在 {}。输入码敲全时短语出现在你指定的候选位置（1 第一位、2 第二位……），同一位置的多条按保存顺序排；保存后输入法自动更新。",
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
            "候选显示内容",
            "留空时候选里显示短语内容；填了则候选里显示它，上屏的仍是短语内容。",
            TextBox::new()
                .width(200.0)
                .text(form.title.clone())
                .placeholder_text("可留空")
                .on_text_changed(context.callback(Message::PhraseTitle)),
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
