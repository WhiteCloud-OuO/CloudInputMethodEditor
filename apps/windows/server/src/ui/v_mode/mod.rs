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

use crate::dispatch::VModeScheme;

const PADDING: f32 = 16.0;
const GAP: f32 = 8.0;
const MAX_TEXT_WIDTH: f32 = 520.0;

const TITLE: &str = "V模式";
const HELP_PINYIN: [&str; 7] = [
    "输入数学表达式进行计算，支持数学函数（例如“sin(30)+lg(100)”）。只输入 v 时按Tab进入/退出高级数学计算界面。",
    "输入数字进行转换（例如“12.12”）。",
    "输入=>加0b/0o/0x/十进制数进行进制转换（例如“=>0x33ccff”）。",
    "输入sj/rq/xq/xz获得时间/日期/星期/现在（例如“xz”）。",
    "输入@加省份简称加英文字母（忽略大小写）获得车牌和所在地（例如“@sua”可以匹配到“苏A”/“江苏省南京市”/“江苏南京”）。",
    "输入#加姓名拼音字母获得取名结果（仅限汉族名字，例如“#zhangwei”可以匹配到“张伟”）。选中姓氏后它会留在组句里，接着输入名字。",
    "",
];

/// 双拼：`@` / `#` 后面按当前双拼方案敲音节，不用手动加 `'`。
const HELP_DOUBLE: [&str; 8] = [
    "双拼方案里按 Ctrl+Alt+V 进入 V 模式；@ / # 后面按当前双拼方案敲音节，不用手动加 '。",
    "输入数学表达式进行计算，支持数学函数（例如“sin(30)+lg(100)”）。只输入 v 时按Tab进入/退出高级数学计算界面。",
    "输入数字进行转换（例如“12.12”）。",
    "输入=>加0b/0o/0x/十进制数进行进制转换（例如“=>0x33ccff”）。",
    "输入sj/rq/xq/xz获得时间/日期/星期/现在（例如“xz”）。",
    "输入@加省份简称的双拼按键，再直接跟字母获得车牌和所在地（不用加 '）。",
    "输入#加姓名的双拼按键获得取名结果（仅限汉族名字，不用加 '）。选中姓氏后它会留在组句里，接着输入名字。",
    "",
];

/// 形码（五笔）：`@` / `#` 吃五笔编码，一个字一段、用 `'` 隔开。
const HELP_SHAPE: [&str; 9] = [
    "五笔方案里按 Ctrl+Alt+V 进入 V 模式；下面的 @ / # 都按五笔编码输入，每个字之间用 ' 分隔。",
    "z 在开头（vz / v#z / v@z）会被忽略；不在开头时按五笔通配一位（例如 v#xtaz 可以匹配到 张）。",
    "输入数学表达式进行计算，支持数学函数（例如“sin(30)+lg(100)”）。只输入 v 时按Tab进入/退出高级数学计算界面。",
    "输入数字进行转换（例如“12.12”）。",
    "输入=>加0b/0o/0x/十进制数进行进制转换（例如“=>0x33ccff”）。",
    "输入jfuj/jjad/jtad/gmdh获得时间/日期/星期/现在（例如“gmdh”）。",
    "输入@加省份字的五笔编码，再按'加车牌字母（忽略大小写）获得车牌和所在地（例如“@iynt'c”可以匹配到“沪C”/“上海市”/“上海”）。",
    "输入#加姓名的五笔编码、每个字之间用'隔开获得取名结果（仅限汉族名字，例如“#xtay'wfnh”可以匹配到“张伟”）。选中姓氏后它会留在组句里，接着输入名字。",
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
    /// 当前输入方案：说明里的时间 / 车牌 / 取名示例跟着换。
    scheme: Rc<Cell<VModeScheme>>,
    calculator: Rc<Cell<bool>>,
    built: Cell<Option<(u64, HelpMode, VModeScheme)>>,
}

impl VModeHelpWindow {
    pub(crate) fn new(
        painter: SharedPainter,
        owner: HWND,
        v_mode: Rc<Cell<bool>>,
        scheme: Rc<Cell<VModeScheme>>,
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
            scheme,
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
        let scheme = self.scheme.get();
        let built = {
            let mut guard = self.painter.borrow_mut();
            let Some(painter) = guard.as_mut() else {
                self.hide();
                return;
            };
            let generation = painter.generation();
            (self.built.get() != Some((generation, mode, scheme)))
                .then(|| (generation, build_document(painter, mode, scheme)))
        };
        if let Some((generation, document)) = built {
            self.template.set_document(document);
            self.built.set(Some((generation, mode, scheme)));
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

/// 说明窗口的标题 / 正文 / 退出提示：V 模式按输入方案选文案，高级计算界面各方案共用。
fn help_content(
    mode: HelpMode,
    scheme: VModeScheme,
) -> (&'static str, &'static [&'static str], Option<&'static str>) {
    match (mode, scheme) {
        (HelpMode::VMode, VModeScheme::Pinyin) => (TITLE, &HELP_PINYIN, None),
        (HelpMode::VMode, VModeScheme::DoublePinyin) => (TITLE, &HELP_DOUBLE, None),
        (HelpMode::VMode, VModeScheme::Shape) => (TITLE, &HELP_SHAPE, None),
        (HelpMode::Calculator, _) => (CALC_TITLE, &CALC_HELP, Some(CALC_FOOTER)),
    }
}

/// 标签用候选项颜色，正文用额外内容颜色，退出提示用角标颜色；正文按固定宽度折行。
fn build_document(
    painter: &mut cloud_ui::painter::Painter,
    mode: HelpMode,
    scheme: VModeScheme,
) -> Document {
    let palette = painter.palette();
    let (title, paragraphs, footer) = help_content(mode, scheme);
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
        scroll_targets: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::{CALC_FOOTER, CALC_TITLE, HelpMode, help_content};
    use crate::dispatch::VModeScheme;

    #[test]
    fn v_mode_help_follows_the_input_scheme() {
        let (_, pinyin, _) = help_content(HelpMode::VMode, VModeScheme::Pinyin);
        let (_, double, _) = help_content(HelpMode::VMode, VModeScheme::DoublePinyin);
        let (_, shape, _) = help_content(HelpMode::VMode, VModeScheme::Shape);
        assert!(pinyin.iter().any(|line| line.contains("@sua")));
        assert!(pinyin.iter().any(|line| line.contains("#zhangwei")));
        assert!(pinyin.iter().any(|line| line.contains("sj/rq/xq/xz")));
        assert!(double.iter().any(|line| line.contains("双拼")));
        assert!(double.iter().any(|line| line.contains("不用手动加")));
        assert!(!double.iter().any(|line| line.contains("@sua")));
        assert!(shape.iter().any(|line| line.contains("@iynt'c")));
        assert!(shape.iter().any(|line| line.contains("#xtay'wfnh")));
        assert!(
            shape
                .iter()
                .any(|line| line.contains("jfuj/jjad/jtad/gmdh"))
        );
        assert!(shape.iter().any(|line| line.contains("v#xtaz")));
        assert!(!shape.iter().any(|line| line.contains("@sua")));
        assert!(!shape.iter().any(|line| line.contains("sj/rq/xq/xz")));

        let (title, _, footer) = help_content(HelpMode::Calculator, VModeScheme::Pinyin);
        assert_eq!(title, CALC_TITLE);
        assert_eq!(footer, Some(CALC_FOOTER));
    }
}
