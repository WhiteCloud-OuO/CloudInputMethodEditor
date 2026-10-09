//! 候选窗底部那一行「在线翻译」：**只是一个显示位**，内容由脚本给。
//!
//! 脚本返回动作 `online = "文本"` / `{ text = …, state = … }` 就写它，`online = false` 清它。
//! 这一行归脚本：换候选、改拼音都不动它，只有组句结束（`reset_composition`）才收 —— 想「换候选就刷新」
//! 就在 `candidates` 事件里重写。
//!
//! 「翻」那件事由脚本自己做（`cloudime.http_post` 直接调厂商接口），Server **不碰网络、不认识任何厂商**：
//! 它只是把脚本给的这一句画出来。接口与示例见 `docs/design/online-translate.md`。

use cloudime_platform::protocol::{OnlineLine, OnlineState};

/// 候选窗底部那一条在线翻译：只存「现在显示什么」+「它是给哪个候选写的」。
#[derive(Default)]
pub(crate) struct Online {
    /// 正在显示的那一行（`None` = 没写 / 已清）。
    display: Option<OnlineLine>,

    /// 写这一行时**高亮候选的文本**：`Ctrl + 反引号` 只在这一行还对应着当前高亮候选时才拿它上屏
    /// （换过候选就是陈旧的，不认）。
    word: String,
}

impl Online {
    /// 现在该画的那一行。
    pub(crate) fn line(&self) -> Option<OnlineLine> {
        self.display.clone()
    }

    /// 动作 `online = "…"`：脚本写这一行（`word` 是此刻高亮候选的文本，见 [`Online::translation_for`]）。
    pub(crate) fn set(&mut self, line: OnlineLine, word: String) {
        self.display = Some(line);
        self.word = word;
    }

    /// 这一行能不能当「高亮候选 `word` 的译文」上屏：显示着、状态是成功、而且是给这个候选写的。
    ///
    /// 等待 / 失败那一行不参与；脚本给别的候选写的那一行也不参与（换回原来那个候选才又算数）。
    pub(crate) fn translation_for(&self, word: &str) -> Option<String> {
        let line = self.display.as_ref()?;
        if line.state != OnlineState::Done || self.word != word {
            return None;
        }
        (!line.text.trim().is_empty()).then(|| line.text.clone())
    }

    /// 动作 `online = false`、或组句结束：清掉。
    pub(crate) fn clear(&mut self) {
        self.display = None;
        self.word.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cloudime_core::Engine;
    use cloudime_dictionary::Dictionary;
    use cloudime_platform::protocol::{
        ClientMessage, KeyEvent, KeyModifiers, OnlineState, PROTOCOL_VERSION, SessionId,
    };

    use crate::dispatch::{Router, RouterConfig};

    /// 一个临时脚本目录 + 一个脚本（补上清单）；`Router::new` 时加载。
    fn script_dir(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cloudime-online-line-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("line.lua"),
            format!(
                "cloudime.script{{ api = 1, trigger_condition = 'combination_key', combination_modifiers = 'ctrl', budget = 1000000, timeout = 10000, \
                 sync = false, handover = 'callback', on_error = false }}\n{body}"
            ),
        )
        .unwrap();
        dir
    }

    /// 装了那个脚本的 Router（开好会话）。
    fn router_with_script(name: &str, body: &str) -> Router {
        let engine = Engine::new(Dictionary::parse("你\tni\t100\n").unwrap());
        let mut router = Router::new(
            engine,
            RouterConfig {
                scripts_dir: Some(script_dir(name, body)),
                ..RouterConfig::default()
            },
        );
        router.handle(ClientMessage::OpenSession {
            session: SessionId(1),
            app: None,
            protocol: PROTOCOL_VERSION,
        });
        router
    }

    /// 敲一个普通字母。
    fn press(router: &mut Router, c: char) {
        let event = KeyEvent::new(
            c.to_ascii_uppercase() as u32,
            Some(c),
            KeyModifiers::default(),
        );
        let _ = router.handle(ClientMessage::Key {
            session: SessionId(1),
            event,
        });
    }

    /// 动作 `online`：脚本写那一行（自绘帧里有它）、换候选不动它、`online = false` 与组句结束清掉。
    #[test]
    fn a_script_writes_the_online_line() {
        let mut router = router_with_script(
            "script-line",
            "cloudime.on('key', function(event)\n\
                 if event.char == 'w' then\n\
                     return { online = { text = '在线 Hello', state = 'result' } }\n\
                 end\n\
                 if event.char == 'c' then return { online = false } end\n\
             end)\n",
        );
        press(&mut router, 'n');
        assert!(
            router.self_drawn_frame().online.is_none(),
            "没写就不占这一行"
        );

        press(&mut router, 'w');
        let line = router
            .self_drawn_frame()
            .online
            .clone()
            .expect("脚本写了 online");
        assert_eq!(line.text, "在线 Hello");
        assert_eq!(line.state, OnlineState::Done);

        // 再敲一个字母（候选 / 高亮可能变）：脚本写的那一行不该被清掉
        press(&mut router, 'i');
        assert!(router.self_drawn_frame().online.is_some());

        // `online = false` 清掉
        press(&mut router, 'c');
        assert!(router.self_drawn_frame().online.is_none());

        // 再写一次，然后结束组句：也清掉
        press(&mut router, 'w');
        assert!(router.self_drawn_frame().online.is_some());
        router.handle(ClientMessage::Key {
            session: SessionId(1),
            event: KeyEvent::new(0x1B, Some('\u{1b}'), KeyModifiers::default()),
        });
        assert!(
            router.self_drawn_frame().online.is_none(),
            "组句结束该清掉那一行"
        );
    }
}
