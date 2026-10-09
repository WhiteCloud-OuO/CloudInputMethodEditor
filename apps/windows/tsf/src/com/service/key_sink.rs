//! `ITfKeyEventSink`：所有键先经 `OnTestKeyDown` 判吃不吃（[`TextService_Impl::would_eat`]，与 Router 的分派对齐），
//! 吃的键在 `OnKeyDown` 里转发给 Server 并按结果更新文档；单击中英切换键（缺省单击 Shift，由
//! [`InputSettings`](cloudime_platform::protocol::InputSettings) 下发）的判定与保留键命中也在这里。
//! 上下文禁了键盘（密码框，见 [`context`](crate::com::context)）时没在组句的键一律放行。

use windows::Win32::Foundation::{FALSE, LPARAM, TRUE, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CAPITAL, VK_SPACE};
use windows::Win32::UI::TextServices::{ITfContext, ITfKeyEventSink_Impl};
use windows::core::{BOOL, GUID, Ref, Result};

use cloudime_platform::protocol::{IndicatorCommand, KeyEvent, KeyOutcome};

use super::TextService_Impl;
use super::next::Next;
use crate::com::composition::{Update, preedit_string};
use crate::com::key::event::{
    caps_lock_on, clear_caps_lock, ctrl_down, is_edit, is_letter, is_nav, shift_down, to_key_event,
};
use crate::com::key::preserved;
use crate::com::log::log;

impl ITfKeyEventSink_Impl for TextService_Impl {
    /// 失焦：把敲了一半的拼音原样落定。焦点本身交给
    /// [`TextService_Impl::set_thread_focus`]；切窗口时这条回调不触发，靠的是 [`crate::com::focus`]。
    fn OnSetFocus(&self, fforeground: BOOL) -> Result<()> {
        let foreground = fforeground.as_bool();
        if !foreground {
            self.commit_pending();
        }
        self.set_thread_focus(foreground);
        Ok(())
    }

    /// 所有键（含之后被吃掉的）都先经过这里，Shift 单击的判定放在这一层。
    fn OnTestKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let vk = wparam.0 as u32;
        self.note_key_down(vk, lparam);
        if self.keyboard_disabled(&pic) {
            return Ok(FALSE);
        }
        if self.char_width_hotkey(vk) {
            return Ok(TRUE);
        }
        // 有 `key` 触发脚本（`script_wants_text`）：**没在组句时每个按键**前现读一份光标前文（异步只读会话）。
        // 读发生在这一键之后，于是下一次按键时脚本读到的就是「上一键之后」的文档 —— 自动序号按 `Enter`
        // 时 `cloudime.context` 里已经有刚敲的 `1.`（挂在这一层是因为它对**所有**键都调，见上面注释）。
        // 组句里不另读：那份前文由起组句那次读写会话送，且组句期间不变。
        if self
            .input_settings
            .get()
            .is_some_and(|input| input.script_wants_text)
            && !self.shared.composing()
            && let Ok(context) = pic.ok()
        {
            crate::com::edit::request_surrounding_now(
                context,
                self.client_id.get(),
                self.engine.clone(),
            );
        }
        Ok(self.would_eat(&self.key_event(vk)).into())
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let vk = wparam.0 as u32;
        self.note_key_down(vk, lparam);
        if self.keyboard_disabled(&pic) {
            return Ok(FALSE);
        }
        if self.char_width_hotkey(vk) {
            self.send_indicator(IndicatorCommand::ToggleCharWidthType);
            return Ok(TRUE);
        }
        let event = self.key_event(vk);
        Ok(self.handle_key(pic, event).into())
    }

    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        self.note_key_up(wparam.0 as u32);
        Ok(FALSE)
    }

    fn OnKeyUp(&self, _pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        self.note_key_up(wparam.0 as u32);
        Ok(FALSE)
    }

    /// 保留键命中：Ctrl + Alt + Space 切中英，Ctrl + Alt + . 切简繁，Ctrl + Alt + , 切标点。
    /// 禁用时都不接管（按键还给应用）。
    fn OnPreservedKey(&self, pic: Ref<ITfContext>, rguid: *const GUID) -> Result<BOOL> {
        let guid = unsafe { *rguid };
        log(&format!("保留键命中 guid={guid:?}"));
        if self.keyboard_disabled(&pic) || self.mode_state.disabled() {
            return Ok(FALSE);
        }
        if guid == preserved::GUID_SWITCH_MODE {
            self.switch_source.set("保留键 Ctrl + Alt + Space");
            self.set_english_mode(!self.mode_state.english());
            return Ok(true.into());
        }
        let command = if guid == preserved::GUID_TOGGLE_SIMP_TRAD {
            IndicatorCommand::ToggleSimpTrad
        } else if guid == preserved::GUID_TOGGLE_PUNCTUATION {
            IndicatorCommand::TogglePunctuation
        } else {
            return Ok(FALSE);
        };
        self.send_indicator(command);
        Ok(true.into())
    }
}

impl TextService_Impl {
    /// 没在组句时看上下文有没有禁键盘（密码框）：禁了整键放行、不组句。组句中不看——那段组句是我们自己的，
    /// 应用要禁会先终止它。每键两次 compartment 读取，微秒级。
    fn keyboard_disabled(&self, pic: &Ref<ITfContext>) -> bool {
        if self.shared.composing() {
            return false;
        }
        let Ok(context) = pic.ok() else {
            return false;
        };
        let disabled = crate::com::context::keyboard_disabled(context);
        if disabled {
            log("上下文禁用键盘（密码框），放行");
        }
        disabled
    }

    fn key_event(&self, vk: u32) -> KeyEvent {
        to_key_event(vk, self.mode_state.english())
    }

    fn note_key_down(&self, vk: u32, lparam: LPARAM) {
        let keys = self.mode_state.switch_keys();
        // 勾着的切换键这一刻**物理按着**没有：`Shift + "` 里的 Shift 就是被当修饰键用的，
        // 抬起时不算单击（TSF 对修饰键的投递顺序不保证，光靠 tap 里「别的键一动就作废」会漏）。
        let switch_held = (keys.shift && shift_down()) || (keys.control && ctrl_down());
        self.key_tap.key_down(vk, lparam, keys, switch_held);
    }

    fn note_key_up(&self, vk: u32) {
        if vk == u32::from(VK_CAPITAL.0) {
            self.mode_state.notify();
        }
        let keys = self.mode_state.switch_keys();
        // 切换键还物理按着的那次抬起是 TSF 的重复通知，不作数也不清标记（见 `KeyTap::key_up`）
        let switch_held = (keys.shift && shift_down()) || (keys.control && ctrl_down());
        let tapped = self.key_tap.key_up(vk, keys, switch_held);
        if !tapped || self.mode_state.disabled() {
            return;
        }
        log(&format!("单击切换键（vk={vk}）切模式"));
        // Caps Lock 亮着时单击切换键（缺省单击 Shift）：先取消大写锁定，再切英文（与微软拼音一致）。
        if caps_lock_on() {
            self.switch_source.set("单击切换键（Caps 亮着）");
            clear_caps_lock();
            self.set_english_mode(true);
        } else {
            self.switch_source.set("单击切换键");
            self.set_english_mode(!self.mode_state.english());
        }
    }

    /// 内置热键 `Shift + Space`：翻转全角 / 半角字符（中英模式、组句中一律生效）。
    fn char_width_hotkey(&self, vk: u32) -> bool {
        vk == u32::from(VK_SPACE.0)
            && !self.raw_input()
            && !self.mode_state.disabled()
            && shift_down()
    }

    /// 这个键吃不吃，与 Router 的分派对齐；`OnTestKeyDown` 用，无副作用。判定见 [`eats_key`]。
    ///
    /// 带 Ctrl/Alt/Win 的组合一律归应用；
    /// 没在组句时字母只有「中文模式、Caps 灭、没按 Shift」才吃——英文模式与 Caps 亮着的字母归应用
    /// （英文模式纯直通，Caps 只管大小写）；Shift 敲的大写也吃，
    /// 让它起一段组句；
    /// 组句中功能键 / 方向键 / 可打印字符都吃；没在组句时数字 / 标点也先「测吃」送去转全角（中英各有一份开关），
    /// Server 不转的回 Passthrough 再放行。
    fn would_eat(&self, event: &KeyEvent) -> bool {
        // 刚由输入法自己重放回应用的那个键：不能再吃（否则应用还是拿不到）。
        // 别的键先到就把标记作废 —— 那次重放的键没回来（比如前台换了窗口），别误伤后面的同键按键。
        if is_replay(&self.replaying, event.virtual_key) {
            return false;
        }
        // 这个程序在「不显示候选框」名单里（`[candidate] program_list_of_hiding_candidate`），
        // 或者输入法被禁用（Ctrl + Space）：完全不接管
        if self.raw_input() || self.mode_state.disabled() {
            return false;
        }
        // 修饰键本身不吃（见 [`is_modifier_key`]）：这一拍与「Ctrl 组合要不要接管」无关。
        if is_modifier_key(event.virtual_key) {
            return false;
        }
        let input = self.input_settings.get();
        let shift_letter_compose = input.is_some_and(|input| input.shift_letter_compose);
        let full_width_chars = input.is_some_and(|input| input.full_width_chars);
        // 脚本声明要的键（清单 `keys`）：没组句也先问一趟 Server —— 脚本不吃就重放回应用。
        if input.is_some_and(|input| wants_declared_key(event, input.script_keys)) {
            return true;
        }
        // 脚本在清单里声明的那几套修饰键（`combination_modifiers`）：组句与否都命中就送来问一趟，
        // 脚本不吃就回 Passthrough、按键原样归应用。一个组合键脚本都没有（`wanted` 为 0）时这条不生效。
        let wanted = input.map_or(0, |input| input.script_key_modifiers);
        if wants_script_key(event, wanted) {
            return true;
        }
        eats_key(
            event,
            self.shared.composing(),
            shift_letter_compose,
            full_width_chars,
        )
    }

    /// 把已经吃下、Server 又没接管的键**重放回应用**：`SendInput` 注入一次同样的键（修饰键不用注入，
    /// 用户手上还按着），并给下一次 `OnTestKeyDown` 打个「这是我注入的、别吃」的标记。
    fn replay_to_app(&self, event: &KeyEvent) {
        self.replaying.set(Some(event.virtual_key));
        let m = event.modifiers;
        if m.ctrl || m.alt || m.shift || m.win {
            // 按住修饰键的组合：**立刻**注入 —— 延后的话用户多半已经松开修饰键，应用只看到光秃秃的
            // 那个键（`Ctrl+C` 变成 `c`，复制失效）；组合键在 Chromium 里也不受「自动重复」影响。
            crate::com::key::event::send_key(event.virtual_key);
        } else {
            // 不带修饰键的键（`Enter` / `Tab` 这类）：延后到物理键松开再注入，免得被当成自动重复丢掉。
            crate::com::key::event::send_key_deferred(event.virtual_key);
        }
        log(&format!(
            "Server 没接管，重放回应用 vk={}",
            event.virtual_key
        ));
    }

    /// 当前设置是不是「完全不接管」（名单里的程序）。
    fn raw_input(&self) -> bool {
        self.input_settings
            .get()
            .is_some_and(|input| input.raw_input)
    }

    /// 不吃的键绝不碰组句（否则光标一移，组句会把拼音重插到别处）。
    fn handle_key(&self, pic: Ref<ITfContext>, event: KeyEvent) -> bool {
        if !self.would_eat(&event) {
            return false;
        }
        self.forward_key(pic, event)
    }

    /// 把按键送给 Server 并按结果更新文档；返回吃不吃。
    fn forward_key(&self, pic: Ref<ITfContext>, event: KeyEvent) -> bool {
        // 没连上 Server（没起、刚重启、转发失败后的退避期）：只吃「可能是在打拼音」的键，
        // 别让拼音字母漏进应用；标点 / 数字 / 英文与 Caps 下的字母本来就会原样交给应用，
        // 这里放行——一律吃掉会表现为「按了没反应」（连不上时按 `-`、数字都没反应）。
        if !self.ensure_connected() {
            // 名单里的程序本来就不接管，断了也照样放行
            if self.raw_input() {
                return false;
            }
            let eat = eats_without_server(&event);
            if eat {
                log(&format!(
                    "没连上 Server，吃掉 vk={} char={:?}",
                    event.virtual_key, event.character
                ));
            }
            return eat;
        }
        // OnTestKeyDown 已声明吃的可打印字符，Server 放行时由输入法自己插入：退回应用的话，企业微信 /
        // 微信 / notepad++ 这类自绘输入框会把它丢掉。功能键（无字符）仍交给应用。
        let passthrough_char = event.character.filter(|c| !c.is_control());
        if let Ok(context) = pic.ok() {
            self.shared.set_last_context(Some(context.clone()));
        }
        // Server 交互在这段借用里做完，放掉借用再走编辑会话。
        let next = {
            let mut guard = self.engine.borrow_mut();
            let Some(client) = guard.as_mut() else {
                return true;
            };
            // 组句被应用终止过：先让 Server 清掉残留的拼音（文本已在文档里，交出的丢弃）。
            let response = if self.shared.take_server_stale() {
                client.commit().and_then(|_| client.key(event))
            } else {
                client.key(event)
            };
            match response {
                Ok(response) => {
                    // 「只在候选窗口」模式应用里不放行内拼音（那一行由 Server 画在候选窗口顶部）。
                    let preedit = if response.frame.preedit_mode.inline() {
                        preedit_string(&response.frame)
                    } else {
                        String::new()
                    };
                    self.shared.set_composing(!response.frame.is_empty());
                    let consumed = matches!(response.outcome, KeyOutcome::Consumed);
                    let m = event.modifiers;
                    log(&format!(
                        "收键 vk={} ctrl={} alt={} shift={} caps={} en={} char={:?} candidates={} preedit={preedit:?} consumed={consumed}",
                        event.virtual_key,
                        m.ctrl,
                        m.alt,
                        m.shift,
                        m.caps,
                        m.english_mode,
                        event.character,
                        response.frame.candidates.items.len()
                    ));
                    Next::Document {
                        commit: response.commit,
                        preedit,
                        caret_shift: response.caret_shift,
                        delete_before: response.delete_before,
                        consumed,
                    }
                }
                Err(error) => {
                    log(&format!("转发按键失败，放行并断开，下一键重连: {error}"));
                    *guard = None;
                    self.last_connect_failure.set(None);
                    self.shared.end_composing();
                    Next::Abort
                }
            }
        };
        // 带 Ctrl / Alt / Win 的组合放行时仍交还应用，别把热键的字母插进文档。
        let insertable = !event.modifiers.has_command_key();
        match (next, passthrough_char) {
            // 放行 + 没在组句 + 可打印字符：输入法插入，吃掉；Server 顺带交出的英文直输段字母拼在前面。
            (
                Next::Document {
                    consumed: false,
                    commit,
                    preedit,
                    ..
                },
                Some(c),
            ) if insertable && preedit.is_empty() => {
                let mut text = commit.unwrap_or_default();
                text.push(c);
                self.update_document(
                    pic,
                    Update {
                        commit: Some(text),
                        ..Update::default()
                    },
                );
                true
            }
            // 放行（Server 没接管）：**重放回应用** —— `OnTestKeyDown` 已经答过「吃」，光返回 false
            // 应用是拿不到的（可打印字符那条已经自己插进文档；这里管的是 Ctrl / Alt / Win 组合与功能键）。
            (
                Next::Document {
                    consumed: false, ..
                },
                _,
            ) => {
                self.replay_to_app(&event);
                true
            }
            (
                Next::Document {
                    commit,
                    preedit,
                    caret_shift,
                    delete_before,
                    ..
                },
                _,
            ) => {
                self.update_document(
                    pic,
                    Update {
                        commit,
                        preedit,
                        caret_shift,
                        delete_before,
                    },
                );
                true
            }
            (Next::Abort, _) => false,
        }
    }
}

/// 连不上 Server 时这个键吃不吃。
///
/// 只有「中文模式下可能是拼音」的字母要吃掉：漏进应用会变成一串字母，比什么都不出更难看。
/// 标点 / 数字（会话外本来就走 Passthrough 交给应用）与英文模式、Caps 亮着时敲的字母都放行——
/// 一律吃掉会表现为「按了没反应」，而且连日志都不留（这就是「中文模式按 `-` 偶发无反应」的成因：
/// 断连到重连成功之间的键全被吞了）。
fn eats_without_server(event: &KeyEvent) -> bool {
    !event.modifiers.has_command_key()
        && is_letter(event.virtual_key)
        && !(event.modifiers.caps || event.modifiers.english_mode)
}

/// 带 Ctrl（不带 Alt / Win）的组合：输入法自己占的那几个（`Ctrl+数字` / `Ctrl+回车` / `Ctrl+反引号`）
/// 也在内 —— 送上去由 Server 那套「谁先定义谁优先」的规则处理。
fn ctrl_combo(event: &KeyEvent) -> bool {
    event.modifiers.ctrl && !event.modifiers.alt && !event.modifiers.win
}

/// 事件修饰键的位图：bit0 ctrl / bit1 alt / bit2 shift / bit3 win（没有修饰键时是 `0`）。
/// 与清单 `combination_modifiers` / `InputSettings.script_key_modifiers` 用同一套位数。
fn modifier_mask(event: &KeyEvent) -> u8 {
    let modifiers = event.modifiers;
    u8::from(modifiers.ctrl)
        | (u8::from(modifiers.alt) << 1)
        | (u8::from(modifiers.shift) << 2)
        | (u8::from(modifiers.win) << 3)
}

/// 这个键是不是脚本声明要的（`InputSettings.script_keys` 位图，见清单的 `keys`）：
/// 是就**没组句也先问一趟 Server**（脚本不吃就重放回应用）。
fn wants_declared_key(event: &KeyEvent, keys: [u64; 4]) -> bool {
    let vk = event.virtual_key as usize;
    vk < 256 && keys[vk / 64] & (1u64 << (vk % 64)) != 0
}

/// 这个键要不要**先问一趟 Server**：它的修饰键组合被某个脚本声明了（`wanted`，见清单的
/// `combination_modifiers`；第 N 位置位 = 「位图 N」那一套）。组句与否都一样 ——
/// 声明的是哪一套，就只有哪一套送上来，脚本不必自己再筛修饰键。
fn wants_script_key(event: &KeyEvent, wanted: u16) -> bool {
    wanted & (1u16 << modifier_mask(event)) != 0
}

/// 这个键是不是「输入法刚 `SendInput` 重放回应用的那个」：是就别再吃（顺带清掉标记）。
/// 别的键先到也清掉 —— 那次重放没回来（前台换了窗口之类），别误伤后面同键的按键。
fn is_replay(replaying: &std::cell::Cell<Option<u32>>, vk: u32) -> bool {
    replaying.take() == Some(vk)
}

/// 修饰键**本身**（Ctrl / Alt / Shift / Win 的左右键）：不是「组合键」，一律不吃。
///
/// 这条很关键：`OnTestKeyDown` 一旦吃了 `Ctrl` 的 key-down，**后面那个键在 TSF 与应用眼里就没有
/// 修饰键了**（`modifiers.ctrl` 变 false），表现成 `Ctrl+A` 打出一个 `a`、全选失效 —— 而 Ctrl 组合
/// 该不该吃是「按 A 那一拍」的事，与按 Ctrl 这一拍无关。
fn is_modifier_key(vk: u32) -> bool {
    // VK_SHIFT / VK_CONTROL / VK_MENU / VK_LWIN / VK_RWIN
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C)
        // 左右 Shift / Ctrl / Alt
        || matches!(vk, 0xA0..=0xA5)
}

/// 这个键吃不吃（[`TextService_Impl::would_eat`] 的纯逻辑，便于单测）。
///
/// - **修饰键本身一律不吃**（[`is_modifier_key`]）：吃了 `Ctrl` 的 key-down，后面那个键在 TSF
///   与应用眼里就没有修饰键了（`Ctrl+A` 会变成打出一个 `a`）。
/// - **带 Ctrl（不带 Alt / Win）的组合：组句里先问一趟 Server** —— 脚本可以绑任意 Ctrl 组合
///   （`Ctrl+字母` / `Ctrl+标点` / `Ctrl+Shift+…` 都算），「问」只是问：Server 没接管就回
///   `Passthrough`，按键由输入法用 `SendInput` **重放回应用**（`replay_to_app`）。
///   没组句时走 [`wants_script_key`] 那条：只认脚本在清单里声明的修饰键组合。
///   **Alt / Win** 不在上面这一条里（AltGr 就是 Ctrl+Alt，Win 是系统键），脚本要它们得在清单里声明，
///   那时同样由重放机制兜着。
/// - 字母：组句中一定吃（拼音要接着写下去）；没在组句时只有「中文模式、Caps 灭、
///   没按 Shift）」才吃。英文模式是纯直通、
///   Caps 只管大小写，这两种字母都归应用。`⇧C` 接 `pan` 出「C盘」靠的是后面这条；
///   组句一开始，后面的 Shift 字母本来就被 `composing` 兜住；
///   状态条「全角 / 半角」打着（`full_width_chars`）时字母一律吃：得送来 Server 换成全角形再上屏，
///   英文模式本来送都不送，不吃就转不了；
/// - 组句中功能键 / 方向键 / 可打印字符都吃；
/// - 没在组句时数字 / 标点也先「测吃」送去转全角（中英各有一份开关），Server 不转的回 Passthrough 再放行。
fn eats_key(
    event: &KeyEvent,
    composing: bool,
    shift_letter_compose: bool,
    full_width_chars: bool,
) -> bool {
    // 修饰键本身一律不吃（见 [`is_modifier_key`]）：吃了 Ctrl，后面的 `Ctrl+A` 就变成打出一个 `a`。
    if is_modifier_key(event.virtual_key) {
        return false;
    }
    let modifiers = event.modifiers;
    // 带 Ctrl（不带 Alt / Win）的组合：**只在组句里**（候选窗口显示着）才问一趟 Server ——
    // 脚本能在打字过程中接管组合键；不在组句时一律归应用，否则记事本里的 `Ctrl+A` / `Ctrl+C`
    // 这类快捷键会被输入法吃掉（`OnTestKeyDown` 那边一旦答「吃」，应用就再也拿不到这个键）。
    // **不在组句时也送**的那几套见 [`wants_script_key`] 与 `would_eat`（清单的 `combination_modifiers`）。
    if composing && ctrl_combo(event) {
        return true;
    }
    if modifiers.has_command_key() {
        return false;
    }
    let vk = event.virtual_key;
    if is_letter(vk) {
        return composing
            || full_width_chars
            || (!modifiers.caps
                && !modifiers.english_mode
                && (!modifiers.shift || shift_letter_compose));
    }
    if composing {
        return is_edit(vk) || is_nav(vk) || event.character.is_some_and(|c| !c.is_control());
    }
    event
        .character
        .is_some_and(|c| c.is_ascii_punctuation() || c.is_ascii_digit())
}

/// 组句里 Ctrl 组合能不能被吃进 Server：见 [`eats_key`] 里那条规则（带 Ctrl、不带 Alt / Win 就吃）。
#[cfg(test)]
mod tests {
    use cloudime_platform::protocol::{KeyEvent, KeyModifiers};

    use super::{
        eats_key, eats_without_server, is_modifier_key, is_replay, wants_declared_key,
        wants_script_key,
    };
    use crate::com::key::event::to_key_event;

    /// 中文模式（`caps` / `english_mode` 都灭）。
    const CHINESE: (bool, bool) = (false, false);

    fn key(vk: u32, caps: bool, english_mode: bool) -> cloudime_platform::protocol::KeyEvent {
        let mut event = to_key_event(vk, english_mode);
        // `to_key_event` 的修饰键来自实时键盘状态（`GetKeyState`）：测试里一律抹平，
        // 免得跑测试时手还按着 Shift / Ctrl 就红。
        event.modifiers = KeyModifiers {
            caps,
            english_mode,
            ..KeyModifiers::default()
        };
        event
    }

    fn with_modifiers(vk: u32, character: char, modifiers: KeyModifiers) -> KeyEvent {
        let mut event = KeyEvent::new(vk, Some(character), modifiers);
        event.modifiers = modifiers;
        event
    }

    /// 脚本声明的键（`keys` → `InputSettings.script_keys` 位图）：命中的才在没组句时也送来问一趟。
    #[test]
    fn only_the_declared_keys_are_sent() {
        let ctrl = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        // 声明了 Enter（0x0D）
        let mut keys = [0u64; 4];
        keys[0] |= 1u64 << 13;
        assert!(wants_declared_key(
            &KeyEvent::new(0x0D, None, KeyModifiers::default()),
            keys
        ));
        assert!(
            wants_declared_key(&KeyEvent::new(0x0D, None, ctrl), keys),
            "带着修饰键也算（声明的是键，不是组合）"
        );
        assert!(
            !wants_declared_key(
                &KeyEvent::new(0x41, Some('a'), KeyModifiers::default()),
                keys
            ),
            "没声明的键不算"
        );
        assert!(
            !wants_declared_key(
                &KeyEvent::new(0x0D, None, KeyModifiers::default()),
                [0u64; 4]
            ),
            "一个都没声明"
        );
    }

    /// 修饰键**本身**一律不吃（`Ctrl` / `Alt` / `Shift` / `Win` 左右键）：吃了它，后面那个键在 TSF
    /// 与应用眼里就没有修饰键了 —— `Ctrl+A` 会变成打出一个 `a`、全选失效。
    #[test]
    fn modifier_keys_are_never_eaten() {
        let ctrl = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        for vk in [
            0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
        ] {
            assert!(is_modifier_key(vk), "vk={vk} 是修饰键");
            assert!(
                !eats_key(&KeyEvent::new(vk, None, ctrl), true, false, false),
                "组句里也不吃 vk={vk}"
            );
        }
        assert!(!is_modifier_key(0x41), "A 不是修饰键");
    }

    /// 重放回去的键不再吃；别的键先到就把标记作废（别误伤后面同键的按键）。
    #[test]
    fn a_replayed_key_is_not_eaten_again() {
        let replaying = std::cell::Cell::new(None);
        assert!(!is_replay(&replaying, 0x41), "没重放过，照旧");
        replaying.set(Some(0x41));
        assert!(is_replay(&replaying, 0x41), "就是刚重放的那个");
        assert_eq!(replaying.get(), None, "匹配过就清掉");
        // 重放的那个没回来，先来了别的键：标记作废，同键的下一次按键照旧吃
        replaying.set(Some(0x41));
        assert!(!is_replay(&replaying, 0x42), "别的键不算");
        assert!(!is_replay(&replaying, 0x41), "标记已作废");
    }

    /// 脚本声明的那几套修饰键（`combination_modifiers` → `InputSettings.script_key_modifiers`）：
    /// 命中的才送来，别的一律归应用。
    #[test]
    fn only_the_declared_modifier_combos_are_sent() {
        let ctrl = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        let ctrl_shift = KeyModifiers {
            shift: true,
            ..ctrl
        };
        let alt = KeyModifiers {
            alt: true,
            ..KeyModifiers::default()
        };
        // 只声明 `ctrl`（位图 0b0001 → 第 1 位）
        let only_ctrl = 1u16 << 0b0001;
        assert!(wants_script_key(
            &with_modifiers(0x41, 'a', ctrl),
            only_ctrl
        ));
        assert!(
            !wants_script_key(&with_modifiers(0x41, 'A', ctrl_shift), only_ctrl),
            "ctrl+shift 是另一套，没声明就不送"
        );
        assert!(!wants_script_key(
            &with_modifiers(0x41, 'a', alt),
            only_ctrl
        ));
        assert!(
            !wants_script_key(
                &with_modifiers(0x41, 'a', KeyModifiers::default()),
                only_ctrl
            ),
            "没有修饰键的普通按键不归这条路"
        );
        // 两组都声明时两套都送
        let both = only_ctrl | (1u16 << 0b0101);
        assert!(wants_script_key(
            &with_modifiers(0x41, 'A', ctrl_shift),
            both
        ));
        // 一个组合键脚本都没有：什么也不送
        assert!(!wants_script_key(&with_modifiers(0x41, 'a', ctrl), 0));
    }

    /// 任意 Ctrl 组合（不带 Alt / Win）**在组句里**都先问一趟 Server：脚本想绑什么绑什么；
    /// `Ctrl+C` 这类也会走一趟，没有人接管就 Passthrough 交回应用。
    /// （没组句时要不要问见上面那个用例。）
    #[test]
    fn any_ctrl_combo_goes_to_the_server_first() {
        let ctrl = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        // 组句中：Ctrl+数字 / 小键盘 / 回车 / 反引号 —— 内置那几个照旧先测吃
        for vk in [0x31, 0x61, 0x0D, 0xC0] {
            assert!(eats_key(
                &with_modifiers(vk, '\u{1}', ctrl),
                true,
                false,
                false
            ));
        }
        // 以前白名单外的（Ctrl+A / Ctrl+Z / Ctrl+F…）现在也吃：脚本能绑任意 Ctrl 组合
        assert!(eats_key(
            &with_modifiers(0x41, 'a', ctrl),
            true,
            false,
            false
        ));
        assert!(eats_key(
            &with_modifiers(0x5A, '\u{1a}', ctrl),
            true,
            false,
            false
        ));
        // 不带 Ctrl 时的老规矩不变
        assert!(eats_key(
            &with_modifiers(0x31, '1', ctrl),
            true,
            false,
            false
        ));
        // 带 Shift 的 Ctrl 组合同样问（脚本里看 event.shift 区分；这里在组句里）
        let ctrl_shift = KeyModifiers {
            shift: true,
            ..ctrl
        };
        assert!(eats_key(
            &with_modifiers(0x54, '\u{14}', ctrl_shift),
            true,
            false,
            false
        ));
        // **没在组句时不问**（`Ctrl+A` / `Ctrl+C` 这类快捷键必须原样归应用）：
        // 一旦在 `OnTestKeyDown` 答「吃」，应用就再拿不到这个键了。
        assert!(!eats_key(
            &with_modifiers(0x41, 'a', ctrl),
            false,
            false,
            false
        ));
        assert!(!eats_key(
            &with_modifiers(0x0D, '\r', ctrl),
            false,
            false,
            false
        ));

        // Alt / Win / Ctrl+Alt（AltGr）一律不碰：它们在系统 / 外壳那一层，输入法不该拦
        let alt = KeyModifiers { alt: true, ..ctrl };
        assert!(!eats_key(
            &with_modifiers(0x31, '\u{1}', alt),
            true,
            false,
            false
        ));
        assert!(!eats_key(
            &with_modifiers(0x54, '\u{14}', alt),
            false,
            false,
            false
        ));
        let win = KeyModifiers { win: true, ..ctrl };
        assert!(!eats_key(
            &with_modifiers(0x41, 'a', win),
            true,
            false,
            false
        ));
        // 只有 Win（Win+Space 切输入法那类）：也不碰 —— 这是脚本把自己玩坏时的逃生门
        let win_only = KeyModifiers {
            win: true,
            ..KeyModifiers::default()
        };
        assert!(!eats_key(
            &with_modifiers(0x20, ' ', win_only),
            true,
            false,
            false
        ));
    }

    #[test]
    fn shifted_letters_without_composition_go_to_the_app() {
        // 中文模式、没在组句：Shift 敲的大写缺省**归应用**（原样打出来）
        let shifted = KeyModifiers {
            shift: true,
            ..KeyModifiers::default()
        };
        assert!(!eats_key(
            &with_modifiers(0x41, 'A', shifted),
            false,
            false,
            false
        ));
        // Shift+字母固定收进组句：没在组句也吃，送去起一段组句（⇧C 接 pan 出 C盘）
        assert!(eats_key(
            &with_modifiers(0x41, 'A', shifted),
            false,
            true,
            false
        ));
        // 组句一开始，后面的 Shift 字母就被 `composing` 兜住，一律吃
        assert!(eats_key(
            &with_modifiers(0x41, 'A', shifted),
            true,
            false,
            false
        ));
        // 不带 Shift 的字母本来就吃
        assert!(eats_key(
            &with_modifiers(0x41, 'a', KeyModifiers::default()),
            false,
            false,
            false
        ));
        // Caps 亮着的字母由应用自己上屏（我们只管大小写位），归应用
        let caps = KeyModifiers {
            caps: true,
            ..KeyModifiers::default()
        };
        assert!(!eats_key(
            &with_modifiers(0x41, 'A', caps),
            false,
            false,
            false
        ));
        // 英文模式纯直通，字母也归应用；只有组句中才吃（先把拼音原样上屏）
        let english = KeyModifiers {
            english_mode: true,
            ..KeyModifiers::default()
        };
        assert!(!eats_key(
            &with_modifiers(0x41, 'a', english),
            false,
            false,
            false
        ));
        assert!(eats_key(
            &with_modifiers(0x41, 'a', english),
            true,
            false,
            false
        ));
        // 带 Ctrl 的组合：没在组句时归应用（记事本的 `Ctrl+C` 不该被吃掉）
        let ctrl_c = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        assert!(!eats_key(
            &with_modifiers(0x43, 'c', ctrl_c),
            false,
            false,
            false
        ));
    }

    #[test]
    fn full_width_chars_eat_letters_in_every_mode() {
        // 状态条「全角 / 半角」打着：字母一律先送来 Server 转全角，英文模式 / Caps / Shift 都不例外
        let english = KeyModifiers {
            english_mode: true,
            ..KeyModifiers::default()
        };
        assert!(eats_key(
            &with_modifiers(0x41, 'a', english),
            false,
            false,
            true
        ));
        let caps = KeyModifiers {
            caps: true,
            ..KeyModifiers::default()
        };
        assert!(eats_key(
            &with_modifiers(0x41, 'A', caps),
            false,
            false,
            true
        ));
        let shifted = KeyModifiers {
            shift: true,
            ..KeyModifiers::default()
        };
        assert!(eats_key(
            &with_modifiers(0x41, 'A', shifted),
            false,
            false,
            true
        ));
        // 没在组句：带 Ctrl 的也归应用（「问」只在组句里有意义）
        let ctrl_c = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        assert!(!eats_key(
            &with_modifiers(0x43, 'c', ctrl_c),
            false,
            false,
            true
        ));
    }

    #[test]
    fn letters_are_eaten_only_in_chinese_mode() {
        let (caps, english) = CHINESE;
        assert!(eats_without_server(&key(0x41, caps, english))); // A
        assert!(!eats_without_server(&key(0x41, true, english))); // Caps 亮着是直通英文
        assert!(!eats_without_server(&key(0x41, caps, true))); // 英文模式
    }

    #[test]
    fn punctuation_digits_and_command_keys_go_to_the_app() {
        let (caps, english) = CHINESE;
        // `-`（0xBD）、数字 2、`@`：中文模式会话外都是原样交给应用的键
        assert!(!eats_without_server(&key(0xBD, caps, english)));
        assert!(!eats_without_server(&key(0x32, caps, english)));
        assert!(!eats_without_server(&key(0x32, true, english)));
        // Ctrl+C 这类组合键一律归应用
        let mut combo = key(0x43, caps, english);
        combo.modifiers = KeyModifiers {
            ctrl: true,
            ..combo.modifiers
        };
        assert!(!eats_without_server(&combo));
    }
}
