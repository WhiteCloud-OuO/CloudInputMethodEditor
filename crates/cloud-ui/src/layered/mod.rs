//! 分层窗口贴图：把云朵渲染器出的预乘 RGBA 位图换成 BGRA，`UpdateLayeredWindow` 一次贴上。
//! 候选窗口与状态条共用；位图带阴影边，在 `Canvas` 里换通道后贴。
//!
//! 这是**渲染**那条路的最后一步（`docs/design/windows.md` §6）：位图就是那块「内存区域」，画好一整帧之后
//! **整块**贴上；贴完窗口自己不会把它擦掉，要变只能再贴一帧。

mod canvas;

use cloudime_render::Pixmap;
use windows::Win32::Foundation::{COLORREF, E_INVALIDARG, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HDC};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Error, Result};

use self::canvas::Canvas;

/// 阴影留白的逻辑像素宽度，状态条还没画过时按它折算占位。
const SHADOW_MARGIN: i32 = 16;

/// 阴影留白的物理像素宽度（`dpi` 96 为 100%）。
pub fn shadow_margin(dpi: u32) -> i32 {
    ((SHADOW_MARGIN * dpi as i32) / 96).max(1)
}

/// 把渲染器出的预乘 RGBA 位图（已含阴影边）贴到分层窗口上，`win_pos` 是位图左上角的屏幕坐标。
/// `alpha` 是整张位图的额外不透明度（255 = 原样），候选窗展开 / 收起过渡淡入时用。
pub fn present(hwnd: HWND, pixmap: &Pixmap, win_pos: (i32, i32), alpha: u8) -> Result<()> {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    if w <= 0 || h <= 0 {
        return Err(Error::from(E_INVALIDARG));
    }
    let mut canvas = Canvas::new(w, h)?;
    // tiny-skia 是 RGBA，DIB 是 BGRA；都是预乘，只换 R / B 两个字节。
    swap_rb(pixmap.data(), canvas.pixels());
    update(hwnd, canvas.dc(), win_pos, (w, h), alpha)
}

/// RGBA → BGRA：只在 4 字节组里换 R / B（预乘值本身不变）。
///
/// 一帧 640×600@200% 是 1.46MB，标量逐像素约 0.5ms；有 SSSE3 时 16 字节一组 shuffle，快四倍左右。
fn swap_rb(source: &[u8], target: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("ssse3") {
            // SAFETY: 上面刚检测过 SSSE3；两个切片长度都由调用方保证一致。
            unsafe { swap_rb_ssse3(source, target) };
            return;
        }
    }
    swap_rb_scalar(source, target);
}

/// 标量兜底（也负责处理 16 字节对齐之后的尾巴）。
fn swap_rb_scalar(source: &[u8], target: &mut [u8]) {
    for (dst, src) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
}

/// SSSE3：一次 16 字节（4 个像素）做一次字节洗牌。
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn swap_rb_ssse3(source: &[u8], target: &mut [u8]) {
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8, _mm_storeu_si128,
    };

    // 每个 4 字节组：2,1,0,3
    let mask = _mm_setr_epi8(2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15);
    let chunks = (source.len() / 16).min(target.len() / 16);
    for index in 0..chunks {
        let offset = index * 16;
        // `_mm_setr_epi8` / `_mm_shuffle_epi8` 本身是安全的，只有读写裸指针那两下要 `unsafe`
        let input = unsafe { _mm_loadu_si128(source.as_ptr().add(offset).cast::<__m128i>()) };
        let output = _mm_shuffle_epi8(input, mask);
        unsafe { _mm_storeu_si128(target.as_mut_ptr().add(offset).cast::<__m128i>(), output) };
    }
    let done = chunks * 16;
    swap_rb_scalar(&source[done..], &mut target[done..]);
}

/// `UpdateLayeredWindow`：整张位图按预乘 alpha 贴上并挪到 `win_pos`。
fn update(
    hwnd: HWND,
    hdc: HDC,
    win_pos: (i32, i32),
    win_size: (i32, i32),
    alpha: u8,
) -> Result<()> {
    let (w, h) = win_size;
    let dst = POINT {
        x: win_pos.0,
        y: win_pos.1,
    };
    let size = SIZE { cx: w, cy: h };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(&dst),
            Some(&size),
            Some(hdc),
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::swap_rb;

    /// SSSE3 那条路与逐字节版结果一致（含不足 16 / 4 字节的尾巴）。
    #[test]
    fn swap_rb_matches_the_bytewise_version() {
        let source: Vec<u8> = (0..=255u8).cycle().take(64 * 4 + 3).collect();
        // 尾巴（不足 4 字节）本来就不该被写：先把目标填成同一份内容，才看得出「没动」
        let mut fast = source.clone();
        swap_rb(&source, &mut fast);

        let mut slow = source.clone();
        for (dst, src) in slow.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let done = source.len() - source.len() % 4;
        assert_eq!(fast[..done], slow[..done]);
        assert_eq!(fast[done..], source[done..], "尾巴不动");
    }
}
