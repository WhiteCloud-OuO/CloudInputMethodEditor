//! 颜色：sRGB 8 位 + alpha，与平台无关；到 tiny-skia / cosmic-text 的换算集中在这里。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,

    pub g: u8,

    pub b: u8,

    /// 不透明度，255 为完全不透明。
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// 纯黑 / 纯白带不透明度，系统语义色（label / secondaryLabel…）都是这个形态。
    pub const fn gray(level: u8, alpha: u8) -> Self {
        Self {
            r: level,
            g: level,
            b: level,
            a: alpha,
        }
    }

    /// 按自己的 alpha 叠在 `background` 上（source-over），得到**不透明**色。
    ///
    /// 给「绘制」那条路用：GDI 没有 alpha 通道，半透明的字色 / 线色得先跟底色混好，画出来才跟渲染一致。
    pub fn over(self, background: Color) -> Color {
        if self.a == 255 {
            return self;
        }
        let alpha = f32::from(self.a) / 255.0;
        let mix = |source: u8, back: u8| {
            (f32::from(source) * alpha + f32::from(back) * (1.0 - alpha)).round() as u8
        };
        Color::rgb(
            mix(self.r, background.r),
            mix(self.g, background.g),
            mix(self.b, background.b),
        )
    }

    pub(crate) fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    pub(crate) fn to_cosmic(self) -> cosmic_text::Color {
        cosmic_text::Color::rgba(self.r, self.g, self.b, self.a)
    }

    /// 乘上一层覆盖率（字形遮罩的像素值）后的预乘颜色。
    pub(crate) fn premultiplied(self, coverage: u8) -> tiny_skia::PremultipliedColorU8 {
        let alpha = mul_u8(self.a, coverage);
        premultiply(self.r, self.g, self.b, alpha)
    }
}

/// 8 位定点乘法：`a * b / 255`，四舍五入。
pub(crate) fn mul_u8(a: u8, b: u8) -> u8 {
    ((u32::from(a) * u32::from(b) + 127) / 255) as u8
}

/// 直通 RGBA → 预乘。
pub(crate) fn premultiply(r: u8, g: u8, b: u8, a: u8) -> tiny_skia::PremultipliedColorU8 {
    tiny_skia::PremultipliedColorU8::from_rgba(mul_u8(r, a), mul_u8(g, a), mul_u8(b, a), a)
        .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplies_with_coverage() {
        let c = Color::rgba(255, 0, 0, 255).premultiplied(128);
        assert_eq!((c.red(), c.green(), c.blue(), c.alpha()), (128, 0, 0, 128));
        let half = Color::gray(0, 128).premultiplied(255);
        assert_eq!((half.red(), half.alpha()), (0, 128));
        assert_eq!(mul_u8(255, 255), 255);
        assert_eq!(mul_u8(0, 255), 0);
    }
}
