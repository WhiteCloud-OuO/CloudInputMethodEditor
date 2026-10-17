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
//! - **焦点**（[`Focus`]）属于**一组**（所有者 + 它名下的资产），一组里**任何时刻恰好一个**持有者：
//!   缺省在所有者身上（独立窗口创建并显示后总是具备焦点）；所有者可以把它**指定**给某件声明了
//!   [`Asset::able_to_get_focus`] 的资产（[`DesignatedAssetToGetFocus`]）—— 那之后由那件资产
//!   响应鼠标 / 键盘，所有者不再接受。**所有辅助窗口默认不具备焦点**（鼠标一律穿透、点不着）。
//!   两件**同一所有者**的辅助窗口之间转移焦点也要**经桥接**（走 [`Bridge::designate_focus`]，
//!   资产之间不直接通讯，见 §3），指认目标用 [`Asset::name`]。
//! - 术语：把自己的焦点**转移给另一个窗口**的过程叫**失焦**（unfocusing），**获得**焦点的过程叫
//!   **聚焦**（focusing）—— 资产收 [`Asset::focusing`] / [`Asset::unfocusing`]，所有者收
//!   [`DesignatedAssetToGetFocus::focusing`] / [`DesignatedAssetToGetFocus::unfocusing`]。
//!
//! 现在名下只有一份资产：候选窗口的「键位映射参考」（[`key_map`]）—— 它**没**声明
//! `able_to_get_focus`，所以眼下整组一直由候选窗口持有焦点。
//!
//! ⚠️ 「**资产克隆**」与「**资产窃取**」（别的独立窗口来克隆本窗口的资产、克隆完把原件销毁）也写在
//! `docs/design/windows.md` §5 —— **本输入法暂时用不上**，先把规则定下来（将来要加的是一个
//! 「可克隆」trait，比如 `Clonable: Asset`）。

pub(super) mod key_map;

use std::cell::Cell;

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

/// **焦点**（focus）在谁身上 —— 「让窗口具备响应键盘 / 鼠标的功能」的那个特性。
///
/// 焦点属于**一组**（一个独立窗口 + 它名下的资产），一组里同时只有**一个**持有者：
///
/// - 缺省在**所有者**（独立窗口）身上 —— 独立窗口创建并显示后总是具备焦点；
/// - 所有者可以把它**指定**给某件资产（[`Assets::designate_focus`]）；那件资产必须先在创建时
///   声明 [`Asset::able_to_get_focus`]，否则接不住；
/// - 指定之后由那件资产响应鼠标 / 键盘，**原所有者不再接受**；
/// - 归还三条路（[`Assets::return_focus_to_owner`]）：所有者主动收回、那件资产**收起**（[`Assets::hide`]）、
///   它走到**生命周期末**（[`Assets::retire`]）—— 后两条都**自动**触发。
///
/// ⚠️ 这是**逻辑焦点**，不是 Win32 的 `SetFocus`：咱们的窗口全是 `WS_EX_NOACTIVATE`，
/// 真去抢系统焦点会把用户正在打字的那个应用弄丢。落地上「具备焦点」= 这一组里**由谁接受输入**：
/// 没有焦点的一方鼠标一律**穿透**（点不着），键也轮不到它。
// 第一轮只落机制：还没有谁来**指定**焦点，所以 `Focus::Asset` 这个变体眼下没被生产代码构造（测试里有）。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Focus {
    /// 在所有者（独立窗口）身上。
    #[default]
    Owner,

    /// 已经指定给名下的第几件资产（[`Assets`] 里的下标）。
    Asset(usize),
}

/// **指定资产获取焦点**（`designated_asset_to_get_focus`）：**独立窗口都实现它**（由
/// [`super::independent_window_template::IndependentWindowTemplate`] 统一实现，所以每块独立窗口都有）。
///
/// 这是**所有者**那一侧的特权：把焦点从自己身上指定给名下某件资产，或者把它收回来。
/// 资产自己说了不算 —— 声明了 [`Asset::able_to_get_focus`] 也只是「能」接，真拿到要所有者指定。
// 第一轮只落机制：还没有谁来调它（后续部分接入「谁在什么时候指定焦点」时会用上）。
#[allow(dead_code)]
pub(super) trait DesignatedAssetToGetFocus {
    /// 把焦点**指定**给名下叫 `name` 的那件资产（`designated_asset_to_get_focus`）。
    /// `false` = 没动 —— 没这个名字、那件没声明 [`Asset::able_to_get_focus`]、或者焦点本来就在它手上。
    fn designate_focus(&self, name: &str) -> bool;

    /// 让焦点**归还给所有者**（`return_focus_to_owner`）。`false` = 没动（焦点本来就在所有者手上）。
    fn return_focus_to_owner(&self) -> bool;

    /// 这一组的焦点这会儿在谁身上。
    fn focus(&self) -> Focus;

    /// **聚焦**：焦点落到自己（所有者）身上了 —— 轮到自己响应鼠标 / 键盘。
    fn focusing(&self) {}

    /// **失焦**：自己把焦点**转移出去了**（指定给了名下某件资产）—— 不再接受鼠标 / 键盘。
    fn unfocusing(&self) {}
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

    /// 请所有者把焦点**指定**给名下叫 `name` 的那件资产 —— **同一所有者的两件辅助窗口之间转移焦点
    /// 只能走这条**（资产之间不直接通讯，见 §3）。`false` = 没动（没这件 / 它接不住 / 本来就在它手上）。
    // 机制已就位：还没有哪件资产自己要求转手（后续部分接入触发时会用上）。
    #[allow(dead_code)]
    fn designate_focus(&self, name: &str) -> bool;

    /// 焦点**交还给所有者**：辅助窗口自己要把焦点放回去时走这条。
    /// 收起与生命周期末那两条由资产表**自动**走（[`Assets::hide`] / [`Assets::retire`]），资产不用操心。
    // 机制已就位：还没有哪件资产自己要求归还（自动归还那条已经在走）。
    #[allow(dead_code)]
    fn return_focus_to_owner(&self);
}

/// 一份「辅助窗口」资产：由宿主独立窗口创建与销毁。
///
/// **所有者唯一且不转移**：谁建的谁就是它唯一的所有者，之后一直待在宿主的 [`Assets`] 里；
/// 它够得着的只有宿主每次经桥接交过来的 [`Bridge`]。
pub(super) trait Asset {
    /// 这件资产的**名字**：同一所有者的名下**不许重名**（[`Assets::add`] 里 `debug_assert` 盯着）。
    /// 跨资产指认（[`Bridge::designate_focus`]）与日志用它 —— 名字要**稳定**，加 / 删别的资产不影响它。
    fn name(&self) -> &'static str;

    /// 宿主贴出一帧 / 开关变了之后调：要什么（几何、开关…）都经 `owner` 要，再把自己摆好。
    ///
    /// `owner` 就是它的所有者（宿主那一块独立窗口），而所有者**只借**这一下。
    fn sync(&self, owner: &dyn Bridge);

    /// 收起（宿主收起、或销毁之前）。
    fn hide(&self);

    /// 换「文字 / 版面类资产」的内容（键位映射参考的键盘版面）。
    fn set_key_map(&self, _layout: KeyMapLayout) {}

    /// 创建时声明自己**允不允许获取焦点**（`able_to_get_focus`）：`false`（缺省）= **不具备焦点** ——
    /// 鼠标 / 键盘一律不响应（所有辅助窗口默认如此）。允许也只是「能」，真拿到还要所有者**指定**
    /// （[`DesignatedAssetToGetFocus`]）。
    fn able_to_get_focus(&self) -> bool {
        false
    }

    /// **聚焦**：所有者把焦点**指定**给它了 —— 从此由它响应鼠标 / 键盘（原所有者不再接受）。
    fn focusing(&self) {}

    /// **失焦**：它把自己的焦点**转移出去**了（交还给所有者、转给同一所有者名下的另一件资产，
    /// 或者收起 / 生命周期末的**自动归还**）—— 把响应输入的那些开关关掉。
    fn unfocusing(&self) {}
}

/// 一个独立窗口名下的全部资产 + 这一组的**焦点**。
///
/// **所有权**：资产由宿主创建、存这里，**不移交也不借出去** —— 这个类型只有「收进来」（[`Self::add`]）
/// 与「放掉」（[`Self::retire`]），没有「取出来」的口子；宿主跟资产之间只有 [`Self::sync`] /
/// [`Self::set_key_map`] 这条**桥接**路（[`Self::sync`] 把自己作为 [`Bridge`] 借给它们用一下）。
///
/// **顺序**：宿主先建好自己那块窗，再 [`Self::add`]；销毁时 `Drop` 的第一件事是
/// [`Self::retire`]（在 `DestroyWindow` 自己之前）—— 资产的寿命就落在这段区间里。
///
/// **焦点**（[`Focus`]）：这一组的成员里**任何时刻恰好一个**持有者，缺省是**所有者**自己；所有者可以把焦点
/// **指定**给某件声明了 [`Asset::able_to_get_focus`] 的资产（[`Self::designate_focus`]，按 [`Asset::name`] 认）。
/// 归还三条路：所有者主动收回（[`Self::return_focus_to_owner`]）、那件资产**收起**（[`Self::hide`]）、它走到
/// **生命周期末**（[`Self::retire`]）—— 后两条**自动**走一遍归还，焦点不会落在一件看不见、或者马上就
/// 没了的东西上。
#[derive(Default)]
pub(super) struct Assets {
    /// 名下的资产（[`Focus::Asset`] 里的下标就是这儿的顺序）。
    items: Vec<Box<dyn Asset>>,

    /// 这一组的焦点在谁身上（缺省在所有者身上）。
    focus: Cell<Focus>,
}

impl Assets {
    /// 收下一份资产（所有权就落在这儿了）。**必须**在宿主自己那块窗建好之后调。
    ///
    /// **不动焦点**：新收进来的资产哪怕声明了 [`Asset::able_to_get_focus`]，也**不会自己拿到焦点** ——
    /// 焦点还在原持有者（所有者、或者同一所有者名下的某件姊妹资产）身上，要它拿还是得所有者**指定**
    /// （`docs/design/windows.md` §4 规则 8）。**资产克隆与资产窃取**都是这条：克隆件 / 窃取件进来时一样，
    /// 特性照旧、焦点不动。
    ///
    /// 名字重了在 debug 下直接叫出来 —— [`Self::designate_focus`] 按名字指认，重名就指不准了。
    pub(super) fn add(&mut self, asset: Box<dyn Asset>) {
        debug_assert!(
            self.index_of(asset.name()).is_none(),
            "同一所有者名下的资产重名：{}",
            asset.name()
        );
        self.items.push(asset);
    }

    /// 这一组的焦点在谁身上。
    pub(super) fn focus(&self) -> Focus {
        self.focus.get()
    }

    /// 把焦点**指定**给名下叫 `name` 的那件资产（`designated_asset_to_get_focus`）。
    /// `false` = 没动 —— 没这个名字、那件没声明 [`Asset::able_to_get_focus`]、或者焦点本来就在它手上。
    // 机制已就位：还没有谁来主动指定（后续部分接入触发时会用上；归还那条已经在走）。
    #[allow(dead_code)]
    pub(super) fn designate_focus(&self, name: &str) -> bool {
        let Some(index) = self.index_of(name) else {
            return false;
        };
        let target = Focus::Asset(index);
        self.allows(target) && self.move_focus(target)
    }

    /// 让焦点**归还给所有者**（`return_focus_to_owner`）。`false` = 没动（本来就在所有者手上）。
    pub(super) fn return_focus_to_owner(&self) -> bool {
        self.move_focus(Focus::Owner)
    }

    /// 宿主贴出一帧 / 开关变了：把全部资产驱动一遍 —— 手上把自己（作为 [`Bridge`]）借给它们，
    /// 它们要什么经桥接回来要。
    pub(super) fn sync(&self, owner: &dyn Bridge) {
        // 焦点只可能落在**所有者**、或一件**声明过** `able_to_get_focus` 的资产上（指定时查过一遍）。
        // 在驱动这一拍上复核：将来谁改了指定的路，这里第一时间叫出来。
        debug_assert!(
            self.allows(self.focus.get()),
            "焦点落在接不住的窗口上：{:?}",
            self.focus.get()
        );
        for asset in &self.items {
            asset.sync(owner);
        }
    }

    /// 全部收起（宿主收起时）。**带着焦点的那件一收起就先自动归还焦点** —— 否则它看不见了、
    /// 所有者又不接输入，这一组没人响应鼠标 / 键盘。
    pub(super) fn hide(&self) {
        self.return_focus_to_owner();
        for asset in &self.items {
            asset.hide();
        }
    }

    /// 换全部资产的版面内容。
    pub(super) fn set_key_map(&self, layout: KeyMapLayout) {
        for asset in &self.items {
            asset.set_key_map(layout.clone());
        }
    }

    /// 放掉全部资产（宿主销毁**之前**的第一件事）：**销毁前一刻先自动归还焦点** ——
    /// 焦点不能落在一件马上就要没了的资产上。
    pub(super) fn retire(&mut self) {
        self.return_focus_to_owner();
        self.items.clear();
    }

    /// 名下叫 `name` 的那件资产在第几位（[`Self::add`] 查重与 [`Self::designate_focus`] 指认都用）。
    fn index_of(&self, name: &str) -> Option<usize> {
        self.items.iter().position(|asset| asset.name() == name)
    }

    /// 焦点换人：老持有者**失焦**、新持有者**聚焦**。返回是否真换了人（本来就在 `target` 手上就是 `false`）。
    ///
    /// 焦点是**转移**不是复制 —— 一组里任何时刻只有一个持有者。
    fn move_focus(&self, target: Focus) -> bool {
        let previous = self.focus.get();
        if previous == target {
            return false;
        }
        self.focus.set(target);
        tracing::debug!(
            from = %self.describe(previous),
            to = %self.describe(target),
            "焦点换人"
        );
        if let Focus::Asset(index) = previous
            && let Some(asset) = self.items.get(index)
        {
            asset.unfocusing();
        }
        if let Focus::Asset(index) = target
            && let Some(asset) = self.items.get(index)
        {
            asset.focusing();
        }
        true
    }

    /// `target` 这会儿接不接得住焦点：所有者永远接得住；资产得声明过 [`Asset::able_to_get_focus`]。
    fn allows(&self, target: Focus) -> bool {
        match target {
            Focus::Owner => true,
            Focus::Asset(index) => self
                .items
                .get(index)
                .is_some_and(|asset| asset.able_to_get_focus()),
        }
    }

    /// 日志里的说法：所有者自己，还是哪件资产（带名字）。
    fn describe(&self, holder: Focus) -> String {
        match holder {
            Focus::Owner => "所有者".to_owned(),
            Focus::Asset(index) => self
                .items
                .get(index)
                .map_or_else(|| format!("资产#{index}"), |asset| asset.name().to_owned()),
        }
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

        fn designate_focus(&self, _name: &str) -> bool {
            false
        }

        fn return_focus_to_owner(&self) {}
    }

    /// 测试用资产：声明自己允不允许获取焦点，并把「聚焦 / 失焦」「收起」「放掉」记成一串事件。
    struct Tracked {
        name: &'static str,
        focusable: bool,
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Tracked {
        fn new(
            name: &'static str,
            focusable: bool,
            events: Rc<RefCell<Vec<&'static str>>>,
        ) -> Self {
            Self {
                name,
                focusable,
                events,
            }
        }

        fn record(&self, what: &'static str) {
            self.events.borrow_mut().push(what);
        }
    }

    impl Asset for Tracked {
        fn name(&self) -> &'static str {
            self.name
        }

        fn sync(&self, _owner: &dyn Bridge) {}

        fn hide(&self) {
            self.record("hide");
        }

        fn able_to_get_focus(&self) -> bool {
            self.focusable
        }

        fn focusing(&self) {
            self.record("focus");
        }

        fn unfocusing(&self) {
            self.record("unfocus");
        }
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.record("drop");
        }
    }

    /// 测试用资产：记下自己被收起过几次、什么时候被放掉。
    struct Fake {
        dropped: Rc<Cell<bool>>,
        hidden: Rc<Cell<u32>>,
    }

    impl Asset for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

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
            fn name(&self) -> &'static str {
                "spy"
            }

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

    /// 焦点缺省在所有者身上；只有**声明过** `able_to_get_focus` 的资产接得住（没这个名字的也不行）。
    #[test]
    fn focus_starts_on_the_owner_and_only_declared_assets_can_take_it() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut assets = Assets::default();
        assets.add(Box::new(Tracked::new("focusable", true, events.clone())));
        assets.add(Box::new(Tracked::new("plain", false, events.clone())));
        assert_eq!(assets.focus(), Focus::Owner, "缺省在所有者身上");

        assert!(
            !assets.designate_focus("plain"),
            "没声明 able_to_get_focus → 接不住"
        );
        assert!(!assets.designate_focus("没有这件"), "没这个名字");
        assert_eq!(assets.focus(), Focus::Owner);
        assert!(events.borrow().is_empty(), "没人拿到过焦点");

        assert!(assets.designate_focus("focusable"));
        assert_eq!(assets.focus(), Focus::Asset(0));
        assert_eq!(*events.borrow(), ["focus"], "接手的**聚焦**了");
        assert!(
            !assets.designate_focus("focusable"),
            "已经在它手上，不用再动"
        );
    }

    /// 焦点在资产之间挪、以及归还给所有者：老的**失焦**、新的**聚焦**（顺序不能反）。
    #[test]
    fn focus_moves_between_assets_and_comes_back_to_the_owner() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut assets = Assets::default();
        assets.add(Box::new(Tracked::new("first", true, events.clone())));
        assets.add(Box::new(Tracked::new("second", true, events.clone())));

        assert!(assets.designate_focus("first"));
        assert!(assets.designate_focus("second"));
        assert_eq!(assets.focus(), Focus::Asset(1));
        assert_eq!(*events.borrow(), ["focus", "unfocus", "focus"]);

        assert!(assets.return_focus_to_owner());
        assert_eq!(assets.focus(), Focus::Owner);
        assert_eq!(*events.borrow(), ["focus", "unfocus", "focus", "unfocus"]);
        assert!(!assets.return_focus_to_owner(), "本来就在所有者手上");
    }

    /// 收起与生命周期末都**自动**归还焦点：收起时先归还再收起，销毁时先归还再放掉 ——
    /// 焦点不会落在一件看不见、或者马上就要没了的资产上。
    #[test]
    fn hiding_and_retiring_return_the_focus_before_they_go() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut assets = Assets::default();
        assets.add(Box::new(Tracked::new("only", true, events.clone())));
        assert!(assets.designate_focus("only"));

        assets.hide();
        assert_eq!(assets.focus(), Focus::Owner, "一收起就自动归还");
        assert_eq!(
            *events.borrow(),
            ["focus", "unfocus", "hide"],
            "先归还再收起"
        );

        events.borrow_mut().clear();
        assert!(assets.designate_focus("only"));
        assets.retire();
        assert_eq!(assets.focus(), Focus::Owner, "销毁前一刻自动归还");
        assert_eq!(
            *events.borrow(),
            ["focus", "unfocus", "drop"],
            "先归还再放掉"
        );
    }

    /// 一组（所有者 + 名下的资产）里**任何时刻恰好一个**持有者 —— 焦点是**转移**不是复制。
    /// 走几轮「指定第一件 → 转手第二件 → 归还」，每件资产的日志都是「聚焦 / 失焦」交替，不会连着来两次聚焦。
    #[test]
    fn focus_is_transferred_never_copied() {
        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        let mut assets = Assets::default();
        assets.add(Box::new(Tracked::new("first", true, first.clone())));
        assets.add(Box::new(Tracked::new("second", true, second.clone())));

        for _ in 0..3 {
            assert!(assets.designate_focus("first"));
            assert!(assets.designate_focus("second"));
            assert!(assets.return_focus_to_owner());
        }
        let alternating = ["focus", "unfocus", "focus", "unfocus", "focus", "unfocus"];
        assert_eq!(*first.borrow(), alternating, "第一件");
        assert_eq!(*second.borrow(), alternating, "第二件");
        assert_eq!(assets.focus(), Focus::Owner);
    }

    /// 桥上也有「把焦点交还给所有者」这条：辅助窗口自己要把焦点放回去时走它
    /// （收起与生命周期末那两条是资产表自动走的，资产不用操心）。
    #[test]
    fn an_asset_can_hand_the_focus_back_through_the_bridge() {
        struct Asked(Rc<Cell<u32>>);

        impl Bridge for Asked {
            fn host(&self) -> Host {
                Host::default()
            }

            fn asset_on(&self) -> bool {
                true
            }

            fn designate_focus(&self, _name: &str) -> bool {
                false
            }

            fn return_focus_to_owner(&self) {
                self.0.set(self.0.get() + 1);
            }
        }

        struct Hands;

        impl Asset for Hands {
            fn name(&self) -> &'static str {
                "hands"
            }

            fn sync(&self, owner: &dyn Bridge) {
                owner.return_focus_to_owner();
            }

            fn hide(&self) {}
        }

        let asked = Rc::new(Cell::new(0));
        let mut assets = Assets::default();
        assets.add(Box::new(Hands));
        assets.sync(&Asked(asked.clone()));
        assert_eq!(asked.get(), 1, "sync 里经桥请所有者把焦点收回");
    }

    /// **同一所有者**的两件辅助窗口之间转移焦点**经桥接**：A1 在 `sync` 里请所有者把焦点交给 A2
    /// （资产之间不直接通讯，只能委托所有者 —— `docs/design/windows.md` §4 规则 2）。
    #[test]
    fn siblings_hand_the_focus_over_through_the_bridge() {
        /// 所有者：把资产表借给桥用（真身上是宿主独立窗口自己）。
        struct Owner<'a>(&'a Assets);

        impl Bridge for Owner<'_> {
            fn host(&self) -> Host {
                Host::default()
            }

            fn asset_on(&self) -> bool {
                true
            }

            fn designate_focus(&self, name: &str) -> bool {
                self.0.designate_focus(name)
            }

            fn return_focus_to_owner(&self) {
                self.0.return_focus_to_owner();
            }
        }

        /// 一件资产：`sync` 时按 `ask` 里写的名字请所有者把焦点转过去（模拟 A1 让位给 A2）。
        struct Sibling {
            name: &'static str,
            ask: Option<&'static str>,
        }

        impl Asset for Sibling {
            fn name(&self) -> &'static str {
                self.name
            }

            fn sync(&self, owner: &dyn Bridge) {
                if let Some(name) = self.ask {
                    assert!(owner.designate_focus(name), "经桥指定给 {name}");
                }
            }

            fn hide(&self) {}

            fn able_to_get_focus(&self) -> bool {
                true
            }
        }

        let mut assets = Assets::default();
        assets.add(Box::new(Sibling {
            name: "a1",
            ask: Some("a2"),
        }));
        assets.add(Box::new(Sibling {
            name: "a2",
            ask: None,
        }));
        assert!(assets.designate_focus("a1"), "先让 A1 拿到焦点");
        assert_eq!(assets.focus(), Focus::Asset(0));

        // 驱动那一拍：A1 自己把焦点让给同一所有者名下的 A2
        assets.sync(&Owner(&assets));
        assert_eq!(assets.focus(), Focus::Asset(1), "焦点转到了 A2");
    }

    /// §4 规则 8：**资产克隆 / 窃取**来的资产与原件具备同样的特性（能不能获得焦点照旧），但**不会第一时间
    /// 拿到焦点** —— 焦点仍在它的所有者、或同一所有者名下的某件姊妹资产身上。
    ///
    /// 克隆与窃取在**这一组**里是同一个落点（往资产表里加一件，`Assets::add`），差别只在原件那边
    /// （窃取会销毁原件、它那一组按规则 7 自动归还焦点；克隆不扰动别人）。
    #[test]
    fn cloned_or_stolen_assets_keep_their_capability_but_never_take_the_focus() {
        // 先有一件能获得焦点的姊妹资产，并且它正拿着焦点
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut assets = Assets::default();
        assets.add(Box::new(Tracked::new("sibling", true, events.clone())));
        assert!(assets.designate_focus("sibling"));
        assert_eq!(assets.focus(), Focus::Asset(0));

        // 「窃取」进来一件：克隆自别的所有者名下、本来能获得焦点的原件
        assets.add(Box::new(Tracked::new("stolen", true, events.clone())));
        assert_eq!(
            assets.focus(),
            Focus::Asset(0),
            "焦点还在姊妹资产身上，没跟着新来的走"
        );
        assert_eq!(*events.borrow(), ["focus"], "新来的没聚焦、也没让别人失焦");

        // 特性照旧：它**能**拿到焦点（只是要所有者指定）
        assert!(assets.designate_focus("stolen"));
        assert_eq!(assets.focus(), Focus::Asset(1));

        // 另一件窃取来的、原本就不能获得焦点的：同样不动焦点，也照样接不住
        assets.add(Box::new(Tracked::new(
            "stolen-plain",
            false,
            events.clone(),
        )));
        assert_eq!(assets.focus(), Focus::Asset(1), "也没跟着走");
        assert!(!assets.designate_focus("stolen-plain"), "特性照旧：接不住");
        assert_eq!(assets.focus(), Focus::Asset(1));
    }
}
