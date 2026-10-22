//! 把候选窗口现有排版代码「录」成自绘面板的 draw 列表。
//!
//! 排版仍然走 `draw_top_line` / `draw_vertical` / `draw_horizontal` / `draw_matrix`
//! 这些函数；它们只调用 [`ControlRender`] 的绘制原语。这里实现一套记录后端：
//! 原语不落位图，而是按「点」坐标转成 [`Element`]，最后塞进
//! [`crate::document::SelfDrawingPanel`] 的 `draw`。这样候选窗口的每行文字、
//! 高亮条、背景都成为可单独更新的 draw 元素，同时不需要重写一套排版。

use std::cell::RefCell;
use std::rc::Rc;

use tiny_skia::{Path, Rect};

use crate::color::Color;
use crate::document::Element;
use crate::elements::{
    ImageElement, LineElement, PathElement, RectElement, StrokeElement, SvgElement, TextElement,
};
use crate::error::RenderError;
use crate::text::{TextPainter, TextSize, TextStyle};

/// 记录绘制原语的画布；坐标从内容区左上角算，单位是点。
pub(super) struct DrawRecorder {
    text: Rc<RefCell<TextPainter>>,
    scale: f32,
    origin: (f32, f32),
    elements: Vec<Element>,
}

impl DrawRecorder {
    pub(super) fn new(text: Rc<RefCell<TextPainter>>, scale: f32, origin: (f32, f32)) -> Self {
        Self {
            text,
            scale: scale.max(f32::EPSILON),
            origin,
            elements: Vec::new(),
        }
    }

    pub(super) fn into_elements(self) -> Vec<Element> {
        self.elements
    }

    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x - self.origin.0) / self.scale,
            (y - self.origin.1) / self.scale,
        )
    }

    fn length(&self, value: f32) -> f32 {
        value / self.scale
    }
}

impl TextElement for DrawRecorder {
    fn text(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> f32 {
        let size = self.text.borrow_mut().measure(text, style);
        let (x, y) = self.point(x, y);
        self.elements.push(Element::StyledText {
            x,
            y,
            text: text.to_owned(),
            size: style.points,
            line_height: self.length(style.line_height),
            family: style.family.clone(),
            color: style.color,
            italic: style.italic,
            strike: style.strike,
            underline: style.underline,
            gamma: style.gamma,
        });
        size.width
    }

    fn measure(&self, text: &str, style: &TextStyle) -> TextSize {
        self.text.borrow_mut().measure(text, style)
    }

    fn supports_char(&mut self, ch: char, style: &TextStyle) -> bool {
        self.text.borrow_mut().supports_char(ch, style)
    }
}

impl SvgElement for DrawRecorder {
    fn svg(
        &mut self,
        svg: &str,
        x: f32,
        y: f32,
        size: f32,
        tint: Option<Color>,
    ) -> Result<(), RenderError> {
        let (x, y) = self.point(x, y);
        self.elements.push(Element::Svg {
            x,
            y,
            size: self.length(size),
            source: svg.to_owned(),
            tint,
        });
        Ok(())
    }
}

impl ImageElement for DrawRecorder {
    fn image(&mut self, image: &tiny_skia::Pixmap, x: f32, y: f32, width: f32, height: f32) {
        let (x, y) = self.point(x, y);
        self.elements.push(Element::Image {
            x,
            y,
            width: self.length(width),
            height: self.length(height),
            image: std::sync::Arc::new(image.clone()),
        });
    }
}

impl LineElement for DrawRecorder {
    fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color) {
        let from = self.point(from.0, from.1);
        let to = self.point(to.0, to.1);
        self.elements.push(Element::Line {
            from,
            to,
            thickness: self.length(thickness),
            color,
        });
    }
}

impl RectElement for DrawRecorder {
    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let (x, y) = self.point(x, y);
        self.elements.push(Element::Rect {
            x,
            y,
            width: self.length(width),
            height: self.length(height),
            radius: 0.0,
            color,
        });
    }

    fn round_rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) {
        let (x, y) = self.point(x, y);
        self.elements.push(Element::Rect {
            x,
            y,
            width: self.length(width),
            height: self.length(height),
            radius: self.length(radius),
            color,
        });
    }

    fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let (cx, cy) = self.point(cx, cy);
        self.elements.push(Element::Circle {
            cx,
            cy,
            radius: self.length(radius),
            color,
        });
    }
}

impl PathElement for DrawRecorder {
    fn path(&mut self, path: &Path, _color: Color) {
        // 候选窗排版目前只用文字 / 矩形 / 圆角矩形，不产生 Path；保留空实现，
        // 避免为了录制一条路径再造一套 Path → PathCommand 的逆向转换。
        let _ = path;
    }
}

impl StrokeElement for DrawRecorder {
    fn stroke_round_rect(&mut self, _rect: Rect, _radius: f32, _thickness: f32, _color: Color) {
        // 同上：候选窗不用描边。
    }
}
