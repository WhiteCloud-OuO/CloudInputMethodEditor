//! UIElement 生命周期：组句帧变化时创建 / 更新 / 结束候选元素。
//!
//! 只有 `ui_less` 线程或 `host_reads_opt_in` 名单里的宿主才创建；普通宿主完全不碰 UIElement，
//! 行为与加 P1 之前一致。`BeginUIElement` 回 `FALSE` 时记 `host_draws`，之后每帧 `UpdateUIElement`。

use windows::Win32::UI::TextServices::ITfUIElement;
use windows::core::BOOL;

use super::element::CandidateElement;
use crate::com::log::log;
use crate::com::service::TextService_Impl;

/// 当前活着的候选元素；`_element` 只用来保活（管理器还会 AddRef，但别依赖它）。
pub(crate) struct ActiveElement {
    _element: ITfUIElement,
    id: u32,
}

impl TextService_Impl {
    /// 组句帧变化后调用：按需创建 / 更新 / 结束候选元素。
    pub(crate) fn sync_ui_element(&self) {
        let opt_in = self
            .input_settings
            .get()
            .is_some_and(|settings| settings.host_reads_opt_in);
        let enabled = self.shared.ui_less() || opt_in;
        if !enabled {
            self.end_ui_element();
            return;
        }
        let Some(manager) = self.ui_element_mgr.borrow().clone() else {
            return;
        };
        if self.shared.with_ui_frame(|frame| frame.is_empty()) {
            self.end_ui_element();
            return;
        }
        if self.ui_element.borrow().is_none() {
            let element: ITfUIElement = CandidateElement::new(self.shared.clone()).into();
            let mut show = BOOL(1);
            let mut id = 0u32;
            if let Err(error) = unsafe { manager.BeginUIElement(&element, &mut show, &mut id) } {
                log(&format!("BeginUIElement 失败，关闭 UIElement: {error}"));
                *self.ui_element_mgr.borrow_mut() = None;
                return;
            }
            self.shared.set_host_draws(!show.as_bool());
            *self.ui_element.borrow_mut() = Some(ActiveElement {
                _element: element,
                id,
            });
            // 宿主说它不画、又不在名单里：这是声明 UI-less 但行为不符，按不弹处理（规范语义）。
            if show.as_bool() && !opt_in {
                self.end_ui_element();
                return;
            }
        }
        if let Some(active) = self.ui_element.borrow().as_ref() {
            let _ = unsafe { manager.UpdateUIElement(active.id) };
        }
    }

    /// 结束当前元素；没有就什么也不做。
    pub(crate) fn end_ui_element(&self) {
        let Some(active) = self.ui_element.borrow_mut().take() else {
            return;
        };
        if let Some(manager) = self.ui_element_mgr.borrow().as_ref() {
            let _ = unsafe { manager.EndUIElement(active.id) };
        }
        self.shared.set_host_draws(false);
    }
}
