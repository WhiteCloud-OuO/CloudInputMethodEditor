//! 主题：字体、颜色、间距。所有可视参数都在这里，单位是点；将来从 TOML 读。
//!
//! 视觉层级（产品决定）：拼音串与序号纯黑（看得清），候选词深灰，译文稍浅，词性最浅。数值沿用调研期 macOS 原型的实测结果。

mod font_spec;
mod palette;

pub use font_spec::FontSpec;
pub use palette::Palette;

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// 拼音串字体（候选窗口顶部那一行）。
    pub pinyin_font: FontSpec,

    /// 候选词字体。
    pub text_font: FontSpec,

    /// 译文与词性字体。
    pub annotation_font: FontSpec,

    /// 序号字体。
    pub index_font: FontSpec,

    /// 翻译 Tip 的字体（候选窗口底部那一行左侧）。
    pub translate_font: FontSpec,

    /// 拼音串、候选词、序号三个字体各自的字族名；`None` 用系统界面字体。
    pub pinyin_family: Option<String>,
    pub text_family: Option<String>,
    pub index_family: Option<String>,

    /// 翻译 Tip 的字族名；`None` 用系统界面字体。
    pub translate_family: Option<String>,

    /// 配色。
    pub colors: Palette,

    /// 窗口内边距。
    pub padding: f32,

    /// 行内上下留白。
    pub row_padding: f32,

    /// 序号与候选词、候选词与译文之间的间距。
    pub column_gap: f32,

    /// 窗口与高亮条的圆角。
    pub corner_radius: f32,

    /// 最多显示几行。
    pub max_rows: usize,

    /// 竖排时窗口的最小宽度（物理像素；100% 缩放时与点相等）。
    pub min_width_pixels: f32,

    /// 文字抗锯齿覆盖率的 gamma：小于 1 笔画显粗。系统渲染对文字有一层类似的加深，深色背景上尤其明显，
    /// 线性混合出来的字会偏细；这个值按真机截图并排调。
    pub text_gamma: f32,
}

impl Theme {
    /// 缺省主题，对齐调研期 macOS 上量的系统外观；候选窗的三项字体由壳按配置覆盖
    /// （[`Theme::set_fonts`]），这里的值也是覆盖不到时的落点。
    pub fn new() -> Self {
        Self {
            // 行高取调研期在 macOS 上量的系统字体在这几个字号下的行高
            pinyin_font: FontSpec::new(12.0, 15.0),
            text_font: FontSpec::new(16.0, 19.0),
            annotation_font: FontSpec::new(12.0, 15.0),
            index_font: FontSpec::new(11.0, 14.0),
            translate_font: FontSpec::new(11.0, 14.0),
            pinyin_family: None,
            text_family: None,
            index_family: None,
            translate_family: None,
            colors: Palette::new(),
            padding: 8.0,
            row_padding: 4.0,
            column_gap: 8.0,
            corner_radius: 8.0,
            max_rows: 9,
            min_width_pixels: 320.0,
            text_gamma: 0.85,
        }
    }

    /// 换上拼音串 / 候选词 / 序号 / 翻译 Tip 四项字体（字族名 + 字号，点）。空字族名表示系统界面字体。
    pub fn set_fonts(
        &mut self,
        pinyin: (Option<String>, f32),
        candidate: (Option<String>, f32),
        index: (Option<String>, f32),
        translate: (Option<String>, f32),
    ) {
        self.pinyin_family = pinyin.0;
        self.pinyin_font = FontSpec::with_size(pinyin.1);
        self.text_family = candidate.0;
        self.text_font = FontSpec::with_size(candidate.1);
        self.index_family = index.0;
        self.index_font = FontSpec::with_size(index.1);
        self.translate_family = translate.0;
        self.translate_font = FontSpec::with_size(translate.1);
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new()
    }
}
