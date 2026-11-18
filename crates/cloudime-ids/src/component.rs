//! `COMP` 分节：难输入部件表。
//!
//! 每项 28 字节、4 字节对齐、没有内部填充，可以整表零拷贝读取。分类用五位千分比存分布，
//! 一个部件可以同时属于多个分类（比如「木」既是旁、也能当底）；部件 SVG 存在 `CTEXT`。

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// 部件表分节的标签。
pub const COMPONENT_TAG: [u8; 4] = *b"COMP";

/// 部件字形的 SVG arena 分节。
pub const COMPONENT_TEXT_TAG: [u8; 4] = *b"CTXT";

/// `Component.svg_off` 取这个值表示没有内置字形。
pub const COMPONENT_NO_SVG: u32 = u32::MAX;

/// 这个部件能用拼音打出来（词库里有单字词条）。
pub const COMPONENT_FLAG_TYPEABLE: u16 = 1;

/// 来自 `--derive` 的自动候选，还没有人工确认。
pub const COMPONENT_FLAG_DERIVED: u16 = 1 << 1;

/// 笔画是按 IDS 递归推算的，不是来源数据。
pub const COMPONENT_FLAG_STROKE_ESTIMATED: u16 = 1 << 2;

/// 最大笔画档位：`13` 表示 13 画及以上。
pub const MAX_STROKE: u8 = 13;

/// 部件在字形里的位置分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ComponentRole {
    /// 0：包围（囗 / 冂 / 辶 / 广 / 疒 …）。
    Surround = 0,

    /// 1：上部。
    Top = 1,

    /// 2：左右。
    Side = 2,

    /// 3：下部。
    Bottom = 3,

    /// 4：其它（重叠、减除、中段、位置不固定）。
    Other = 4,
}

impl ComponentRole {
    /// 全部角色，按存储下标排。
    pub const ALL: [Self; 5] = [
        Self::Surround,
        Self::Top,
        Self::Side,
        Self::Bottom,
        Self::Other,
    ];

    /// 从存储下标取角色。
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }

    /// 存储下标。
    pub fn index(self) -> u8 {
        self as u8
    }

    /// 中文名，UI 标签与报告用。
    pub fn name(self) -> &'static str {
        match self {
            Self::Surround => "圍",
            Self::Top => "頭",
            Self::Side => "旁",
            Self::Bottom => "底",
            Self::Other => "其它",
        }
    }
}

/// 一个难输入部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, Immutable, KnownLayout, IntoBytes)]
#[repr(C)]
pub struct Component {
    /// 部件本身。
    pub codepoint: u32,

    /// 五个分类的千分比，总和 1000。
    pub roles: [u16; 5],

    /// 1..=13；13 表示 13+；0 = 未知。
    pub stroke: u8,

    /// 主分类下标（0..=4）。
    pub primary: u8,

    /// [`COMPONENT_FLAG_TYPEABLE`] 等标记。
    pub flags: u16,

    /// 预留。
    pub reserved: u16,

    /// `CTEXT` 里 SVG 的字节偏移；[`COMPONENT_NO_SVG`] = 没有。
    pub svg_off: u32,

    /// SVG 字节长度。
    pub svg_len: u16,

    /// 预留。
    pub reserved2: u16,
}

impl Component {
    /// 部件字符。
    pub fn character(&self) -> Option<char> {
        char::from_u32(self.codepoint)
    }

    /// 主分类。
    pub fn primary_role(&self) -> ComponentRole {
        ComponentRole::from_index(self.primary).unwrap_or(ComponentRole::Other)
    }

    /// 某个分类的千分比。
    pub fn role_score(&self, role: ComponentRole) -> u16 {
        self.roles[role.index() as usize]
    }

    /// 能不能用拼音打出来。
    pub fn is_typeable(&self) -> bool {
        self.flags & COMPONENT_FLAG_TYPEABLE != 0
    }

    /// 有没有内置 SVG 字形。
    pub fn has_svg(&self) -> bool {
        self.svg_off != COMPONENT_NO_SVG
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::*;

    #[test]
    fn layout_has_no_padding_and_fits_the_file_format() {
        assert_eq!(size_of::<Component>(), 28);
        assert_eq!(align_of::<Component>(), 4);
    }

    #[test]
    fn roles_round_trip_through_the_index() {
        for role in ComponentRole::ALL {
            assert_eq!(ComponentRole::from_index(role.index()), Some(role));
            assert!(!role.name().is_empty());
        }
        assert_eq!(ComponentRole::from_index(5), None);
    }
}
