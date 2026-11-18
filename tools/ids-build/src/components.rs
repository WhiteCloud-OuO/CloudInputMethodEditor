//! `components.tsv`：难输入部件的字符、笔画、分类覆盖。
//!
//! 列：`字符 <TAB> 笔画 <TAB> 分类 <TAB> 启用`。笔画 `13` = 13+，留空按
//! Unihan → IDS 递归补；分类留空用 IDS 位置统计算出来的主分类。

use std::collections::{HashMap, HashSet};

use cloudime_ids::{
    COMPONENT_FLAG_STROKE_ESTIMATED, COMPONENT_FLAG_TYPEABLE, COMPONENT_NO_SVG, Component,
    ComponentRole, MAX_STROKE,
};

use crate::BuildError;
use crate::classify::Classification;
use crate::read::Rows;
use crate::report::ReportRow;
use crate::stroke::{self, Unihan};

/// 笔画是从哪儿来的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeSource {
    /// `components.tsv` 里写着。
    Tsv,

    /// Unihan `kTotalStrokes`。
    Unihan,

    /// 按 IDS 树递归数出来。
    Ids,

    /// 算不出来，`stroke = 0`。
    Unknown,
}

impl StrokeSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tsv => "TSV",
            Self::Unihan => "Unihan",
            Self::Ids => "IDS 推算",
            Self::Unknown => "未知",
        }
    }
}

/// 组装结果：`COMP` 表 + 给人工复核的报告行 + 部件 SVG arena。
#[derive(Debug, Default)]
pub struct Resolved {
    pub components: Vec<Component>,
    pub report: Vec<ReportRow>,
    pub component_text: Vec<u8>,
}

/// 一行原始数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentRow {
    pub character: char,
    pub stroke: Option<u8>,
    pub category: Option<u8>,
    pub enabled: bool,
}

/// 解析 `components.tsv`；行号从 1 起。
pub fn parse(text: &str) -> Result<Vec<ComponentRow>, BuildError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut seen = std::collections::BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let character = fields
            .next()
            .and_then(single_character)
            .ok_or_else(|| bad_line(line_number, "第一列不是单个字符"))?;
        let stroke = optional_number(fields.next(), 1, MAX_STROKE, line_number, "笔画要在 1..=13")?;
        let category = optional_number(
            fields.next(),
            0,
            ComponentRole::Other.index(),
            line_number,
            "分类要在 0..=4",
        )?;
        let enabled = match fields.next().map(str::trim) {
            None | Some("") | Some("1") => true,
            Some("0") => false,
            Some(other) => {
                return Err(bad_line(
                    line_number,
                    &format!("启用列只能空 / 0 / 1，实际是 {other}"),
                ));
            }
        };
        if let Some(previous) = seen.insert(character, line_number) {
            return Err(bad_line(
                line_number,
                &format!("部件 {character} 重复（第一次在第 {previous} 行）"),
            ));
        }
        rows.push(ComponentRow {
            character,
            stroke,
            category,
            enabled,
        });
    }
    Ok(rows)
}

/// 把 TSV 行组装成 `COMP` 表：补笔画、用位置统计补分类、按码位排序。
///
/// TSV 的分类覆盖优先；没有自动统计的部件按「其它」写满 1000 千分比。
pub fn resolve(
    rows: &[ComponentRow],
    ids: &Rows,
    unihan: &Unihan,
    classifications: &HashMap<char, Classification>,
    typeable: &HashSet<char>,
    glyphs: &HashMap<char, String>,
) -> Resolved {
    let ids_by_character = ids.first_ids_by_character();
    let mut resolved = Resolved::default();
    for row in rows.iter().filter(|row| row.enabled) {
        let character = row.character;
        let mut flags = 0u16;
        let (stroke, stroke_source) = if let Some(value) = row.stroke {
            (value, StrokeSource::Tsv)
        } else if let Some(value) = unihan.stroke(character) {
            (value.min(MAX_STROKE), StrokeSource::Unihan)
        } else if let Some(value) = stroke::from_ids(character, &ids_by_character, unihan) {
            flags |= COMPONENT_FLAG_STROKE_ESTIMATED;
            (value, StrokeSource::Ids)
        } else {
            (0, StrokeSource::Unknown)
        };
        if stroke == 0 {
            eprintln!(
                "警告：部件 {character} (U+{:04X}) 笔画未知，归入「未分类」",
                character as u32
            );
        }
        if typeable.contains(&character) {
            flags |= COMPONENT_FLAG_TYPEABLE;
        }

        let classification = classifications.get(&character).copied();
        let auto_primary = classification.map(|item| item.primary);
        let primary = row
            .category
            .or(auto_primary)
            .unwrap_or(ComponentRole::Other.index());
        let roles = classification.map_or_else(
            || {
                let mut roles = [0u16; 5];
                roles[primary as usize] = 1000;
                roles
            },
            |item| item.roles,
        );
        let override_conflict = matches!(
            (row.category, auto_primary),
            (Some(overridden), Some(automatic)) if overridden != automatic
        );

        let (svg_off, svg_len) = match glyphs.get(&character) {
            Some(path) => {
                let svg = svg_document(path);
                let offset = u32::try_from(resolved.component_text.len()).unwrap_or(u32::MAX);
                if offset == u32::MAX || svg.len() > u16::MAX as usize {
                    eprintln!("警告：部件 {character} 的 SVG 太大，跳过");
                    (COMPONENT_NO_SVG, 0)
                } else {
                    resolved.component_text.extend_from_slice(svg.as_bytes());
                    (offset, svg.len() as u16)
                }
            }
            None => (COMPONENT_NO_SVG, 0),
        };

        resolved.components.push(Component {
            codepoint: character as u32,
            roles,
            stroke,
            primary,
            flags,
            reserved: 0,
            svg_off,
            svg_len,
            reserved2: 0,
        });
        resolved.report.push(ReportRow {
            character,
            stroke,
            stroke_source,
            primary,
            auto_primary,
            roles,
            override_conflict,
        });
    }
    resolved
        .components
        .sort_by_key(|component| component.codepoint);
    resolved.report.sort_by_key(|row| row.character as u32);
    resolved
}

fn svg_document(paths: &str) -> String {
    let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200">"#);
    for segment in paths.split('|').map(str::trim).filter(|s| !s.is_empty()) {
        svg.push_str(r#"<path d=""#);
        svg.push_str(segment);
        svg.push_str(r#"" fill="black"/>"#);
    }
    svg.push_str("</svg>");
    svg
}

fn optional_number(
    field: Option<&str>,
    min: u8,
    max: u8,
    line: usize,
    reason: &str,
) -> Result<Option<u8>, BuildError> {
    let Some(value) = field.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let number: u8 = value
        .parse()
        .map_err(|_| bad_line(line, &format!("{reason}；实际是 {value}")))?;
    if !(min..=max).contains(&number) {
        return Err(bad_line(line, reason));
    }
    Ok(Some(number))
}

fn single_character(field: &str) -> Option<char> {
    let field = field.trim();
    let mut characters = field.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

fn bad_line(line: usize, reason: &str) -> BuildError {
    BuildError::BadLine {
        line,
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}# 注释\n\
        \n\
        木\t4\t2\t1\n\
        艹\t3\t\t1\n\
        𠂇\t\t\t1\n\
        水\t4\t\t0\n";

    #[test]
    fn parses_columns_and_skips_comments() {
        let rows = parse(SAMPLE).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].character, '木');
        assert_eq!(rows[0].stroke, Some(4));
        assert_eq!(rows[0].category, Some(2));
        assert!(!rows[3].enabled);
        assert_eq!(rows[1].stroke, Some(3));
        assert_eq!(rows[1].category, None);
    }

    #[test]
    fn rejects_duplicates_and_out_of_range_values() {
        assert!(matches!(
            parse("木\t4\t\t1\n木\t5\t\t1\n"),
            Err(BuildError::BadLine { line: 2, .. })
        ));
        assert!(matches!(
            parse("木\t14\t\t1\n"),
            Err(BuildError::BadLine { line: 1, .. })
        ));
        assert!(matches!(
            parse("木\t4\t5\t1\n"),
            Err(BuildError::BadLine { line: 1, .. })
        ));
        assert!(matches!(
            parse("木\t4\t\t2\n"),
            Err(BuildError::BadLine { line: 1, .. })
        ));
        assert!(matches!(
            parse("木木\t4\t\t1\n"),
            Err(BuildError::BadLine { line: 1, .. })
        ));
    }

    #[test]
    fn resolve_fills_strokes_and_sorts_by_codepoint() {
        let ids = crate::read::parse("U+6797\t林\t^⿰木木$(G)\n").unwrap();
        let unihan = Unihan::parse("U+6797\tkTotalStrokes\t8\n");
        let rows = vec![
            ComponentRow {
                character: '林',
                stroke: None,
                category: None,
                enabled: true,
            },
            ComponentRow {
                character: '木',
                stroke: Some(4),
                category: Some(2),
                enabled: true,
            },
            ComponentRow {
                character: '水',
                stroke: Some(4),
                category: None,
                enabled: false,
            },
        ];
        let resolved = resolve(
            &rows,
            &ids,
            &unihan,
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        );
        assert_eq!(resolved.components.len(), 2);
        assert_eq!(resolved.report.len(), 2);
        assert_eq!(resolved.components[0].character(), Some('木'));
        assert_eq!(resolved.components[0].codepoint, '木' as u32);
        assert_eq!(resolved.components[0].stroke, 4);
        assert_eq!(resolved.components[0].roles[2], 1000);
        assert_eq!(resolved.components[1].codepoint, '林' as u32);
        // Unihan 的 8 画比 IDS 递归优先。
        assert_eq!(resolved.components[1].stroke, 8);
        assert_eq!(
            resolved.components[1].flags & COMPONENT_FLAG_STROKE_ESTIMATED,
            0
        );
        assert_eq!(resolved.report[1].stroke_source, StrokeSource::Unihan);
    }

    #[test]
    fn resolve_uses_computed_roles_and_flags_category_overrides() {
        let ids = crate::read::parse(
            "U+6797\t林\t^⿰木木$(G)\n\
             U+76F8\t相\t^⿰木目$(G)\n",
        )
        .unwrap();
        let classifications = crate::classify::classify(&ids);
        let rows = vec![
            ComponentRow {
                character: '木',
                stroke: Some(4),
                category: None,
                enabled: true,
            },
            ComponentRow {
                character: '目',
                stroke: Some(5),
                category: Some(1),
                enabled: true,
            },
        ];
        let resolved = resolve(
            &rows,
            &ids,
            &Unihan::default(),
            &classifications,
            &HashSet::new(),
            &HashMap::new(),
        );
        let wood = &resolved.components[0];
        assert_eq!(wood.primary, 2);
        assert!(wood.roles[2] > 0);
        let eye = &resolved.components[1];
        assert_eq!(eye.primary, 1);
        assert!(resolved.report[1].override_conflict);
        assert_eq!(resolved.report[0].auto_primary, Some(2));
    }

    #[test]
    fn resolve_falls_back_to_ids_and_marks_estimated() {
        let ids = crate::read::parse("U+6797\t林\t^⿰木木$(G)\n").unwrap();
        let unihan = Unihan::parse("U+6728\tkTotalStrokes\t4\n");
        let rows = vec![ComponentRow {
            character: '林',
            stroke: None,
            category: None,
            enabled: true,
        }];
        let components = resolve(
            &rows,
            &ids,
            &unihan,
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        )
        .components;
        assert_eq!(components[0].stroke, 8);
        assert_ne!(components[0].flags & COMPONENT_FLAG_STROKE_ESTIMATED, 0);
    }

    #[test]
    fn resolve_marks_typeable_components() {
        let rows = vec![ComponentRow {
            character: '木',
            stroke: Some(4),
            category: None,
            enabled: true,
        }];
        let typeable = HashSet::from(['木']);
        let resolved = resolve(
            &rows,
            &Rows::default(),
            &Unihan::default(),
            &HashMap::new(),
            &typeable,
            &HashMap::new(),
        );
        assert_ne!(
            resolved.components[0].flags & cloudime_ids::COMPONENT_FLAG_TYPEABLE,
            0
        );
    }

    #[test]
    fn resolve_leaves_unknown_strokes_at_zero() {
        let ids = crate::read::Rows::default();
        let rows = vec![ComponentRow {
            character: '𪚥',
            stroke: None,
            category: None,
            enabled: true,
        }];
        let components = resolve(
            &rows,
            &ids,
            &Unihan::default(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        )
        .components;
        assert_eq!(components[0].stroke, 0);
    }
}
