//! 用户脚本：Server 在按键那一拍把事件派给 `scripts\` 下的 Lua 脚本（`cloudime.on`）。

use std::path::PathBuf;

use cloudime_platform::protocol::DocumentText;

use crate::support::*;

/// 一个空的临时脚本目录；同名目录先清掉。
fn script_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cloudime-server-scripts-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 每个测试脚本都要的清单，`write_script` 自动加在最前面（缺清单的脚本判无效）。
/// **不写 `name`** —— 脚本名取文件名。
const MANIFEST: &str = "cloudime.script{ api = 1, budget = 1000000, timeout = 10000, \
                        sync = false, handover = 'callback', on_error = false }\n";

/// 写一个测试脚本（自动补一份合法清单）。
fn write_script(
    dir: impl AsRef<std::path::Path>,
    name: &str,
    body: impl AsRef<str>,
) -> std::io::Result<()> {
    std::fs::write(
        dir.as_ref().join(name),
        format!("{MANIFEST}{}", body.as_ref()),
    )
}

/// 脚本目录指到这个临时目录的配置。
fn config_with_scripts(dir: PathBuf) -> RouterConfig {
    RouterConfig {
        scripts_dir: Some(dir),
        ..RouterConfig::default()
    }
}

/// 脚本能收到 `startup` 与每一键：键处理之前派发，载荷里带着虚拟键码与当时的模式。
#[test]
fn a_script_sees_startup_and_every_key() {
    let dir = script_dir("sees-key");
    let log = dir.join("seen.txt");
    // 脚本是用户自己写的本机程序（完整标准库）：这里直接把收到的事件写进一个文件，测试读它。
    write_script(&dir, "watch.lua", format!(
            "cloudime.log('测试脚本起来了')\n\
             local path = [[{log}]]\n\
             cloudime.on('startup', function()\n\
                 local file = io.open(path, 'a')\n\
                 file:write('startup\\n')\n\
                 file:close()\n\
             end)\n\
             cloudime.on('key', function(event)\n\
                 local file = io.open(path, 'a')\n\
                 file:write(string.format('vk=%d mode=%s composing=%s\\n', event.vk, event.mode, tostring(event.composing)))\n\
                 file:close()\n\
             end)\n",
            log = log.display()
        ),
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    // 建 Router 时就派过一次 startup
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().next(),
        Some("startup")
    );

    // 敲一个 n：脚本看到的是「这一键处理之前」的状态（还没在组句）
    press(&mut router, letter('n'));
    let written = std::fs::read_to_string(&log).unwrap();
    assert!(written.contains("startup"), "{written}");
    assert!(
        written.contains("vk=78 mode=chinese composing=false"),
        "{written}"
    );
}

/// 脚本报错不影响输入法：按键照常出候选。
#[test]
fn a_broken_script_does_not_break_input() {
    let dir = script_dir("broken");
    write_script(
        &dir,
        "boom.lua",
        "cloudime.on('key', function() error('派发期炸') end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = press(&mut router, letter('n'));
    assert!(
        !candidate_texts(&frame).is_empty(),
        "脚本炸了，候选照出：{:?}",
        candidate_texts(&frame)
    );
}

/// 脚本目录不存在（新装用户还没建）时照常跑，不影响输入。
#[test]
fn a_missing_script_directory_is_fine() {
    let dir = script_dir("gone");
    std::fs::remove_dir_all(&dir).unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = press(&mut router, letter('n'));
    assert!(!candidate_texts(&frame).is_empty());
}

/// `passthrough`：这一键不吃、原样交给应用（游戏里抢键就靠它）。
#[test]
fn a_script_can_pass_a_key_through() {
    let dir = script_dir("passthrough");
    write_script(
        &dir,
        "game.lua",
        "cloudime.on('key', function() return { passthrough = true } end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    // 中文模式下字母本来会被吃掉起组句，脚本说放行就放行
    let (outcome, commit, _frame) = press(&mut router, letter('n'));
    assert_eq!(outcome, KeyOutcome::Passthrough);
    assert!(commit.is_none());
}

/// `commit`：脚本吃掉这一键、上屏自己的文本（文本扩展就是这么写的）。
#[test]
fn a_script_can_commit_its_own_text() {
    let dir = script_dir("commit");
    write_script(
        &dir,
        "expand.lua",
        "cloudime.on('key', function(event)\n\
             if event.char == 'n' then return { commit = '云朵输入法' } end\n\
         end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (outcome, commit, frame) = press(&mut router, letter('n'));
    assert_eq!(outcome, KeyOutcome::Consumed);
    assert_eq!(commit.as_deref(), Some("云朵输入法"));
    // 上屏的是整段文本，组句没起来
    assert!(frame.candidates.items.is_empty());

    // 脚本没接管的键照旧走引擎
    let (outcome, _commit, frame) = press(&mut router, letter('h'));
    assert_eq!(outcome, KeyOutcome::Consumed);
    assert!(!candidate_texts(&frame).is_empty());
}

/// `notice`：脚本在候选窗里显示一行提示，随这一帧下发。
#[test]
fn a_script_can_show_a_notice() {
    let dir = script_dir("notice");
    write_script(
        &dir,
        "hint.lua",
        "cloudime.on('key', function() return { notice = '脚本提示' } end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = press(&mut router, letter('n'));
    assert_eq!(frame.notice.as_deref(), Some("脚本提示"));
}

/// 输入法自己占用的组合键（`Ctrl+数字` / `Ctrl+Enter` / `Ctrl+反引号` / `Shift+反引号`）**不派发给脚本**：
/// 谁先定义谁优先，脚本抢不走；别的 `Ctrl` 组合照旧派发（脚本说了算）。
#[test]
fn the_ime_keeps_its_own_combos() {
    let dir = script_dir("reserved-combos");
    write_script(
        &dir,
        "grab.lua",
        "cloudime.on('key', function() return { notice = '脚本被调了' } end)\n",
    )
    .unwrap();
    let mut router = router_with(config_with_scripts(dir));
    let ctrl = KeyModifiers {
        ctrl: true,
        ..KeyModifiers::default()
    };
    let shift = KeyModifiers {
        shift: true,
        ..KeyModifiers::default()
    };
    let backquote = |modifiers| KeyEvent::new(0xC0, Some('`'), modifiers);

    // `Ctrl+数字` 只在组句里有意义，先打一段组句
    press(&mut router, letter('n'));
    press(&mut router, letter('i'));

    // Ctrl+1：由输入法自己处理（杀词，会写它自己的那条提示），脚本没被调
    let (_, _, frame) = press(&mut router, letter_with('1', ctrl));
    let notice = frame.notice.clone().unwrap_or_default();
    assert!(
        !notice.is_empty(),
        "Ctrl+1 该由输入法自己处理（杀词并给提示）"
    );
    assert_ne!(notice, "脚本被调了", "输入法自己的组合键不该派发给脚本");

    // Ctrl+反引号 / Shift+反引号：同样不派发
    for modifiers in [ctrl, shift] {
        let (_, _, frame) = press(&mut router, backquote(modifiers));
        assert_ne!(
            frame.notice.as_deref(),
            Some("脚本被调了"),
            "输入法自己的组合键不该派发给脚本"
        );
    }

    // Ctrl+Enter：原样上屏（输入法自己处理），组句结束
    let (_, commit, _) = press(&mut router, KeyEvent::new(0x0D, Some('\r'), ctrl));
    assert!(commit.is_some(), "Ctrl+Enter 该由输入法自己原样上屏");

    // 别的 Ctrl 组合（Ctrl+Z 这种不在输入法白名单里的）照旧派发：脚本说了算
    press(&mut router, letter('n'));
    let (_, _, frame) = press(&mut router, letter_with('z', ctrl));
    assert_eq!(frame.notice.as_deref(), Some("脚本被调了"));
}

/// `cloudime.candidate.redraw()`：脚本在**异步回调**里要重画 —— 即使回调不返回动作表，
/// Server 也会把这一屏重算一遍（重新派发 `candidates` 事件）再重画。
#[test]
fn a_script_can_ask_for_a_repaint_from_a_callback() {
    let dir = script_dir("redraw");
    let address = one_shot_server("ok");
    write_script(
        &dir,
        "ask.lua",
        format!(
            "cloudime.on('startup', function()\n\
                 cloudime.http_get([[{address}]], 5000, function(result)\n\
                     cloudime.candidate.redraw()\n\
                 end)\n\
             end)\n\
             cloudime.on('candidates', function(list)\n\
                 seen = (seen or 0) + 1\n\
                 return {{ notice = '第 ' .. seen .. ' 次排版' }}\n\
             end)\n"
        ),
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    // 打一段组句：每敲一个键都会重排一次（`candidates` 跟着跑），所以这里先跑到「第 2 次排版」
    let (_, _, frame) = press(&mut router, letter('n'));
    assert_eq!(frame.notice.as_deref(), Some("第 1 次排版"));
    let (_, _, frame) = press(&mut router, letter('i'));
    assert_eq!(
        frame.notice.as_deref(),
        Some("第 2 次排版"),
        "敲键本来就会重排一次"
    );

    // 等 HTTP 结果回来，驱动 tick：回调只要了重画（没返回动作表），Server 也该重新排版一次
    let started = std::time::Instant::now();
    let want = "第 3 次排版";
    let mut seen = None;
    while started.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        router.tick();
        let frame = match router.handle(ClientMessage::Poll { session: SESSION }) {
            Some(ServerMessage::Update { frame, .. }) => frame,
            other => panic!("expected Update, got {other:?}"),
        };
        if frame.notice.as_deref() == Some(want) {
            seen = Some(want);
            break;
        }
    }
    assert_eq!(
        seen,
        Some(want),
        "回调里的 redraw() 该让 Server 重算这一屏（candidates 再跑一遍）"
    );
}

/// 多个脚本都改同一项时，后面的盖前面的。
#[test]
fn a_later_script_wins_in_the_merge() {
    let dir = script_dir("merge");
    write_script(
        &dir,
        "a-first.lua",
        "cloudime.on('key', function() return { commit = '先' } end)\n",
    )
    .unwrap();
    write_script(
        &dir,
        "b-last.lua",
        "cloudime.on('key', function() return { commit = '后' } end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, commit, _frame) = press(&mut router, letter('n'));
    assert_eq!(commit.as_deref(), Some("后"));
}

/// 返回值类型写错（`commit = 42`）只当没写，这一键照旧走引擎。
#[test]
fn a_bad_return_value_is_ignored() {
    let dir = script_dir("bad-return");
    write_script(
        &dir,
        "typo.lua",
        "cloudime.on('key', function() return { commit = 42 } end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (outcome, commit, frame) = press(&mut router, letter('n'));
    assert_eq!(outcome, KeyOutcome::Consumed);
    assert!(commit.is_none());
    assert!(!candidate_texts(&frame).is_empty());
}

/// 一屏候选的文本（owned，便于与另一台的对比）。
fn candidate_texts_of(frame: &Frame) -> Vec<String> {
    candidate_texts(frame)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// `order`：引擎排完之后脚本临时接管顺序 —— 改的是布局本身，后面的选择跟着走。
#[test]
fn a_script_can_reorder_the_candidates() {
    // 先看引擎自己排的样子
    let plain = {
        let mut router = router();
        let (_outcome, _commit, frame) = press(&mut router, letter('n'));
        candidate_texts_of(&frame)
    };
    assert!(plain.len() > 2, "样例词库该给出多个候选：{plain:?}");

    let dir = script_dir("order");
    write_script(
        &dir,
        "reorder.lua",
        "cloudime.on('candidates', function(list)\n\
             return { order = { 2, 1 } }\n\
         end)\n",
    )
    .unwrap();
    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = press(&mut router, letter('n'));
    let scripted = candidate_texts_of(&frame);
    // 头两个换了个儿，其余不动
    assert_eq!(scripted[0], plain[1]);
    assert_eq!(scripted[1], plain[0]);
    assert_eq!(scripted[2..], plain[2..]);
}

/// `display`：只改候选里显示的内容，上屏的仍是 `text`。
#[test]
fn a_script_can_change_the_candidate_display() {
    let dir = script_dir("display");
    write_script(
        &dir,
        "label.lua",
        "cloudime.on('candidates', function(list)\n\
             return { display = { [1] = '①脚本显示' } }\n\
         end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = press(&mut router, letter('n'));
    let first = &frame.candidates.items[0];
    assert_eq!(first.display.as_deref(), Some("①脚本显示"));
    assert_ne!(first.text, "①脚本显示", "上屏文本不该被改");
}

/// 脚本能看到 `cloudime.context`：应用里已经输入、不在候选窗口里的那段文本。
#[test]
fn a_script_can_read_the_surrounding_text() {
    let dir = script_dir("context");
    let log = dir.join("context.txt");
    write_script(
        &dir,
        "watch.lua",
        format!(
            "cloudime.on('key', function()\n\
                 local file = io.open([[{log}]], 'a')\n\
                 file:write(cloudime.context .. '\\n')\n\
                 file:close()\n\
             end)\n",
            log = log.display()
        ),
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    // 先起一段组句（前文只在组句期间有效），再让 DLL 送前文，最后一键让脚本读它
    press(&mut router, letter('n'));
    router.handle(ClientMessage::Surrounding {
        session: SESSION,
        text: "已经输入的文字".to_owned(),
        document: None,
    });
    press(&mut router, letter('i'));

    let written = std::fs::read_to_string(&log).unwrap();
    assert!(written.contains("已经输入的文字"), "{written}");
}

/// 脚本能读输入框整篇 / 光标前那一段（`cloudime.text.*`）：DLL 送来的快照 + 按显示宽度上限切。
/// 第一次调用还没有快照时给 `nil`；只要还有脚本，`SyncMode` 的回复就一直请 DLL 带一份新的。
#[test]
fn a_script_can_read_the_document_text() {
    let dir = script_dir("document");
    let log = dir.join("document.txt");
    write_script(
        &dir,
        "watch.lua",
        format!(
            "cloudime.on('key', function()\n\
                 local file = io.open([[{log}]], 'a')\n\
                 local all = cloudime.text.all()\n\
                 local before = cloudime.text.before(4)\n\
                 file:write('all=' .. (all and (all.text .. '/' .. tostring(all.truncated)) or 'nil') .. '\\n')\n\
                 file:write('before=' .. (before and before.text or 'nil') .. '\\n')\n\
                 file:close()\n\
             end)\n",
            log = log.display()
        ),
    )
    .unwrap();

    // `SyncMode` 的回复里要不要请 DLL 带整篇：没脚本永远不问（读整篇要花时间）；
    // 有脚本就一直请 —— 每段组句起始读一份新的，快照才跟得上文档。
    let sync = |router: &mut Router| match router.handle(ClientMessage::SyncMode {
        session: SESSION,
        in_text_input: true,
        caps: false,
    }) {
        Some(ServerMessage::ModeSync { want_document, .. }) => want_document,
        other => panic!("expected mode sync, got {other:?}"),
    };
    let mut plain = router();
    assert!(!sync(&mut plain), "没脚本不该读整篇");

    let mut router = router_with(config_with_scripts(dir));
    assert!(sync(&mut router), "有脚本：该请 DLL 带上");

    // 第一次按键：还没有快照 → nil
    press(&mut router, letter('n'));
    // DLL 送来快照（光标在 `ab` 之后）
    router.handle(ClientMessage::Surrounding {
        session: SESSION,
        text: "ab".to_owned(),
        document: Some(DocumentText {
            text: "ab中文cd".to_owned(),
            caret: 2,
        }),
    });
    assert!(sync(&mut router), "有脚本就一直请，好让快照跟着文档走");
    // 没请 DLL 读的那段组句送 `None`：不能把上面那份快照抹掉
    router.handle(ClientMessage::Surrounding {
        session: SESSION,
        text: "ab".to_owned(),
        document: None,
    });
    press(&mut router, letter('i'));

    let written = std::fs::read_to_string(&log).unwrap();
    assert!(written.contains("all=nil"), "第一次该是 nil：{written}");
    assert!(
        written.contains("all=ab中文cd/false"),
        "整篇该原样给出来（`None` 不抹快照）：{written}"
    );
    assert!(
        written.contains("before=ab"),
        "光标前只有 `ab`（上限 4 够用）：{written}"
    );
}

/// 一次只回一次的最小本机 HTTP 服务（回环，不联网），返回它的地址。
fn one_shot_server(body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0u8; 512];
        let _ = std::io::Read::read(&mut stream, &mut request);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
    });
    format!("http://{address}/")
}

/// `adjust`：脚本给「加权 / 降权」，排序仍由 Core 做 —— 权重那一层重排，结构键不动。
#[test]
fn a_script_can_boost_and_demote_words() {
    // 没有脚本时：开放（5000）在 开饭（800）前面
    let plain = {
        let mut router = router();
        let (_outcome, _commit, frame) = type_letters(&mut router, "kaifa");
        candidate_texts_of(&frame)
    };
    assert!(
        plain.iter().position(|text| text == "开放") < plain.iter().position(|text| text == "开饭"),
        "{plain:?}"
    );

    let dir = script_dir("adjust");
    write_script(
        &dir,
        "boost.lua",
        "cloudime.on('key', function()\n\
             return { adjust = { ['开饭'] = 1000.0 } }\n\
         end)\n",
    )
    .unwrap();
    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = type_letters(&mut router, "kaifa");
    let boosted = candidate_texts_of(&frame);
    assert!(
        boosted.iter().position(|text| text == "开饭")
            < boosted.iter().position(|text| text == "开放"),
        "{boosted:?}"
    );
    // 结构键仍在 Core：完整覆盖的 开发 还在最前
    assert_eq!(boosted[0], "开发");
}

/// HTTP 是**异步**的：结果在以后某一拍 `tick` 才交给回调，回调要改的（这里用加权当观测点）落进 Core。
#[test]
fn a_script_http_result_lands_on_a_later_tick() {
    let dir = script_dir("http");
    let address = one_shot_server("ok");
    write_script(
        &dir,
        "translate.lua",
        format!(
            "cloudime.on('startup', function()\n\
                 cloudime.http_get([[{address}]], 5000, function(result)\n\
                     if result.status == 200 then\n\
                         return {{ adjust = {{ ['开饭'] = 2.0 }} }}\n\
                     end\n\
                 end)\n\
             end)\n"
        ),
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    // 请求是在加载脚本（派发 startup）那一刻发出去的，此刻还没有结果
    assert_eq!(router.engine_mut().word_adjustment_count(), 0);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        // 工人循环与 DLL 的 Poll 都会调它 —— 测试里手动推
        router.tick();
        if router.engine_mut().word_adjustment_count() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(router.engine_mut().word_adjustment_count(), 1);
}

/// 载荷里带着「这是哪个应用」：`key` 与 `candidates` 都读得到，脚本据此按应用分支
///（比如「在游戏里放行按键、在编辑器里照旧」）。
#[test]
fn the_payloads_carry_the_app_name() {
    let dir = script_dir("app");
    let log = dir.join("app.txt");
    write_script(
        &dir,
        "watch.lua",
        format!(
            "local path = [[{log}]]\n\
             local function note(text)\n\
                 local file = io.open(path, 'a')\n\
                 file:write(text .. '\\n')\n\
                 file:close()\n\
             end\n\
             cloudime.on('key', function(event) note('key:' .. tostring(event.app)) end)\n\
             cloudime.on('candidates', function(list)\n\
                 if list[1] then note('candidates:' .. tostring(list.app)) end\n\
             end)\n",
            log = log.display()
        ),
    )
    .unwrap();

    // 这个会话的宿主应用叫 notepad.exe
    let mut router = router_in(config_with_scripts(dir), Some("notepad.exe".to_owned()));
    press(&mut router, letter('n'));

    let written = std::fs::read_to_string(&log).unwrap();
    assert!(written.contains("key:notepad.exe"), "{written}");
    assert!(written.contains("candidates:notepad.exe"), "{written}");
}

/// `candidates` 载荷里带着 Core 的排序数字：`pinyin` 与 `weight`，脚本据此决定加权 / 降权。
#[test]
fn the_candidates_payload_carries_pinyin_and_weight() {
    let dir = script_dir("numbers");
    write_script(&dir, "read.lua", "local seen = false\n\
         cloudime.on('candidates', function(list)\n\
             if seen or not list[1] then return end\n\
             seen = true\n\
             return { notice = string.format('%s|%s|%s', list[1].text, list[1].pinyin, tostring(list[1].weight)) }\n\
         end)\n",
    )
    .unwrap();

    let mut router = router_with(config_with_scripts(dir));
    let (_outcome, _commit, frame) = press(&mut router, letter('k'));
    let notice = frame.notice.expect("脚本该把 Core 的数字写成提示");
    let parts: Vec<&str> = notice.split('|').collect();
    assert_eq!(parts.len(), 3, "{notice}");
    assert!(!parts[0].is_empty() && !parts[1].is_empty(), "{notice}");
    let weight: f64 = parts[2].parse().unwrap_or_default();
    assert!(weight > 0.0, "{notice}");
}
