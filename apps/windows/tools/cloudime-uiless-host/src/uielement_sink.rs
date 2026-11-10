//! 测试宿主的 `ITfUIElementSink`：`BeginUIElement` 一律回 `pbShow=FALSE`（自己画），
//! 在 `UpdateUIElement` 里把候选页读出来打印，验证输入法交的数据。

use std::cell::Cell;
use std::rc::Rc;

use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfUIElementMgr, ITfUIElementSink, ITfUIElementSink_Impl,
};
use windows::core::{BOOL, Interface, Result, implement};

#[implement(ITfUIElementSink)]
pub(crate) struct UiElementSink {
    manager: ITfUIElementMgr,
    /// 最近一次 `Begin` 的元素 id；`End` 清空。`--auto-pick` 用它去 QI Behavior。
    element_id: Rc<Cell<Option<u32>>>,
}

impl UiElementSink {
    pub(crate) fn new(manager: ITfUIElementMgr, element_id: Rc<Cell<Option<u32>>>) -> Self {
        Self {
            manager,
            element_id,
        }
    }

    fn dump(&self, id: u32) {
        let Ok(element) = (unsafe { self.manager.GetUIElement(id) }) else {
            println!("  [uielement] GetUIElement({id}) 失败");
            return;
        };
        let Ok(candidates) = element.cast::<ITfCandidateListUIElement>() else {
            println!("  [uielement] 元素不是 ITfCandidateListUIElement");
            return;
        };
        let count = unsafe { candidates.GetCount() }.unwrap_or(0);
        let selection = unsafe { candidates.GetSelection() }.ok();
        let current_page = unsafe { candidates.GetCurrentPage() }.ok();
        let mut items = Vec::new();
        for index in 0..count {
            match unsafe { candidates.GetString(index) } {
                Ok(text) => items.push(text.to_string()),
                Err(error) => items.push(format!("<GetString 失败: {error}>")),
            }
        }
        println!(
            "  [uielement] count={count} selection={selection:?} page={current_page:?} items={items:?}"
        );
    }
}

impl ITfUIElementSink_Impl for UiElementSink_Impl {
    fn BeginUIElement(&self, id: u32, pbshow: *mut BOOL) -> Result<()> {
        self.element_id.set(Some(id));
        // 宿主自己画：输入法那边据此收掉自绘候选窗。
        unsafe {
            if !pbshow.is_null() {
                *pbshow = BOOL(0);
            }
        }
        println!("[uielement] Begin id={id} -> pbShow=FALSE");
        Ok(())
    }

    fn UpdateUIElement(&self, id: u32) -> Result<()> {
        println!("[uielement] Update id={id}");
        self.dump(id);
        Ok(())
    }

    fn EndUIElement(&self, id: u32) -> Result<()> {
        self.element_id.set(None);
        println!("[uielement] End id={id}");
        Ok(())
    }
}
