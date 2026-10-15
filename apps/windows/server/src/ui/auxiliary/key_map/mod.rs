//! 键位映射参考：**候选窗口的一件资产**（见 [`super`] 那份说明）。
//!
//! 把当前双键拼音方案的小写键（声母 / 韵母 / 零声母）逐行列出来，给新手对着敲。
//! 窗口本身（分层窗、主题背景 + 圆角、阴影、贴宿主摆位）走辅助窗口模板 [`AuxiliaryWindowTemplate`]；
//! 内容（几行文字）由 Server 按当前方案生成，经宿主（候选窗口）转过来。
//!
//! 开关在设置页「输入 → 双键拼音 → 启用键位映射参考」（`use_input_scheme.cfg` 的 `key_map_reference`）；
//! 开着时组句里按 `Ctrl + F1` 弹出 / 收起。

use windows::core::Result;

use super::{Asset, Bridge};
use crate::ui::auxiliary_window_template::AuxiliaryWindowTemplate;
use crate::ui::painter::SharedPainter;
use cloudime_platform::input_schemes::KeyMapLayout;

/// 键位映射参考窗口。
pub(crate) struct KeyMapWindow {
    /// 辅助窗口模板：分层面板 + 主题背景 / 圆角 + 阴影 + 贴宿主摆位。
    template: AuxiliaryWindowTemplate,
}

impl KeyMapWindow {
    /// 建一个隐藏的键位映射参考窗口（宿主独立窗口建好自己那块窗之后调）。
    pub(crate) fn new(painter: SharedPainter) -> Result<Self> {
        Ok(Self {
            template: AuxiliaryWindowTemplate::new(painter)?,
        })
    }
}

impl Asset for KeyMapWindow {
    fn sync(&self, owner: &dyn Bridge) {
        // 桥接：几何与开关都**经所有者**要（资产够不着别的窗口）
        let host = owner.host();
        // 关着 / 宿主正展开着（一整屏候选要让位）→ 收起；宿主没显示（没矩形）也收起
        if !owner.asset_on() || host.expanded {
            self.hide();
            return;
        }
        match host.rect {
            Some(rect) => self.template.show(rect, host.layout),
            None => self.hide(),
        }
    }

    fn hide(&self) {
        self.template.hide();
    }

    fn set_key_map(&self, layout: KeyMapLayout) {
        self.template.set_key_map(layout);
    }
}
