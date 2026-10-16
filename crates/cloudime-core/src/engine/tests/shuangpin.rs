//! 双键拼音：按键先按方案还原成完整拼音，再走原来的查询。

use super::*;
use crate::shuangpin;

/// 一份只够测试用的自然码片段：`l` = ai、`a` = a、`t` = ve/ue、`x` = ie；
/// 其余键靠单字母声母（`k`、`f`、`x` 这些）自己当声母。
fn table() -> shuangpin::Table {
    let mut remapping = std::collections::BTreeMap::new();
    let mut put = |key: char, finals: &[&str]| {
        remapping.insert(
            key,
            shuangpin::KeyMapping {
                initials: Vec::new(),
                finals: finals.iter().map(|text| (*text).to_owned()).collect(),
            },
        );
    };
    put('l', &["ai"]);
    put('a', &["a"]);
    put('t', &["ve", "ue"]);
    put('x', &["ie"]);
    shuangpin::Table {
        zero_initials: [('a', 'a'), ('o', 'o'), ('e', 'e')]
            .into_iter()
            .map(|(key, final_)| ((key, key), final_.to_string()))
            .collect(),
        remapping,
        exchange: Vec::new(),
        auxiliary: false,
    }
}

fn candidates(engine: &mut Engine, keys: &str) -> Vec<String> {
    engine.set_input(keys);
    let query = engine.query().expect("能查出候选");
    query
        .candidates
        .items
        .iter()
        .map(|candidate| candidate.text.clone())
        .collect()
}

/// 两键一个音节：`klfa` → `kai'fa` → 开发。
#[test]
fn shuangpin_keys_are_expanded_before_lookup() {
    let mut engine = engine();
    engine.set_shuangpin(Some(table()));
    let texts = candidates(&mut engine, "klfa");
    assert!(texts.iter().any(|text| text == "开发"), "{texts:?}");
}

/// 同键多义两种解法都会试：`xt` 先解出非法的 `xve`（查不出东西），再解出 `xue` → 学。
#[test]
fn illegal_expansions_are_skipped() {
    let dictionary = Dictionary::parse("学\txue\t9000\n略\tlve\t9000\n").unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_shuangpin(Some(table()));
    let texts = candidates(&mut engine, "xt");
    assert!(texts.iter().any(|text| text == "学"), "{texts:?}");
    // `lt` 的两种解法（lve / lue）里 lve 是合法音节，直接命中
    let texts = candidates(&mut engine, "lt");
    assert!(texts.iter().any(|text| text == "略"), "{texts:?}");
}

/// 一张只够测试用的辅助码表。
struct Codes(std::collections::BTreeMap<String, Vec<String>>);

impl Codes {
    fn new(entries: &[(&str, &str)]) -> Self {
        let mut map: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for (word, code) in entries {
            map.entry((*word).to_owned())
                .or_default()
                .push(code.to_uppercase());
        }
        Self(map)
    }
}

impl crate::shuangpin::AuxiliaryTable for Codes {
    fn codes(&self, word: &str) -> Option<&[String]> {
        self.0.get(word).map(Vec::as_slice)
    }
}

/// 辅助码只筛选：对不上就丢掉，对得上照旧出；没挂表时完全不筛。
#[test]
fn auxiliary_codes_filter_candidates() {
    let mut table = table();
    table.auxiliary = true;

    // 开发 = 开(K) + 发(F)；`klfaF` 把第二个字的辅助码敲成 F → 命中
    let mut hit = engine();
    hit.set_shuangpin(Some(table.clone()));
    hit.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开发", "KF")]))));
    let texts = candidates(&mut hit, "klfaF");
    assert!(texts.iter().any(|text| text == "开发"), "{texts:?}");

    // 敲错（X）→ 丢掉
    let mut wrong = engine();
    wrong.set_shuangpin(Some(table.clone()));
    wrong.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开发", "KF")]))));
    let texts = candidates(&mut wrong, "klfaX");
    assert!(!texts.iter().any(|text| text == "开发"), "{texts:?}");

    // 没敲辅助码 → 照旧出
    let mut plain = engine();
    plain.set_shuangpin(Some(table));
    plain.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开发", "KF")]))));
    let texts = candidates(&mut plain, "klfa");
    assert!(texts.iter().any(|text| text == "开发"), "{texts:?}");
}

/// 候选右侧要显示的辅助码（「在候选项显示辅助码」那个开关用的）：**光标前那个音节**对应的那个字
/// —— 打 `kl|`（一个音节）看「开」的码，打 `klfa|`（两个音节）看「发」的码；
/// 候选里对不上那个位置的字（单字的「开」配两个音节）就不显示。
#[test]
fn the_auxiliary_shown_next_to_a_candidate_follows_the_cursor() {
    // 两个用例各来一份（helper 名 `engine` / `table` 一会儿会被局部变量遮住，先都取出来）
    let (mut app, mut word_codes) = (engine(), engine());
    let (mut aux, mut word_aux) = (table(), table());
    aux.auxiliary = true;
    word_aux.auxiliary = true;
    app.set_shuangpin(Some(aux));
    app.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开", "K"), ("发", "F")]))));
    word_codes.set_shuangpin(Some(word_aux));
    word_codes.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开发", "KF")]))));

    app.set_input("kl");
    let query = app.query().expect("一个音节");
    let kai = query
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开")
        .expect("开");
    assert_eq!(
        kai.auxiliary.as_deref(),
        Some("K"),
        "光标前那个音节是「开」"
    );

    app.set_input("klfa");
    let query = app.query().expect("两个音节");
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开发")
        .expect("开发");
    assert_eq!(
        kaifa.auxiliary.as_deref(),
        Some("F"),
        "光标前那个音节是「发」"
    );
    let kai = query
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开")
        .expect("开");
    assert_eq!(kai.auxiliary, None, "单字对不上第二个音节的位置");

    // 表里只有整词码（一格一位）时按那一位切
    word_codes.set_input("klfa");
    let query = word_codes.query().expect("两个音节");
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开发")
        .expect("开发");
    assert_eq!(kaifa.auxiliary.as_deref(), Some("F"), "整词码切开取那一位");

    // 只按了半个音节：尾巴那一位也算「光标前那个音节」
    app.set_input("klf");
    let query = app.query().expect("半个音节");
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开发")
        .expect("开发");
    assert_eq!(kaifa.auxiliary.as_deref(), Some("F"));
}

/// 辅助码**敲一半也算**：`kl` + `R`（「开」的码 `RS` 的第一位）时，「开」照样在候选里，
/// 而且那一位记成「已经敲上」—— 候选窗口把它画下划线（见 `docs/design/input-schemes.md` §4）。
#[test]
fn a_partly_typed_auxiliary_code_keeps_the_candidate() {
    let (mut app, mut aux) = (engine(), table());
    aux.auxiliary = true;
    app.set_shuangpin(Some(aux));
    app.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开", "RS")]))));

    let find = |engine: &mut Engine| {
        engine
            .query()
            .expect("查询本身不报错")
            .candidates
            .items
            .iter()
            .find(|candidate| candidate.text == "开")
            .cloned()
    };

    // 敲了一位：候选还在，完整码照旧 `RS`，已经敲上的是 `R`
    app.set_input("klR");
    let kai = find(&mut app).expect("敲了一半的辅助码也算命中");
    assert_eq!(kai.auxiliary.as_deref(), Some("RS"));
    assert_eq!(kai.auxiliary_typed.as_deref(), Some("R"), "已经敲上 R");

    // 敲满两位：两位都算敲上
    app.set_input("klRS");
    let kai = find(&mut app).expect("敲满");
    assert_eq!(kai.auxiliary_typed.as_deref(), Some("RS"));

    // 敲错一位：这个候选对不上，被筛掉
    app.set_input("klX");
    assert!(find(&mut app).is_none(), "敲错的码不认");
}

/// **半音节后面直接跟辅助码**：`k` 是「开」的第一键，第二键跳过、直接敲辅助码 `H` ——
/// 「开」照样在候选里（敲一半的码也算命中），右侧那段码记成已敲一位。
#[test]
fn an_auxiliary_code_can_follow_a_half_syllable() {
    let (mut app, mut aux) = (engine(), table());
    aux.auxiliary = true;
    app.set_shuangpin(Some(aux));
    app.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开", "HB")]))));

    let find = |engine: &mut Engine| {
        engine.query().ok().and_then(|query| {
            query
                .candidates
                .items
                .iter()
                .find(|candidate| candidate.text == "开")
                .cloned()
        })
    };

    // `kH`：只按了 `k` 就敲辅助码 `H`（第二键跳过）
    app.set_input("kH");
    let kai = find(&mut app).expect("半音节 + 辅助码该出「开」");
    assert_eq!(kai.auxiliary.as_deref(), Some("HB"));
    assert_eq!(kai.auxiliary_typed.as_deref(), Some("H"));

    // 敲错：筛掉
    app.set_input("kX");
    assert!(find(&mut app).is_none(), "敲错的码不认");
}

/// 开了辅助码：**小写是拼音键、大写是辅助码** —— 大写只能跟在完整音节后面（`klfaF`）；
/// **第一个键**敲大写（Caps Lock / Shift 卡着）就把它压成小写，照旧当正常双拼打（后面的辅助码照旧）；
/// 第二个键起敲大写还是切不出来。
#[test]
fn uppercase_is_an_auxiliary_code_not_a_pinyin_key() {
    let mut table = table();
    table.auxiliary = true;
    let mut engine = engine();
    engine.set_shuangpin(Some(table));
    engine.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开发", "KF")]))));

    // 第一个键大写：压成小写，照旧出中文候选，拼音行也显示小写
    engine.set_input("Klfa");
    let query = engine
        .query()
        .expect("大写开头压成小写，不该报「切不出来」");
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|candidate| candidate.text == "开发"),
        "{:?}",
        query.candidates.items
    );
    let display = query.typed_display.clone().unwrap_or_default();
    assert!(
        !display.chars().any(|c| c.is_ascii_uppercase()),
        "候选窗里显示小写：{display:?}"
    );
    // 回车原样上屏也跟着用这一份（与拼音行一致）
    engine.set_input("Klfa");
    assert_eq!(engine.take_raw(), "klfa");

    // **只动第一个键**：后面紧跟的辅助码照样认（`KlfaF` = 开发 + 码 F）
    let texts = candidates(&mut engine, "KlfaF");
    assert!(
        texts.iter().any(|text| text == "开发"),
        "第一个键压成小写、辅助码照旧：{texts:?}"
    );

    // 第二个键起敲大写：音节位上出现大写 → 切不出来
    engine.set_input("kLfa");
    assert!(engine.query().is_err(), "音节位上有大写 → 不当拼音");

    let texts = candidates(&mut engine, "klfa");
    assert!(
        texts.iter().any(|text| text == "开发"),
        "全小写照旧：{texts:?}"
    );

    let texts = candidates(&mut engine, "klfaF");
    assert!(texts.iter().any(|text| text == "开发"), "{texts:?}");
}

/// **手动分词时能用简拼**：`k'f`（「开」「发」的声母各一个键）也匹配「开发」 ——
/// 用户用 `'` 分段以后，一段一个键就按那个键的声母算。
#[test]
fn a_manual_boundary_allows_jian_pin() {
    let mut app = engine();
    app.set_shuangpin(Some(table()));
    let texts = candidates(&mut app, "k'f");
    assert!(texts.iter().any(|text| text == "开发"), "{texts:?}");

    // 不分词时还是「两键一个音节」：`kf` 里 `f` 不是韵母键，整段解不出来
    app.set_input("kf");
    assert!(app.query().is_err(), "不分词就不按简拼读");
}

/// 选「整句」候选时按键要**整段吃掉**：它的音节数可能比按下的键多 —— `buesy`（3 个音节、5 个键）
/// 会出 4 个音节的「不受影响」（少敲变体读出来的）。这时候 `typed_covered` 算不出按键数（返回 0），
/// 必须整段吃掉，否则选完拼音串原样留着（用户报的 `不受影响buesy`）。
#[test]
fn committing_a_sentence_with_more_syllables_than_keys_eats_the_whole_scope() {
    let mut app = engine();
    app.set_shuangpin(Some(table()));
    app.set_input("klfa"); // 两个音节（开发），只有 4 个键
    // 手搓一条 4 个音节的整句候选（真实数据里由「少敲」变体产出，这里按它的形状构造）
    let sentence = Candidate {
        text: "开开发发".to_owned(),
        display: None,
        kind: CandidateKind::Sentence,
        syllables: vec!["kai".into(), "kai".into(), "fa".into(), "fa".into()],
        reading: None,
        auxiliary: None,
        auxiliary_typed: None,
    };
    assert_eq!(app.commit(&sentence).as_deref(), Some("开开发发"));
    assert!(
        app.composition().is_empty(),
        "整段吃掉了，拼音串里不该留着原来的按键"
    );
}

/// 按键只按方案解释：解不出来就是切不出来，**不按全拼再读一遍**。
///
/// `ceshi` 在这张表里（`c-e` 的 `e` 不是韵母、`s-h` 的 `h` 也不是）解不出任何音节；
/// 从前会拿按键原样走一趟全拼，于是匹配到「测试」—— 那不是这套方案的读法。
#[test]
fn keys_that_do_not_decode_are_not_read_as_full_pinyin() {
    let dictionary = Dictionary::parse("测试\tce shi\t9000\n").unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_shuangpin(Some(table()));
    engine.set_input("ceshi");
    assert!(engine.query().is_err(), "按键解不出音节，就该切不出来");
}

/// 解法解出的音节序列**就是切分**：不再交给全键那套切分器自由切一遍 ——
/// 简拼会把 `kai` 再切成 `k` + `ai`，于是「可爱」（`ke ai`）也能凑进 `klfa` 的候选里。
#[test]
fn the_expansion_is_not_re_segmented_with_abbreviations() {
    let dictionary = Dictionary::parse("开发\tkai fa\t9000\n可爱\tke ai\t9000\n").unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_shuangpin(Some(table()));
    let texts = candidates(&mut engine, "klfa");
    assert!(texts.iter().any(|text| text == "开发"), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text == "可爱"),
        "`klfa` 只解出 `kai'fa`，简拼凑出来的「可爱」不该出现：{texts:?}"
    );
}

/// 尾巴上那半个音节有几种可能时按**并集**出候选，但**不收成公共前缀**：
/// 只写着零声母的键（`k` 上写了两组零声母）只出那两组，同前缀的第三组音节的词不进来。
#[test]
fn a_trailing_key_only_matches_the_syllables_it_can_reach() {
    let mut table = table();
    table.zero_initials.insert(('k', 'a'), "a".to_owned());
    table.zero_initials.insert(('k', 'l'), "ai".to_owned());
    let dictionary = Dictionary::parse("啊\ta\t9000\n爱\tai\t9000\n安\tan\t9000\n").unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_shuangpin(Some(table));
    let texts = candidates(&mut engine, "k");
    assert!(texts.iter().any(|text| text == "啊"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "爱"), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text == "安"),
        "`k` 只接得到 `a` / `ai`，`an` 不在里面：{texts:?}"
    );
}

/// 没挂表（全键拼音）时行为一点不变。
#[test]
fn without_a_table_nothing_changes() {
    let mut engine = engine();
    assert!(engine.shuangpin().is_none());
    engine.set_input("kaifa");
    let query = engine.query().unwrap();
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|candidate| candidate.text == "开发"),
        "全拼照旧"
    );
}

/// 拼音行：缺省（`show_raw_input = true`）显示敲的原始按键；关掉显示展开后的完整拼音。
#[test]
fn the_pinyin_row_follows_show_raw_input() {
    let mut raw = engine();
    raw.set_shuangpin(Some(table()));
    raw.set_input("klfa");
    let query = raw.query().unwrap();
    assert_eq!(
        query.marked_text(),
        "kl'fa",
        "缺省显示原始按键，并按音节补 `'`"
    );
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|candidate| candidate.text == "开发"),
        "候选照旧按展开后的拼音出"
    );

    // 开了辅助码：辅助码贴着它那个音节，分词照旧
    let mut aux = table();
    aux.auxiliary = true;
    let mut with_aux = engine();
    with_aux.set_shuangpin(Some(aux));
    with_aux.set_input("klfaF");
    assert_eq!(with_aux.query().unwrap().marked_text(), "kl'faF");

    let mut expanded = engine();
    expanded.set_shuangpin(Some(table()));
    expanded.set_show_raw_input(false);
    expanded.set_input("klfa");
    let query = expanded.query().unwrap();
    assert_eq!(query.marked_text(), "kai'fa");
    assert_eq!(
        query.segments_cursor(),
        6,
        "光标在展开后的串末尾（`kai'fa|`），不是按键数那个位置"
    );

    // 显示原始按键时光标照旧按按键算（`kl'fa|`）
    let mut raw_cursor = engine();
    raw_cursor.set_shuangpin(Some(table()));
    raw_cursor.set_input("klfa");
    assert_eq!(raw_cursor.query().unwrap().segments_cursor(), 5);

    // 首道那种映射（`e` = `sh`、`s` = `ou`）：`es` 展开成 `shou`，光标在它末尾
    let mut shoudao = table();
    shoudao.remapping.insert(
        'e',
        shuangpin::KeyMapping {
            initials: vec!["sh".to_owned()],
            finals: vec!["e".to_owned()],
        },
    );
    shoudao.remapping.insert(
        's',
        shuangpin::KeyMapping {
            initials: Vec::new(),
            finals: vec!["ou".to_owned()],
        },
    );
    let mut shoudao_engine = engine();
    shoudao_engine.set_shuangpin(Some(shoudao));
    shoudao_engine.set_show_raw_input(false);
    shoudao_engine.set_input("es");
    let query = shoudao_engine.query().unwrap();
    assert_eq!(query.marked_text(), "shou");
    assert_eq!(query.segments_cursor(), 4, "`shou|`，不是 `sh|ou`");
}

/// 选中候选要把对应的**按键**从缓冲区吃掉：`klfa` 选「开发」后拼音串里不该再留着 `klfa`。
#[test]
fn selecting_a_candidate_consumes_the_shuangpin_keys() {
    // 整个词：四键都吃掉
    let mut whole = engine();
    whole.set_shuangpin(Some(table()));
    whole.set_input("klfa");
    let word = whole
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开发")
        .unwrap()
        .clone();
    assert_eq!(whole.commit(&word).as_deref(), Some("开发"));
    assert!(
        whole.composition().is_empty(),
        "四键都被吃掉，组句该结束（不该剩下按键）"
    );

    // 只覆盖前一个音节：只吃掉前两键（`kl`），`fa` 留着
    let mut single_engine = engine();
    single_engine.set_shuangpin(Some(table()));
    single_engine.set_input("klfa");
    let single = single_engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开")
        .unwrap()
        .clone();
    assert_eq!(single_engine.commit(&single), None, "还剩 `fa`，组句没完");
    assert_eq!(single_engine.composition().selected_text(), "开");
    assert_eq!(single_engine.composition().text(), "fa");

    // 开了辅助码：辅助码那一位也算进去（`klfaF` 五键都吃掉）
    let mut aux = table();
    aux.auxiliary = true;
    let mut with_aux = engine();
    with_aux.set_shuangpin(Some(aux));
    with_aux.set_auxiliary_codes(Some(Box::new(Codes::new(&[("开发", "KF")]))));
    with_aux.set_input("klfaF");
    let word = with_aux
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "开发")
        .unwrap()
        .clone();
    with_aux.commit(&word);
    assert!(
        with_aux.composition().is_empty(),
        "辅助码那一位也被吃掉，组句该结束"
    );
}

/// 候选的**末音节只按了第一个键**时（`yzd` 出的「云朵」：`d` 是「朵」的声母，第二个键还没按），
/// 选中它要把这半个音节的键也吃掉 —— 从前算不出按键数就落到全拼那套对齐上，
/// 结果拼音串里残留半截按键（`云朵zd`）。
#[test]
fn committing_a_candidate_whose_last_syllable_is_half_typed() {
    let mut table = table();
    table.remapping.insert(
        'z',
        shuangpin::KeyMapping {
            initials: Vec::new(),
            finals: vec!["un".to_owned()],
        },
    );
    let mut engine = Engine::new(Dictionary::parse("云朵\tyun duo\t9000\n").unwrap());
    engine.set_shuangpin(Some(table));
    engine.set_input("yzd");
    let word = engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "云朵")
        .expect("`yzd` 解出 `yun` + 半个音节 `d…`，云朵是它的前缀候选")
        .clone();
    assert_eq!(
        engine.commit(&word).as_deref(),
        Some("云朵"),
        "三个键都该被吃掉、组句该结束"
    );
    assert!(
        engine.composition().is_empty(),
        "不该剩下按键：{:?}",
        engine.composition().text()
    );
}

/// 回车原样上屏：`show_raw_input` 开（缺省）时上屏原始按键，关时上屏展开后的完整拼音（`'` 不上屏）。
#[test]
fn raw_commit_follows_show_raw_input() {
    let mut raw = engine();
    raw.set_shuangpin(Some(table()));
    raw.set_input("klfa");
    assert_eq!(raw.take_raw(), "klfa");

    let mut expanded = engine();
    expanded.set_shuangpin(Some(table()));
    expanded.set_show_raw_input(false);
    expanded.set_input("klfa");
    assert_eq!(expanded.take_raw(), "kaifa");
}

/// `v` / `u` / `i` 三种面板（表达式、区位、符号）仅全键拼音可用：双拼下 `v` 只是一个普通方案键。
#[test]
fn expression_mode_is_off_under_shuangpin() {
    let shortcut_kind = |query: Option<crate::engine::Query>| {
        query.is_some_and(|query| {
            query
                .candidates
                .items
                .iter()
                .any(|candidate| candidate.kind == CandidateKind::Shortcut)
        })
    };

    let mut full = engine();
    full.set_input("v1+2");
    assert!(
        shortcut_kind(full.query().ok()),
        "全键拼音下 `v1+2` 走表达式，出快捷候选"
    );

    let mut dual = engine();
    dual.set_shuangpin(Some(table()));
    dual.set_input("v1+2");
    assert!(!shortcut_kind(dual.query().ok()), "双拼下不再走表达式面板");

    // `v` 只是一个普通方案键：不算表达式模式（候选窗左下角也不会冒出「输入数字或表达式」）
    let mut vhgo = engine();
    vhgo.set_shuangpin(Some(table()));
    vhgo.set_input("vhgo");
    assert!(!vhgo.expression_mode(), "双拼下 `v` 不是表达式前缀");
    assert!(full.expression_mode(), "全键拼音下 `v` 开头就是表达式模式");
}

/// 双拼下按**原始按键**匹配英文词，由 `use_input_scheme.cfg` 的「中英混合输入」说了算
/// （`[input] mixture_input` 那份只管全键拼音）；不看「显示原始输入按键」，也不做拼错纠正。
#[test]
fn english_words_are_matched_by_the_raw_keys_under_shuangpin() {
    let english = || crate::dictionary::WordList::parse("hello\thello\t6000\n").unwrap();
    let has_english = |query: Option<crate::engine::Query>| {
        query.is_some_and(|query| {
            query
                .candidates
                .items
                .iter()
                .any(|candidate| candidate.kind == CandidateKind::English)
        })
    };

    // 全键拼音：还是那份配置项说了算
    let mut full = engine().with_english(english());
    full.set_input("hello");
    assert!(has_english(full.query().ok()), "全键拼音照旧掺英文词");
    let mut full_off = engine().with_english(english());
    full_off.set_mixture_input(false);
    full_off.set_input("hello");
    assert!(!has_english(full_off.query().ok()), "关掉就不掺");

    // 双拼：关掉这份 `.cfg` 设置就一律不匹配（`[input] mixture_input` 管不着它）
    let mut off = engine().with_english(english());
    off.set_shuangpin(Some(table()));
    off.set_shuangpin_mixture_input(false);
    off.set_mixture_input(true);
    off.set_input("hello");
    assert!(
        !has_english(off.query().ok()),
        "双拼关掉中英混合输入：一个英文候选都不出"
    );

    // 开着：原始按键 `hello` 在方案里解不出音节，照样按英文词匹配
    let mut on = engine().with_english(english());
    on.set_shuangpin(Some(table()));
    on.set_shuangpin_mixture_input(true);
    on.set_mixture_input(false);
    on.set_show_raw_input(false);
    on.set_input("hello");
    assert!(has_english(on.query().ok()), "双拼下按原始按键掺英文词");

    // 开了辅助码：第一个键大写压成小写（正常双拼），英文照掺
    let mut aux_table = table();
    aux_table.auxiliary = true;
    let mut with_aux = engine().with_english(english());
    with_aux.set_shuangpin(Some(aux_table));
    with_aux.set_shuangpin_mixture_input(true);
    with_aux.set_input("Hello");
    assert!(
        has_english(with_aux.query().ok()),
        "第一个键大写压成小写，英文照掺"
    );
}
