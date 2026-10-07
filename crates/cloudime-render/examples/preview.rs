//! 离线预览：`cargo run --release -p cloudime-render --example preview -- --out target/render-preview`
//! 把样例帧按竖 / 横排画成 PNG，与系统候选窗截图并排比；`--measure` 只量几段文字的宽度与原生对数；
//! 末尾列出验收行每个字形落到了哪家字体。不是日常工具，改渲染器时拿来核对。

use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use cloudime_render::{
    FontLibrary, Frame, Layout, Preedit, PreeditSegment, PreeditStyle, Renderer, Row, Shadow,
    StatusCell, Theme, Tone,
};

#[derive(Parser)]
struct Args {
    /// PNG 输出目录。
    #[arg(long, default_value = "target/render-preview")]
    out: PathBuf,

    /// 点 → 像素倍数（Retina 为 2）。
    #[arg(long, default_value_t = 2.0)]
    scale: f32,

    /// 中日字形回退用的 locale。
    #[arg(long, default_value = "zh-CN")]
    locale: String,

    /// 不画阴影（对照壳自己带系统阴影的截图时用）。
    #[arg(long)]
    no_shadow: bool,

    /// 只量几段文字的宽度（点），不出图；与调研期 macOS 上量的行高对数。
    #[arg(long)]
    measure: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cloudime_render=debug".into()),
        )
        .init();
    let args = Args::parse();
    std::fs::create_dir_all(&args.out)?;

    let started = Instant::now();
    let library = FontLibrary::system(&args.locale)?;
    println!(
        "字体库：{:?}，界面字体 {}",
        started.elapsed(),
        library.ui_family()
    );
    println!("已加载字族：{}", library.families().join(" / "));
    let mut renderer = Renderer::new(library);
    if args.measure {
        for text in [
            "int. hello · int. hi",
            "hello",
            "ni'hao",
            "1/6",
            "你好",
            "phr. you change",
            "int. ",
            "·",
            " · ",
            "hi",
            "你好像",
            "開発する",
        ] {
            let widths: Vec<String> = [11.0, 12.0, 16.0]
                .into_iter()
                .map(|size| format!("{size}pt={:.2}", renderer.measure_points(text, size)))
                .collect();
            println!("{text:<24} {}", widths.join("  "));
        }
        return Ok(());
    }
    let shadow = (!args.no_shadow).then_some(Shadow::panel());

    let scenes: [(&str, Frame, Layout); 7] = [
        ("matrix-horizontal", matrix(), Layout::Horizontal),
        ("matrix-vertical", matrix(), Layout::Vertical),
        ("nihao-vertical", nihao(), Layout::Vertical),
        ("nihao-horizontal", nihao(), Layout::Horizontal),
        ("corrected-vertical", corrected_japanese(), Layout::Vertical),
        (
            "corrected-horizontal",
            corrected_japanese(),
            Layout::Horizontal,
        ),
        ("probe", probe(), Layout::Vertical),
    ];
    let theme = Theme::new();
    for (scene, frame, layout) in &scenes {
        let started = Instant::now();
        let rendered = renderer.render(frame, *layout, &theme, args.scale, shadow.as_ref())?;
        let elapsed = started.elapsed();
        let path = args.out.join(format!("{scene}.png"));
        rendered.pixmap.save_png(&path)?;
        let (w, h) = rendered.content_size_points();
        println!(
            "{:<28} {:>4.0}×{:<4.0}pt  {:>8.2?}  {}",
            scene,
            w,
            h,
            elapsed,
            path.display()
        );
    }

    // Windows 的悬浮状态条：按 Server 那份排布表画一排图标按钮，顺序与显隐跟真实状态条一致；
    // 每个按钮取它写在 cfg 里的第一张图标（也就是中文 / 半角 / 中文标点 / 简体那一份）
    let names = arrangement();
    let mut cells = Vec::new();
    for name in &names {
        cells.push(StatusCell::icon(icon(name)?));
    }
    let status = renderer.render_status(&cells, &theme, args.scale, shadow.as_ref())?;
    let path = args.out.join("status.png");
    status.rendered.pixmap.save_png(&path)?;
    let (w, h) = status.rendered.content_size_points();
    println!(
        "{:<28} {:>4.0}×{:<4.0}pt  {} 个按钮 {:?}  格边界 {:?}  {}",
        "status",
        w,
        h,
        names.len(),
        names,
        status.cell_edges,
        path.display()
    );

    for probe in [
        "云朵输入法 hello 🙂 日本語 骨直曜",
        "開発(かいはつ)する",
        "int. hello · int. hi",
    ] {
        println!(
            "「{probe}」各字形字体：{}",
            renderer.trace_families(probe, &Theme::new()).join(" → ")
        );
    }
    Ok(())
}

/// 悬浮状态条的图标源：直接读 Server 那边的 `icons\`（同一仓库，不另存一份）。
fn icon(name: &str) -> std::io::Result<String> {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/windows/server/src/ui/status/icons");
    std::fs::read_to_string(dir.join(format!("{name}.svg")))
}

/// 状态条上有哪些按钮、什么顺序：读 Server 那份 `icons-arrangement.cfg`，一个 `pos` 算一个按钮
/// （取它写在最前面的那个状态），`pos=-1` 的不显示。
fn arrangement() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/windows/server/src/ui/status");
    let text = std::fs::read_to_string(dir.join("icons-arrangement.cfg")).unwrap_or_default();
    let mut buttons: Vec<(i32, String)> = Vec::new();
    for line in text.lines() {
        let (mut name, mut pos) = (None, None);
        for field in line.split(';') {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key.trim() {
                "button" => name = Some(value.trim().to_owned()),
                "pos" => pos = value.trim().parse::<i32>().ok(),
                _ => {}
            }
        }
        let (Some(name), Some(pos)) = (name, pos) else {
            continue;
        };
        if pos < 0 || buttons.iter().any(|(other, _)| *other == pos) {
            continue;
        }
        buttons.push((pos, name));
    }
    buttons.sort_by_key(|(pos, _)| *pos);
    buttons.into_iter().map(|(_, name)| name).collect()
}

// 样例帧：与真机上敲同样拼音看到的候选窗对照，所以内容要和引擎当时给的一致（人工从截图抄）。

/// 真机敲「nihao」看到的第一页（2026-09-13 从截图抄），拼音行带光标、注解、页码。
/// 横排展开成矩阵：6 行 × 9 列，第二行高亮着一条被截断的长候选，末行不满，有一个云端词。
fn matrix() -> Frame {
    let words = [
        "是",
        "时",
        "事",
        "市",
        "使",
        "世",
        "式",
        "十",
        "实",
        "时候",
        "事情",
        "世界",
        "是不是因为我们今天没有去",
        "实际",
        "市场",
        "使用",
        "十分",
        "试试",
        "视频",
        "室内",
        "食物",
        "失败",
        "始终",
        "适合",
        "诗人",
        "石头",
        "时代",
        "示范",
        "士兵",
        "事实上",
        "实验室",
        "视角",
        "世纪",
        "试卷",
        "释放",
        "拾起",
        "师傅",
        "诗歌",
        "时尚",
        "失去",
        "湿度",
        "十月",
        "石油",
        "史诗",
        "驶向",
        "誓言",
        "逝去",
        "柿子",
        "嗜好",
    ];
    let mut rows: Vec<Row> = words
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut row = Row::plain(i % 9, *text);
            // 序号只标在高亮所在的第二行
            if i / 9 != 1 {
                row.index.clear();
            }
            row
        })
        .collect();
    rows[12].annotation = vec![
        ("phr. ".into(), Tone::Faint),
        ("is it because we didn't go today".into(), Tone::Gloss),
    ];
    // 给一格挂个来源角标：预览里能同时看到横排的「角标与候选词最小间隔 2 个字宽」与
    // 「每格最小宽度 6 个字宽 + 角标」。
    rows[0].badge = Some("造".into());
    Frame {
        preedit: Some(Preedit {
            segments: vec![PreeditSegment {
                text: "shi".into(),
                style: PreeditStyle::Typed,
            }],
            cursor: 3,
        }),
        rows,
        highlighted: Some(12),
        highlight_animation: None,
        columns: 9,
        min_cell_width: 0.0,
        tip: None,
        footer: Some("2/12".into()),
        status: None,
    }
}

fn nihao() -> Frame {
    let mut frame = Frame {
        preedit: Some(Preedit::plain("ni'hao", 6)),
        rows: vec![
            annotated(
                0,
                "你好",
                &[
                    ("int. ", Tone::Faint),
                    ("hello", Tone::Gloss),
                    (" · ", Tone::Faint),
                    ("int. ", Tone::Faint),
                    ("hi", Tone::Gloss),
                ],
                false,
            ),
            annotated(1, "👋", &[("你好", Tone::Gloss)], false),
            annotated(2, "你好好", &[], false),
            annotated(
                3,
                "你好像",
                &[("phr. ", Tone::Faint), ("you seem", Tone::Fresh)],
                false,
            ),
            annotated(4, "你好久", &[], false),
            annotated(5, "你好看", &[], false),
            annotated(6, "你哈", &[], false),
            annotated(
                7,
                "你换",
                &[
                    ("phr. ", Tone::Faint),
                    ("you change", Tone::Fresh),
                    (" · ", Tone::Faint),
                    ("phr. ", Tone::Faint),
                    ("you swap", Tone::Fresh),
                ],
                false,
            ),
            annotated(
                8,
                "你会",
                &[("phr. ", Tone::Faint), ("you will", Tone::Fresh)],
                false,
            ),
        ],
        highlighted: Some(0),
        highlight_animation: None,
        columns: 0,
        min_cell_width: 0.0,
        tip: None,
        footer: Some("1/6".to_owned()),
        status: None,
    };
    // 第一项挂个来源角标：横排预览里能看出「角标与候选词的间隔 = 2 个字宽」
    // （竖排仍按原来的 4pt，角标在自己的列里）。
    frame.rows[0].badge = Some("短".to_owned());
    frame
}

/// 纠错后的拼音行（删除线 + 淡色剩余）加日文注解（汉字注假名）。
fn corrected_japanese() -> Frame {
    Frame {
        preedit: Some(Preedit {
            segments: vec![
                PreeditSegment {
                    text: "kai".to_owned(),
                    style: PreeditStyle::Typed,
                },
                PreeditSegment {
                    text: "fs".to_owned(),
                    style: PreeditStyle::Struck,
                },
                PreeditSegment {
                    text: "'fa".to_owned(),
                    style: PreeditStyle::Rest,
                },
            ],
            cursor: 5,
        }),
        rows: vec![
            annotated(
                0,
                "开发",
                &[
                    ("v. ", Tone::Faint),
                    ("開発", Tone::Gloss),
                    ("(かいはつ)", Tone::Faint),
                    ("する", Tone::Gloss),
                ],
                false,
            ),
            annotated(
                1,
                "开",
                &[
                    ("v. ", Tone::Faint),
                    ("開", Tone::Fresh),
                    ("(ひら)", Tone::Faint),
                    ("く", Tone::Fresh),
                ],
                false,
            ),
        ],
        highlighted: Some(1),
        highlight_animation: None,
        columns: 0,
        min_cell_width: 0.0,
        tip: None,
        footer: None,
        status: Some("已删除「开放」".to_owned()),
    }
}

/// 四条验收用的一行：汉字（zh 字形）、英文、彩色 emoji、日文假名与汉字。
fn probe() -> Frame {
    Frame {
        preedit: None,
        rows: vec![Row::plain(0, "云朵输入法 hello 🙂 日本語 骨直曜")],
        highlighted: None,
        highlight_animation: None,
        columns: 0,
        min_cell_width: 0.0,
        tip: None,
        footer: None,
        status: None,
    }
}

fn annotated(index: usize, text: &str, annotation: &[(&str, Tone)], _cloud: bool) -> Row {
    Row {
        index: (index + 1).to_string(),
        text: text.to_owned(),
        annotation: annotation
            .iter()
            .map(|(s, tone)| ((*s).to_owned(), *tone))
            .collect(),
        badge: None,
    }
}
