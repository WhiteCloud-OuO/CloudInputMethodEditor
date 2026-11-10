//! UI-less 宿主的候选元素：`ITfUIElement` + `ITfCandidateListUIElement` + Behavior。
//!
//! getter 直接读 [`Shared`](crate::com::composition::Shared) 里的最新帧（同一条 TSF 线程），
//! 不做 IPC；`GetString` 第一次被调用时记 `host_reads` 并上报（推断宿主可能自己画）。

use std::cell::Cell;
use std::rc::Rc;

use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, S_FALSE};
use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfCandidateListUIElement_Impl, ITfCandidateListUIElementBehavior,
    ITfCandidateListUIElementBehavior_Impl, ITfDocumentMgr, ITfUIElement, ITfUIElement_Impl,
    TF_CLUIE_COUNT, TF_CLUIE_CURRENTPAGE, TF_CLUIE_DOCUMENTMGR, TF_CLUIE_PAGEINDEX,
    TF_CLUIE_SELECTION, TF_CLUIE_STRING,
};
use windows::core::{BOOL, BSTR, Error, GUID, Result, implement};

use cloudime_platform::protocol::CandidateAction;

use crate::com::composition::Shared;

/// 一次组句里的候选元素；帧内容由 `Shared` 提供，宿主在 `UpdateUIElement` 后同步读。
#[implement(
    ITfUIElement,
    ITfCandidateListUIElement,
    ITfCandidateListUIElementBehavior
)]
pub(crate) struct CandidateElement {
    shared: Rc<Shared>,
    shown: Cell<bool>,
}

impl CandidateElement {
    pub(crate) fn new(shared: Rc<Shared>) -> Self {
        Self {
            shared,
            shown: Cell::new(true),
        }
    }
}

impl ITfUIElement_Impl for CandidateElement_Impl {
    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from("云朵输入法候选"))
    }

    fn GetGUID(&self) -> Result<GUID> {
        Ok(GUID::zeroed())
    }

    fn Show(&self, bshow: BOOL) -> Result<()> {
        self.shown.set(bshow.as_bool());
        // 宿主接管 / 交还绘制权：host_draws 跟着变，Server 据此收 / 放自己的候选窗。
        self.shared.set_host_draws(!bshow.as_bool());
        Ok(())
    }

    fn IsShown(&self) -> Result<BOOL> {
        Ok(self.shown.get().into())
    }
}

impl ITfCandidateListUIElement_Impl for CandidateElement_Impl {
    fn GetUpdatedFlags(&self) -> Result<u32> {
        // 每次更新都置全位：宿主据此重读一遍，候选窗内容本来就不大。
        Ok(TF_CLUIE_DOCUMENTMGR
            | TF_CLUIE_COUNT
            | TF_CLUIE_SELECTION
            | TF_CLUIE_STRING
            | TF_CLUIE_PAGEINDEX
            | TF_CLUIE_CURRENTPAGE)
    }

    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn GetCount(&self) -> Result<u32> {
        let count = self
            .shared
            .with_ui_frame(|frame| frame.candidates.items.len());
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    fn GetSelection(&self) -> Result<u32> {
        let selection = self.shared.with_ui_frame(|frame| {
            (frame.highlight < frame.candidates.items.len())
                .then(|| u32::try_from(frame.highlight).unwrap_or(u32::MAX))
        });
        selection.ok_or_else(|| Error::from_hresult(S_FALSE))
    }

    fn GetString(&self, uindex: u32) -> Result<BSTR> {
        // 宿主把候选串读走：推断它可能自己画；只有 `host_drawn_candidates` 名单里的进程才据此收窗。
        self.shared.note_host_reads();
        let text = self.shared.with_ui_frame(|frame| {
            frame
                .candidates
                .items
                .get(uindex as usize)
                .map(|candidate| {
                    BSTR::from(candidate.display.as_deref().unwrap_or(&candidate.text))
                })
        });
        text.ok_or_else(|| Error::from_hresult(E_INVALIDARG))
    }

    fn GetPageIndex(&self, pindex: *mut u32, usize: u32, pupagecnt: *mut u32) -> Result<()> {
        // 只交当页：页数恒 1，第一页从 0 开始。
        unsafe {
            if !pupagecnt.is_null() {
                *pupagecnt = 1;
            }
            if !pindex.is_null() && usize > 0 {
                *pindex = 0;
            }
        }
        Ok(())
    }

    fn SetPageIndex(&self, _pindex: *const u32, _upagecnt: u32) -> Result<()> {
        // 分页由输入法自己的翻页键决定；宿主这边接受但不改页。
        Ok(())
    }

    fn GetCurrentPage(&self) -> Result<u32> {
        Ok(0)
    }
}

impl ITfCandidateListUIElementBehavior_Impl for CandidateElement_Impl {
    fn SetSelection(&self, nindex: u32) -> Result<()> {
        let _ = self
            .shared
            .ui_element_action(CandidateAction::Select(nindex as usize));
        // 高亮变了：让宿主重读一遍（`UpdateUIElement` 会回调 getter）。
        crate::com::service::with_active(|service| service.sync_ui_element());
        Ok(())
    }

    fn Finalize(&self) -> Result<()> {
        if let Some((frame, commit)) = self.shared.ui_element_action(CandidateAction::Finalize)
            && let Some(commit) = commit
        {
            // 不在元素回调里直接开编辑会话：把上屏文本交给轮询那一拍应用。
            self.shared.defer_ui_commit(frame, commit);
        }
        Ok(())
    }

    fn Abort(&self) -> Result<()> {
        let _ = self.shared.ui_element_action(CandidateAction::Abort);
        self.shared.end_composing();
        Ok(())
    }
}
