use std::path::PathBuf;

use clap::Parser;

/// 生成 IDS 组字数据。
#[derive(Debug, Parser)]
#[command(name = "ids-build", about = "BabelStone IDS.TXT → ids.qj")]
pub struct Args {
    /// 输入：BabelStone IDS.TXT。
    #[arg(long, default_value = "assets/ids/IDS.TXT")]
    pub ids: PathBuf,

    /// 输出：`.qj` 文件（目录不存在会自动建）。
    #[arg(long, default_value = "data/generated/ids.qj")]
    pub out: PathBuf,

    /// 难输入部件清单；文件不在就只生成 IDS 查询。
    #[arg(long, default_value = "assets/ids/components.tsv")]
    pub components: PathBuf,

    /// Unihan `IRGSources`（可选）：给 `components.tsv` 里没写笔画的部件补笔画。
    #[arg(long)]
    pub unihan: Option<PathBuf>,

    /// 分类报告输出（可选）；建议写 `data/generated/components-report.tsv`。
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// 词库 TSV（可选）：单字词条用来标记「可拼音打出」的部件。
    #[arg(long)]
    pub dict: Option<PathBuf>,

    /// 只输出自动候选，不生成 `ids.qj`。
    #[arg(long)]
    pub derive: bool,

    /// 候选清单输出。
    #[arg(long, default_value = "data/generated/components-candidates.tsv")]
    pub derive_out: PathBuf,

    /// 候选至少出现在多少个字里。
    #[arg(long, default_value_t = 20)]
    pub derive_min: usize,

    /// 候选里也列出「能用拼音打出」的字（默认只列难输入的）。
    #[arg(long)]
    pub derive_include_typeable: bool,
}
