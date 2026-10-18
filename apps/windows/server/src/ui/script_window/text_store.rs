//! `text_edit` 的 TSF 文本存储：把 `ITextStoreACP` 的读写 / 选区 / 光标矩形桥到 windowless
//! RichEdit（`ITextServices`）。候选窗位置、组合文本、提交都靠这一层。
//!
//! 只实现单行纯文本需要的部分；富文本属性 / 嵌入对象一律返回 `E_NOTIMPL`。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{
    E_NOINTERFACE, E_NOTIMPL, HWND, LPARAM, LRESULT, POINT, RECT, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Controls::RichEdit::ITextServices;
use windows::Win32::UI::Controls::{
    EM_CHARFROMPOS, EM_GETSEL, EM_POSFROMCHAR, EM_REPLACESEL, EM_SCROLLCARET, EM_SETSEL,
};
use windows::Win32::UI::TextServices::{
    ITextStoreACP, ITextStoreACP_Impl, ITextStoreACPSink, TEXT_STORE_LOCK_FLAGS, TS_AE_NONE,
    TS_AS_LAYOUT_CHANGE, TS_AS_SEL_CHANGE, TS_AS_TEXT_CHANGE, TS_DEFAULT_SELECTION,
    TS_E_NOSELECTION, TS_IAS_NOQUERY, TS_IAS_QUERYONLY, TS_LC_CHANGE, TS_RT_PLAIN, TS_RUNINFO,
    TS_SELECTION_ACP, TS_SELECTIONSTYLE, TS_ST_NONE, TS_STATUS, TS_TEXTCHANGE,
};
use windows::Win32::UI::WindowsAndMessaging::{WM_GETTEXT, WM_GETTEXTLENGTH};
use windows::core::{
    BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement,
};

/// 供宿主窗口通知 TSF 的句柄（与 COM 对象共享 sink / mask）。
#[derive(Clone)]
pub(super) struct TextStoreHandle {
    state: Rc<StoreState>,
}

impl TextStoreHandle {
    /// RichEdit 自己吃掉的鼠标 / 键盘改动后，告诉 TSF 选区变了。
    pub(super) fn notify_selection(&self) {
        if self.state.mask.get() & TS_AS_SEL_CHANGE == 0 {
            return;
        }
        let sink = self.state.sink.borrow().clone();
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnSelectionChange();
            }
        }
    }

    /// 布局（字号 / 位置）变了；TSF 用来重算候选窗位置。
    pub(super) fn notify_layout(&self) {
        if self.state.mask.get() & TS_AS_LAYOUT_CHANGE == 0 {
            return;
        }
        let sink = self.state.sink.borrow().clone();
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnLayoutChange(TS_LC_CHANGE, 0);
            }
        }
    }
}

#[derive(Default)]
struct StoreState {
    sink: RefCell<Option<ITextStoreACPSink>>,
    mask: Cell<u32>,
}

/// 建一个桥到 `services` 的 TSF 文本存储；返回 COM 接口和给宿主用的通知句柄。
pub(super) fn create(
    hwnd: HWND,
    services: ITextServices,
    line_height: i32,
) -> (ITextStoreACP, TextStoreHandle) {
    let state = Rc::new(StoreState::default());
    let store = RichEditTextStore {
        hwnd,
        services,
        line_height: line_height.max(1),
        state: state.clone(),
    };
    (store.into(), TextStoreHandle { state })
}

#[implement(ITextStoreACP)]
struct RichEditTextStore {
    hwnd: HWND,
    services: ITextServices,
    line_height: i32,
    state: Rc<StoreState>,
}

impl RichEditTextStore {
    fn send(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut result = LRESULT(0);
        unsafe {
            let _ = self
                .services
                .TxSendMessage(msg, wparam, lparam, &mut result);
        }
        result
    }

    fn text(&self) -> Vec<u16> {
        let length = self.send(WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0.max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = self
            .send(
                WM_GETTEXT,
                WPARAM(length + 1),
                LPARAM(buffer.as_mut_ptr() as isize),
            )
            .0
            .max(0) as usize;
        buffer.truncate(copied.min(length));
        buffer
    }

    fn text_len(&self) -> i32 {
        self.send(WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0.max(0) as i32
    }

    fn selection(&self) -> (i32, i32) {
        let mut start = 0u32;
        let mut end = 0u32;
        self.send(
            EM_GETSEL,
            WPARAM(&mut start as *mut u32 as usize),
            LPARAM(&mut end as *mut u32 as isize),
        );
        (start as i32, end as i32)
    }

    fn set_selection(&self, start: i32, end: i32) {
        self.send(
            EM_SETSEL,
            WPARAM(start.max(0) as usize),
            LPARAM(end.max(0) as isize),
        );
    }

    fn replace_range(&self, start: i32, end: i32, text: &[u16]) -> TS_TEXTCHANGE {
        let mut text = text.to_vec();
        text.push(0);
        self.set_selection(start, end);
        self.send(EM_REPLACESEL, WPARAM(1), LPARAM(text.as_ptr() as isize));
        let _ = self.send(EM_SCROLLCARET, WPARAM(0), LPARAM(0));
        TS_TEXTCHANGE {
            acpStart: start,
            acpOldEnd: end,
            acpNewEnd: start + text.len() as i32 - 1,
        }
    }

    fn notify_text(&self, change: TS_TEXTCHANGE) {
        if self.state.mask.get() & TS_AS_TEXT_CHANGE == 0 {
            return;
        }
        let sink = self.state.sink.borrow().clone();
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnTextChange(TS_ST_NONE, &change);
            }
        }
    }

    fn caret_rect(&self, start: i32, end: i32) -> RECT {
        let position = |index: i32| {
            let packed = self
                .send(EM_POSFROMCHAR, WPARAM(0), LPARAM(index.max(0) as isize))
                .0 as u32;
            let x = (packed & 0xFFFF) as i16 as i32;
            let y = ((packed >> 16) & 0xFFFF) as i16 as i32;
            (x, y)
        };
        let (x, y) = position(start);
        let (right, _) = if end > start {
            position(end)
        } else {
            (x + 1, y)
        };
        let mut top_left = POINT { x, y };
        let mut bottom_right = POINT {
            x: right,
            y: y + self.line_height,
        };
        unsafe {
            let _ = ClientToScreen(self.hwnd, &mut top_left);
            let _ = ClientToScreen(self.hwnd, &mut bottom_right);
        }
        RECT {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x.max(top_left.x + 1),
            bottom: bottom_right.y.max(top_left.y + 1),
        }
    }
}

impl ITextStoreACP_Impl for RichEditTextStore_Impl {
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<IUnknown>, dwmask: u32) -> Result<()> {
        if unsafe { *riid } != ITextStoreACPSink::IID {
            return Err(E_NOINTERFACE.into());
        }
        let sink: ITextStoreACPSink = punk.ok()?.cast()?;
        *self.state.sink.borrow_mut() = Some(sink);
        self.state.mask.set(dwmask);
        Ok(())
    }

    fn UnadviseSink(&self, _punk: Ref<IUnknown>) -> Result<()> {
        *self.state.sink.borrow_mut() = None;
        self.state.mask.set(0);
        Ok(())
    }

    fn RequestLock(&self, dwlockflags: u32) -> Result<HRESULT> {
        let sink = self.state.sink.borrow().clone();
        if let Some(sink) = sink {
            unsafe {
                sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(dwlockflags))?;
            }
        }
        Ok(S_OK)
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
        _cch: u32,
        pacpresultstart: *mut i32,
        pacpresultend: *mut i32,
    ) -> Result<()> {
        let length = self.text_len();
        let start = acpteststart.clamp(0, length);
        let end = acptestend.clamp(start, length);
        unsafe {
            *pacpresultstart = start;
            *pacpresultend = end;
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
        if ulindex != 0 && ulindex != TS_DEFAULT_SELECTION {
            return Err(TS_E_NOSELECTION.into());
        }
        if pcfetched.is_null() {
            return Ok(());
        }
        if ulcount == 0 {
            unsafe {
                *pcfetched = 0;
            }
            return Ok(());
        }
        let (start, end) = self.selection();
        unsafe {
            *pselection = TS_SELECTION_ACP {
                acpStart: start,
                acpEnd: end,
                style: TS_SELECTIONSTYLE {
                    ase: TS_AE_NONE,
                    fInterimChar: BOOL(0),
                },
            };
            *pcfetched = 1;
        }
        Ok(())
    }

    fn SetSelection(&self, ulcount: u32, pselection: *const TS_SELECTION_ACP) -> Result<()> {
        if ulcount == 0 || pselection.is_null() {
            return Ok(());
        }
        let selection = unsafe { *pselection };
        self.set_selection(selection.acpStart, selection.acpEnd);
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
        let text = self.text();
        let length = text.len() as i32;
        let start = acpstart.clamp(0, length);
        let end = acpend.clamp(start, length);
        let range = (end - start) as u32;
        let copied = if pchplain.is_null() {
            0
        } else {
            range.min(cchplainreq)
        };
        if !pchplain.is_null() && copied > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    text.as_ptr().add(start as usize),
                    pchplain.0,
                    copied as usize,
                );
            }
        }
        if !pcchplainret.is_null() {
            unsafe {
                *pcchplainret = if pchplain.is_null() { range } else { copied };
            }
        }
        if !prgruninfo.is_null() && cruninforeq > 0 {
            unsafe {
                *prgruninfo = TS_RUNINFO {
                    uCount: copied,
                    r#type: TS_RT_PLAIN,
                };
            }
            if !pcruninforet.is_null() {
                unsafe {
                    *pcruninforet = 1;
                }
            }
        } else if !pcruninforet.is_null() {
            unsafe {
                *pcruninforet = 0;
            }
        }
        if !pacpnext.is_null() {
            unsafe {
                *pacpnext = end;
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
        let text = if cch == 0 || pchtext.0.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize).to_vec() }
        };
        let change = self.replace_range(acpstart, acpend, &text);
        self.notify_text(change);
        Ok(change)
    }

    fn GetFormattedText(
        &self,
        _acpstart: i32,
        _acpend: i32,
    ) -> Result<windows::Win32::System::Com::IDataObject> {
        Err(E_NOTIMPL.into())
    }

    fn GetEmbedded(
        &self,
        _acppos: i32,
        _rguidservice: *const GUID,
        _riid: *const GUID,
    ) -> Result<IUnknown> {
        Err(E_NOTIMPL.into())
    }

    fn QueryInsertEmbedded(
        &self,
        _pguidservice: *const GUID,
        _pformatetc: *const windows::Win32::System::Com::FORMATETC,
    ) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn InsertEmbedded(
        &self,
        _dwflags: u32,
        _acpstart: i32,
        _acpend: i32,
        _pdataobject: Ref<windows::Win32::System::Com::IDataObject>,
    ) -> Result<TS_TEXTCHANGE> {
        Err(E_NOTIMPL.into())
    }

    fn InsertTextAtSelection(
        &self,
        dwflags: u32,
        pchtext: &PCWSTR,
        cch: u32,
        pacpstart: *mut i32,
        pacpend: *mut i32,
        pchange: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        let (selection_start, selection_end) = self.selection();
        if dwflags & TS_IAS_QUERYONLY != 0 {
            if dwflags & TS_IAS_NOQUERY == 0 {
                if !pacpstart.is_null() {
                    unsafe {
                        *pacpstart = selection_start;
                    }
                }
                if !pacpend.is_null() {
                    unsafe {
                        *pacpend = selection_end;
                    }
                }
            }
            return Ok(());
        }
        let text = if cch == 0 || pchtext.0.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize).to_vec() }
        };
        let change = self.replace_range(selection_start, selection_end, &text);
        if !pacpstart.is_null() {
            unsafe {
                *pacpstart = change.acpStart;
            }
        }
        if !pacpend.is_null() {
            unsafe {
                *pacpend = change.acpNewEnd;
            }
        }
        if !pchange.is_null() {
            unsafe {
                *pchange = change;
            }
        }
        self.notify_text(change);
        Ok(())
    }

    fn InsertEmbeddedAtSelection(
        &self,
        _dwflags: u32,
        _pdataobject: Ref<windows::Win32::System::Com::IDataObject>,
        _pacpstart: *mut i32,
        _pacpend: *mut i32,
        _pchange: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
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
        acphalt: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const GUID,
        _dwflags: u32,
        pacpnext: *mut i32,
        pffound: *mut BOOL,
        plfoundoffset: *mut i32,
    ) -> Result<()> {
        unsafe {
            if !pacpnext.is_null() {
                *pacpnext = acphalt;
            }
            if !pffound.is_null() {
                *pffound = BOOL(0);
            }
            if !plfoundoffset.is_null() {
                *plfoundoffset = 0;
            }
        }
        Ok(())
    }

    fn RetrieveRequestedAttrs(
        &self,
        _ulcount: u32,
        _paattrvals: *mut windows::Win32::UI::TextServices::TS_ATTRVAL,
        pcfetched: *mut u32,
    ) -> Result<()> {
        if !pcfetched.is_null() {
            unsafe {
                *pcfetched = 0;
            }
        }
        Ok(())
    }

    fn GetEndACP(&self) -> Result<i32> {
        Ok(self.text_len())
    }

    fn GetActiveView(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetACPFromPoint(&self, _vcview: u32, ptscreen: *const POINT, _dwflags: u32) -> Result<i32> {
        if ptscreen.is_null() {
            return Ok(-1);
        }
        let mut point = unsafe { *ptscreen };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut point);
        }
        let packed = ((point.y as u32 & 0xFFFF) << 16) | (point.x as u32 & 0xFFFF);
        let index = self
            .send(EM_CHARFROMPOS, WPARAM(0), LPARAM(packed as isize))
            .0;
        Ok(if index < 0 { -1 } else { index as i32 })
    }

    fn GetTextExt(
        &self,
        _vcview: u32,
        acpstart: i32,
        acpend: i32,
        prc: *mut RECT,
        pfclipped: *mut BOOL,
    ) -> Result<()> {
        if prc.is_null() {
            return Ok(());
        }
        unsafe {
            *prc = self.caret_rect(acpstart, acpend);
            if !pfclipped.is_null() {
                *pfclipped = BOOL(0);
            }
        }
        Ok(())
    }

    fn GetScreenExt(&self, _vcview: u32) -> Result<RECT> {
        let mut rect = RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(self.hwnd, &mut rect);
            let mut top_left = POINT {
                x: rect.left,
                y: rect.top,
            };
            let mut bottom_right = POINT {
                x: rect.right,
                y: rect.bottom,
            };
            let _ = ClientToScreen(self.hwnd, &mut top_left);
            let _ = ClientToScreen(self.hwnd, &mut bottom_right);
            Ok(RECT {
                left: top_left.x,
                top: top_left.y,
                right: bottom_right.x,
                bottom: bottom_right.y,
            })
        }
    }

    fn GetWnd(&self, _vcview: u32) -> Result<HWND> {
        Ok(self.hwnd)
    }
}
