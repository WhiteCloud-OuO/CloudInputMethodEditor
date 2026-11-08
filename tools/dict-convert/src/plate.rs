//! 车牌一览 xlsx → `data/generated/plates.qj`。
//!
//! 原始表两列：`车牌`（`京A、C、E` / `苏E、苏U` 这种）与`对应地名`。
//! 一行可能含多个字母；拆成一条条「省份简称 + 字母」，再按简称的拼音 + 字母建 key。

use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};
use cloudime_format::Metadata;
use cloudime_plate::{PlateRow, write_qj};

use crate::error::ConvertError;

pub(crate) fn convert(input: &Path, out: &Path, metadata: Metadata) -> Result<(), ConvertError> {
    let mut workbook = open_workbook_auto(input)?;
    let sheet = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| ConvertError::PlateSheet(input.to_path_buf()))?;
    let range = workbook.worksheet_range(&sheet)?;

    let mut rows = Vec::new();
    for row in range.rows().skip(1) {
        let Some(plate) = row.first().and_then(cell_text) else {
            continue;
        };
        let Some(place) = row.get(1).and_then(cell_text) else {
            continue;
        };
        let short = short_place(&place);
        let mut province = None;
        for token in plate.split(['、', '，', ',']) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let mut chars = token.chars();
            let Some(first) = chars.next() else {
                continue;
            };
            let rest: String = chars.collect();
            if first.is_ascii_alphabetic() {
                let Some(province) = province else {
                    continue;
                };
                push_row(
                    &mut rows,
                    province,
                    first.to_ascii_uppercase(),
                    &place,
                    &short,
                );
            } else {
                province = Some(first);
                for letter in rest.chars().filter(char::is_ascii_alphabetic) {
                    push_row(
                        &mut rows,
                        first,
                        letter.to_ascii_uppercase(),
                        &place,
                        &short,
                    );
                }
            }
        }
    }
    if rows.is_empty() {
        return Err(ConvertError::PlateEmpty(input.to_path_buf()));
    }
    let count = rows.len();
    write_qj(out, rows, metadata)?;
    tracing::info!(input = %input.display(), out = %out.display(), entries = count, "已写车牌对照表");
    Ok(())
}

fn cell_text(cell: &Data) -> Option<String> {
    let text = match cell {
        Data::String(text) => text.clone(),
        Data::Float(value) => value.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn push_row(rows: &mut Vec<PlateRow>, province: char, letter: char, full: &str, short: &str) {
    let Some(pinyin) = province_pinyin(province) else {
        tracing::warn!(province = %province, "车牌里的省份简称没有拼音，跳过");
        return;
    };
    rows.push(PlateRow {
        key: format!("{pinyin}{}", letter.to_ascii_lowercase()),
        plate: format!("{province}{letter}"),
        full: full.to_owned(),
        short: short.to_owned(),
    });
}

/// 省份简称的拼音（key 用）。按 GB 车牌简称表固定。
fn province_pinyin(province: char) -> Option<&'static str> {
    Some(match province {
        '京' => "jing",
        '津' => "jin",
        '冀' => "ji",
        '晋' => "jin",
        '蒙' => "meng",
        '辽' => "liao",
        '吉' => "ji",
        '黑' => "hei",
        '沪' => "hu",
        '苏' => "su",
        '浙' => "zhe",
        '皖' => "wan",
        '闽' => "min",
        '赣' => "gan",
        '鲁' => "lu",
        '豫' => "yu",
        '鄂' => "e",
        '湘' => "xiang",
        '粤' => "yue",
        '桂' => "gui",
        '琼' => "qiong",
        '渝' => "yu",
        '川' => "chuan",
        '贵' => "gui",
        '云' => "yun",
        '藏' => "zang",
        '陕' => "shan",
        '甘' => "gan",
        '青' => "qing",
        '宁' => "ning",
        '新' => "xin",
        _ => return None,
    })
}

/// 完整地名 → 短地名：去掉括号备注、省 / 市等行政后缀。
fn short_place(full: &str) -> String {
    let mut short = full.trim().to_owned();
    if let Some(index) = short.find(['（', '(']) {
        short.truncate(index);
    }
    for suffix in [
        "特别行政区",
        "自治区",
        "自治州",
        "地区",
        "盟",
        "市",
        "省",
        "县",
        "区",
    ] {
        if short.ends_with(suffix) {
            short.truncate(short.len() - suffix.len());
            break;
        }
    }
    let short = short.replace('省', "");
    if short.is_empty() {
        full.to_owned()
    } else {
        short
    }
}
