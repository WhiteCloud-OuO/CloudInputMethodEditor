use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// 所有 IDS 字符串首尾相接的 UTF-8 arena。
pub const TEXT_TAG: [u8; 4] = *b"TEXT";

/// 按 IDS 字节序排序的键表。
pub const KEYS_TAG: [u8; 4] = *b"KEYS";

/// 开放寻址哈希表，槽里是 `KEYS` 下标。
pub const HASH_TAG: [u8; 4] = *b"HASH";

/// 一个 IDS 对应的字 / 来源 / 标记。
pub const ENTR_TAG: [u8; 4] = *b"ENTR";

/// `KEYS` 的一项：IDS 在 `TEXT` 里的位置，以及它对应的一段 `ENTR`。
///
/// 字段顺序按「无内部填充」排：`u32, u32, u16, u16, u16, u16` = 16 字节，
/// 这样写文件时可以直接 `as_bytes()`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, Immutable, KnownLayout, IntoBytes)]
#[repr(C)]
pub struct IdsKey {
    /// `TEXT` 里的字节偏移。
    pub start: u32,

    /// `ENTR` 里的起始下标。
    pub first_entry: u32,

    /// IDS 的字节长度。
    pub len: u16,

    /// 这个 IDS 对应几个字。
    pub entry_count: u16,

    /// 预留。
    pub reserved: u16,

    /// 预留。
    pub reserved2: u16,
}

/// `ENTR` 的一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, Immutable, KnownLayout, IntoBytes)]
#[repr(C)]
pub struct IdsEntry {
    /// Unicode 标量值。
    pub codepoint: u32,

    /// BabelStone 来源位图（见 [`SOURCE_LETTERS`](crate::SOURCE_LETTERS)）。
    pub sources: u16,

    /// [`FLAG_VARIATION`](crate::FLAG_VARIATION) 等标记。
    pub flags: u16,
}
