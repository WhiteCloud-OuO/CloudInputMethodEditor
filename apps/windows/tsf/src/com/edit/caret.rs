//! 脚本要光标位置（`cloudime.caret()`）时的只读编辑会话：量一次插入点，报给 Server。
//!
//! 与读光标前文的会话一样必须**异步**（沉浸式应用上同步编辑会话会把宿主整个搞崩，见
//! [`super::update`] 文件头）。于是脚本拿到的是**这次按键处理完**的位置，正好是它要摆窗口的那个点。

use std::panic::{AssertUnwindSafe, catch_unwind};

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, TF_ES_READ,
};
use windows::core::{Error, Result, implement};

use crate::com::edit::anchor::insertion_rect;
use crate::com::log::log;
use crate::com::service::SharedClient;

/// 一次性的只读会话：量插入点，经引擎报给 Server。
#[implement(ITfEditSession)]
pub(crate) struct CaretSession {
    context: ITfContext,
    engine: SharedClient,
}

impl ITfEditSession_Impl for CaretSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        // 从框架的 C++ 调进来：panic 不能越过 FFI。
        let result = catch_unwind(AssertUnwindSafe(|| {
            let rect = insertion_rect(&self.context, ec);
            if let Ok(mut guard) = self.engine.try_borrow_mut()
                && let Some(client) = guard.as_mut()
                && let Err(error) = client.caret(rect)
            {
                log(&format!("上报光标位置失败: {error}"));
            }
        }));
        if result.is_err() {
            log("读光标位置的编辑会话 panic（已兜住）");
            return Err(Error::from(E_FAIL));
        }
        Ok(())
    }
}

/// 请求一个异步只读会话，量一次插入点（只有 Server 的 `want_caret` 时才调）。
pub(crate) fn request_caret_now(context: &ITfContext, client_id: u32, engine: SharedClient) {
    let session = CaretSession {
        context: context.clone(),
        engine,
    };
    if let Err(error) = super::update::request(context, client_id, session.into(), TF_ES_READ) {
        log(&format!("读光标位置的编辑会话没被受理: {error}"));
    }
}
