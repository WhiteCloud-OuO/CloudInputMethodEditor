//! 系统语音（SAPI 的 `ISpVoice`）：`Shift + 反引号` 念候选的译文。
//!
//! 「讲述人」自己没有可调的接口，它念东西用的也是系统这套语音引擎，所以这里直接调 SAPI。
//! 取哪条声线按**要念的文本**认语言：拉丁字母用英语声线（本机的 `Microsoft Zira`），
//! 汉字 / 假名用中文学线；都不认（数字、标点）就沿用当前那条。
//!
//! 几个不得不自己写的东西：`windows` crate 只生成了接口，SAPI 的 coclass GUID、声线类别
//! （`SPCAT_VOICES`）和 `Speak` 的标志位都没生成，见下面的常量。
//! COM 对象绑在创建它的线程上，所以声线用线程局部存一份，跟着 Server 的消息循环线程走；
//! `Speak` 用异步 + 「新的顶掉旧的」，不会挡住按键。

/// 英语（美国）与简体中文的 LCID（声线注册表里 `Attributes\Language` 的值）。
const EN_US: u32 = 0x409;
const ZH_CN: u32 = 0x804;

/// 念 `text`：异步，立刻返回，新的顶掉还没念完的；念不出来只记日志，绝不影响打字。
pub(crate) fn speak(text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        emit(text);
    }
}

/// 这段文本该用哪条声线：汉字 / 假名 → 中文，字母 → 英语，认不出返回 `None`（沿用当前声线）。
fn language_of(text: &str) -> Option<u32> {
    for c in text.chars() {
        if is_han_or_kana(c) {
            return Some(ZH_CN);
        }
        if c.is_alphabetic() {
            return Some(EN_US);
        }
    }
    None
}

/// 汉字（基本区）或平假名 / 片假名。
fn is_han_or_kana(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3040}'..='\u{30FF}').contains(&c)
}

/// 真的念：Windows 上走 SAPI。
#[cfg(all(windows, not(test)))]
fn emit(text: &str) {
    match sapi::Voice::with(|voice| voice.speak(text)) {
        Ok(()) => tracing::debug!(text, "发音"),
        Err(error) => tracing::warn!(%error, text, "念不出来（不影响打字）"),
    }
}

/// 测试里不真发声（出声吵人、还要机器上有音频设备），只记下要念的文本给断言用。
/// 非 Windows 上也照样记，好让按键测试跨平台跑。
#[cfg(test)]
fn emit(text: &str) {
    RECORDED.with(|last| *last.borrow_mut() = Some(text.to_owned()));
}

/// 非 Windows 上什么也不做（Server 实际只跑在 Windows）。
#[cfg(all(not(windows), not(test)))]
fn emit(_text: &str) {}

/// 测试用：取走最近一次「要念的文本」。
#[cfg(test)]
pub(crate) fn take_recorded() -> Option<String> {
    RECORDED.with(|last| last.borrow_mut().take())
}

#[cfg(test)]
thread_local! {
    /// 测试里 `emit` 记下的文本。
    static RECORDED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(windows, not(test)))]
mod sapi {
    use std::cell::RefCell;

    use windows::Win32::Media::Speech::{ISpObjectToken, ISpObjectTokenCategory, ISpVoice};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::core::{BSTR, GUID, HSTRING, PCWSTR, Result, w};

    use super::language_of;

    /// `CLSID_SpVoice`。
    const CLSID_SP_VOICE: GUID = GUID::from_u128(0x96749377_3391_11d2_9ee3_00c04f797396);

    /// `CLSID_SpObjectTokenCategory`：用来枚举声线。
    const CLSID_SP_TOKEN_CATEGORY: GUID = GUID::from_u128(0xa910187f_0c7a_45ac_92cc_59edafb77b53);

    /// `SPCAT_VOICES`：桌面 SAPI 的声线都注册在这个键下面。
    const SPCAT_VOICES: PCWSTR = w!("HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech\\Voices");

    /// `SPF_ASYNC`：立刻返回，不挡消息循环。
    const SPF_ASYNC: u32 = 1;

    /// `SPF_PURGEBEFORESPEAK`：新的这句顶掉还没念完的。
    const SPF_PURGEBEFORESPEAK: u32 = 2;

    thread_local! {
        /// 发声器：COM 对象绑线程，建一次留在这儿。建不起来时下次调用会再试一次。
        static VOICE: RefCell<Option<Voice>> = const { RefCell::new(None) };
    }

    /// 一个发声器 + 这台机器上枚举到的声线。
    pub(super) struct Voice {
        voice: ISpVoice,

        /// 语言 LCID → 声线。
        voices: Vec<(u32, ISpObjectToken)>,

        /// 当前挂着哪条（按语言去重，避免同一串里反复换声线）。
        language: Option<u32>,
    }

    impl Voice {
        /// 在线程局部里拿一份（没有就建）。
        pub(super) fn with<T>(action: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
            VOICE.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = Some(Self::open()?);
                }
                action(slot.as_mut().expect("上面刚建好"))
            })
        }

        /// 初始化 COM（本线程第一次）并建发声器、枚举声线。
        fn open() -> Result<Self> {
            unsafe {
                // 已经初始化过（`S_FALSE`）也算成功
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
                let voice: ISpVoice = CoCreateInstance(&CLSID_SP_VOICE, None, CLSCTX_ALL)?;
                Ok(Self {
                    voice,
                    voices: Self::enumerate()?,
                    language: None,
                })
            }
        }

        /// 枚举桌面 SAPI 的声线，读不到语言的跳过。
        fn enumerate() -> Result<Vec<(u32, ISpObjectToken)>> {
            unsafe {
                let category: ISpObjectTokenCategory =
                    CoCreateInstance(&CLSID_SP_TOKEN_CATEGORY, None, CLSCTX_ALL)?;
                category.SetId(SPCAT_VOICES, false)?;
                let tokens = category.EnumTokens(PCWSTR::null(), PCWSTR::null())?;
                let mut voices = Vec::new();
                loop {
                    let mut one: Option<ISpObjectToken> = None;
                    if tokens.Next(1, &mut one, None).is_err() {
                        break;
                    }
                    let Some(token) = one else {
                        break;
                    };
                    if let Some(language) = token_language(&token) {
                        voices.push((language, token));
                    }
                }
                Ok(voices)
            }
        }

        /// 念一句：按文本挑声线（挑不到就沿用当前的），异步播出去。
        pub(super) fn speak(&mut self, text: &str) -> Result<()> {
            if let Some(language) = language_of(text)
                && self.language != Some(language)
                && let Some((_, token)) = self.voices.iter().find(|(found, _)| *found == language)
            {
                unsafe { self.voice.SetVoice(token)? };
                self.language = Some(language);
            }
            unsafe {
                self.voice
                    .Speak(&HSTRING::from(text), SPF_ASYNC | SPF_PURGEBEFORESPEAK, None)
            }
        }
    }

    /// 一条声线的语言（`Attributes\Language` 是十六进制 LCID）；读不到返回 `None`。
    fn token_language(token: &ISpObjectToken) -> Option<u32> {
        unsafe {
            let raw = token
                .OpenKey(w!("Attributes"))
                .ok()?
                .GetStringValue(&BSTR::from("Language"))
                .ok()?
                .to_string()
                .ok()?;
            u32::from_str_radix(&raw, 16).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_picked_from_the_text() {
        assert_eq!(language_of("sorrow"), Some(EN_US));
        assert_eq!(language_of("開発"), Some(ZH_CN));
        assert_eq!(language_of("かいはつ"), Some(ZH_CN));
        assert_eq!(language_of("云朵"), Some(ZH_CN));
        // 认不出的（数字、标点）沿用当前声线
        assert_eq!(language_of("3.14"), None);
        assert_eq!(language_of(""), None);
        // 拉丁字母后面跟汉字：按第一个认得出的字算
        assert_eq!(language_of("cat 猫"), Some(EN_US));
    }

    #[test]
    fn speaking_records_what_it_would_say() {
        take_recorded();
        speak("  sorrow  ");
        assert_eq!(take_recorded().as_deref(), Some("sorrow"));
        // 空串不念
        speak("   ");
        assert_eq!(take_recorded(), None);
    }
}
