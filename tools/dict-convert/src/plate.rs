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
                    short.as_deref(),
                );
            } else {
                province = Some(first);
                for letter in rest.chars().filter(char::is_ascii_alphabetic) {
                    push_row(
                        &mut rows,
                        first,
                        letter.to_ascii_uppercase(),
                        &place,
                        short.as_deref(),
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

fn push_row(
    rows: &mut Vec<PlateRow>,
    province: char,
    letter: char,
    full: &str,
    short: Option<&str>,
) {
    let Some(pinyin) = province_pinyin(province) else {
        tracing::warn!(province = %province, "车牌里的省份简称没有拼音，跳过");
        return;
    };
    rows.push(PlateRow {
        key: format!("{pinyin}{}", letter.to_ascii_lowercase()),
        plate: format!("{province}{letter}"),
        full: full.to_owned(),
        short: short.unwrap_or_default().to_owned(),
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

/// 完整地名 → 短地名：只有 `xx省` / `xx市` / `xx自治州` / `xx地区` / `xx盟` 才有短地名；
/// 出租车、公安系统、省直机关、片区、括号备注等一律没有。
fn short_place(full: &str) -> Option<String> {
    let full = full.trim();
    if full.contains('（') || full.contains('(') {
        return None;
    }
    let allowed = full.ends_with('省')
        || full.ends_with('市')
        || full.ends_with("自治州")
        || full.ends_with("地区")
        || full.ends_with("盟");
    if !allowed {
        return None;
    }
    let mut short = full.to_owned();
    let mut autonomous = false;
    for suffix in ["自治州", "地区", "盟", "市", "省"] {
        if short.ends_with(suffix) {
            short.truncate(short.len() - suffix.len());
            autonomous = suffix == "自治州";
            break;
        }
    }
    if autonomous {
        short = strip_ethnic_suffix(short);
    }
    let short = short.replace('省', "");
    let short = short.trim();
    (!short.is_empty()).then(|| short.to_owned())
}

/// 去掉自治州名字末尾的民族名（`昌吉回族` → `昌吉`、`海西蒙古族藏族` → `海西`）。
fn strip_ethnic_suffix(mut name: String) -> String {
    const ETHNIC: &[&str] = &[
        "柯尔克孜",
        "维吾尔",
        "哈萨克",
        "布依",
        "朝鲜",
        "土家",
        "哈尼",
        "傈僳",
        "景颇",
        "蒙古",
        "达斡尔",
        "鄂温克",
        "鄂伦春",
        "乌孜别克",
        "俄罗斯",
        "塔塔尔",
        "仡佬",
        "锡伯",
        "门巴",
        "珞巴",
        "基诺",
        "布朗",
        "阿昌",
        "普米",
        "塔吉克",
        "德昂",
        "保安",
        "裕固",
        "独龙",
        "撒拉",
        "毛南",
        "仫佬",
        "东乡",
        "纳西",
        "拉祜",
        "藏",
        "回",
        "彝",
        "苗",
        "侗",
        "壮",
        "白",
        "傣",
        "羌",
        "满",
        "瑶",
        "畲",
        "黎",
        "水",
        "土",
        "怒",
        "佤",
        "京",
    ];
    loop {
        let mut stripped = false;
        for ethnic in ETHNIC {
            for suffix in [format!("{ethnic}族"), (*ethnic).to_owned()] {
                if name.ends_with(&suffix) {
                    name.truncate(name.len() - suffix.len());
                    stripped = true;
                    break;
                }
            }
            if stripped {
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::short_place;

    #[test]
    fn short_names_only_for_administrative_places() {
        assert_eq!(short_place("江苏省南京市").as_deref(), Some("江苏南京"));
        assert_eq!(short_place("北京市").as_deref(), Some("北京"));
        assert_eq!(
            short_place("内蒙古呼和浩特市").as_deref(),
            Some("内蒙古呼和浩特")
        );
        assert_eq!(short_place("内蒙古兴安盟").as_deref(), Some("内蒙古兴安"));
        assert_eq!(
            short_place("黑龙江省大兴安岭地区").as_deref(),
            Some("黑龙江大兴安岭")
        );
        assert_eq!(
            short_place("新疆昌吉回族自治州").as_deref(),
            Some("新疆昌吉")
        );
        assert_eq!(
            short_place("云南楚雄彝族自治州").as_deref(),
            Some("云南楚雄")
        );
        assert_eq!(short_place("福建省直系统、平潭综合实验区"), None);
        assert_eq!(short_place("北京市出租车"), None);
        assert_eq!(short_place("上海市区"), None);
        assert_eq!(short_place("原松花江地区（今哈尔滨部分）"), None);
    }
}
