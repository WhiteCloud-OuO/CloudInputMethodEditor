//! 组句 preedit：把 Server 回的拼音行经 TSF 组句（[`ITfComposition`]）显示在文档光标处。
//! 只放最朴素的一行拼音；富样式的拼音行在候选窗口里另画。所有写操作都在异步读写编辑会话里做（理由见 `edit_session`）。

mod shared;
mod sink;

use std::mem::ManuallyDrop;
use std::rc::Rc;

use windows::Win32::UI::TextServices::{
    INSERT_TEXT_AT_SELECTION_FLAGS, ITfComposition, ITfCompositionSink, ITfContext,
    ITfContextComposition, ITfInsertAtSelection, ITfRange, ITfRangeACP, TF_AE_END, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows::core::{Interface, Result};

use cloudime_platform::protocol::{Frame, PreeditKind};

pub(crate) use self::shared::Shared;
use self::sink::CompositionSink;
use super::edit::{InputContext, anchor_rect, caret_rect, input_context};
use super::service::SharedClient;

/// 内联要显示的拼音行（跳过被纠错划掉的原字母）；空串表示没有组句内容。
pub(crate) fn preedit_string(frame: &Frame) -> String {
    frame
        .preedit
        .iter()
        .filter(|segment| segment.kind != PreeditKind::Corrected)
        .map(|segment| segment.text.as_str())
        .collect()
}

/// 一次组句更新的内容（上屏文本 + 拼音行 + 光标 / 删字）。
#[derive(Debug, Default, Clone)]
pub(crate) struct Update {
    /// 本次要落定上屏的文本。
    pub commit: Option<String>,

    /// 本次组句拼音行；空串表示收起组句。
    pub preedit: String,

    /// 上屏后把光标再挪几个字符（成对补全：负数是停到括号中间，正数是跳过已经补好的右半边）。
    pub caret_shift: i16,

    /// 上屏前先删掉光标前几个字符（符号映射的两键规则把上一个键的输出换掉）。
    pub delete_before: u16,
}

impl Update {
    /// 没有上屏文本、没有拼音行、也不用动光标 / 删字：这一拍什么都不用做。
    pub(crate) fn is_empty(&self) -> bool {
        self.commit.is_none()
            && self.preedit.is_empty()
            && self.caret_shift == 0
            && self.delete_before == 0
    }
}

/// 在编辑会话回调（持写锁 `ec`）里调：先落定 `commit`，再按 `preedit` 起 / 改 / 收组句，最后把光标位置报给 Server。
/// 新起一段组句时顺手判输入框私密不私密（变了就告诉 Server）、把光标前的文字送给 Server（本地整句模型的前文）。
pub(crate) fn apply(
    shared: &Rc<Shared>,
    engine: &SharedClient,
    context: &ITfContext,
    ec: u32,
    update: &Update,
) -> Result<()> {
    // 先读输入框状态、再上屏。上屏会把组句收掉（`has_composition()` 变 false），而有的宿主（Windows 11
    // 记事本实测）给 IME 的是**按选区 / 组句圈起来的局部上下文**——整篇读不到、上屏那一刻只剩刚上屏
    // 那一段、起组句那一拍甚至是空的 `0..0`（详见 `docs/notes/crate-notes.md`「读输入框文本」的坑）。
    // 读放在上屏之前，至少和「起组句时读」的语义一致。
    // 一段组句里只问一次。行内模式看组句刚起；`preedit = window` 模式应用里根本没有组句，
    // 得另用一个标记，否则每敲一键都要重读一遍光标前文、重报一次私密状态。
    let report_input = !shared.has_composition() && !shared.context_reported();
    if report_input {
        shared.set_context_reported(true);
    }
    let input = report_input.then(|| input_context(context, ec, super::service::want_document()));

    match update.commit.as_deref() {
        Some(text) => commit_text(
            shared,
            context,
            ec,
            text,
            update.caret_shift,
            update.delete_before,
        )?,
        // 没有要上屏的文本：只是把光标往前跳过补好的右半边
        None if update.caret_shift != 0 => shift_caret(context, ec, update.caret_shift)?,
        None => {}
    }
    let preedit = &update.preedit;
    if preedit.is_empty() {
        end_composition(shared, ec)?;
    } else {
        update_preedit(shared, context, ec, preedit)?;
    }
    if let Some(InputContext {
        private,
        before,
        document,
    }) = input
    {
        report_privacy(engine, private);
        if before.is_some() || document.is_some() {
            report_surrounding(engine, before.unwrap_or_default(), document);
        }
    }
    report_caret(shared, engine, context, ec);
    Ok(())
}

/// 告诉 Server 输入框私密与否（客户端只在变了时真发）；引擎正被别处借着（罕见）就算了，下段组句再报。
fn report_privacy(engine: &SharedClient, private: bool) {
    if let Ok(mut guard) = engine.try_borrow_mut()
        && let Some(client) = guard.as_mut()
        && let Err(error) = client.set_private(private)
    {
        super::log::log(&format!("报私密状态失败: {error}"));
    }
}

/// 把光标前文（和 Server 请过的整篇快照）送给 Server；引擎正被别处借着（罕见）就算了，
/// Server 退回会话历史 / 下一段组句再读。
fn report_surrounding(
    engine: &SharedClient,
    before: String,
    document: Option<cloudime_platform::protocol::DocumentText>,
) {
    let chars = before.chars().count();
    let document_chars = document.as_ref().map_or(0, |doc| doc.text.chars().count());
    if let Ok(mut guard) = engine.try_borrow_mut()
        && let Some(client) = guard.as_mut()
    {
        match client.surrounding(before, document) {
            Ok(()) => super::log::log(&format!("送光标前文 {chars} 字、整篇 {document_chars} 字")),
            Err(error) => super::log::log(&format!("送光标前文失败: {error}")),
        }
    }
}

/// 组句进行中才报位置；组句已收 Server 会按空帧 / `Commit` 自行收窗口。
fn report_caret(shared: &Shared, engine: &SharedClient, context: &ITfContext, ec: u32) {
    let rect = match shared.composition() {
        Some(composition) => {
            let Ok(range) = (unsafe { composition.GetRange() }) else {
                return;
            };
            anchor_rect(context, ec, &range)
        }
        // 「只在候选窗口」模式应用里不放行内拼音：没有组句范围可量，量插入点。
        None if shared.composing() => caret_rect(context, ec),
        None => return,
    };
    // 引擎正被别处借着（罕见）就跳过这拍，Server 保持上次位置。
    if let Ok(mut guard) = engine.try_borrow_mut()
        && let Some(client) = guard.as_mut()
        && let Err(error) = client.position_candidates(rect)
    {
        super::log::log(&format!("上报候选窗口位置失败: {error}"));
    }
}

/// 有组句就把组句范围替换成 `text` 再结束组句，否则在选区插入；`caret_shift` / `delete_before` 见 [`apply`]。
fn commit_text(
    shared: &Shared,
    context: &ITfContext,
    ec: u32,
    text: &str,
    caret_shift: i16,
    delete_before: u16,
) -> Result<()> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    match shared.composition() {
        Some(composition) => {
            let range = unsafe { composition.GetRange()? };
            unsafe { range.SetText(ec, 0, &utf16)? };
            settle_caret(context, ec, &range, utf16.len(), caret_shift);
            unsafe { composition.EndComposition(ec)? };
            shared.set_composition(None);
        }
        None => {
            if delete_before > 0 {
                delete_before_caret(context, ec, delete_before)?;
            }
            let insert: ITfInsertAtSelection = context.cast()?;
            // 标志不能用 NOQUERY：它不回传 range，windows-rs 会把 NULL 当失败。
            let range = unsafe {
                insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &utf16)?
            };
            // 不移光标的话下一次插入又落在原处，字会从右往左堆。
            settle_caret(context, ec, &range, utf16.len(), caret_shift);
        }
    }
    Ok(())
}

/// 落光标。成对补全（`caret_shift` 不为 0）优先**注入方向键**：TSF 的 `SetSelection` 在部分宿主里会被应用
/// 自己改回来（真机上成对补全的光标一直停在末尾，而同一个应用对方向键的响应是好的）；注入不了
///（AppContainer / UIPI 拦下来）再退回 TSF 位移。普通上屏（`caret_shift == 0`）只走 TSF。
fn settle_caret(
    context: &ITfContext,
    ec: u32,
    range: &ITfRange,
    text_chars: usize,
    caret_shift: i16,
) {
    if caret_shift != 0 && nudge_caret_by_key(i32::from(caret_shift)) {
        return;
    }
    place_caret(context, ec, range, text_chars, caret_shift);
}

/// 挪光标；挪不动只记日志——文本已经写进去了，不该因为落点算不准把这一键整体当失败。
fn place_caret(
    context: &ITfContext,
    ec: u32,
    range: &ITfRange,
    text_chars: usize,
    caret_shift: i16,
) {
    if let Err(error) = move_selection(context, ec, range, text_chars, caret_shift) {
        super::log::log(&format!("挪光标失败: {error}"));
    }
}

/// 注入 `steps` 个方向键（负左、正右），让**应用自己**挪光标。返回是否注入成功。
///
/// 在编辑会话里调：注入的输入先排进应用的消息队列，会话返回后应用才处理，那时文本已经落好了。
/// AppContainer（商店应用）里 `SendInput` 会被拒，返回不够数，调用方退回 TSF 位移并记日志。
fn nudge_caret_by_key(steps: i32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VK_LEFT, VK_RIGHT,
    };
    if steps == 0 {
        return true;
    }
    let key = if steps < 0 { VK_LEFT } else { VK_RIGHT };
    let input = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let mut inputs = Vec::new();
    for _ in 0..steps.unsigned_abs() {
        inputs.push(input(KEYBD_EVENT_FLAGS(0)));
        inputs.push(input(KEYEVENTF_KEYUP));
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    let ok = sent == inputs.len() as u32;
    if !ok {
        super::log::log(&format!(
            "注入方向键挪光标失败（只送进 {sent} 个输入），退回 TSF 位移"
        ));
    }
    ok
}

/// 删掉光标前 `count` 个字符（符号映射的两键规则把上一个键的输出换掉，`~=` → `≈`）。
///
/// 真机上发现：TSF 只在「收成哪一端就挪哪一端」时才真正拉开范围。原来的写法收成**末尾**再
/// `ShiftStart`，范围不动，于是两键规则只上屏后一个符号、前一个留在原地（`！≠`）。
/// 改成收成**起点**再 `ShiftStart`（与 `surrounding.rs` 往左读前文同款），真拉开了才 `SetText`；
/// 拉不开再按 ACP 位置直接圈 `[光标-count, 光标]`；都不行只记日志。
fn delete_before_caret(context: &ITfContext, ec: u32, count: u16) -> Result<()> {
    let mut selections = [TF_SELECTION::default()];
    let mut fetched = 0;
    unsafe { context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selections, &mut fetched)? };
    let count = i32::from(count);
    for selection in selections {
        let Some(selected) = ManuallyDrop::into_inner(selection.range) else {
            continue;
        };
        // 用 `GetSelection` 给的**原始**范围去挪：`surrounding.rs` 往左读前文一直这么用、一直好用；
        // 之前先 `Clone` 再挪，真机上 `ShiftStart` 报告「挪了 0」，怎么都不动。
        let by_acp = unsafe { selected.Clone()? };
        if pull_start_back(&selected, ec, count) {
            return Ok(());
        }
        if delete_by_acp(&by_acp, ec, count) {
            super::log::log(&format!("删光标前 {count} 个字符：按 ACP 位置圈"));
            return Ok(());
        }
        super::log::log(&format!("删光标前 {count} 个字符失败：范围拉不开"));
        break;
    }
    Ok(())
}

/// 收成**起点**再把起点往回挪 `count` 格（`surrounding.rs` 往左读前文同款），真挪动了就删掉整段。
///
/// 判据用 `ShiftStart` 回报的 `pchSkipped`，和读前文一致；**不要**去问 `IsEmpty`——那个在部分宿主上会
/// 直接失败，把本来挪动了的范围误判成没挪动（真机踩过）。
fn pull_start_back(range: &ITfRange, ec: u32, count: i32) -> bool {
    if unsafe { range.Collapse(ec, TF_ANCHOR_START) }.is_err() {
        return false;
    }
    let mut shifted = 0;
    if unsafe { range.ShiftStart(ec, -count, &mut shifted, std::ptr::null()) }.is_err() {
        return false;
    }
    let deleted = shifted > 0 && unsafe { range.SetText(ec, 0, &[]) }.is_ok();
    super::log::log(&format!(
        "删光标前 {count} 个字符：收成起点 + ShiftStart 挪了 {shifted}"
    ));
    deleted
}

/// 按 ACP 位置圈出 `[光标-count, 光标]` 并删掉；范围不支持 ACP 或位置越界时返回 `false`。
fn delete_by_acp(caret: &ITfRange, ec: u32, count: i32) -> bool {
    let Ok(acp) = caret.cast::<ITfRangeACP>() else {
        return false;
    };
    let (mut anchor, mut extent) = (0i32, 0i32);
    if unsafe { acp.GetExtent(&mut anchor, &mut extent) }.is_err() {
        return false;
    }
    let Some(start) = anchor
        .checked_add(extent)
        .and_then(|end| end.checked_sub(count))
        .filter(|start| *start >= 0)
    else {
        return false;
    };
    // 空范围上 SetText 本来就什么都不删，不必先用 IsEmpty 挡一道（那个在部分宿主上会失败）
    unsafe { acp.SetExtent(start, count) }.is_ok() && unsafe { acp.SetText(ec, 0, &[]) }.is_ok()
}

fn update_preedit(shared: &Rc<Shared>, context: &ITfContext, ec: u32, preedit: &str) -> Result<()> {
    let composition = match shared.composition() {
        Some(composition) => composition,
        None => start_composition(shared, context, ec)?,
    };
    let utf16: Vec<u16> = preedit.encode_utf16().collect();
    let range = unsafe { composition.GetRange()? };
    unsafe { range.SetText(ec, 0, &utf16)? };
    super::display_attribute::mark(context, ec, &range);
    place_caret(context, ec, &range, utf16.len(), 0);
    Ok(())
}

/// 在当前选区处起一个空组句；组句 sink 交给框架持有。
fn start_composition(shared: &Rc<Shared>, context: &ITfContext, ec: u32) -> Result<ITfComposition> {
    let insert: ITfInsertAtSelection = context.cast()?;
    let range = unsafe { insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])? };
    let context_composition: ITfContextComposition = context.cast()?;
    let sink: ITfCompositionSink = CompositionSink::new(shared.clone()).into();
    let composition = unsafe { context_composition.StartComposition(ec, &range, &sink)? };
    shared.set_composition(Some(composition.clone()));
    Ok(composition)
}

/// 清空组句文本再结束，避免残留拼音。
fn end_composition(shared: &Shared, ec: u32) -> Result<()> {
    if let Some(composition) = shared.take_composition() {
        let range = unsafe { composition.GetRange()? };
        unsafe { range.SetText(ec, 0, &[])? };
        unsafe { composition.EndComposition(ec)? };
    }
    Ok(())
}

/// 把光标放到 `range` 覆盖文本里第 `text_chars + caret_shift` 个字符处（0 就是末尾，负数往回、正数往前）。
///
/// 从**开头**数、挪**起点**：TSF 只在「收成哪一端就挪哪一端」时才真正移动范围
/// （收成末尾再 `ShiftStart` 在真机上不动，成对补全的光标就留在末尾）。
fn move_selection(
    context: &ITfContext,
    ec: u32,
    range: &ITfRange,
    text_chars: usize,
    caret_shift: i16,
) -> Result<()> {
    let caret = unsafe { range.Clone()? };
    unsafe { caret.Collapse(ec, TF_ANCHOR_START)? };
    let steps = i64::try_from(text_chars)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::from(caret_shift))
        .clamp(0, i64::from(i32::MAX));
    if steps > 0 {
        let mut shifted = 0;
        let steps = i32::try_from(steps).unwrap_or(i32::MAX);
        unsafe { caret.ShiftStart(ec, steps, &mut shifted, std::ptr::null())? };
    }
    // 挪完收成点：`ShiftStart` 动的是起点、末尾留在原处，范围可能反向
    unsafe { caret.Collapse(ec, TF_ANCHOR_START)? };
    set_selection(context, ec, caret)
}

/// 没有要走文本的这一拍（成对补全跳过右半边）：把当前光标右移几个字符。
///
/// 先试注入方向键（应用自己的光标逻辑永远可用），注入不了再走 TSF：收成**末尾**再 `ShiftEnd`。
fn shift_caret(context: &ITfContext, ec: u32, caret_shift: i16) -> Result<()> {
    if nudge_caret_by_key(i32::from(caret_shift)) {
        return Ok(());
    }
    // GetSelection 给的 range 归调用方释放：借它当光标，挪好这一轮就放掉
    let mut selections = [TF_SELECTION::default()];
    let mut fetched = 0;
    unsafe { context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selections, &mut fetched)? };
    let mut moved = None;
    for selection in selections {
        let range = ManuallyDrop::into_inner(selection.range);
        if moved.is_none()
            && let Some(range) = range
        {
            let caret = unsafe { range.Clone()? };
            unsafe { caret.Collapse(ec, TF_ANCHOR_END)? };
            shift_collapsed(&caret, ec, caret_shift)?;
            moved = Some(caret);
        }
    }
    match moved {
        Some(caret) => set_selection(context, ec, caret),
        None => Ok(()),
    }
}

/// `range` 已经是「点」（光标）：左移收成**起点**再挪起点、右移收成**末尾**再挪末尾，挪完再收成那个点。
///
/// 必须同端：收成末尾再 `ShiftStart` 在真机上不动（成对补全的光标、两键规则的删字都踩过）；
/// 往左读前文的 `surrounding.rs` 用的就是「收成起点 + `ShiftStart`」，那条路一直是好的。
fn shift_collapsed(range: &ITfRange, ec: u32, shift: i16) -> Result<()> {
    if shift == 0 {
        return Ok(());
    }
    let mut shifted = 0;
    unsafe {
        if shift < 0 {
            range.Collapse(ec, TF_ANCHOR_START)?;
            range.ShiftStart(ec, i32::from(shift), &mut shifted, std::ptr::null())?;
            range.Collapse(ec, TF_ANCHOR_START)
        } else {
            range.Collapse(ec, TF_ANCHOR_END)?;
            range.ShiftEnd(ec, i32::from(shift), &mut shifted, std::ptr::null())?;
            range.Collapse(ec, TF_ANCHOR_END)
        }
    }
}

/// 把 `caret`（一个点）设成当前选区。
fn set_selection(context: &ITfContext, ec: u32, caret: ITfRange) -> Result<()> {
    let selection = TF_SELECTION {
        range: ManuallyDrop::new(Some(caret)),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_END,
            fInterimChar: false.into(),
        },
    };
    // SetSelection 不接管 range 的所有权，之后手动释放。
    let result = unsafe { context.SetSelection(ec, std::slice::from_ref(&selection)) };
    drop(ManuallyDrop::into_inner(selection.range));
    result
}
