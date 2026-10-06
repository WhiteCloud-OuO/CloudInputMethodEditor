//! 系统「字体」对话框：选字族与字号（`[candidate]` 那三个字体项用）。

use windows::Win32::Graphics::Gdi::LOGFONTW;
use windows::Win32::UI::Controls::Dialogs::{
    CF_INITTOLOGFONTSTRUCT, CF_SCREENFONTS, CHOOSEFONTW, ChooseFontW,
};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows::core::w;

use cloudime_platform::FontChoice;

/// 弹一次系统字体对话框，返回选中的字族与字号（点）；用户取消返回 `None`。
/// 窗口标题即设置窗口（单一实例那段也是这么找它的）。
pub(super) fn pick_font(current: &FontChoice) -> Option<FontChoice> {
    let mut logfont = LOGFONTW {
        // 负的 lfHeight 表示字符高度，正好是字号对应的像素值
        lfHeight: -(current.size.round() as i32),
        ..Default::default()
    };
    for (slot, ch) in logfont
        .lfFaceName
        .iter_mut()
        .zip(current.family.encode_utf16())
    {
        *slot = ch;
    }
    let mut chooser = CHOOSEFONTW {
        lStructSize: size_of::<CHOOSEFONTW>() as u32,
        hwndOwner: unsafe { FindWindowW(None, w!("云朵输入法 设置")) }.unwrap_or_default(),
        lpLogFont: &mut logfont,
        Flags: CF_SCREENFONTS | CF_INITTOLOGFONTSTRUCT,
        ..Default::default()
    };
    if !unsafe { ChooseFontW(&mut chooser) }.as_bool() {
        return None;
    }
    let family: String = logfont
        .lfFaceName
        .iter()
        .take_while(|ch| **ch != 0)
        .filter_map(|ch| char::from_u32(u32::from(*ch)))
        .collect();
    // iPointSize 是「点的十分之一」
    let size = if chooser.iPointSize > 0 {
        chooser.iPointSize as f32 / 10.0
    } else {
        current.size
    };
    Some(FontChoice { family, size })
}
