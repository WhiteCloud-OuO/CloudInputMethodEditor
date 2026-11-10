//! 最小 `ITextStoreACP`：足够 TSF 建组句、读写文本、量光标矩形。
//!
//! 只实现 TSF 组句真正会走的那几个方法；格式化文本 / 嵌入对象 / 属性一律 `E_NOTIMPL`。
//! 锁是同步授予的（`RequestLock` 里直接回调 `OnLockGranted`），宿主 UI 线程不会被跨线程锁住。

use std::cell::RefCell;

use windows::Win32::Foundation::{E_NOTIMPL, HWND, POINT, RECT, S_FALSE, S_OK};
use windows::Win32::System::Com::{FORMATETC, IDataObject};
use windows::Win32::UI::TextServices::{
    ITextStoreACP, ITextStoreACP_Impl, ITextStoreACPSink, TEXT_STORE_LOCK_FLAGS,
    TEXT_STORE_TEXT_CHANGE_FLAGS, TS_AE_START, TS_ATTRVAL, TS_RT_PLAIN, TS_RUNINFO,
    TS_SELECTION_ACP, TS_SELECTIONSTYLE, TS_STATUS, TS_TEXTCHANGE,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{
    BOOL, Error, GUID, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement,
};

struct StoreState {
    text: Vec<u16>,
    caret: i32,
    sink: Option<ITextStoreACPSink>,
    hwnd: HWND,
}

/// 测试宿主的文本存区；内容在 `text()` 里读出来打印。
#[implement(ITextStoreACP)]
pub(crate) struct TextStore {
    state: RefCell<StoreState>,
}

impl TextStore {
    pub(crate) fn new(hwnd: HWND) -> Self {
        Self {
            state: RefCell::new(StoreState {
                text: Vec::new(),
                caret: 0,
                sink: None,
                hwnd,
            }),
        }
    }

    /// 当前文本（控制台打印用）。
    pub(crate) fn text(&self) -> String {
        String::from_utf16_lossy(&self.state.borrow().text)
    }

    fn notify(&self, change: TS_TEXTCHANGE) {
        let sink = self.state.borrow().sink.clone();
        if let Some(sink) = sink {
            let _ = unsafe { sink.OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change) };
        }
    }
}

impl ITextStoreACP_Impl for TextStore_Impl {
    fn AdviseSink(&self, _riid: *const GUID, punk: Ref<IUnknown>, _dwmask: u32) -> Result<()> {
        if let Ok(sink) = punk.ok()?.cast::<ITextStoreACPSink>() {
            self.state.borrow_mut().sink = Some(sink);
        }
        Ok(())
    }

    fn UnadviseSink(&self, _punk: Ref<IUnknown>) -> Result<()> {
        self.state.borrow_mut().sink = None;
        Ok(())
    }

    fn RequestLock(&self, dwlockflags: u32) -> Result<windows::core::HRESULT> {
        let sink = self.state.borrow().sink.clone();
        if let Some(sink) = sink {
            unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(dwlockflags))? };
            Ok(S_OK)
        } else {
            Ok(S_FALSE)
        }
    }

    fn GetStatus(&self) -> Result<TS_STATUS> {
        Ok(TS_STATUS {
            dwDynamicFlags: 0,
            dwStaticFlags: 0,
        })
    }

    fn QueryInsert(
        &self,
        acpteststart: i32,
        acptestend: i32,
        cch: u32,
        pacpresultstart: *mut i32,
        pacpresultend: *mut i32,
    ) -> Result<()> {
        let len = self.state.borrow().text.len() as i32;
        let start = acpteststart.clamp(0, len);
        let end = acptestend.clamp(start, len);
        unsafe {
            if !pacpresultstart.is_null() {
                *pacpresultstart = start;
            }
            if !pacpresultend.is_null() {
                *pacpresultend = end + cch as i32;
            }
        }
        Ok(())
    }

    fn GetSelection(
        &self,
        ulindex: u32,
        ulcount: u32,
        pselection: *mut TS_SELECTION_ACP,
        pcfetched: *mut u32,
    ) -> Result<()> {
        let state = self.state.borrow();
        let caret = state.caret.clamp(0, state.text.len() as i32);
        unsafe {
            if ulindex == 0 && ulcount > 0 && !pselection.is_null() {
                *pselection = TS_SELECTION_ACP {
                    acpStart: caret,
                    acpEnd: caret,
                    style: TS_SELECTIONSTYLE {
                        ase: TS_AE_START,
                        fInterimChar: BOOL(0),
                    },
                };
                if !pcfetched.is_null() {
                    *pcfetched = 1;
                }
            } else if !pcfetched.is_null() {
                *pcfetched = 0;
            }
        }
        Ok(())
    }

    fn SetSelection(&self, ulcount: u32, pselection: *const TS_SELECTION_ACP) -> Result<()> {
        if ulcount > 0 && !pselection.is_null() {
            let selection = unsafe { *pselection };
            let mut state = self.state.borrow_mut();
            let len = state.text.len() as i32;
            state.caret = selection.acpEnd.clamp(0, len);
        }
        Ok(())
    }

    fn GetText(
        &self,
        acpstart: i32,
        acpend: i32,
        pchplain: PWSTR,
        cchplainreq: u32,
        pcchplainret: *mut u32,
        prgruninfo: *mut TS_RUNINFO,
        cruninforeq: u32,
        pcruninforet: *mut u32,
        pacpnext: *mut i32,
    ) -> Result<()> {
        let state = self.state.borrow();
        let len = state.text.len() as i32;
        let start = acpstart.clamp(0, len);
        let end = acpend.clamp(start, len);
        let count = ((end - start) as u32).min(cchplainreq);
        if !pchplain.is_null() && count > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    state.text[start as usize..].as_ptr(),
                    pchplain.0,
                    count as usize,
                );
            }
        }
        unsafe {
            if !pcchplainret.is_null() {
                *pcchplainret = count;
            }
            if !prgruninfo.is_null() && cruninforeq > 0 {
                *prgruninfo = TS_RUNINFO {
                    uCount: count,
                    r#type: TS_RT_PLAIN,
                };
            }
            if !pcruninforet.is_null() {
                *pcruninforet = u32::from(count > 0);
            }
            if !pacpnext.is_null() {
                *pacpnext = start + count as i32;
            }
        }
        Ok(())
    }

    fn SetText(
        &self,
        _dwflags: u32,
        acpstart: i32,
        acpend: i32,
        pchtext: &PCWSTR,
        cch: u32,
    ) -> Result<TS_TEXTCHANGE> {
        let inserted = unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize) }.to_vec();
        let mut state = self.state.borrow_mut();
        let len = state.text.len() as i32;
        let start = acpstart.clamp(0, len) as usize;
        let end = acpend.clamp(start as i32, len) as usize;
        state.text.splice(start..end, inserted.iter().copied());
        state.caret = (start + inserted.len()) as i32;
        let change = TS_TEXTCHANGE {
            acpStart: start as i32,
            acpOldEnd: end as i32,
            acpNewEnd: (start + inserted.len()) as i32,
        };
        drop(state);
        self.notify(change);
        Ok(change)
    }

    fn GetFormattedText(&self, _acpstart: i32, _acpend: i32) -> Result<IDataObject> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn GetEmbedded(
        &self,
        _acppos: i32,
        _rguidservice: *const GUID,
        _riid: *const GUID,
    ) -> Result<IUnknown> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn QueryInsertEmbedded(
        &self,
        _pguidservice: *const GUID,
        _pformatetc: *const FORMATETC,
    ) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn InsertEmbedded(
        &self,
        _dwflags: u32,
        _acpstart: i32,
        _acpend: i32,
        _pdataobject: Ref<IDataObject>,
    ) -> Result<TS_TEXTCHANGE> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn InsertTextAtSelection(
        &self,
        _dwflags: u32,
        pchtext: &PCWSTR,
        cch: u32,
        pacpstart: *mut i32,
        pacpend: *mut i32,
        pchange: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        let inserted = unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize) }.to_vec();
        let mut state = self.state.borrow_mut();
        let start = state.caret.clamp(0, state.text.len() as i32) as usize;
        state.text.splice(start..start, inserted.iter().copied());
        state.caret = (start + inserted.len()) as i32;
        let change = TS_TEXTCHANGE {
            acpStart: start as i32,
            acpOldEnd: start as i32,
            acpNewEnd: (start + inserted.len()) as i32,
        };
        drop(state);
        self.notify(change);
        unsafe {
            if !pacpstart.is_null() {
                *pacpstart = change.acpStart;
            }
            if !pacpend.is_null() {
                *pacpend = change.acpNewEnd;
            }
            if !pchange.is_null() {
                *pchange = change;
            }
        }
        Ok(())
    }

    fn InsertEmbeddedAtSelection(
        &self,
        _dwflags: u32,
        _pdataobject: Ref<IDataObject>,
        _pacpstart: *mut i32,
        _pacpend: *mut i32,
        _pchange: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn RequestSupportedAttrs(
        &self,
        _dwflags: u32,
        _cfilterattrs: u32,
        _pafilterattrs: *const GUID,
    ) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsAtPosition(
        &self,
        _acppos: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const GUID,
        _dwflags: u32,
    ) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsTransitioningAtPosition(
        &self,
        _acppos: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const GUID,
        _dwflags: u32,
    ) -> Result<()> {
        Ok(())
    }

    fn FindNextAttrTransition(
        &self,
        _acpstart: i32,
        _acphalt: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const GUID,
        _dwflags: u32,
        _pacpnext: *mut i32,
        _pffound: *mut BOOL,
        _plfoundoffset: *mut i32,
    ) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn RetrieveRequestedAttrs(
        &self,
        _ulcount: u32,
        _paattrvals: *mut TS_ATTRVAL,
        pcfetched: *mut u32,
    ) -> Result<()> {
        if !pcfetched.is_null() {
            unsafe { *pcfetched = 0 };
        }
        Ok(())
    }

    fn GetEndACP(&self) -> Result<i32> {
        Ok(self.state.borrow().text.len() as i32)
    }

    fn GetActiveView(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetACPFromPoint(&self, _vcview: u32, _ptscreen: *const POINT, _dwflags: u32) -> Result<i32> {
        Ok(-1)
    }

    fn GetTextExt(
        &self,
        _vcview: u32,
        _acpstart: i32,
        _acpend: i32,
        prc: *mut RECT,
        pfclipped: *mut BOOL,
    ) -> Result<()> {
        unsafe {
            if !prc.is_null() {
                *prc = RECT {
                    left: 40,
                    top: 40,
                    right: 40,
                    bottom: 60,
                };
            }
            if !pfclipped.is_null() {
                *pfclipped = BOOL(0);
            }
        }
        Ok(())
    }

    fn GetScreenExt(&self, _vcview: u32) -> Result<RECT> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.state.borrow().hwnd, &mut rect)? };
        Ok(rect)
    }

    fn GetWnd(&self, _vcview: u32) -> Result<HWND> {
        Ok(self.state.borrow().hwnd)
    }
}
