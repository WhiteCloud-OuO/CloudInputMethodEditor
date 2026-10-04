//! 「词库」页：导入的第三方词库，以及导入 / 移除与生僻项查询开关。
//!
//! 随包的 `Dict.db` 与 `UserWordBank.db`（用户自造词库，缺省在安装目录的 `WordBank\`）是内置词库、始终加载，
//! 不出现在列表里；列表只列用户导入的第三方词库，目录里有的全部加载，没有启用清单。

use cloudime_platform::WordBank;
use windows_reactor::*;

use crate::panel::controls::{bank_row, entry_title, field, note, page};
use crate::panel::{Message, Settings};

/// 词库目录：随包根（装机时就是程序目录）下的 `WordBank\`。
pub(crate) fn bank(settings: &Settings) -> WordBank {
    let root = cloudime_platform::resources::bundled_root()
        .unwrap_or_else(|| settings.data_dir().to_path_buf());
    WordBank::locate(&root)
}

fn word_bank_list(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let bank = bank(settings);
    let files = bank.imported();
    if files.is_empty() {
        return note("未发现第三方词库。");
    }
    let mut rows: Vec<KeyedView> = Vec::with_capacity(files.len());
    for file in files {
        let label = entry_title(&file.name, file.entries, &file.license, file.broken);
        let remove = Message::RemoveWordBank(file.file.clone());
        rows.push(bank_row(&file.file, label, remove, context));
    }
    StackPanel::new().spacing(6.0).keyed_children(rows)
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let body = StackPanel::new().spacing(12.0).children([
        note("用户词库（自造词）：输入2次就记住，打得越多权重越大，排序也会越靠前。按下Ctrl+候选项数字可以删除自造词。\
        \n（注：本地整句模型所在的候选项，按下Ctrl+对应候选项数字两次后也会入库到用户词库。）\
        \n软件词库：打的越多权重越大，按下Ctrl+候选项数字可以重置权重。\
        \n软件自带的 Dict.db 与 UserWordBank.db 是内置词库，始终加载。"),
        field(
            "从词库中查询生僻项条目",
            "开启时，候选与整句会从词库的生僻字 / 生僻词（方言字、罕见词等）里取词；关闭可加快查询，候选里不出现。",
            ToggleSwitch::new()
                .is_on(settings.config.word_bank.rare_items)
                .on_toggled(context.callback(Message::RareItems)),
        ),
        word_bank_list(settings, context),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .children((
                Button::new()
                    .on_click(context.message(Message::ImportDictionary))
                    .content("导入词库"),
                note("只支持 .db 文件格式的词库文件。yaml格式可通过输入法自带工具转换为db文件。"),
            )),
        note(&settings.dictionary_status),
    ]);
    page("词库", body)
}

/// 挪进 `WordBank\removed`，不真删。内置词库不让移除。
pub(crate) fn remove(settings: &mut Settings, file: &str) {
    let bank = bank(settings);
    if WordBank::is_builtin(file) {
        settings.dictionary_status = format!("「{file}」是内置词库，不能移除。");
        return;
    }
    let Some(entry) = bank.files().into_iter().find(|(_, name, _)| name == file) else {
        return;
    };
    let removed = bank.dir.join("removed");
    if let Err(error) = std::fs::create_dir_all(&removed) {
        settings.dictionary_status = format!("移除失败：{error}");
        return;
    }
    if let Err(error) = std::fs::rename(&entry.2, removed.join(&entry.1)) {
        settings.dictionary_status = format!("移除失败：{error}");
        return;
    }
    settings.dictionary_status = format!("已移除「{}」，输入法将自动更新。", entry.0);
}

/// 多选 `.db` 词库，逐个复制并汇总结果。
pub(crate) fn import(settings: &mut Settings) {
    let Some(sources) = rfd::FileDialog::new()
        .add_filter("词库文件（.db）", &["db"])
        .set_title("导入词库")
        .pick_files()
    else {
        return;
    };
    let bank = bank(settings);
    let mut results = Vec::new();
    let mut succeeded = 0;
    for source in &sources {
        let is_db = source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("db"));
        if !is_db {
            results.push(format!("{} 不是 .db 词库文件，已跳过。", source.display()));
            continue;
        }
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if WordBank::is_builtin(file_name) {
            results.push(format!(
                "{} 与内置词库同名，已跳过；请改名后再导入。",
                source.display()
            ));
            continue;
        }
        match cloudime_core::dictionary::import::import(source, &bank.dir) {
            Ok(imported) => {
                succeeded += 1;
                results.push(format!(
                    "已导入「{}」，共 {} 条。",
                    imported.name, imported.entries
                ));
            }
            Err(error) => {
                let message = format!("{} 导入失败：{error}", source.display());
                crate::log::warn(&message);
                results.push(message);
            }
        }
    }
    let mut summary = format!(
        "导入完成：成功 {} 个，失败 {} 个。",
        succeeded,
        sources.len() - succeeded
    );
    if succeeded > 0 {
        summary.push_str("输入法将自动加载。");
    }
    settings.dictionary_status = format!("{summary}\n{}", results.join("\n"));
}
