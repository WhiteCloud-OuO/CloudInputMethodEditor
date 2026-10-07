//! 一次要绘制的组句状态：preedit 行加候选页。

pub mod preedit;

pub use preedit::{PreeditKind, PreeditSegment};

use serde::{Deserialize, Serialize};

use cloudime_core::CandidateList;

use crate::{LayoutMode, PreeditMode};

/// Server 告诉 DLL「现在屏幕上该是什么样」：组句的拼音行、候选页、高亮与页码。
/// 空 [`Frame`]（`preedit` 与 `candidates` 都空）表示没有在组句，DLL 收起候选窗口。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// 组句拼音行的分段，按顺序拼成整行。
    pub preedit: Vec<PreeditSegment>,

    /// 拼音显示在哪（`[general] preedit`）：DLL 按 [`PreeditMode::inline`] 决定要不要往应用里放行内拼音，
    /// 窗口顶部画不画拼音行由 Server 自己按 [`PreeditMode::in_window`] 定。
    #[serde(default)]
    pub preedit_mode: PreeditMode,

    /// 光标在拼音行里的位置，按 `preedit` 拼接后的字符（`char`）数算。
    pub cursor: usize,

    /// 当前页的候选（已排好序、不带译文由后续 [`super::ServerMessage::Update`] 补）。
    pub candidates: CandidateList,

    /// 当前页里高亮的候选下标（页内，从 0 起）。
    pub highlight: usize,

    /// 当前页码（从 0 起）。
    pub page: usize,

    /// 总页数；翻页键是否可用看它。
    pub page_count: usize,

    /// 候选排布（竖排 / 横排）。DLL 是纯渲染端，布局由 Server 按 `[general] layout` 配置随帧下发。
    pub layout: LayoutMode,

    /// 展开「更多候选项」时一行排几格（矩阵）：竖排展开成 5 列、横排展开成 5 行时这里给的是列数。
    /// `0` = 没展开，渲染端按 `layout` 竖排 / 横排画，忽略本字段。
    #[serde(default)]
    pub columns: usize,

    /// 候选窗底部那一行左侧的翻译 Tip：高亮候选在本地词典里的释义 + 学没学会。
    /// 只给 Server 自绘的候选窗用（DLL 是纯渲染端、候选窗不在它手里，忽略不影响行为），
    /// 与 [`columns`](Self::columns) 同理，不单独升协议版本。`None` = 这个词条没有译文。
    #[serde(default)]
    pub tip: Option<cloudime_translate::Tip>,

    /// 多释义选择：`Some` 时自绘的候选窗整屏换成「词条 + 各条释义」（Ctr + 反引号进去，
    /// 数字键选、Esc 退出）。同样只给自绘窗用，发给 DLL 的那份不带。
    #[serde(default)]
    pub tip_choices: Option<TipChoices>,

    /// 屏幕提示：画在 preedit 行下方，显示到下一次按键。当前没有来源写入，字段保留。无则 `None`。
    /// 不参与 [`is_empty`](Self::is_empty)：单有提示不算在组句，否则空组句也会撑开候选窗口。
    #[serde(default)]
    pub notice: Option<String>,
}

impl Frame {
    /// 没有在组句：DLL 据此收起候选窗口。提示不算数（见 [`notice`](Self::notice)）。
    pub fn is_empty(&self) -> bool {
        self.preedit.is_empty() && self.candidates.items.is_empty()
    }
}

/// 多释义选择：这个词条有好几条释义，让用户先挑一条（Ctrl + 反引号之后）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TipChoices {
    /// 被翻译的词条：画在候选窗顶部那一行当标题。
    pub word: String,

    /// 各条释义，序号按顺序从 1 起。
    pub senses: Vec<cloudime_translate::Sense>,
}
