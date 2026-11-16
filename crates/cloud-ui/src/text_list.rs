//! 文本列表在窗口内容里的原地替换：脚本增删项目后，整块换掉同名的那一条 `ListBox`。
//!
//! 独立窗口与辅助窗口共用这一份；只依赖 `cloudime_render` 的 `Document`，不碰窗口壳。

use cloudime_render::{Document, Element, ListBox};

/// 用新的 `text_list` 元素替换文档里同名的那一块：滚动偏移保留并夹到新范围。
///
/// 选中项由脚本侧副本算好（它会跟着 `selected_index_changed` 同步），这里不再按旧行文本覆盖，
/// 否则脚本显式改 `selected_index` 会被旧选中项盖回去。
pub(crate) fn replace(document: &mut Document, name: &str, mut list: ListBox) -> bool {
    let Some(index) = document.elements.iter().position(
        |element| matches!(element, Element::ListBox(current) if current.region_name == name),
    ) else {
        return false;
    };
    if let Some(Element::ListBox(old)) = document.elements.get(index) {
        list.scroll_offset = old.scroll_offset.min(list.max_scroll_px());
    }
    document.elements[index] = Element::ListBox(list);
    true
}
