use cloudime_format::FormatError;

use crate::parser::op::IdsOp;

/// IDS 表达式本身写错了。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// 空串 / 只有空白。
    #[error("表达式是空的")]
    Empty,

    /// 运算符的子节点不够。
    #[error(
        "表达式不完整：{}",
        .op.map_or_else(|| "缺少部件".to_owned(), |op| format!("{op} 需要 {} 个部件", op.arity()))
    )]
    Incomplete { op: Option<IdsOp> },

    /// 整串解析完了还有字符（`⿰木木木`）。
    #[error("表达式有多余字符：{0}")]
    ExtraInput(String),

    /// `{n}` 写法不对，或编号超出 1..=255。
    #[error("未编码部件写法不对：{0}")]
    BadUnencoded(String),
}

/// 打开数据或执行查询时的错误。
#[derive(Debug, thiserror::Error)]
pub enum IdsError {
    #[error(transparent)]
    Format(#[from] FormatError),

    #[error(transparent)]
    Parse(#[from] ParseError),

    /// `KEYS` 里的 IDS 越出 `TEXT` 或不是 UTF-8。
    #[error("第 {0} 个 IDS 键的字符串越界或不是 UTF-8")]
    BadKey(usize),

    /// `KEYS` 里的条目范围越出 `ENTR`。
    #[error("第 {0} 个 IDS 键的条目范围越界")]
    BadEntryRange(usize),

    /// 条目里的码位不是合法 Unicode 标量值。
    #[error("第 {0} 个 IDS 键里有非法码位")]
    BadCodepoint(usize),

    /// `KEYS` 没有按 IDS 字节序严格递增。
    #[error("IDS 键没有按字节序排列（第 {0} 个）")]
    UnsortedKey(usize),

    /// 同一个 IDS 下的字没有按码位严格递增。
    #[error("IDS 条目没有按码位排列（第 {0} 个键）")]
    UnsortedEntry(usize),

    /// 哈希表不是本库建出来的（容量、空槽或条目编号不对）。
    #[error("IDS 哈希表与键表不一致")]
    BadHash,

    /// `COMP` 里的部件数据不合法（码位乱序、笔画越界、分类越界、分类和不是 1000）。
    #[error("部件表第 {0} 项不合法")]
    BadComponent(usize),

    /// 精确查询里出现了 `？`。
    #[error("精确查询里不能有通配符")]
    WildcardNotAllowed,

    /// 整个查询只有一个 `？`。
    #[error("不能只查询一个通配符，请补上结构或部件")]
    BareWildcard,

    /// 查询里带了 `{n}`：未编码部件不参与查询。
    #[error("含未编码部件 {{{0}}}，不能查询")]
    Unencoded(u8),
}
