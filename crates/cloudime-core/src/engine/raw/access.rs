//! 不查询候选、不提交、不记录的原样文本读取。
use crate::engine::{Engine, RawPreedit};

impl Engine {
    /// 读取整段尚未上屏的原样文本，包括光标后的内容，不改变输入或学习状态。
    ///
    /// 保留实际键串、大小写及显式分隔符。
    /// 首位始终为 0，末位始终为完整文本末尾。
    pub fn raw_preedit(&self) -> RawPreedit {
        let text = self.composition.raw_text();
        let mut cursor_bytes = self.composition.raw_cursor().min(text.len());
        while !text.is_char_boundary(cursor_bytes) {
            cursor_bytes -= 1;
        }
        RawPreedit { text, cursor_bytes }
    }
}
