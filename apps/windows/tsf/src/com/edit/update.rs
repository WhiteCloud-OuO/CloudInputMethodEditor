//! 编辑会话：TSF 不允许直接改文档，要经 `RequestEditSession` 申请，在回调里拿着 edit cookie 写。
//!
//! 必须用异步会话（不带 `TF_ES_SYNC`）：沉浸式应用（Win11 新记事本等）的文本存区隔着进程边界，
//! 同步读写会让 `textinputframework.dll` 访问违例把宿主整个搞崩。回调可能在 `OnKeyDown` 返回之后才跑。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::time::Instant;

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    TF_ES_READWRITE, TF_ES_SYNC,
};
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
use windows::core::{Error, Result, implement};

use crate::com::composition::{Shared, Update, apply};
use crate::com::log::log;
use crate::com::service::SharedClient;

/// 一次性的读写会话：把本次按键的组句更新写进 `context`。
#[implement(ITfEditSession)]
pub(crate) struct UpdateSession {
    /// 目标文档上下文。
    context: ITfContext,

    /// 引擎层：量到光标矩形后报给 Server 摆候选窗口。
    engine: SharedClient,

    /// 组句状态。
    shared: Rc<Shared>,

    /// 本次要写进文档的东西。
    update: Update,
}

impl ITfEditSession_Impl for UpdateSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let started = Instant::now();
        // 从框架的 C++ 调进来：panic 不能越过 FFI。
        let result = catch_unwind(AssertUnwindSafe(|| {
            apply(&self.shared, &self.engine, &self.context, ec, &self.update)
        }));
        crate::com::log::timing(format_args!(
            "时序 editsession_us={} commit={} preedit_len={}",
            started.elapsed().as_micros(),
            self.update.commit.is_some(),
            self.update.preedit.chars().count()
        ));
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                log(&format!("组句更新失败: {error}"));
                Err(error)
            }
            Err(_) => {
                log("组句更新回调 panic（已兜住）");
                Err(Error::from(E_FAIL))
            }
        }
    }
}

/// 请求一个异步读写会话。`Ok` 只说明已受理，写入结果在回调里记日志。
pub(crate) fn request_update(
    context: &ITfContext,
    client_id: u32,
    engine: SharedClient,
    shared: Rc<Shared>,
    mut update: Update,
) -> Result<()> {
    // 成对补全在 Chromium / Electron 宿主（QQ、VS Code、飞书等）里必须**在按键上下文同步提交**：
    // 异步会话只更新 TSF 选区，宿主的插入点不跟着走，紧接着注入的方向键便从一个失效的插入点算起。
    // 非 Chromium 仍走异步（Win11 记事本的跨进程文本存区同步读写会让 textinputhandler.dll 崩溃）。
    let sync = update.commit.is_some()
        && update.caret_shift != 0
        && update.preedit.is_empty()
        && update.delete_before == 0
        && update.delete_after == 0
        && (is_chromium_context(context) || host_is_chromium_like());
    if update.commit.is_some() && update.caret_shift != 0 {
        log(&format!(
            "成对补全提交: sync={sync} chromium={} class={:?} host={:?}",
            is_chromium_context(context),
            context_view_class(context),
            crate::com::host_app_name()
        ));
    }
    if sync {
        update.sync_pair = true;
        let session = UpdateSession {
            context: context.clone(),
            engine: engine.clone(),
            shared: shared.clone(),
            update: update.clone(),
        };
        match request(
            context,
            client_id,
            session.into(),
            TF_ES_SYNC | TF_ES_READWRITE,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                log(&format!("成对补全同步会话失败，退回异步: {error}"));
            }
        }
    }
    let session = UpdateSession {
        context: context.clone(),
        engine,
        shared,
        update,
    };
    request(context, client_id, session.into(), TF_ES_READWRITE)
}

/// 这个 TSF 上下文是不是 Chromium / Electron 的视图。
///
/// 只按窗口类判断（`Chrome_WidgetWin_*` / `Chrome_RenderWidgetHostHWND`），比按 exe 名单稳：
/// QQNT、VS Code、飞书这些外壳的进程名各不相同，但输入框的 TSF 视图都是 Chromium 的。
pub(crate) fn is_chromium_context(context: &ITfContext) -> bool {
    context_view_class(context).is_some_and(|class| is_chromium_class(&class))
}

fn is_chromium_class(class: &str) -> bool {
    class.starts_with("Chrome_WidgetWin_") || class == "Chrome_RenderWidgetHostHWND"
}

/// 已知 Chromium / Electron 外壳的 exe 名：视图类拿不到时兜底（QQNT 的视图可能报 `TXGuiFoundation`）。
const CHROMIUM_HOSTS: &[&str] = &["QQ.exe", "Code.exe", "Feishu.exe", "Weixin.exe"];

fn host_is_chromium_like() -> bool {
    crate::com::host_app_name().is_some_and(|name| host_matches_chromium(&name))
}

fn host_matches_chromium(name: &str) -> bool {
    CHROMIUM_HOSTS
        .iter()
        .any(|host| name.eq_ignore_ascii_case(host))
}

/// 当前上下文视图的窗口类名（诊断 / Chromium 判定用）。
fn context_view_class(context: &ITfContext) -> Option<String> {
    let view = unsafe { context.GetActiveView() }.ok()?;
    let hwnd = unsafe { view.GetWnd() }.ok()?;
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    (len > 0).then(|| String::from_utf16_lossy(&class[..len as usize]))
}

/// 异步提交一个编辑会话；会话对象由框架持有到回调跑完。
pub(super) fn request(
    context: &ITfContext,
    client_id: u32,
    session: ITfEditSession,
    flags: TF_CONTEXT_EDIT_CONTEXT_FLAGS,
) -> Result<()> {
    unsafe { context.RequestEditSession(client_id, &session, flags)? }.ok()
}

#[cfg(test)]
mod tests {
    use super::{host_matches_chromium, is_chromium_class};

    #[test]
    fn chromium_view_classes_are_recognized() {
        assert!(is_chromium_class("Chrome_WidgetWin_1"));
        assert!(is_chromium_class("Chrome_RenderWidgetHostHWND"));
        assert!(!is_chromium_class("Edit"));
        assert!(!is_chromium_class("TXGuiFoundation"));
    }

    #[test]
    fn chromium_hosts_fall_back_case_insensitively() {
        assert!(host_matches_chromium("QQ.exe"));
        assert!(host_matches_chromium("qq.EXE"));
        assert!(host_matches_chromium("Code.exe"));
        assert!(!host_matches_chromium("notepad.exe"));
    }
}
