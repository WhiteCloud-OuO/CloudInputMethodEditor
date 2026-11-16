//! 文本列表在窗口内容里的原地替换：脚本增删项目后，整块换掉同名的那一条 `ListBox`。
//!
//! 独立窗口与辅助窗口共用这一份；只依赖 `cloudime_render` 的 `Document`，不碰窗口壳。

use cloudime_render::{Document, Element, ListBox};

/// 用新的 `text_list` 元素替换文档里同名的那一块：选中按行文本保留，滚动偏移夹到新范围。
///
/// 找不到同名列表时返回 `false`；调用方决定怎么记日志。
pub(crate) fn replace(document: &mut Document, name: &str, mut list: ListBox) -> bool {
    let Some(index) = document.elements.iter().position(
        |element| matches!(element, Element::ListBox(current) if current.region_name == name),
    ) else {
        return false;
    };
    let (key, scroll) = match document.elements.get(index) {
        Some(Element::ListBox(old)) => (
            (old.selected_index >= 0)
                .then(|| old.row(old.selected_index as usize))
                .flatten()
                .and_then(|row| row.key.clone()),
            old.scroll_offset,
        ),
        _ => (None, 0.0),
    };
    if list.allow_select
        && let Some(key) = key
        && let Some(row) = list.rows.iter().find(|row| row.key.as_ref() == Some(&key))
    {
        list.selected_index = row.index as i32;
    }
    list.scroll_offset = scroll.min(list.max_scroll_px());
    document.elements[index] = Element::ListBox(list);
    true
}
