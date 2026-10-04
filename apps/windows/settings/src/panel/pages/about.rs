//! 「关于」页：版本、检查更新、许可证、随包数据的来源与署名（第三方许可要求署名在分发物里可见）、隐私与反馈。

use cloudime_platform::UpdateChannel;
use cloudime_update::Checker;
use windows_reactor::*;

use crate::panel::controls::{field, note, page};
use crate::panel::{Message, Settings};

/// CLOUDIME_VERSION 由 build.rs 给：-dev 版接 git 短哈希。
pub(crate) const VERSION: &str = env!("CLOUDIME_VERSION");

pub(crate) const WEBSITE_URL: &str = "https://cloudime.app";

pub(crate) const REPOSITORY_URL: &str = "https://github.com/WhiteCloud-OuO/CloudInputMethodEditor";

/// 与仓库根 `LICENSE` 一致。
const LICENSE_NOTE: &str = "自由软件，GPL-3.0-or-later 许可证：可以自由使用、修改与再分发，修改后分发须同样开源。官方渠道免费。";

/// 数据署名与许可，和 `assets/*/README.md`、`docs/design/landscape.md` 一致。
const ATTRIBUTIONS: &[(&str, &str)] = &[
    (
        "词库",
        "通用规范汉字表；现代汉语常用词表（liuxilu 校对版）；THUOCL（清华大学自然语言处理实验室，MIT）；读音取自 Unihan（Unicode License v3）。",
    ),
    (
        "语言模型",
        "中文维基百科（CC BY-SA 4.0）与 LCCC（清华大学 CoAI，MIT）语料统计。",
    ),
    (
        "英文词表",
        "ESDB / SCOWL（© Kevin Atkinson，按其许可保留版权声明）；CSpell 词典（MIT）。",
    ),
];

const PRIVACY_NOTE: &str = "云朵输入法不上传任何数据。开着「自动检查更新」时每天向官网读一次版本列表，请求不带任何标识，上面可以关。开着云联想时，光标附近的文字与拼音会发给你在「云服务」页填的 AI 服务商（缺省 DeepSeek）的服务器，不经过作者。「调试」页的输入日志只写在本机，可以关掉或清空。";

const FEEDBACK_NOTE: &str = "遇到问题点「打包日志到桌面」，把生成的 zip 发给作者即可（含三个进程的日志与配置文件，不含密钥）。缺省日志不含你敲的内容；排查排序问题时作者可能请你在「调试」页临时打开详细日志。";

/// 「立即检查」旁边那行字。
fn update_status(settings: &Settings) -> String {
    if settings.update_checking {
        return "正在检查…".to_owned();
    }
    if let Some(error) = &settings.update_error {
        return format!("检查失败：{error}");
    }
    let update = &settings.config.update;
    match Checker::available_in(&settings.update_state, VERSION, update) {
        Some(found) => format!("有新版本 {}", found.version),
        None if Checker::is_dev_version(VERSION) => "本地开发版，不检查更新".to_owned(),
        None if settings.update_state.checked_at > 0 => "已是最新版本".to_owned(),
        None => String::new(),
    }
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let update = &settings.config.update;
    let available = Checker::available_in(&settings.update_state, VERSION, update).is_some();
    let mut update_buttons: Vec<View> = vec![
        Button::new()
            .on_click(context.message(Message::CheckUpdateNow))
            .content("立即检查"),
    ];
    if available {
        update_buttons.push(
            Button::new()
                .on_click(context.message(Message::OpenDownload))
                .content("下载新版"),
        );
    }
    update_buttons.push(note(&update_status(settings)));
    let mut attributions: Vec<View> = Vec::with_capacity(ATTRIBUTIONS.len());
    for (name, text) in ATTRIBUTIONS {
        attributions.push(note(&format!("{name}：{text}")));
    }
    let body = StackPanel::new().spacing(12.0).children([
        TextBlock::new()
            .text(format!("云朵 Windows {VERSION}"))
            .font_size(16.0)
            .font_weight(FontWeight::SEMI_BOLD)
            .into(),
        // CLOUDIME_BUILD 由 build.rs 从 git 取
        note(&format!(
            "构建 {}",
            option_env!("CLOUDIME_BUILD").unwrap_or("本地构建")
        )),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .children((
                Button::new()
                    .on_click(context.message(Message::OpenWebsite))
                    .content("官网"),
                Button::new()
                    .on_click(context.message(Message::OpenRepository))
                    .content("GitHub"),
                Button::new()
                    .on_click(context.message(Message::OpenDataDir))
                    .content("打开数据目录"),
                Button::new()
                    .on_click(context.message(Message::OpenLogDir))
                    .content("打开日志目录"),
                Button::new()
                    .on_click(context.message(Message::ExportLogs))
                    .content("打包日志到桌面"),
            )),
        note(LICENSE_NOTE),
        field(
            "自动检查更新",
            "每天向官网读一次版本列表，有新版在这里提示，不会自动下载安装。",
            ToggleSwitch::new()
                .is_on(update.check)
                .on_toggled(context.callback(Message::UpdateCheck)),
        ),
        field(
            "更新渠道",
            "测试版渠道还会提示 alpha / beta / rc 版本。",
            ComboBox::new()
                .items_source(UpdateChannel::ALL.iter().map(|channel| channel.label()))
                .selected_index(
                    UpdateChannel::ALL
                        .iter()
                        .position(|channel| *channel == update.channel)
                        .unwrap_or(0),
                )
                .on_selection_changed(context.callback(Message::UpdateChannel)),
        ),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .keyed_children(
                update_buttons
                    .into_iter()
                    .enumerate()
                    .map(|(index, view)| KeyedView::new(format!("update-{index}"), view)),
            ),
        TextBlock::new()
            .text("数据来源与署名")
            .font_weight(FontWeight::SEMI_BOLD)
            .into(),
        StackPanel::new().spacing(6.0).keyed_children(
            attributions
                .into_iter()
                .enumerate()
                .map(|(index, view)| KeyedView::new(index.to_string(), view)),
        ),
        note(PRIVACY_NOTE),
        note(FEEDBACK_NOTE),
    ]);
    page("关于", body)
}
