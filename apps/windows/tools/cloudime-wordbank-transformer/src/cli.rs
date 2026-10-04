//! 命令行参数。

use std::path::PathBuf;

use clap::Parser;

use crate::source::Language;

/// 把第三方词库（yaml / tsv / dat）转成云朵的 `.db` 词库。
#[derive(Debug, Parser)]
#[command(name = "cwt", version, about)]
pub(crate) struct Args {
    /// 待转换的源文件：`.yaml` / `.yml` 按 Rime 词典；`.tsv` / `.dat` 及其它按制表符文本。
    pub(crate) input: PathBuf,

    /// 转换后的 `.db` 路径。
    pub(crate) output: PathBuf,

    /// 忽略文件开头的非格式文本（前言、表头等），直到第一条数据行。
    #[arg(long = "ignore_head")]
    pub(crate) ignore_head: bool,

    /// 排除权重低于 `NUM` 的条目（严格小于：`--exclude_below 100` 删掉权重 1..99）。
    #[arg(long = "exclude_below", value_name = "NUM")]
    pub(crate) exclude_below: Option<u32>,

    /// 排除某种语言的条目。
    #[arg(long = "exclude_lang", value_name = "LANG")]
    pub(crate) exclude_lang: Option<Language>,

    /// 转换成功后删除源文件：`1` 删除、`0` 保留（缺省）。
    #[arg(long = "remove_source", value_name = "0|1", default_value_t = 0)]
    pub(crate) remove_source: u8,
}
