//! 部件分类报告：给人工复核用的 TSV。
//!
//! 每次生成都建议写一份到 `data/generated/components-report.tsv`（gitignore），
//! 重点看「覆盖冲突」「低置信」「笔画未知」三类行。

use std::path::Path;

use cloudime_ids::ComponentRole;

use crate::BuildError;
use crate::components::StrokeSource;

/// 报告的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportRow {
    pub character: char,
    pub stroke: u8,
    pub stroke_source: StrokeSource,
    pub primary: u8,
    pub auto_primary: Option<u8>,
    pub roles: [u16; 5],
    pub override_conflict: bool,
}

/// 写报告；父目录不存在会建。
pub fn write(path: &Path, rows: &[ReportRow]) -> Result<(), BuildError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| BuildError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, render(rows)).map_err(|source| BuildError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// 报告正文（纯函数，测试直接断言）。
pub fn render(rows: &[ReportRow]) -> String {
    let mut text = String::new();
    text.push_str("# 难输入部件分类报告\n");
    text.push_str("# 主分类阈值 300‰；分类页收录阈值 100‰；平票顺序 旁 > 头 > 底 > 围 > 其它\n");
    text.push_str(
        "字符\t码位\t笔画\t笔画来源\t主分类\t自动主分类\t围\t头\t旁\t底\t其它\t低置信\t覆盖冲突\n",
    );
    for row in rows {
        let low_confidence = row
            .auto_primary
            .is_some_and(|primary| row.roles[primary as usize] < 300);
        text.push_str(&format!(
            "{}\tU+{:04X}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            row.character,
            row.character as u32,
            row.stroke,
            row.stroke_source.name(),
            role_name(row.primary),
            row.auto_primary.map_or("—".to_owned(), role_name),
            row.roles[0],
            row.roles[1],
            row.roles[2],
            row.roles[3],
            row.roles[4],
            if low_confidence { "是" } else { "" },
            if row.override_conflict { "是" } else { "" },
        ));
    }
    text
}

fn role_name(index: u8) -> String {
    ComponentRole::from_index(index)
        .map_or_else(|| "其它".to_owned(), |role| role.name().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_header_and_rows() {
        let rows = [ReportRow {
            character: '木',
            stroke: 4,
            stroke_source: StrokeSource::Tsv,
            primary: 2,
            auto_primary: Some(2),
            roles: [0, 0, 900, 100, 0],
            override_conflict: false,
        }];
        let text = render(&rows);
        assert!(text.contains("字符\t码位\t笔画"));
        assert!(text.contains("木\tU+6728\t4\tTSV\t旁\t旁\t0\t0\t900\t100\t0"));
        assert!(text.contains("# 主分类阈值 300‰"));
    }

    #[test]
    fn marks_low_confidence_and_overrides() {
        let rows = [ReportRow {
            character: '一',
            stroke: 1,
            stroke_source: StrokeSource::Ids,
            primary: 1,
            auto_primary: Some(4),
            roles: [0, 250, 250, 250, 250],
            override_conflict: true,
        }];
        let text = render(&rows);
        assert!(text.contains("\t是\t是\n"));
    }
}
