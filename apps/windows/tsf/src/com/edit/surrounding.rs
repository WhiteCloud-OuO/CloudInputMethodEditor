//! 组句起始时读应用光标前的文字，给本地整句模型当前文，
//! 顺手按输入范围判这个输入框私密不私密（[`private_input`]）。在起组句的那次读写会话里做（此时选区还是原来的插入点，
//! 拼音还没插进去），不另开会话。

use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::VT_UNKNOWN;
use windows::Win32::UI::TextServices::{
    GUID_PROP_INPUTSCOPE, IS_ALPHANUMERIC_PIN, IS_NUMERIC_PASSWORD, IS_NUMERIC_PIN, IS_PASSWORD,
    IS_PRIVATE, ITfContext, ITfEditSession, ITfEditSession_Impl, ITfInputScope, ITfRange,
    InputScope, TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_READ, TF_SELECTION,
};
use windows::core::{Error, Interface, implement};

use cloudime_platform::protocol::DocumentText;

use crate::com::composition::{report_privacy, report_surrounding};
use crate::com::service::SharedClient;

/// 往前读多少字。
const LOOKBACK: i32 = 64;

/// 一次性的**只读**会话：按键前现读一份光标前文（与私密判定）发给 Server。
///
/// 和读写会话一样必须**异步**：沉浸式应用上同步编辑会话会把宿主整个搞崩（见 `com/edit/update.rs`
/// 文件头）。于是它读到的是**上一次按键之后**的文档 —— 对「按键触发的脚本」来说正好：脚本在这一次
/// 按键里要的就是上一键敲完的那个状态。
#[implement(ITfEditSession)]
pub(crate) struct SurroundingSession {
    context: ITfContext,
    engine: SharedClient,
}

impl ITfEditSession_Impl for SurroundingSession_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let input = input_context(&self.context, ec, false);
            report_privacy(&self.engine, input.private);
            // 读不到也送一条**空的**：让 Server 把上一段的前文清掉。记事本 / OpenCode 这类宿主不给
            // 光标前文，`cloudime.context` 该是空串，而不是上一段组句的残留（残留会让脚本误判、胡乱上屏）。
            report_surrounding(&self.engine, input.before.unwrap_or_default(), None);
        }));
        if result.is_err() {
            crate::com::log::log("按键前读光标前文 panic（已兜住）");
            return Err(Error::from(E_FAIL));
        }
        Ok(())
    }
}

/// 请求一个异步只读会话，按键前现读光标前文（只有 `InputSettings.script_wants_text` 时才调）。
pub(crate) fn request_surrounding_now(context: &ITfContext, client_id: u32, engine: SharedClient) {
    let session = SurroundingSession {
        context: context.clone(),
        engine,
    };
    if let Err(error) = super::update::request(context, client_id, session.into(), TF_ES_READ) {
        crate::com::log::log(&format!("读光标前文的编辑会话没被受理: {error}"));
    }
}

/// 起组句时对输入框的判断：私密不私密、光标前的文字，以及（Server 请过的话）一份整篇快照。
pub(crate) struct InputContext {
    /// 输入范围声明了私密 / 密码 / PIN（[`SECRET_SCOPES`]）：不读前文，Server 侧不学不记不发云端。
    pub(crate) private: bool,

    /// 当前选区起点之前最多 [`LOOKBACK`] 个 UTF-16 单元的文本。私密、没有选区、读不到时为 `None`。
    pub(crate) before: Option<String>,

    /// 整篇快照（`cloudime.text.*` 用）：只在 Server 请过时读（[`document_text`]）。
    pub(crate) document: Option<DocumentText>,
}

/// 起组句时读一次：先判私密，不私密再读前文（Server 要的话连整篇一起读）。
pub(crate) fn input_context(context: &ITfContext, ec: u32, document: bool) -> InputContext {
    let Some(range) = selection_start(context, ec) else {
        return InputContext {
            private: false,
            before: None,
            document: None,
        };
    };
    if private_input(context, ec, &range) {
        crate::com::log::log("私密输入框，不读光标前文");
        return InputContext {
            private: true,
            before: None,
            document: None,
        };
    }
    InputContext {
        private: false,
        before: text_before_caret(ec, &range),
        document: document.then(|| document_text(ec, &range)).flatten(),
    }
}

/// 复制一份**独立**的 range。COM 接口的 `.clone()` 只是 `AddRef`、仍指向同一个对象，
/// 挪动副本会连带原件 —— 读光标前后两半时，第二次读会带上第一次挪过的范围（整篇翻倍）。
/// 用 COM 自己的 `Clone`（`ITfRange::Clone`，返回一个真正的新 range）才对。
fn clone_range(range: &ITfRange) -> Option<ITfRange> {
    unsafe { range.Clone() }.ok()
}

/// 整篇快照的硬上限（UTF-16 单元）：读一篇要先分配缓冲，总得有个天花板。
/// 200 000 个单元 ≈ 10 万汉字 / 400 KB —— 再多也够脚本用了，再高只会拖慢每一次读取。
const DOCUMENT_LIMIT: i32 = 200_000;

/// 读一份「光标附近」的文档快照：光标前后各一半、最多 `2 × [`DOCUMENT_LIMIT`]` 个 UTF-16 单元。
/// `DocumentText.caret` 是光标在返回文本里的字符下标（`cloudime.text.before` 按它切）。
/// 私密框、读不到、空文档返回 `None`。
fn document_text(ec: u32, caret: &ITfRange) -> Option<DocumentText> {
    // 分两半读（光标前 / 光标后）：拼起来就是「以光标为中心」，光标下标天然是前一半的字符数。
    // 某一侧为空（光标就在文档两头）不算失败，两侧都为空才是没有文本。
    let before = side_text(ec, caret, true).unwrap_or_default();
    let after = side_text(ec, caret, false).unwrap_or_default();
    let caret_index = before.chars().count();
    let mut text = before;
    text.push_str(&after);
    (!text.is_empty()).then_some(DocumentText {
        text,
        caret: caret_index,
    })
}

/// 从光标往一侧读最多 [`DOCUMENT_LIMIT`] / 2 个 UTF-16 单元：`before = true` 读光标之前，否则读之后。
fn side_text(ec: u32, caret: &ITfRange, before: bool) -> Option<String> {
    let range = clone_range(caret)?;
    let half = DOCUMENT_LIMIT / 2;
    let mut shifted = 0i32;
    let result = if before {
        unsafe { range.ShiftStart(ec, -half, &mut shifted, std::ptr::null()) }
    } else {
        unsafe { range.ShiftEnd(ec, half, &mut shifted, std::ptr::null()) }
    };
    result.ok()?;
    let mut buf = vec![0u16; half as usize];
    let mut fetched = 0u32;
    unsafe { range.GetText(ec, 0, &mut buf, &mut fetched) }.ok()?;
    let text = String::from_utf16_lossy(&buf[..fetched as usize]);
    (!text.is_empty()).then_some(text)
}

/// 光标之前最多 [`LOOKBACK`] 个 UTF-16 单元的文本；读不到 / 为空是 `None`。
fn text_before_caret(ec: u32, caret: &ITfRange) -> Option<String> {
    let range = clone_range(caret)?;
    let mut shifted = 0i32;
    unsafe { range.ShiftStart(ec, -LOOKBACK, &mut shifted, std::ptr::null()) }.ok()?;
    if shifted == 0 {
        return None;
    }
    let mut buf = [0u16; LOOKBACK as usize];
    let mut fetched = 0u32;
    unsafe { range.GetText(ec, 0, &mut buf, &mut fetched) }.ok()?;
    let text = String::from_utf16_lossy(&buf[..fetched as usize]);
    (!text.is_empty()).then_some(text)
}

/// 选区折成起点（插入点）。
fn selection_start(context: &ITfContext, ec: u32) -> Option<ITfRange> {
    let mut selection = [TF_SELECTION::default()];
    let mut fetched = 0u32;
    unsafe {
        context
            .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)
            .ok()?;
    }
    if fetched == 0 {
        return None;
    }
    // GetSelection 移交 range 的所有权（ManuallyDrop），取出后由这里释放。
    let range = unsafe { ManuallyDrop::take(&mut selection[0].range) }?;
    unsafe { range.Collapse(ec, TF_ANCHOR_START) }.ok()?;
    Some(range)
}

/// 算作私密的输入范围：密码 / PIN 之外还有 `IS_PRIVATE`——Chromium（Edge / Chrome）给密码框与无痕窗口里所有输入框报的
/// 都是它（含义是「别学」），不是 `IS_PASSWORD`。真正的密码框另有 `KEYBOARD_DISABLED` compartment 让整键放行
/// （见 [`crate::com::context`]），到不了这里；这里兜的是没禁键盘但声明了私密的输入框：照常组句，但不读前文、不学、不记、不发云端。
const SECRET_SCOPES: [InputScope; 5] = [
    IS_PASSWORD,
    IS_PRIVATE,
    IS_NUMERIC_PASSWORD,
    IS_NUMERIC_PIN,
    IS_ALPHANUMERIC_PIN,
];

/// 输入框声明了私密类输入范围（`GUID_PROP_INPUTSCOPE` 里含 [`SECRET_SCOPES`] 之一）。拿不到属性按不私密。
fn private_input(context: &ITfContext, ec: u32, range: &ITfRange) -> bool {
    match input_scopes(context, ec, range) {
        Ok(scopes) => {
            crate::com::log::log(&format!("输入范围: {scopes:?}"));
            scopes.iter().any(|scope| SECRET_SCOPES.contains(scope))
        }
        // 不支持输入范围属性的应用（如记事本）GetValue 会失败，按不私密，不记日志
        Err(_) => false,
    }
}

/// 应用给 `range` 声明的全部输入范围；哪一步拿不到就说哪一步。
fn input_scopes(
    context: &ITfContext,
    ec: u32,
    range: &ITfRange,
) -> Result<Vec<InputScope>, String> {
    let property = unsafe { context.GetAppProperty(&GUID_PROP_INPUTSCOPE) }
        .map_err(|error| format!("GetAppProperty {error}"))?;
    let value =
        unsafe { property.GetValue(ec, range) }.map_err(|error| format!("GetValue {error}"))?;
    // SAFETY: 只在 vt 是 VT_UNKNOWN 时读 punkVal 那个联合体成员。
    let scope: ITfInputScope = unsafe {
        let inner = &value.Anonymous.Anonymous;
        if inner.vt != VT_UNKNOWN {
            return Err(format!("vt={}", inner.vt.0));
        }
        inner
            .Anonymous
            .punkVal
            .as_ref()
            .ok_or_else(|| "punkVal 空".to_owned())?
            .cast()
            .map_err(|error| format!("cast ITfInputScope {error}"))?
    };
    let mut scopes: *mut InputScope = std::ptr::null_mut();
    let mut count = 0u32;
    unsafe { scope.GetInputScopes(&mut scopes, &mut count) }
        .map_err(|error| format!("GetInputScopes {error}"))?;
    if scopes.is_null() {
        return Err("GetInputScopes 返回空数组".to_owned());
    }
    // SAFETY: GetInputScopes 返回 count 个元素的 CoTaskMem 数组，由调用方释放。
    unsafe {
        let list = std::slice::from_raw_parts(scopes, count as usize).to_vec();
        CoTaskMemFree(Some(scopes.cast()));
        Ok(list)
    }
}
