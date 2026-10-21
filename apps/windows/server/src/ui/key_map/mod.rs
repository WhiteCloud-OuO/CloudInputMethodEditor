//! 键位映射参考：**候选窗口的一件资产**（资产模型见 [`cloud_ui::auxiliary`]）。
//!
//! 把当前双键拼音方案的小写键（声母 / 韵母 / 零声母）逐行列出来，给新手对着敲。
//! 窗口本身（主题背景 + 圆角、贴宿主摆位）走辅助窗口模板 [`AuxiliaryWindowTemplate`] —— 那块是**绘制**
//! （GDI 直画窗口 DC，见 `docs/design/windows.md` §6），所以没有阴影、圆角是窗口区域切出来的硬边；
//! 内容（几行文字）由 Server 按当前方案生成，经宿主（候选窗口）转过来。
//!
//! 开关在设置页「输入 → 双键拼音 → 启用键位映射参考」（`use_input_scheme.cfg` 的 `key_map_reference`）；
//! 开着时组句里按 `Ctrl + F1` 弹出 / 收起。

use std::rc::Rc;

use windows::core::Result;

use cloud_ui::auxiliary::{Asset, Bridge};
use cloud_ui::auxiliary_window_template::AuxiliaryWindowTemplate;
use cloud_ui::painter::SharedPainter;
use cloudime_platform::input_schemes::KeyMapLayout;

/// 键位映射参考窗口。
pub(crate) struct KeyMapWindow {
    /// 辅助窗口模板：主题背景 / 圆角 / 阴影都出一张位图，贴宿主摆位。
    template: Rc<AuxiliaryWindowTemplate>,
}

impl KeyMapWindow {
    /// 建一个隐藏的键位映射参考窗口（宿主独立窗口建好自己那块窗之后调）。
    ///
    /// 它是块**纯参考面板**：`able_to_get_focus` 按缺省（`false`）—— 不具备焦点，鼠标一律穿透、点不着。
    /// 将来要它接输入时，把这里的 `false` 改成 `true`，再在宿主那边**指定**焦点给它
    /// （[`cloud_ui::auxiliary::DesignatedAssetToGetFocus`]）。
    pub(crate) fn new(painter: SharedPainter) -> Result<Self> {
        let template = Rc::new(AuxiliaryWindowTemplate::new(painter, false)?);
        template.attach();
        Ok(Self { template })
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

    fn set_symbol_key_map(&self, layout: KeyMapLayout) {
        self.template.set_symbol_key_map(layout);
    }

    /// 这件资产的名字：同一所有者名下唯一（宿主的 `Assets::add` 盯着），跨资产指认与日志用它。
    fn name(&self) -> &'static str {
        "key_map"
    }

    /// 创建时声明过才有（见 [`Self::new`]）：取的是模板上那一份，两处不会各说各的。
    fn able_to_get_focus(&self) -> bool {
        self.template.focusable()
    }

    /// **聚焦**：焦点指定给它了 —— 这块面板开始接鼠标（鼠标穿透关掉）。
    fn focusing(&self) {
        self.template.set_focused(true);
    }

    /// **失焦**：把焦点交出去了（归还给宿主、转给同一所有者名下的别的资产，或收起 / 生命周期末的
    /// 自动归还）—— 回到鼠标穿透。
    fn unfocusing(&self) {
        self.template.set_focused(false);
    }
}
