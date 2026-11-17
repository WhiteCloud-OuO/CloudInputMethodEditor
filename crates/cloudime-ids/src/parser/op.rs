//! IDS 结构运算符：符号、元数、缩写。

/// 一个 Ideographic Description Character。
///
/// 元数 1 的是镜像 / 旋转，2 是并排 / 包围 / 重叠 / 减除，3 是三列 / 三层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdsOp {
    /// ⿰ 左右。
    LeftRight,

    /// ⿱ 上下。
    AboveBelow,

    /// ⿲ 左中右。
    LeftMiddleRight,

    /// ⿳ 上中下。
    AboveMiddleBelow,

    /// ⿴ 全包围。
    FullSurround,

    /// ⿵ 上包围。
    SurroundAbove,

    /// ⿶ 下包围。
    SurroundBelow,

    /// ⿷ 左包围。
    SurroundLeft,

    /// ⿸ 左上包围。
    SurroundUpperLeft,

    /// ⿹ 右上包围。
    SurroundUpperRight,

    /// ⿺ 左下包围。
    SurroundLowerLeft,

    /// ⿼ 右包围。
    SurroundRight,

    /// ⿽ 右下包围。
    SurroundLowerRight,

    /// ⿻ 重叠。
    Overlaid,

    /// ⿾ 水平镜像。
    HorizontalReflection,

    /// ⿿ 旋转。
    Rotation,

    /// ㇯ 减除。
    Subtraction,
}

impl IdsOp {
    /// 全部运算符，按符号顺序排。
    pub const ALL: [Self; 17] = [
        Self::LeftRight,
        Self::AboveBelow,
        Self::LeftMiddleRight,
        Self::AboveMiddleBelow,
        Self::FullSurround,
        Self::SurroundAbove,
        Self::SurroundBelow,
        Self::SurroundLeft,
        Self::SurroundUpperLeft,
        Self::SurroundUpperRight,
        Self::SurroundLowerLeft,
        Self::SurroundRight,
        Self::SurroundLowerRight,
        Self::Overlaid,
        Self::HorizontalReflection,
        Self::Rotation,
        Self::Subtraction,
    ];

    /// 从 IDS 符号取运算符。
    pub fn from_char(symbol: char) -> Option<Self> {
        Some(match symbol {
            '⿰' => Self::LeftRight,
            '⿱' => Self::AboveBelow,
            '⿲' => Self::LeftMiddleRight,
            '⿳' => Self::AboveMiddleBelow,
            '⿴' => Self::FullSurround,
            '⿵' => Self::SurroundAbove,
            '⿶' => Self::SurroundBelow,
            '⿷' => Self::SurroundLeft,
            '⿸' => Self::SurroundUpperLeft,
            '⿹' => Self::SurroundUpperRight,
            '⿺' => Self::SurroundLowerLeft,
            '⿼' => Self::SurroundRight,
            '⿽' => Self::SurroundLowerRight,
            '⿻' => Self::Overlaid,
            '⿾' => Self::HorizontalReflection,
            '⿿' => Self::Rotation,
            '㇯' => Self::Subtraction,
            _ => return None,
        })
    }

    /// 运算符本身的字符。
    pub fn symbol(self) -> char {
        match self {
            Self::LeftRight => '⿰',
            Self::AboveBelow => '⿱',
            Self::LeftMiddleRight => '⿲',
            Self::AboveMiddleBelow => '⿳',
            Self::FullSurround => '⿴',
            Self::SurroundAbove => '⿵',
            Self::SurroundBelow => '⿶',
            Self::SurroundLeft => '⿷',
            Self::SurroundUpperLeft => '⿸',
            Self::SurroundUpperRight => '⿹',
            Self::SurroundLowerLeft => '⿺',
            Self::SurroundRight => '⿼',
            Self::SurroundLowerRight => '⿽',
            Self::Overlaid => '⿻',
            Self::HorizontalReflection => '⿾',
            Self::Rotation => '⿿',
            Self::Subtraction => '㇯',
        }
    }

    /// 子节点个数。
    pub fn arity(self) -> usize {
        match self {
            Self::LeftMiddleRight | Self::AboveMiddleBelow => 3,
            Self::HorizontalReflection | Self::Rotation => 1,
            _ => 2,
        }
    }

    /// 不带特殊字符键盘时用的两字母缩写。
    pub fn abbreviation(self) -> &'static str {
        match self {
            Self::LeftRight => "LR",
            Self::LeftMiddleRight => "LL",
            Self::AboveBelow => "UD",
            Self::AboveMiddleBelow => "UU",
            Self::FullSurround => "OC",
            Self::SurroundAbove => "OD",
            Self::SurroundBelow => "OU",
            Self::SurroundLeft => "OR",
            Self::SurroundUpperLeft => "RD",
            Self::SurroundUpperRight => "LD",
            Self::SurroundLowerLeft => "RU",
            Self::SurroundLowerRight => "LU",
            Self::SurroundRight => "OL",
            Self::Overlaid => "XX",
            Self::HorizontalReflection => "MI",
            Self::Rotation => "RO",
            Self::Subtraction => "SU",
        }
    }

    /// 两字母缩写取运算符；大小写敏感。
    pub fn from_abbreviation(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        Self::from_abbreviation_bytes([bytes[0], bytes[1]])
    }

    /// 已经确认是两个 ASCII 字节的缩写。
    pub(crate) fn from_abbreviation_bytes(bytes: [u8; 2]) -> Option<Self> {
        Some(match &bytes {
            b"LR" => Self::LeftRight,
            b"LL" => Self::LeftMiddleRight,
            b"UD" => Self::AboveBelow,
            b"UU" => Self::AboveMiddleBelow,
            b"OC" => Self::FullSurround,
            b"OD" => Self::SurroundAbove,
            b"OU" => Self::SurroundBelow,
            b"OR" => Self::SurroundLeft,
            b"RD" => Self::SurroundUpperLeft,
            b"LD" => Self::SurroundUpperRight,
            b"RU" => Self::SurroundLowerLeft,
            b"LU" => Self::SurroundLowerRight,
            b"OL" => Self::SurroundRight,
            b"XX" => Self::Overlaid,
            b"MI" => Self::HorizontalReflection,
            b"RO" => Self::Rotation,
            b"SU" => Self::Subtraction,
            _ => return None,
        })
    }
}

impl std::fmt::Display for IdsOp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.symbol().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_round_trip_and_arities_match() {
        assert_eq!(IdsOp::ALL.len(), 17);
        for op in IdsOp::ALL {
            assert_eq!(IdsOp::from_char(op.symbol()), Some(op));
            assert_eq!(IdsOp::from_abbreviation(op.abbreviation()), Some(op));
        }
        assert_eq!(IdsOp::LeftMiddleRight.arity(), 3);
        assert_eq!(IdsOp::AboveMiddleBelow.arity(), 3);
        assert_eq!(IdsOp::Rotation.arity(), 1);
        assert_eq!(IdsOp::Subtraction.arity(), 2);
        assert_eq!(IdsOp::from_char('木'), None);
        assert_eq!(IdsOp::from_abbreviation("lr"), None);
    }
}
