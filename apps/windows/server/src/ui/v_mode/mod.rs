//! V 模式辅助窗口：拼音串只有一个 `v` 时贴在候选窗旁边，列出可用写法。
//!
//! 它和「键位映射参考」一样是候选窗口的一件资产（[`cloud_ui::auxiliary`]），
//! 用辅助窗口模板显示一块静态 `Document`；内容只描述能力，不参与按键。

use std::cell::Cell;
use std::rc::Rc;

use windows::Win32::Foundation::HWND;
use windows::core::Result;

use cloud_ui::auxiliary::{Asset, Bridge};
use cloud_ui::auxiliary_window_template::AuxiliaryWindowTemplate;
use cloud_ui::painter::SharedPainter;
use cloudime_render::{Document, Element, FontSlot};

const PADDING: f32 = 16.0;
const GAP: f32 = 8.0;
const MAX_TEXT_WIDTH: f32 = 520.0;

const TITLE: &str = "V模式";
const HELP: [&str; 7] = [
    "输入数学表达式进行计算，支持数学函数（例如“sin(30)+lg(100)”）。只输入 v 时按Tab进入/退出高级数学计算界面。",
    "输入数字进行转换（例如“12.12”）。",
    "输入=>加0b/0o/0x/十进制数进行进制转换（例如“=>0x33ccff”）。",
    "输入sj/rq/xq/xz获得时间/日期/星期/现在（例如“xz”）。",
    "输入@加省份简称加英文字母（忽略大小写）获得车牌和所在地（例如“@sua”可以匹配到“苏A”/“江苏省南京市”/“江苏南京”）。",
    "输入#加姓名拼音字母获得取名结果（仅限汉族名字，例如“#zhangwei”可以匹配到“张伟”）。选中姓氏后它会留在组句里，接着输入名字。",
    "",
];

const CALC_TITLE: &str = "高级数学计算界面";
const CALC_HELP: [&str; 7] = [
    "三角函数：sin/cos/tan/cot（角度），sinr/cosr/tanr/cotr（弧度）",
    "反三角：asin/acos/atan（返回角度）",
    "对数与根号：lg/ln/log(底数,真数)、sqrt",
    "双曲：sinh/cosh/tanh",
    "统计与组合：arrange/combine/avg/vari/sum",
    "运算：+ - * / % ^ !，@ 复数模（只能后缀）",
    "虚数单位：i（例如 3+4*i）",
];
const CALC_FOOTER: &str = "按Tab/Esc退出高级数学计算";

#[derive(Clone, Copy, PartialEq, Eq)]
enum HelpMode {
    VMode,
    Calculator,
}

/// V 模式说明窗口：显示 / 收起由候选窗口的 `shown` 共享格子控制。
pub(crate) struct VModeHelpWindow {
    template: Rc<AuxiliaryWindowTemplate>,
    painter: SharedPainter,
    v_mode: Rc<Cell<bool>>,
    calculator: Rc<Cell<bool>>,
    built: Cell<Option<(u64, HelpMode)>>,
}

impl VModeHelpWindow {
    pub(crate) fn new(
        painter: SharedPainter,
        owner: HWND,
        v_mode: Rc<Cell<bool>>,
        calculator: Rc<Cell<bool>>,
    ) -> Result<Self> {
        let template = Rc::new(AuxiliaryWindowTemplate::new(
            painter.clone(),
            false,
            Some(owner),
        )?);
        template.attach();
        template.set_top_most(true);
        Ok(Self {
            template,
            painter,
            v_mode,
            calculator,
            built: Cell::new(None),
        })
    }
}

impl Asset for VModeHelpWindow {
    fn sync(&self, owner: &dyn Bridge) {
        let host = owner.host();
        let mode = if self.calculator.get() {
            Some(HelpMode::Calculator)
        } else if self.v_mode.get() {
            Some(HelpMode::VMode)
        } else {
            None
        };
        let Some(mode) = mode else {
            self.hide();
            return;
        };
        if host.expanded {
            self.hide();
            return;
        }
        let Some(rect) = host.rect else {
            self.hide();
            return;
        };
        let built = {
            let mut guard = self.painter.borrow_mut();
            let Some(painter) = guard.as_mut() else {
                self.hide();
                return;
            };
            let generation = painter.generation();
            (self.built.get() != Some((generation, mode)))
                .then(|| (generation, build_document(painter, mode)))
        };
        if let Some((generation, document)) = built {
            self.template.set_document(document);
            self.built.set(Some((generation, mode)));
        }
        self.template.show(rect, host.layout);
    }

    fn hide(&self) {
        self.template.hide();
    }

    fn name(&self) -> &'static str {
        "v-mode-help"
    }
}

/// 标签用候选项颜色，正文用额外内容颜色，退出提示用角标颜色；正文按固定宽度折行。
fn build_document(painter: &mut cloud_ui::painter::Painter, mode: HelpMode) -> Document {
    let palette = painter.palette();
    let (title, paragraphs, footer) = match mode {
        HelpMode::VMode => (TITLE, &HELP[..], None),
        HelpMode::Calculator => (CALC_TITLE, &CALC_HELP[..], Some(CALC_FOOTER)),
    };
    let (_, title_size) = painter.text_font(FontSlot::Candidate);
    let (_, body_size) = painter.text_font(FontSlot::Translate);
    let title_width = painter
        .measure_text(title, FontSlot::Candidate, title_size)
        .0;
    let title_height = painter
        .measure_text(title, FontSlot::Candidate, title_size)
        .1;
    let body_height = painter.measure_text("国", FontSlot::Translate, body_size).1 + 2.0;

    let mut lines = Vec::new();
    {
        let mut measure = |text: &str| painter.measure_text(text, FontSlot::Translate, body_size).0;
        for paragraph in paragraphs {
            if paragraph.is_empty() {
                continue;
            }
            lines.extend(wrap(paragraph, MAX_TEXT_WIDTH, &mut measure));
        }
    }
    let footer_lines = footer.map_or_else(Vec::new, |footer| {
        let mut measure = |text: &str| painter.measure_text(text, FontSlot::Translate, body_size).0;
        wrap(footer, MAX_TEXT_WIDTH, &mut measure)
    });

    let text_width = lines
        .iter()
        .chain(footer_lines.iter())
        .map(|line| painter.measure_text(line, FontSlot::Translate, body_size).0)
        .fold(title_width, f32::max);
    let width = (text_width + PADDING * 2.0).max(360.0);
    let footer_height = if footer_lines.is_empty() {
        0.0
    } else {
        GAP + footer_lines.len() as f32 * body_height
    };
    let height =
        PADDING * 2.0 + title_height + GAP + lines.len() as f32 * body_height + footer_height;

    let mut elements = Vec::with_capacity(lines.len() + footer_lines.len() + 1);
    elements.push(Element::Text {
        x: PADDING,
        y: PADDING,
        text: title.to_owned(),
        size: title_size,
        slot: FontSlot::Candidate,
        color: palette.text,
        italic: false,
    });
    let body_top = PADDING + title_height + GAP;
    for (index, line) in lines.into_iter().enumerate() {
        elements.push(Element::Text {
            x: PADDING,
            y: body_top + body_height * index as f32,
            text: line,
            size: body_size,
            slot: FontSlot::Translate,
            color: palette.extra,
            italic: false,
        });
    }
    let footer_top = body_top + body_height * (elements.len() - 1) as f32 + GAP;
    for (index, line) in footer_lines.into_iter().enumerate() {
        elements.push(Element::Text {
            x: PADDING,
            y: footer_top + body_height * index as f32,
            text: line,
            size: body_size,
            slot: FontSlot::Translate,
            color: palette.badge,
            italic: false,
        });
    }
    Document {
        width,
        height,
        elements,
        regions: Vec::new(),
        control_names: Vec::new(),
        instances: Default::default(),
    }
}

/// 按宽度逐字折行；中文没有空格也能断，英文示例太长时会被拆开。
fn wrap(text: &str, max_width: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let mut candidate = current.clone();
        candidate.push(ch);
        if !current.is_empty() && measure(&candidate) > max_width {
            lines.push(current);
            current = String::new();
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
