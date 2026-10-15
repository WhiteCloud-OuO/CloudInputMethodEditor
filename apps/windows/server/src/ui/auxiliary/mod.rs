//! 辅助窗口：挂在某个**独立窗口**上的辅助信息 / 提示窗口 —— 它是那块独立窗口的**资产**（asset）。
//!
//! 规矩见 `docs/design/windows.md`（摘要如下）：
//!
//! - **所有者唯一且不转移**：每个辅助窗口**有且只有一个所有者**（它的宿主独立窗口），
//!   所有权在它整个生命周期里**不移交、也不被直接借用** —— 它就一直待在宿主的 [`Assets`] 里
//!   （那里只有「收进来」与「放掉」，没有「取出来」的口子）。
//! - **只能经桥接跟所有者打交道**：辅助窗口**不能**直接跟别的辅助窗口、或不是它所有者的独立窗口通讯；
//!   要几何、要开关、要内容都得**委托所有者**去办 —— 这条委托路叫**桥接**（bridging），
//!   就是 [`Bridge`]：所有权在宿主手里，每次驱动资产时把 `&dyn Bridge` 借给它用一下。
//! - **生命周期始终绑定**到宿主：宿主创建**之后**才有它，宿主销毁**之前**先放掉它
//!   （所以宿主 `Drop` 的第一件事是 [`Assets::retire`]，之后才 `DestroyWindow` 自己那块窗）；
//! - 内容也由宿主转达（Server 送来的东西经宿主落到资产上，[`Asset::set_key_map`]）。
//!
//! 现在名下只有一份资产：候选窗口的「键位映射参考」（[`key_map`]）。
//!
//! ⚠️ 「**资产克隆**」与「**资产窃取**」（别的独立窗口来克隆本窗口的资产、克隆完把原件销毁）也写在
//! `docs/design/windows.md` §4 —— **本输入法暂时用不上**，先把规则定下来（将来要加的是一个
//! 「可克隆」trait，比如 `Clonable: Asset`）。

pub(super) mod key_map;

use windows::Win32::Foundation::RECT;

use cloudime_platform::LayoutMode;
use cloudime_platform::input_schemes::KeyMapLayout;

/// 宿主独立窗口喂给资产的那点信息：宿主的**内容区**屏幕矩形、排布方向、是不是展开着（Tab）。
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Host {
    /// 宿主内容区的屏幕矩形；宿主没显示时为 `None`（资产这时该收起）。
    pub rect: Option<RECT>,

    /// 宿主的排布方向：资产按它决定贴宿主右侧（竖排）还是下方（横排）。
    pub layout: LayoutMode,

    /// 宿主是不是展开着（候选窗的 Tab「更多候选项」）：展开时资产让位。
    pub expanded: bool,
}

/// **桥接**（bridging）：辅助窗口**委托它的所有者**去办事的那条路。
///
/// 规矩：辅助窗口**不能**直接跟别的辅助窗口、也不能跟不是它所有者的独立窗口通讯 ——
/// 要几何、要开关、要内容，都得委托所有者。这个 trait 由**所有者**（宿主独立窗口）实现，
/// 每次驱动资产时把 `&dyn Bridge` **借**给它用一下（所有权仍在宿主手里，不转移）。
pub(super) trait Bridge {
    /// 宿主（所有者）自己的形状：内容区屏幕矩形 / 排布方向 / 是不是展开着。
    fn host(&self) -> Host;

    /// 这件资产这会儿开着没有（用户按 `Ctrl+F1` 之类切的）。
    fn asset_on(&self) -> bool;
}

/// 一份「辅助窗口」资产：由宿主独立窗口创建与销毁。
///
/// **所有者唯一且不转移**：谁建的谁就是它唯一的所有者，之后一直待在宿主的 [`Assets`] 里；
/// 它够得着的只有宿主每次经桥接交过来的 [`Bridge`]。
pub(super) trait Asset {
    /// 宿主贴出一帧 / 开关变了之后调：要什么（几何、开关…）都经 `owner` 要，再把自己摆好。
    ///
    /// `owner` 就是它的所有者（宿主那一块独立窗口），而所有者**只借**这一下。
    fn sync(&self, owner: &dyn Bridge);

    /// 收起（宿主收起、或销毁之前）。
    fn hide(&self);

    /// 换「文字 / 版面类资产」的内容（键位映射参考的键盘版面）。
    fn set_key_map(&self, _layout: KeyMapLayout) {}
}

/// 一个独立窗口名下的全部资产。
///
/// **所有权**：资产由宿主创建、存这里，**不移交也不借出去** —— 这个类型只有「收进来」（[`Self::add`]）
/// 与「放掉」（[`Self::retire`]），没有「取出来」的口子；宿主跟资产之间只有 [`Self::sync`] /
/// [`Self::set_key_map`] 这条**桥接**路（[`Self::sync`] 把自己作为 [`Bridge`] 借给它们用一下）。
///
/// **顺序**：宿主先建好自己那块窗，再 [`Self::add`]；销毁时 `Drop` 的第一件事是
/// [`Self::retire`]（在 `DestroyWindow` 自己之前）—— 资产的寿命就落在这段区间里。
#[derive(Default)]
pub(super) struct Assets(Vec<Box<dyn Asset>>);

impl Assets {
    /// 收下一份资产（所有权就落在这儿了）。**必须**在宿主自己那块窗建好之后调。
    pub(super) fn add(&mut self, asset: Box<dyn Asset>) {
        self.0.push(asset);
    }

    /// 宿主贴出一帧 / 开关变了：把全部资产驱动一遍 —— 手上把自己（作为 [`Bridge`]）借给它们，
    /// 它们要什么经桥接回来要。
    pub(super) fn sync(&self, owner: &dyn Bridge) {
        for asset in &self.0 {
            asset.sync(owner);
        }
    }

    /// 全部收起（宿主收起时）。
    pub(super) fn hide(&self) {
        for asset in &self.0 {
            asset.hide();
        }
    }

    /// 换全部资产的版面内容。
    pub(super) fn set_key_map(&self, layout: KeyMapLayout) {
        for asset in &self.0 {
            asset.set_key_map(layout.clone());
        }
    }

    /// 放掉全部资产（宿主销毁**之前**的第一件事）。
    pub(super) fn retire(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use cloudime_platform::LayoutMode;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// 测试用的所有者：固定一套「开着没有 + 宿主形状」。
    struct Owner {
        on: bool,
        expanded: bool,
    }

    impl Bridge for Owner {
        fn host(&self) -> Host {
            Host {
                rect: None,
                layout: LayoutMode::Vertical,
                expanded: self.expanded,
            }
        }

        fn asset_on(&self) -> bool {
            self.on
        }
    }

    /// 测试用资产：记下自己被收起过几次、什么时候被放掉。
    struct Fake {
        dropped: Rc<Cell<bool>>,
        hidden: Rc<Cell<u32>>,
    }

    impl Asset for Fake {
        fn sync(&self, _owner: &dyn Bridge) {}
        fn hide(&self) {
            self.hidden.set(self.hidden.get() + 1);
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }

    /// `retire()` 一调资产就没了（宿主销毁之前先调它）；`hide()` 只收起、不放掉。
    #[test]
    fn retiring_assets_drops_them_while_hiding_keeps_them() {
        let dropped = Rc::new(Cell::new(false));
        let hidden = Rc::new(Cell::new(0));
        let mut assets = Assets::default();
        assets.add(Box::new(Fake {
            dropped: dropped.clone(),
            hidden: hidden.clone(),
        }));

        assets.hide();
        assert_eq!(hidden.get(), 1);
        assert!(!dropped.get(), "收起不等于放掉");

        assets.retire();
        assert!(dropped.get(), "retire 才放掉资产");
    }

    /// 资产拿到的只有所有者经**桥接**交过来的那一个口子：`sync` 时它读到的是所有者说的那套值。
    #[test]
    fn assets_see_their_owners_bridge() {
        struct Spy(Rc<RefCell<Option<(bool, bool)>>>);
        impl Asset for Spy {
            fn sync(&self, owner: &dyn Bridge) {
                *self.0.borrow_mut() = Some((owner.asset_on(), owner.host().expanded));
            }
            fn hide(&self) {}
        }

        let seen = Rc::new(RefCell::new(None));
        let mut assets = Assets::default();
        assets.add(Box::new(Spy(seen.clone())));
        assets.sync(&Owner {
            on: true,
            expanded: true,
        });
        assert_eq!(*seen.borrow(), Some((true, true)));
    }
}
