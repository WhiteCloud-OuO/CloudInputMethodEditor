//! 脚本窗口：`cloudime.window.create` / `cloudime.window.create_auxiliary` 的请求与句柄。
//!
//! 脚本在**工人线程**上跑，窗口在 **UI 线程**上：两边不能互相调用，所以这里只把脚本要的东西
//! 读成一份**普通数据**（[`Request`]），由 Server 在派发结束后取走、经命令通道交给 UI 线程执行
//! （与 `cloudime.candidate.set_*` / `cloudime.apply_theme` 同一个路子）。
//!
//! 内容是一份声明式的元素表（[`Document`]，坐标一律是**点**）：脚本不直接画，UI 线程拿着它经
//! `Renderer::render_document` 一次出图 —— 这样脚本再怎么写也卡不住 UI 线程，反过来也一样。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use cloudime_render::{
    ButtonColors, ButtonShape, ButtonState, Color, Document, Element, FlatSvgButton, FontSlot,
    HitRect, HitShape, Label, LabelRun, Panel, PathCommand, Pixmap, Region, ScrollAxis, ScrollBar,
    StaticImage, StaticShape, SvgAlign, TextBlock, TextBox, TextBoxScroll, TextEdit, flip_image,
    gray_image, load_image, mask_image, rotate_image, scale_image, svg_size,
};

use crate::mlua;

/// 窗口编号：脚本那一侧只当句柄的一部分，UI 线程按它认窗口。
pub type WindowId = u32;

/// 脚本侧看到的编辑框文本缓存：`(窗口编号, 控件名) → 最近一次文本`。
/// UI 线程通过 `text_changed` 事件刷新，`get_text` 直接读它（不阻塞工人线程）。
pub(crate) type TextCache = Rc<RefCell<HashMap<(WindowId, String), String>>>;

/// 一个脚本窗口最多画几个元素（超了当场报错 —— 一份内容要在一帧里画完）。
const MAX_ELEMENTS: usize = 512;

/// 内容区尺寸上下限（点）：太小放不下一个元素，太大一块位图太重。
const MIN_SIZE: f32 = 8.0;
const MAX_SIZE: f32 = 2048.0;

/// 一段文字最多多少个字符。
const MAX_TEXT: usize = 4096;

/// 一份内容最多几个区域（点击 / 拖拽范围）。
const MAX_REGIONS: usize = 128;

/// 一个区域最多合并 / 剪除几个形状。
const MAX_REGION_PARTS: usize = 32;

/// 一个标签最多几段不同颜色。
const MAX_LABEL_RUNS: usize = 64;

/// 一个区域 / 动态控件最多声明几个事件处理器。
const MAX_EVENT_HANDLERS: usize = 32;

/// 面板最多套几层（防止脚本写出无限嵌套）。
const MAX_PANEL_DEPTH: usize = 8;

/// 图像解码后的像素数上限（约 4096 × 4096）：脚本给的图不该把整块内存吃掉。
const MAX_IMAGE_PIXELS: u32 = 16 * 1024 * 1024;

/// 从文件读 SVG 时的大小上限（字节）：SVG 是文本，2 MB 已经够复杂图标用了。
const MAX_SVG_BYTES: u64 = 2 * 1024 * 1024;

/// 图像缓存最多留几张（按最近使用排序，超了丢最久没用的）。
const MAX_CACHED_IMAGES: usize = 8;

/// 图像缓存里所有位图加起来最多多少像素（约 32M，128MB 的 RGBA）。
const MAX_CACHED_PIXELS: u64 = 32 * 1024 * 1024;

/// 图像缓存：路径 → 解码好的位图（`Arc` 与内容共享，不复制像素）。
///
/// 每次 `set_content` 都会重新读元素表，但同一张图不需要重新解码：按**文件长度 + 修改时间**
/// 判断要不要重读，没变就直接复用。文件换了（用户改图）下次 `set_content` 就能看到新图。
#[derive(Default)]
pub(crate) struct ImageCache {
    /// 最近用过的排在前面（最简单的 LRU）。
    entries: Vec<CachedImage>,

    /// 这些位图加起来多少像素。
    pixels: u64,
}

/// 脚本拿在手里的图像句柄（`cloudime.image.load`）：内部是可变的 `Arc<Pixmap>`，
/// 与元素共享位图；Lua 丢掉句柄后，已经放进内容里的图仍然活着。
#[derive(Clone)]
pub(crate) struct ImageHandle {
    image: Rc<RefCell<Arc<Pixmap>>>,
}

impl ImageHandle {
    pub(crate) fn new(image: Arc<Pixmap>) -> Self {
        Self {
            image: Rc::new(RefCell::new(image)),
        }
    }

    pub(crate) fn snapshot(&self) -> Arc<Pixmap> {
        self.image.borrow().clone()
    }
}

impl mlua::UserData for ImageHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("scale", |_, this, (a, b): (f32, Option<f32>)| {
            let current = this.snapshot();
            let (width, height) = match b {
                Some(height) => {
                    if !a.is_finite() || !height.is_finite() || a < 1.0 || height < 1.0 {
                        return Err(mlua::Error::RuntimeError(
                            "scale(width, height) 要两个正数".to_owned(),
                        ));
                    }
                    (a.round() as u32, height.round() as u32)
                }
                None => {
                    if !a.is_finite() || a <= 0.0 {
                        return Err(mlua::Error::RuntimeError(
                            "scale(factor) 要给正数".to_owned(),
                        ));
                    }
                    (
                        (current.width() as f32 * a).round().max(1.0) as u32,
                        (current.height() as f32 * a).round().max(1.0) as u32,
                    )
                }
            };
            let next = scale_image(&current, width, height)
                .map_err(|error| mlua::Error::RuntimeError(format!("缩放图像失败：{error}")))?;
            *this.image.borrow_mut() = Arc::new(next);
            Ok(())
        });
        methods.add_method("rotate", |_, this, degrees: u16| {
            let next = rotate_image(&this.snapshot(), degrees)
                .map_err(|error| mlua::Error::RuntimeError(format!("旋转图像失败：{error}")))?;
            *this.image.borrow_mut() = Arc::new(next);
            Ok(())
        });
        methods.add_method("flip", |_, this, direction: String| {
            let horizontal = match direction.to_ascii_lowercase().as_str() {
                "horizontal" | "h" => true,
                "vertical" | "v" => false,
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "flip 只支持 horizontal / vertical，给的是 {other}"
                    )));
                }
            };
            let next = flip_image(&this.snapshot(), horizontal);
            *this.image.borrow_mut() = Arc::new(next);
            Ok(())
        });
        methods.add_method("to_gray", |_, this, ()| {
            let next = gray_image(&this.snapshot());
            *this.image.borrow_mut() = Arc::new(next);
            Ok(())
        });
        methods.add_method("use_path_mask", |_, this, path: mlua::Table| {
            let commands = if path.get::<Option<mlua::Table>>("commands")?.is_some() {
                parse_path(&path)?
            } else {
                parse_path_commands(&path)?
            };
            let next = mask_image(&this.snapshot(), &commands)
                .map_err(|error| mlua::Error::RuntimeError(format!("路径蒙版失败：{error}")))?;
            *this.image.borrow_mut() = Arc::new(next);
            Ok(())
        });
    }
}

struct CachedImage {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    image: Arc<Pixmap>,
}

impl ImageCache {
    /// 读一张图（缓存命中就不解码）。路径按 Server 的工作目录（安装目录）解析。
    pub(crate) fn load(&mut self, path: &Path) -> mlua::Result<Arc<Pixmap>> {
        let metadata = std::fs::metadata(path).map_err(|error| {
            mlua::Error::RuntimeError(format!("读不了图像 {}：{error}", path.display()))
        })?;
        let len = metadata.len();
        let modified = metadata.modified().ok();
        if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
            let same = self.entries[index].len == len && self.entries[index].modified == modified;
            if same {
                let entry = self.entries.remove(index);
                let image = entry.image.clone();
                self.entries.insert(0, entry);
                return Ok(image);
            }
            let stale = self.entries.remove(index);
            self.pixels -= cached_pixels(&stale.image);
        }
        let image = load_image(path).map_err(|error| {
            mlua::Error::RuntimeError(format!("读不了图像 {}：{error}", path.display()))
        })?;
        let pixels = u64::from(image.width()) * u64::from(image.height());
        if pixels > u64::from(MAX_IMAGE_PIXELS) {
            return Err(mlua::Error::RuntimeError(format!(
                "图像 {} 太大（{} × {}，上限 4096 × 4096）",
                path.display(),
                image.width(),
                image.height()
            )));
        }
        let image = Arc::new(image);
        self.entries.insert(
            0,
            CachedImage {
                path: path.to_path_buf(),
                len,
                modified,
                image: image.clone(),
            },
        );
        self.pixels += pixels;
        while self.entries.len() > MAX_CACHED_IMAGES || self.pixels > MAX_CACHED_PIXELS {
            let Some(old) = self.entries.pop() else {
                break;
            };
            self.pixels -= cached_pixels(&old.image);
        }
        Ok(image)
    }
}

fn cached_pixels(image: &Pixmap) -> u64 {
    u64::from(image.width()) * u64::from(image.height())
}

/// 独立窗口的创建参数（尺寸是**点**）。
#[derive(Debug, Clone)]
pub struct WindowSpec {
    /// 日志与 `window` 事件里的名字。
    pub name: String,

    /// 内容区宽（点）。
    pub width: f32,

    /// 内容区高（点）。
    pub height: f32,

    /// 接不接鼠标（`false` = 鼠标穿透，纯展示）。
    pub interactive: bool,

    /// 要不要阴影（与候选窗同源的那一层）。
    pub shadow: bool,

    /// 要不要输入法画的面板底色 / 圆角。`false` = 内容区透明，由脚本自己画背景
    ///（窗口外观完全归脚本；阴影仍按 `shadow` 决定）。
    pub background: bool,
}

/// 辅助窗口（宿主名下的资产）的创建参数。
#[derive(Debug, Clone)]
pub struct AuxiliarySpec {
    /// 日志与事件里的名字。
    pub name: String,

    /// 宿主（必须是脚本建过的独立窗口）。
    pub owner: WindowId,

    /// 内容区宽（点）；`None` = 跟内容走 / 模板默认。
    pub width: Option<f32>,

    /// 内容区高（点）；`None` = 跟内容走 / 模板默认。
    pub height: Option<f32>,

    /// 与宿主之间的间隙（点）；`None` = 模板默认。
    pub gap: Option<f32>,

    /// 接不接鼠标（`true` 才在创建时声明 `able_to_get_focus`）。
    pub interactive: bool,

    /// 要不要阴影。
    pub shadow: bool,
}

/// 脚本对窗口的一次操作：工人线程产出，UI 线程按顺序执行。
#[derive(Debug, Clone)]
pub enum Request {
    /// 建一块独立窗口（内容随后用 [`Request::SetContent`] 送）。
    Create(WindowId, WindowSpec),

    /// 建一件辅助窗口（宿主名下的一件资产）。
    CreateAuxiliary(WindowId, AuxiliarySpec),

    /// 换一整份内容（尺寸可以跟着变）。
    SetContent(WindowId, Document),

    /// 显示；`position` 是内容区左上角的屏幕坐标（点），`None` = 上次的位置 / 光标旁。
    Show(WindowId, Option<(f32, f32)>),

    /// 收起。
    Hide(WindowId),

    /// 销毁（独立窗口连名下资产一起放掉）。
    Close(WindowId),

    /// 把逻辑焦点指定给这件辅助窗口 / 交还给宿主；独立窗口则是拿 / 放键盘焦点。
    Focus(WindowId, bool),

    /// 动态改一个**已有区域**的命中范围：`combine` = 合并进来（并集），否则剪除（差集）。
    /// 只影响鼠标命中，不重画（区域不参与绘制）。
    EditRegion {
        id: WindowId,
        name: String,
        shape: HitShape,
        combine: bool,
    },

    /// 设置某个 `text_edit` 的文本（按控件名找）。
    SetText {
        id: WindowId,
        name: String,
        text: String,
    },

    /// 全选（`true`）或取消选择（`false`）。
    SelectAll {
        id: WindowId,
        name: String,
        select: bool,
    },

    /// 把文本里的 `find` 全部替换成 `text`。
    ReplaceText {
        id: WindowId,
        name: String,
        find: String,
        text: String,
    },

    /// 清空文本。
    ClearText { id: WindowId, name: String },
}

/// 窗口句柄是「独立窗口」还是「辅助窗口」（同一套方法，落点不同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Independent,
    Auxiliary,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Independent => "window",
            Self::Auxiliary => "auxiliary",
        }
    }
}

/// 给一个窗口句柄装上方法：`set_content` / `show` / `hide` / `close` / `focus` /
/// `combine_region` / `prune_region` / `region_contains` / `region_relation`。
///
/// 句柄是一张普通表（`kind` / `name` / `width` / `height` 只是给人看的），方法闭包里带着编号；
/// 脚本怎么改那张表都影响不到运行时 —— 与 `cloudime` 表是只读代理同一个规矩。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle(
    lua: &mlua::Lua,
    kind: Kind,
    id: WindowId,
    name: &str,
    size: (Option<f32>, Option<f32>),
    requests: Rc<RefCell<Vec<Request>>>,
    images: Rc<RefCell<ImageCache>>,
    texts: TextCache,
) -> mlua::Result<mlua::Table> {
    let handle = lua.create_table()?;
    handle.set("id", id)?;
    handle.set("kind", kind.as_str())?;
    handle.set("name", name)?;
    handle.set("width", size.0)?;
    handle.set("height", size.1)?;

    // 这份窗口的区域镜像：`set_content` / `combine_region` / `prune_region` 时同步更新，
    // `region_contains` / `region_relation` 就在工人线程上直接查它（不用等 UI 线程回话）。
    let regions: Rc<RefCell<Vec<Region>>> = Rc::new(RefCell::new(Vec::new()));

    let content_requests = requests.clone();
    let content_images = images.clone();
    let content_regions = regions.clone();
    let content_texts = texts.clone();
    handle.set(
        "set_content",
        lua.create_function(move |_, (this, content): (mlua::Table, mlua::Table)| {
            let document = parse_document(
                &content,
                fallback_size(&this),
                &mut content_images.borrow_mut(),
            )?;
            cache_document_texts(&document, id, &content_texts);
            *content_regions.borrow_mut() = document.regions.clone();
            content_requests
                .borrow_mut()
                .push(Request::SetContent(id, document));
            Ok(())
        })?,
    )?;

    let get_texts = texts.clone();
    handle.set(
        "get_text",
        lua.create_function(move |_, (_this, name): (mlua::Table, String)| {
            Ok(get_texts.borrow().get(&(id, name)).cloned())
        })?,
    )?;

    let set_text_requests = requests.clone();
    let set_texts = texts.clone();
    handle.set(
        "set_text",
        lua.create_function(
            move |_, (_this, name, text): (mlua::Table, String, String)| {
                set_texts
                    .borrow_mut()
                    .insert((id, name.clone()), text.clone());
                set_text_requests
                    .borrow_mut()
                    .push(Request::SetText { id, name, text });
                Ok(())
            },
        )?,
    )?;

    let select_requests = requests.clone();
    handle.set(
        "select_all",
        lua.create_function(
            move |_, (_this, name, select): (mlua::Table, String, Option<bool>)| {
                select_requests.borrow_mut().push(Request::SelectAll {
                    id,
                    name,
                    select: select.unwrap_or(true),
                });
                Ok(())
            },
        )?,
    )?;

    let replace_requests = requests.clone();
    let replace_texts = texts.clone();
    handle.set(
        "replace",
        lua.create_function(
            move |_, (_this, name, find, text): (mlua::Table, String, String, String)| {
                if find.is_empty() {
                    return Ok(());
                }
                let current = replace_texts
                    .borrow()
                    .get(&(id, name.clone()))
                    .cloned()
                    .unwrap_or_default();
                replace_texts
                    .borrow_mut()
                    .insert((id, name.clone()), current.replace(&find, &text));
                replace_requests.borrow_mut().push(Request::ReplaceText {
                    id,
                    name,
                    find,
                    text,
                });
                Ok(())
            },
        )?,
    )?;

    let clear_requests = requests.clone();
    let clear_texts = texts.clone();
    handle.set(
        "clear",
        lua.create_function(move |_, (_this, name): (mlua::Table, String)| {
            clear_texts
                .borrow_mut()
                .insert((id, name.clone()), String::new());
            clear_requests
                .borrow_mut()
                .push(Request::ClearText { id, name });
            Ok(())
        })?,
    )?;

    let show_requests = requests.clone();
    handle.set(
        "show",
        lua.create_function(
            move |_, (_this, position): (mlua::Table, Option<mlua::Table>)| {
                let position = position.as_ref().map(parse_position).transpose()?;
                show_requests.borrow_mut().push(Request::Show(id, position));
                Ok(())
            },
        )?,
    )?;

    let layout_requests = requests.clone();
    let layout_images = images.clone();
    let layout_regions = regions.clone();
    let layout_texts = texts.clone();
    handle.set(
        "load_layout",
        lua.create_function(move |lua, (this, path): (mlua::Table, String)| {
            let document = load_layout_document(
                lua,
                Path::new(&path),
                fallback_size(&this),
                &mut layout_images.borrow_mut(),
            )?;
            cache_document_texts(&document, id, &layout_texts);
            this.set("width", document.width)?;
            this.set("height", document.height)?;
            *layout_regions.borrow_mut() = document.regions.clone();
            layout_requests
                .borrow_mut()
                .push(Request::SetContent(id, document));
            Ok(())
        })?,
    )?;

    let hide_requests = requests.clone();
    handle.set(
        "hide",
        lua.create_function(move |_, _this: mlua::Table| {
            hide_requests.borrow_mut().push(Request::Hide(id));
            Ok(())
        })?,
    )?;

    let close_requests = requests.clone();
    let close_regions = regions.clone();
    handle.set(
        "close",
        lua.create_function(move |_, _this: mlua::Table| {
            close_regions.borrow_mut().clear();
            close_requests.borrow_mut().push(Request::Close(id));
            Ok(())
        })?,
    )?;

    let combine_requests = requests.clone();
    let combine_regions = regions.clone();
    handle.set(
        "combine_region",
        lua.create_function(
            move |_, (_this, name, shape): (mlua::Table, String, mlua::Table)| {
                let shape = shape_from_table(&shape)?;
                if let Some(region) = combine_regions
                    .borrow_mut()
                    .iter_mut()
                    .find(|region| region.name == name)
                {
                    region.combine(shape.clone());
                }
                combine_requests.borrow_mut().push(Request::EditRegion {
                    id,
                    name,
                    shape,
                    combine: true,
                });
                Ok(())
            },
        )?,
    )?;

    let prune_requests = requests.clone();
    let prune_regions = regions.clone();
    handle.set(
        "prune_region",
        lua.create_function(
            move |_, (_this, name, shape): (mlua::Table, String, mlua::Table)| {
                let shape = shape_from_table(&shape)?;
                if let Some(region) = prune_regions
                    .borrow_mut()
                    .iter_mut()
                    .find(|region| region.name == name)
                {
                    region.prune(shape.clone());
                }
                prune_requests.borrow_mut().push(Request::EditRegion {
                    id,
                    name,
                    shape,
                    combine: false,
                });
                Ok(())
            },
        )?,
    )?;

    // 查询：点是否在区域里（与鼠标命中同一套判断，含边界）。
    let contains_regions = regions.clone();
    handle.set(
        "region_contains",
        lua.create_function(
            move |_, (_this, name, x, y): (mlua::Table, String, f32, f32)| {
                let found = contains_regions
                    .borrow()
                    .iter()
                    .find(|region| region.name == name)
                    .map(|region| region.contains(x, y));
                Ok(found)
            },
        )?,
    )?;

    // 查询：两块区域的关系（相离 / 相交 / 内含 / 重合）。只有面积上的重叠才算相交，贴一条边算相离。
    let relation_regions = regions.clone();
    handle.set(
        "region_relation",
        lua.create_function(
            move |lua, (_this, name_a, name_b): (mlua::Table, String, String)| {
                let regions = relation_regions.borrow();
                let (Some(a), Some(b)) = (
                    regions.iter().find(|region| region.name == name_a),
                    regions.iter().find(|region| region.name == name_b),
                ) else {
                    return Ok(mlua::Value::Nil);
                };
                let a_in_b = a.contains_region(b);
                let b_in_a = b.contains_region(a);
                let (relation, contains) = if a_in_b && b_in_a {
                    ("coincide", None)
                } else if a_in_b {
                    ("contains", Some("a"))
                } else if b_in_a {
                    ("contains", Some("b"))
                } else if a.intersects(b) {
                    ("intersect", None)
                } else {
                    ("disjoint", None)
                };
                let result = lua.create_table()?;
                result.set("relation", relation)?;
                if let Some(side) = contains {
                    result.set("contains", side)?;
                }
                Ok(mlua::Value::Table(result))
            },
        )?,
    )?;

    // `focus`：独立窗口 = 拿键盘焦点（`key` 载荷里会带 `window`）；辅助窗口 = 把逻辑焦点指定给它 /
    // 交还给宿主（所有权在宿主，资产自己说了不算）。
    let focus_requests = requests;
    handle.set(
        "focus",
        lua.create_function(move |_, (_this, focus): (mlua::Table, Option<bool>)| {
            focus_requests
                .borrow_mut()
                .push(Request::Focus(id, focus.unwrap_or(true)));
            Ok(())
        })?,
    )?;
    Ok(handle)
}

/// `create{…}` 的参数：`name` / `width` / `height` / `interactive` / `shadow`。
pub(crate) fn parse_window(options: &mlua::Table) -> mlua::Result<WindowSpec> {
    let name = options
        .get::<Option<String>>("name")?
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "脚本窗口".to_owned());
    Ok(WindowSpec {
        name,
        width: size(options, "width", 240.0)?,
        height: size(options, "height", 120.0)?,
        interactive: options.get::<Option<bool>>("interactive")?.unwrap_or(false),
        shadow: options.get::<Option<bool>>("shadow")?.unwrap_or(true),
        background: options.get::<Option<bool>>("background")?.unwrap_or(true),
    })
}

/// `create_auxiliary{…}` 的参数：`owner` 必给，尺寸 / 间隙可选。
pub(crate) fn parse_auxiliary(options: &mlua::Table, id: WindowId) -> mlua::Result<AuxiliarySpec> {
    let owner = match options.get::<mlua::Value>("owner")? {
        mlua::Value::Table(handle) => handle.get::<WindowId>("id").map_err(|_| {
            mlua::Error::RuntimeError(
                "owner 不是 cloudime.window.create 建出来的窗口句柄".to_owned(),
            )
        })?,
        mlua::Value::Integer(owner) if owner > 0 => owner as WindowId,
        _ => {
            return Err(mlua::Error::RuntimeError(
                "create_auxiliary 要给 owner（宿主：cloudime.window.create 建出来的那个窗口）"
                    .to_owned(),
            ));
        }
    };
    if owner == id {
        return Err(mlua::Error::RuntimeError(
            "create_auxiliary 的 owner 不能是它自己".to_owned(),
        ));
    }
    Ok(AuxiliarySpec {
        name: options
            .get::<Option<String>>("name")?
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "脚本辅助窗口".to_owned()),
        owner,
        width: optional_size(options, "width")?,
        height: optional_size(options, "height")?,
        gap: options
            .get::<Option<f32>>("gap")?
            .map(|gap| gap.clamp(0.0, 256.0)),
        interactive: options.get::<Option<bool>>("interactive")?.unwrap_or(false),
        shadow: options.get::<Option<bool>>("shadow")?.unwrap_or(true),
    })
}

/// 句柄表里记着的尺寸（`set_content` 没写 `width` / `height` 时用它）。
fn fallback_size(handle: &mlua::Table) -> (Option<f32>, Option<f32>) {
    (
        handle.get::<Option<f32>>("width").ok().flatten(),
        handle.get::<Option<f32>>("height").ok().flatten(),
    )
}

/// 读一份 JSON 布局并解析成 [`Document`]；错误里带上文件路径。
pub(crate) fn load_layout_document(
    lua: &mlua::Lua,
    path: &Path,
    fallback: (Option<f32>, Option<f32>),
    images: &mut ImageCache,
) -> mlua::Result<Document> {
    let layout = crate::layout_file::read(lua, path)?;
    let mut document = parse_document(&layout.table, fallback, images).map_err(|error| {
        mlua::Error::RuntimeError(format!("布局文件 {}：{error}", path.display()))
    })?;
    document.control_names = layout.control_names;
    Ok(document)
}

/// 把一份内容里的编辑框初始文本写进脚本侧缓存（换内容 / 建布局时用）。
pub(crate) fn cache_document_texts(document: &Document, id: WindowId, cache: &TextCache) {
    let mut cache = cache.borrow_mut();
    cache.retain(|(window, _), _| *window != id);
    for placement in document.text_edits() {
        let Some(Element::TextEdit(edit)) = document.element_at_path(&placement.path) else {
            continue;
        };
        if !edit.region_name.is_empty() {
            cache.insert((id, edit.region_name.clone()), edit.text.clone());
        }
    }
    for placement in document.text_boxes() {
        let Some(Element::TextBox(edit)) = document.element_at_path(&placement.path) else {
            continue;
        };
        if !edit.region_name.is_empty() {
            cache.insert((id, edit.region_name.clone()), edit.text.clone());
        }
    }
}

/// 内容表 → [`Document`]：`width` / `height` 可省（用句柄里那份），元素是 1 起的数组。
fn parse_document(
    content: &mlua::Table,
    fallback: (Option<f32>, Option<f32>),
    images: &mut ImageCache,
) -> mlua::Result<Document> {
    let width = content
        .get::<Option<f32>>("width")?
        .or(fallback.0)
        .unwrap_or(240.0);
    let height = content
        .get::<Option<f32>>("height")?
        .or(fallback.1)
        .unwrap_or(120.0);
    let width = check_size(width, "width")?;
    let height = check_size(height, "height")?;
    let mut elements = Vec::new();
    let mut regions = parse_regions(content)?;
    let mut button_regions = Vec::new();
    for entry in content.sequence_values::<mlua::Value>() {
        let entry = entry
            .map_err(|error| mlua::Error::RuntimeError(format!("内容里的元素读不动：{error}")))?;
        let mlua::Value::Table(table) = entry else {
            return Err(mlua::Error::RuntimeError(format!(
                "内容的第 {} 项不是表（元素要写成 {{ type = … }}）",
                elements.len() + 1
            )));
        };
        if elements.len() >= MAX_ELEMENTS {
            return Err(mlua::Error::RuntimeError(format!(
                "一份内容最多 {MAX_ELEMENTS} 个元素"
            )));
        }
        let mut element = parse_element(&table, images, 0)?;
        let handlers = parse_handlers(&table)?;
        if !handlers.is_empty()
            && !matches!(
                &element,
                Element::FlatSvgButton(_)
                    | Element::TextEdit(_)
                    | Element::TextBox(_)
                    | Element::ScrollBar(_)
            )
        {
            return Err(mlua::Error::RuntimeError(
                "events 只能写在 flat_svg_button / text_edit / text_box / scroll_bar 这类动态控件上；普通元素请用 regions"
                    .to_owned(),
            ));
        }
        if let Element::FlatSvgButton(button) = &mut element {
            if button.region_name.trim().is_empty() {
                button.region_name = format!("flat_svg_button_{}", elements.len() + 1);
            }
            if regions
                .iter()
                .chain(button_regions.iter())
                .any(|region: &Region| region.name == button.region_name)
            {
                return Err(mlua::Error::RuntimeError(format!(
                    "扁平按钮的区域名重复：{}",
                    button.region_name
                )));
            }
            let mut region = Region::new(
                button.region_name.clone(),
                button.hit_shape((0.0, 0.0)),
                false,
            );
            region.handlers = handlers.clone();
            button_regions.push(region);
        }
        if let Element::TextEdit(edit) = &mut element {
            if edit.region_name.trim().is_empty() {
                edit.region_name = format!("text_edit_{}", elements.len() + 1);
            }
            if regions
                .iter()
                .chain(button_regions.iter())
                .any(|region: &Region| region.name == edit.region_name)
            {
                return Err(mlua::Error::RuntimeError(format!(
                    "文本编辑框的区域名重复：{}",
                    edit.region_name
                )));
            }
            let mut region =
                Region::new(edit.region_name.clone(), edit.hit_shape((0.0, 0.0)), false);
            region.handlers = handlers.clone();
            button_regions.push(region);
        }
        if let Element::TextBox(edit) = &mut element {
            if edit.region_name.trim().is_empty() {
                edit.region_name = format!("text_box_{}", elements.len() + 1);
            }
            if regions
                .iter()
                .chain(button_regions.iter())
                .any(|region: &Region| region.name == edit.region_name)
            {
                return Err(mlua::Error::RuntimeError(format!(
                    "多行编辑框的区域名重复：{}",
                    edit.region_name
                )));
            }
            let mut region =
                Region::new(edit.region_name.clone(), edit.hit_shape((0.0, 0.0)), false);
            region.handlers = handlers.clone();
            button_regions.push(region);
        }
        if let Element::ScrollBar(bar) = &mut element {
            if bar.region_name.trim().is_empty() {
                bar.region_name = format!("scroll_bar_{}", elements.len() + 1);
            }
            if regions
                .iter()
                .chain(button_regions.iter())
                .any(|region: &Region| region.name == bar.region_name)
            {
                return Err(mlua::Error::RuntimeError(format!(
                    "滚动条的区域名重复：{}",
                    bar.region_name
                )));
            }
            let mut region = Region::new(bar.region_name.clone(), bar.hit_shape((0.0, 0.0)), false);
            region.handlers = handlers;
            button_regions.push(region);
        }
        elements.push(element);
    }
    regions.extend(button_regions);

    // 面板里的 `text_edit`：补自动区域名，并给命中测试补一块绝对坐标的区域。
    let mut path = Vec::new();
    assign_nested_text_names(&mut elements, &mut path);
    for region in nested_text_edit_regions(&elements, (0.0, 0.0), false) {
        if regions.iter().any(|existing| existing.name == region.name) {
            return Err(mlua::Error::RuntimeError(format!(
                "文本编辑框的区域名重复：{}",
                region.name
            )));
        }
        regions.push(region);
    }

    Ok(Document {
        width,
        height,
        elements,
        regions,
        control_names: Vec::new(),
    })
}

/// 嵌套 `text_edit` 没写 `name` 时用路径生成一个合法区域名（顶层仍用 `text_edit_N`）。
fn assign_nested_text_names(elements: &mut [Element], path: &mut Vec<usize>) {
    for (index, element) in elements.iter_mut().enumerate() {
        path.push(index);
        match element {
            Element::TextEdit(edit) if edit.region_name.is_empty() => {
                let suffix = path
                    .iter()
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join("_");
                edit.region_name = format!("text_edit_{suffix}");
            }
            Element::TextBox(edit) if edit.region_name.is_empty() => {
                let suffix = path
                    .iter()
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join("_");
                edit.region_name = format!("text_box_{suffix}");
            }
            Element::Panel(panel) => assign_nested_text_names(&mut panel.children, path),
            _ => {}
        }
        path.pop();
    }
}

fn nested_text_edit_regions(elements: &[Element], origin: (f32, f32), nested: bool) -> Vec<Region> {
    let mut regions = Vec::new();
    for element in elements {
        match element {
            Element::TextEdit(edit) if nested => {
                let mut region =
                    Region::new(edit.region_name.clone(), edit.hit_shape(origin), false);
                region.handlers = edit.handlers.clone();
                regions.push(region);
            }
            Element::TextBox(edit) if nested => {
                let mut region =
                    Region::new(edit.region_name.clone(), edit.hit_shape(origin), false);
                region.handlers = edit.handlers.clone();
                regions.push(region);
            }
            Element::Panel(panel) => regions.extend(nested_text_edit_regions(
                &panel.children,
                (origin.0 + panel.x, origin.1 + panel.y),
                true,
            )),
            _ => {}
        }
    }
    regions
}

/// 内容里的 `regions`：`{ { name = "plus", x = …, y = …, w = …, h = …, radius = …, drag = true } }`。
///
/// 区域是**鼠标点击 / 拖拽**的命中范围（元素只管画）。名字在同一份内容里唯一；
/// 后写的区域盖住先写的（与元素的绘制顺序一致）。
fn parse_regions(content: &mlua::Table) -> mlua::Result<Vec<Region>> {
    let Some(list) = content.get::<Option<mlua::Table>>("regions")? else {
        return Ok(Vec::new());
    };
    let mut regions: Vec<Region> = Vec::new();
    for entry in list.sequence_values::<mlua::Table>() {
        let entry = entry?;
        if regions.len() >= MAX_REGIONS {
            return Err(mlua::Error::RuntimeError(format!(
                "一份内容最多 {MAX_REGIONS} 个区域"
            )));
        }
        let name: String = entry.get("name").map_err(|_| {
            mlua::Error::RuntimeError("区域缺 name（同一份内容里唯一，事件里带它）".to_owned())
        })?;
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(mlua::Error::RuntimeError(
                "区域的 name 不能是空的".to_owned(),
            ));
        }
        if regions.iter().any(|region| region.name == name) {
            return Err(mlua::Error::RuntimeError(format!("区域名字重复：{name}")));
        }
        let shape = shape_from_table(&entry)?;
        let mut region = Region::new(
            name,
            shape,
            entry.get::<Option<bool>>("drag")?.unwrap_or(true),
        );
        region.handlers = parse_handlers(&entry)?;
        for shape in parse_region_shapes(&entry, "combine")? {
            region.combine(shape);
        }
        for shape in parse_region_shapes(&entry, "prune")? {
            region.prune(shape);
        }
        regions.push(region);
    }
    Ok(regions)
}

/// 事件表：`events = { mouse_clicked = "close", … }`；动作名必须是窗口事件认识的。
fn parse_handlers(table: &mlua::Table) -> mlua::Result<Vec<(String, String)>> {
    let Some(events) = table.get::<Option<mlua::Table>>("events")? else {
        return Ok(Vec::new());
    };
    let mut handlers = Vec::new();
    for entry in events.pairs::<String, String>() {
        let (action, handler) = entry?;
        let action = action.trim();
        let handler = handler.trim();
        let Some(action) = WindowAction::from_name(action) else {
            return Err(mlua::Error::RuntimeError(format!(
                "events 的事件名不认识：{action}（可用：click / right_click / hover / leave / wheel / \
                 focus / blur / mouse_enter / mouse_leave / mouse_pressed / mouse_released / \
                 mouse_clicked / mouse_double_clicked / text_changed / key_pressed / focused / unfocused）"
            )));
        };
        let action = action.as_str();
        if handler.is_empty() {
            return Err(mlua::Error::RuntimeError(format!(
                "events 的 {action} 处理器名不能为空"
            )));
        }
        if handlers.iter().any(|(name, _)| name == action) {
            return Err(mlua::Error::RuntimeError(format!(
                "events 的事件名重复：{action}"
            )));
        }
        if handlers.len() >= MAX_EVENT_HANDLERS {
            return Err(mlua::Error::RuntimeError(format!(
                "一个控件最多声明 {MAX_EVENT_HANDLERS} 个事件处理器"
            )));
        }
        handlers.push((action.to_owned(), handler.to_owned()));
    }
    Ok(handlers)
}

/// 一个形状表 → [`HitShape`]（区域 / 合并 / 剪除共用）。
///
/// 形状不必是矩形：`shape` 显式写 `rect` / `round_rect` / `circle` / `path`；不写就按字段猜
///（有 `commands` 当路径，有 `cx` / `cy` 当圆，其余当矩形 / 圆角矩形）。
fn shape_from_table(table: &mlua::Table) -> mlua::Result<HitShape> {
    match table.get::<Option<String>>("shape")?.as_deref() {
        Some("circle") => circle_shape(table),
        Some("path") => path_shape(table),
        Some("rect") | Some("round_rect") => rect_shape(table),
        Some(other) => Err(mlua::Error::RuntimeError(format!(
            "形状的 shape 不认识：{other}（可用：rect / round_rect / circle / path）"
        ))),
        None if has_field(table, "commands")? => path_shape(table),
        None if has_field(table, "cx")? || has_field(table, "cy")? => circle_shape(table),
        None => rect_shape(table),
    }
}

/// 表里有没有这个字段（`nil` 也算没有）。
fn has_field(table: &mlua::Table, key: &str) -> mlua::Result<bool> {
    Ok(!matches!(
        table.raw_get::<mlua::Value>(key)?,
        mlua::Value::Nil
    ))
}

/// 矩形 / 圆角矩形：`{ x, y, w, h, radius }`。
fn rect_shape(table: &mlua::Table) -> mlua::Result<HitShape> {
    let x = number(table, "x", 0.0)?;
    let y = number(table, "y", 0.0)?;
    let width = positive(table, "w", "width")?;
    let height = positive(table, "h", "height")?;
    let radius = number(table, "radius", 0.0)?.max(0.0);
    Ok(if radius > 0.0 {
        HitShape::RoundRect {
            x,
            y,
            width,
            height,
            radius,
        }
    } else {
        HitShape::Rect(HitRect::new(x, y, width, height))
    })
}

/// 圆：`{ cx, cy, radius }`。
fn circle_shape(table: &mlua::Table) -> mlua::Result<HitShape> {
    Ok(HitShape::Circle {
        cx: number(table, "cx", 0.0)?,
        cy: number(table, "cy", 0.0)?,
        radius: positive(table, "radius", "radius")?,
    })
}

/// 任意闭合路径：`{ commands = { {"M", …}, … } }`（与 `path` 元素同一套命令）。
fn path_shape(table: &mlua::Table) -> mlua::Result<HitShape> {
    let commands = parse_path(table)?;
    HitShape::from_path(&commands).ok_or_else(|| {
        mlua::Error::RuntimeError(
            "path 形状的 commands 画不出闭合区域（至少要有一段 M / L / Q / C / Z）".to_owned(),
        )
    })
}

/// `regions[i].combine` / `.prune`：可以写一个形状表，也可以写一张形状表的数组。
fn parse_region_shapes(entry: &mlua::Table, key: &str) -> mlua::Result<Vec<HitShape>> {
    let mlua::Value::Table(table) = entry.get::<mlua::Value>(key)? else {
        return Ok(Vec::new());
    };
    // 单个形状：带了形状字段就是形状本身，否则当数组
    if has_field(&table, "x")?
        || has_field(&table, "w")?
        || has_field(&table, "cx")?
        || has_field(&table, "cy")?
        || has_field(&table, "shape")?
        || has_field(&table, "commands")?
    {
        return Ok(vec![shape_from_table(&table)?]);
    }
    let mut shapes = Vec::new();
    for item in table.sequence_values::<mlua::Table>() {
        if shapes.len() >= MAX_REGION_PARTS {
            return Err(mlua::Error::RuntimeError(format!(
                "一个区域最多 {MAX_REGION_PARTS} 个合并 / 剪除形状"
            )));
        }
        shapes.push(shape_from_table(&item?)?);
    }
    Ok(shapes)
}

/// 一个元素：按 `type` 分派到七类里的一类。
fn parse_element(
    table: &mlua::Table,
    images: &mut ImageCache,
    depth: usize,
) -> mlua::Result<Element> {
    let kind: String = table.get("type").map_err(|_| {
        mlua::Error::RuntimeError(
            "元素缺 type（可用：rect / circle / line / text / svg / img / path）".to_owned(),
        )
    })?;
    match kind.as_str() {
        "rect" => Ok(Element::Rect {
            x: number(table, "x", 0.0)?,
            y: number(table, "y", 0.0)?,
            width: positive(table, "w", "width")?,
            height: positive(table, "h", "height")?,
            radius: number(table, "radius", 0.0)?.max(0.0),
            color: color(table, "color", true)?.expect("要了颜色"),
        }),
        "circle" => Ok(Element::Circle {
            cx: number(table, "cx", 0.0)?,
            cy: number(table, "cy", 0.0)?,
            radius: positive(table, "radius", "radius")?,
            color: color(table, "color", true)?.expect("要了颜色"),
        }),
        "line" => Ok(Element::Line {
            from: point(table, "from")?,
            to: point(table, "to")?,
            thickness: number(table, "thickness", 1.0)?.max(0.5),
            color: color(table, "color", true)?.expect("要了颜色"),
        }),
        "text" => {
            let text: String = table
                .get("text")
                .map_err(|_| mlua::Error::RuntimeError("text 元素缺 text".to_owned()))?;
            if text.chars().count() > MAX_TEXT {
                return Err(mlua::Error::RuntimeError(format!(
                    "一段文字最多 {MAX_TEXT} 个字符"
                )));
            }
            let size = number(table, "size", 16.0)?.clamp(1.0, 256.0);
            let slot = match table.get::<Option<String>>("font")? {
                None => FontSlot::Candidate,
                Some(name) => FontSlot::from_name(&name).ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "font 不认识：{name}（可用：candidate / pinyin / item_number / translate）"
                    ))
                })?,
            };
            Ok(Element::Text {
                x: number(table, "x", 0.0)?,
                y: number(table, "y", 0.0)?,
                text,
                size,
                slot,
                color: color(table, "color", false)?.unwrap_or(Color::rgb(0, 0, 0)),
                italic: table.get::<Option<bool>>("italic")?.unwrap_or(false),
            })
        }
        "svg" => {
            let source = match table.get::<Option<String>>("source")? {
                Some(source) => source,
                None => {
                    let path: String = table.get("path").map_err(|_| {
                        mlua::Error::RuntimeError(
                            "svg 元素缺 source（整段 SVG 文本）或 path（SVG 文件路径）".to_owned(),
                        )
                    })?;
                    read_svg(&path)?
                }
            };
            Ok(Element::Svg {
                x: number(table, "x", 0.0)?,
                y: number(table, "y", 0.0)?,
                size: positive(table, "size", "size")?,
                source,
                tint: color(table, "tint", false)?,
            })
        }
        "img" => {
            let path: String = table.get("path").map_err(|_| {
                mlua::Error::RuntimeError("img 元素缺 path（PNG 文件路径）".to_owned())
            })?;
            Ok(Element::Image {
                x: number(table, "x", 0.0)?,
                y: number(table, "y", 0.0)?,
                width: positive(table, "w", "width")?,
                height: positive(table, "h", "height")?,
                image: images.load(Path::new(&path))?,
            })
        }
        "path" => {
            let commands = parse_path(table)?;
            Ok(Element::Path {
                commands,
                color: color(table, "color", true)?.expect("要了颜色"),
            })
        }
        "label" => parse_label(table),
        "text_block" => parse_text_block(table),
        "static_shape" => parse_static_shape(table),
        "static_image" => parse_static_image(table, images),
        "panel" => parse_panel(table, images, depth),
        "flat_svg_button" => parse_flat_svg_button(table),
        "text_edit" => parse_text_edit(table),
        "text_box" => parse_text_box(table),
        "hscroll_bar" | "hscrollbar" => parse_scroll_bar(table, ScrollAxis::Horizontal),
        "vscroll_bar" | "vhscroll_bar" | "vhsroll_bar" => {
            parse_scroll_bar(table, ScrollAxis::Vertical)
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "元素 type 不认识：{other}（可用：rect / circle / line / text / svg / img / path / \
             label / text_block / static_shape / static_image / panel / flat_svg_button / text_edit / \
             hscroll_bar / vscroll_bar）"
        ))),
    }
}

/// `flat_svg_button`：背景形状 + 自动区域 + SVG，三种鼠标状态各有一套颜色。
fn parse_flat_svg_button(table: &mlua::Table) -> mlua::Result<Element> {
    let svg = match table.get::<Option<String>>("svg")? {
        Some(source) => source,
        None => {
            let path: String = table.get("svg_path").map_err(|_| {
                mlua::Error::RuntimeError(
                    "flat_svg_button 缺 svg（整段 SVG）或 svg_path（SVG 文件）".to_owned(),
                )
            })?;
            read_svg(&path)?
        }
    };
    let art = svg_size(&svg).ok_or_else(|| {
        mlua::Error::RuntimeError("flat_svg_button 的 SVG 解析失败（拿不到画板尺寸）".to_owned())
    })?;
    if !(art.0.is_finite() && art.1.is_finite() && art.0 > 0.0 && art.1 > 0.0) {
        return Err(mlua::Error::RuntimeError(
            "flat_svg_button 的 SVG 画板尺寸不对".to_owned(),
        ));
    }
    let explicit_width = optional_positive(table, "w")?;
    let explicit_height = optional_positive(table, "h")?;
    let (width, height) = match (explicit_width, explicit_height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (width, width * art.1 / art.0),
        (None, Some(height)) => (height * art.0 / art.1, height),
        (None, None) => art,
    };
    let shape = match table
        .get::<Option<String>>("shape")?
        .as_deref()
        .unwrap_or("rect")
    {
        "rect" => ButtonShape::Rect,
        "round_rect" => ButtonShape::RoundRect {
            radius: number(table, "radius", 3.0)?.max(0.0),
        },
        "circle" => ButtonShape::Circle {
            radius: number(table, "radius", width.min(height) * 0.5)?.max(0.0),
        },
        "path" => ButtonShape::Path {
            commands: parse_path(table)?,
        },
        other => {
            return Err(mlua::Error::RuntimeError(format!(
                "flat_svg_button 的 shape 不认识：{other}（可用：rect / round_rect / circle / path）"
            )));
        }
    };
    Ok(Element::FlatSvgButton(FlatSvgButton {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        width,
        height,
        shape,
        svg,
        svg_size: art,
        normal: button_colors(
            table,
            "normal",
            Color::rgba(0, 0, 0, 0),
            Color::rgb(0, 0, 0),
        )?,
        hovered: button_colors(
            table,
            "hovered",
            Color::rgba(0, 0, 0, 0x1F),
            Color::rgb(0, 0, 0),
        )?,
        pressed: button_colors(
            table,
            "pressed",
            Color::rgba(0, 0, 0, 0x33),
            Color::rgb(0, 0, 0),
        )?,
        state: ButtonState::Normal,
        region_name: table.get::<Option<String>>("name")?.unwrap_or_default(),
    }))
}

/// `text_edit`：背景胶囊 + 可选 SVG + Windowless RichEdit 承载的文本区。
fn parse_text_edit(table: &mlua::Table) -> mlua::Result<Element> {
    let width = positive(table, "w", "width")?;
    let height = positive(table, "h", "height")?;
    let (svg, svg_size, svg_align) = match table.get::<Option<String>>("svg")? {
        Some(source) => {
            let size = svg_size(&source).ok_or_else(|| {
                mlua::Error::RuntimeError("text_edit 的 SVG 解析失败（拿不到画板尺寸）".to_owned())
            })?;
            (Some(source), size, parse_svg_align(table)?)
        }
        None => {
            if let Some(path) = table.get::<Option<String>>("svg_path")? {
                let source = read_svg(&path)?;
                let size = svg_size(&source).ok_or_else(|| {
                    mlua::Error::RuntimeError(
                        "text_edit 的 SVG 文件解析失败（拿不到画板尺寸）".to_owned(),
                    )
                })?;
                (Some(source), size, parse_svg_align(table)?)
            } else {
                (None, (height, height), SvgAlign::Left)
            }
        }
    };
    let border_width = match table.get::<Option<f32>>("edge_width")? {
        Some(width) => width,
        None => number(table, "border_width", 1.0)?,
    }
    .clamp(0.0, 16.0);
    let font_size = table
        .get::<Option<f32>>("font_size")?
        .or(table.get::<Option<f32>>("size")?)
        .map(|size| size.clamp(1.0, 256.0));
    let font_flag = table.get::<Option<u8>>("font_flag")?.unwrap_or(0);
    if font_flag & !0x0F != 0 {
        return Err(mlua::Error::RuntimeError(format!(
            "font_flag 只能是 0–15 的组合（1 加粗 / 2 斜体 / 4 下划线 / 8 删除线），现在是 {font_flag}"
        )));
    }
    let font_name = table
        .get::<Option<String>>("font_name")?
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    Ok(Element::TextEdit(TextEdit {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        width,
        height,
        radius: number(table, "radius", height * 0.5)?.max(0.0),
        fill: color(table, "back_color", false)?.or(color(table, "fill", false)?),
        border: color(table, "edge_color", false)?.or(color(table, "color", false)?),
        border_width,
        svg,
        svg_size,
        svg_align,
        svg_tint: color(table, "fill_color", false)?.or(color(table, "svg_color", false)?),
        selection_background: color(table, "sel_back_color", false)?.or(color(
            table,
            "selection_background",
            false,
        )?),
        selection_color: color(table, "sel_text_color", false)?.or(color(
            table,
            "selection_color",
            false,
        )?),
        caret_color: color(table, "cursor_color", false)?.or(color(table, "caret_color", false)?),
        text_color: color(table, "text_color", false)?,
        read_only: table.get::<Option<bool>>("read_only")?.unwrap_or(false),
        passkey: table
            .get::<Option<bool>>("passkey")?
            .or(table.get::<Option<bool>>("password")?)
            .unwrap_or(false),
        font_name,
        font_flag,
        handlers: parse_handlers(table)?,
        text: table.get::<Option<String>>("text")?.unwrap_or_default(),
        font: font_slot(table)?,
        size: font_size,
        region_name: table.get::<Option<String>>("name")?.unwrap_or_default(),
    }))
}

fn parse_text_box(table: &mlua::Table) -> mlua::Result<Element> {
    let width = positive(table, "w", "width")?;
    let height = positive(table, "h", "height")?;
    let border_width = match table.get::<Option<f32>>("edge_width")? {
        Some(width) => width,
        None => number(table, "border_width", 1.0)?,
    }
    .clamp(0.0, 16.0);
    let font_size = table
        .get::<Option<f32>>("font_size")?
        .or(table.get::<Option<f32>>("size")?)
        .map(|size| size.clamp(1.0, 256.0));
    let font_flag = table.get::<Option<u8>>("font_flag")?.unwrap_or(0);
    if font_flag & !0x0F != 0 {
        return Err(mlua::Error::RuntimeError(format!(
            "font_flag 只能是 0–15 的组合（1 加粗 / 2 斜体 / 4 下划线 / 8 删除线），现在是 {font_flag}"
        )));
    }
    let font_name = table
        .get::<Option<String>>("font_name")?
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    let line_spacing = table
        .get::<Option<u64>>("line_spacing")?
        .unwrap_or(8)
        .min(u16::MAX as u64) as u16;
    Ok(Element::TextBox(TextBox {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        width,
        height,
        radius: number(table, "radius", 6.0)?.max(0.0),
        fill: color(table, "back_color", false)?.or(color(table, "fill", false)?),
        border: color(table, "edge_color", false)?.or(color(table, "color", false)?),
        border_width,
        selection_background: color(table, "sel_back_color", false)?.or(color(
            table,
            "selection_background",
            false,
        )?),
        selection_color: color(table, "sel_text_color", false)?.or(color(
            table,
            "selection_color",
            false,
        )?),
        caret_color: color(table, "cursor_color", false)?.or(color(table, "caret_color", false)?),
        text_color: color(table, "text_color", false)?,
        read_only: table.get::<Option<bool>>("read_only")?.unwrap_or(false),
        auto_line_wrap: table.get::<Option<bool>>("auto_line_wrap")?.unwrap_or(true),
        line_spacing,
        font_name,
        font_flag,
        handlers: parse_handlers(table)?,
        text: table.get::<Option<String>>("text")?.unwrap_or_default(),
        font: font_slot(table)?,
        size: font_size,
        region_name: table.get::<Option<String>>("name")?.unwrap_or_default(),
        h_scroll: TextBoxScroll::default(),
        v_scroll: TextBoxScroll::default(),
    }))
}

/// `hscroll_bar` / `vscroll_bar`：整条背景胶囊 + 滑块胶囊；值域 `min_value..=max_value`。
fn parse_scroll_bar(table: &mlua::Table, axis: ScrollAxis) -> mlua::Result<Element> {
    let min = u64_value(table, "min_value", 0)?;
    let max = u64_value(table, "max_value", 100)?;
    if max < min {
        return Err(mlua::Error::RuntimeError(format!(
            "滚动条的 max_value（{max}）不能小于 min_value（{min}）"
        )));
    }
    let value = u64_value(table, "value", 0)?.clamp(min, max);
    let step = u64_value(table, "step", 1)?;
    if step == 0 {
        return Err(mlua::Error::RuntimeError(
            "滚动条的 step 不能为 0".to_owned(),
        ));
    }
    Ok(Element::ScrollBar(ScrollBar {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        width: optional_positive(table, "w")?.unwrap_or(8.0),
        height: optional_positive(table, "h")?.unwrap_or(8.0),
        background: color(table, "background_color", false)?
            .or(color(table, "back_color", false)?)
            .or(color(table, "background", false)?)
            .or(color(table, "color", false)?)
            .or(color(table, "fill", false)?),
        thumb: color(table, "thumb_color", false)?
            .or(color(table, "slider_color", false)?)
            .or(color(table, "thumb", false)?)
            .or(color(table, "highlight", false)?),
        axis,
        value,
        min,
        max,
        step,
        region_name: table.get::<Option<String>>("name")?.unwrap_or_default(),
    }))
}

/// 非负整数属性（Lua 数字可能是浮点；四舍五入后夹到 `u64`）。
fn u64_value(table: &mlua::Table, key: &str, default: u64) -> mlua::Result<u64> {
    let Some(value) = table.get::<Option<f64>>(key)? else {
        return Ok(default);
    };
    if !value.is_finite() || value < 0.0 {
        return Err(mlua::Error::RuntimeError(format!(
            "滚动条 {key} 要是非负数（现在是 {value}）"
        )));
    }
    Ok(value.round().min(u64::MAX as f64) as u64)
}

fn parse_svg_align(table: &mlua::Table) -> mlua::Result<SvgAlign> {
    match table
        .get::<Option<String>>("svg_align")?
        .as_deref()
        .unwrap_or("left")
    {
        "left" => Ok(SvgAlign::Left),
        "right" => Ok(SvgAlign::Right),
        other => Err(mlua::Error::RuntimeError(format!(
            "svg_align 不认识：{other}（可用：left / right）"
        ))),
    }
}

/// 按钮某一状态的颜色：`{ background = …, svg = … }`。
fn button_colors(
    table: &mlua::Table,
    key: &str,
    background: Color,
    svg: Color,
) -> mlua::Result<ButtonColors> {
    let Some(state) = table.get::<Option<mlua::Table>>(key)? else {
        return Ok(ButtonColors { background, svg });
    };
    Ok(ButtonColors {
        background: color(&state, "background", false)?.unwrap_or(background),
        svg: color(&state, "svg", false)?.unwrap_or(svg),
    })
}

/// `label`：一行文字，`runs` 里每段自带颜色。
fn parse_label(table: &mlua::Table) -> mlua::Result<Element> {
    let runs = table.get::<Option<mlua::Table>>("runs")?.ok_or_else(|| {
        mlua::Error::RuntimeError("label 缺 runs（每段 { text, color }）".to_owned())
    })?;
    let mut parsed = Vec::new();
    for entry in runs.sequence_values::<mlua::Table>() {
        if parsed.len() >= MAX_LABEL_RUNS {
            return Err(mlua::Error::RuntimeError(format!(
                "一个 label 最多 {MAX_LABEL_RUNS} 段文字"
            )));
        }
        let entry = entry?;
        let text: String = entry
            .get("text")
            .map_err(|_| mlua::Error::RuntimeError("label 的 runs 里有一段缺 text".to_owned()))?;
        check_text(&text)?;
        parsed.push(LabelRun {
            text,
            color: color(&entry, "color", false)?.unwrap_or(Color::rgb(0, 0, 0)),
        });
    }
    if parsed.is_empty() {
        return Err(mlua::Error::RuntimeError(
            "label 的 runs 不能是空的".to_owned(),
        ));
    }
    Ok(Element::Label(Label {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        size: number(table, "size", 16.0)?.clamp(1.0, 256.0),
        slot: font_slot(table)?,
        runs: parsed,
    }))
}

/// `text_block`：多行文本，统一颜色、行距可调。
fn parse_text_block(table: &mlua::Table) -> mlua::Result<Element> {
    let text: String = table
        .get("text")
        .map_err(|_| mlua::Error::RuntimeError("text_block 缺 text".to_owned()))?;
    check_text(&text)?;
    let size = number(table, "size", 16.0)?.clamp(1.0, 256.0);
    Ok(Element::TextBlock(TextBlock {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        size,
        slot: font_slot_or(table, FontSlot::ItemNumber)?,
        line_spacing: number(table, "line_spacing", 2.0)?.clamp(-size * 2.0, size * 4.0),
        color: color(table, "color", false)?.unwrap_or(Color::rgb(0, 0, 0)),
        text,
    }))
}

/// `static_shape`：`shape = "line" | "rect" | "round_rect" | "circle" | "path"`。
fn parse_static_shape(table: &mlua::Table) -> mlua::Result<Element> {
    let kind: String = table
        .get("shape")
        .map_err(|_| mlua::Error::RuntimeError("static_shape 缺 shape".to_owned()))?;
    let color = || {
        color(table, "color", true)?
            .ok_or_else(|| mlua::Error::RuntimeError("static_shape 缺 color".to_owned()))
    };
    let shape = match kind.as_str() {
        "line" => StaticShape::Line {
            from: point(table, "from")?,
            to: point(table, "to")?,
            thickness: number(table, "thickness", 1.0)?.max(0.5),
            color: color()?,
        },
        "rect" => StaticShape::Rect {
            x: number(table, "x", 0.0)?,
            y: number(table, "y", 0.0)?,
            width: positive(table, "w", "width")?,
            height: positive(table, "h", "height")?,
            color: color()?,
        },
        "round_rect" => StaticShape::RoundRect {
            x: number(table, "x", 0.0)?,
            y: number(table, "y", 0.0)?,
            width: positive(table, "w", "width")?,
            height: positive(table, "h", "height")?,
            radius: number(table, "radius", 6.0)?.max(0.0),
            color: color()?,
        },
        "circle" => StaticShape::Circle {
            cx: number(table, "cx", 0.0)?,
            cy: number(table, "cy", 0.0)?,
            radius: positive(table, "radius", "radius")?,
            color: color()?,
        },
        "path" => StaticShape::Path {
            commands: parse_path(table)?,
            color: color()?,
        },
        other => {
            return Err(mlua::Error::RuntimeError(format!(
                "static_shape 的 shape 不认识：{other}（可用：line / rect / round_rect / circle / path）"
            )));
        }
    };
    Ok(Element::StaticShape(shape))
}

/// `static_image`：`image` 给 `cloudime.image.load` 的句柄，或 `path` 给文件（PNG / BMP / JPG）。
fn parse_static_image(table: &mlua::Table, images: &mut ImageCache) -> mlua::Result<Element> {
    let image = match table.get::<mlua::Value>("image")? {
        mlua::Value::UserData(handle) => handle.borrow::<ImageHandle>()?.snapshot(),
        mlua::Value::Nil => {
            let path: String = table.get("path").map_err(|_| {
                mlua::Error::RuntimeError("static_image 缺 image 或 path".to_owned())
            })?;
            images.load(Path::new(&path))?
        }
        other => {
            return Err(mlua::Error::RuntimeError(format!(
                "static_image 的 image 要 cloudime.image.load 的句柄，给的是 {}",
                other.type_name()
            )));
        }
    };
    Ok(Element::StaticImage(StaticImage {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        width: optional_positive(table, "w")?.unwrap_or(image.width() as f32),
        height: optional_positive(table, "h")?.unwrap_or(image.height() as f32),
        image,
    }))
}

/// `panel`：容器 + 背景，`children` 里是面板内相对坐标的元素。
fn parse_panel(
    table: &mlua::Table,
    images: &mut ImageCache,
    depth: usize,
) -> mlua::Result<Element> {
    if depth >= MAX_PANEL_DEPTH {
        return Err(mlua::Error::RuntimeError(format!(
            "panel 最多套 {MAX_PANEL_DEPTH} 层"
        )));
    }
    let mut children = Vec::new();
    if let Some(list) = table.get::<Option<mlua::Table>>("children")? {
        for child in list.sequence_values::<mlua::Table>() {
            if children.len() >= MAX_ELEMENTS {
                return Err(mlua::Error::RuntimeError(format!(
                    "一个 panel 最多 {MAX_ELEMENTS} 个子控件"
                )));
            }
            children.push(parse_element(&child?, images, depth + 1)?);
        }
    }
    Ok(Element::Panel(Panel {
        x: number(table, "x", 0.0)?,
        y: number(table, "y", 0.0)?,
        width: positive(table, "w", "width")?,
        height: positive(table, "h", "height")?,
        radius: number(table, "radius", 6.0)?.max(0.0),
        // 缺省只画边缘：`#FFE0E0E0`、1 点；要填色才写 `fill`。
        color: color(table, "color", false)?.unwrap_or(Color::rgba(0xE0, 0xE0, 0xE0, 0xFF)),
        border_width: number(table, "border_width", 1.0)?.clamp(0.0, 16.0),
        fill: color(table, "fill", false)?,
        children,
    }))
}

/// 单个控件允许的最大字符数（与 `text` 元素共用 [`MAX_TEXT`]）。
fn check_text(text: &str) -> mlua::Result<()> {
    if text.chars().count() > MAX_TEXT {
        return Err(mlua::Error::RuntimeError(format!(
            "一段文字最多 {MAX_TEXT} 个字符"
        )));
    }
    Ok(())
}

/// 控件与 `text` 元素共用的字体槽位解析（缺省 [`FontSlot::Candidate`]）。
fn font_slot(table: &mlua::Table) -> mlua::Result<FontSlot> {
    font_slot_or(table, FontSlot::Candidate)
}

/// 同上，但可以指定缺省槽位（`text_block` 缺省用序号字体）。
fn font_slot_or(table: &mlua::Table, default: FontSlot) -> mlua::Result<FontSlot> {
    match table.get::<Option<String>>("font")? {
        None => Ok(default),
        Some(name) => FontSlot::from_name(&name).ok_or_else(|| {
            mlua::Error::RuntimeError(format!(
                "font 不认识：{name}（可用：candidate / pinyin / item_number / translate）"
            ))
        }),
    }
}

/// 从文件读一段 SVG 文本（`svg` 元素的 `path`）。
fn read_svg(path: &str) -> mlua::Result<String> {
    let path = Path::new(path);
    let size = std::fs::metadata(path)
        .map_err(|error| {
            mlua::Error::RuntimeError(format!("svg 文件读不到：{}（{error}）", path.display()))
        })?
        .len();
    if size > MAX_SVG_BYTES {
        return Err(mlua::Error::RuntimeError(format!(
            "svg 文件太大：{}（{size} 字节，最多 {MAX_SVG_BYTES}）",
            path.display()
        )));
    }
    std::fs::read(path)
        .map_err(|error| {
            mlua::Error::RuntimeError(format!("svg 文件读不到：{}（{error}）", path.display()))
        })
        .and_then(|bytes| {
            String::from_utf8(bytes).map_err(|error| {
                mlua::Error::RuntimeError(format!(
                    "svg 文件不是 UTF-8：{}（{error}）",
                    path.display()
                ))
            })
        })
}

/// `path` 的 `commands`：`{ {"M", 0, 0}, {"L", 10, 0}, {"C", …}, {"Z"} }`。
fn parse_path(table: &mlua::Table) -> mlua::Result<Vec<PathCommand>> {
    let list: mlua::Table = table
        .get("commands")
        .map_err(|_| mlua::Error::RuntimeError("path 缺 commands".to_owned()))?;
    parse_path_commands(&list)
}

/// 路径命令数组 → [`PathCommand`]（`path` 元素与图像蒙版共用）。
fn parse_path_commands(list: &mlua::Table) -> mlua::Result<Vec<PathCommand>> {
    let mut commands = Vec::new();
    for entry in list.sequence_values::<mlua::Table>() {
        let entry = entry?;
        let verb: String = entry.get(1)?;
        let arg = |index: usize| -> mlua::Result<f32> {
            entry.get::<Option<f32>>(index)?.ok_or_else(|| {
                mlua::Error::RuntimeError(format!("路径命令 {verb} 缺第 {index} 个坐标"))
            })
        };
        commands.push(match verb.to_ascii_uppercase().as_str() {
            "M" => PathCommand::MoveTo(arg(2)?, arg(3)?),
            "L" => PathCommand::LineTo(arg(2)?, arg(3)?),
            "Q" => PathCommand::QuadTo(arg(2)?, arg(3)?, arg(4)?, arg(5)?),
            "C" => PathCommand::CubicTo(arg(2)?, arg(3)?, arg(4)?, arg(5)?, arg(6)?, arg(7)?),
            "Z" => PathCommand::Close,
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "路径命令不认识：{other}（可用：M / L / Q / C / Z）"
                )));
            }
        });
    }
    if commands.is_empty() {
        return Err(mlua::Error::RuntimeError(
            "path 的 commands 是空的".to_owned(),
        ));
    }
    Ok(commands)
}

/// `show{ x = …, y = … }`：屏幕坐标（点）。两个都给才行。
fn parse_position(table: &mlua::Table) -> mlua::Result<(f32, f32)> {
    Ok((point_value(table, "x")?, point_value(table, "y")?))
}

fn point_value(table: &mlua::Table, key: &str) -> mlua::Result<f32> {
    table
        .get::<Option<f32>>(key)?
        .ok_or_else(|| mlua::Error::RuntimeError(format!("show 的位置要给全：缺 {key}")))
}

/// `from = { x = 0, y = 0 }`，也接受 `from = { 0, 0 }`。
fn point(table: &mlua::Table, key: &str) -> mlua::Result<(f32, f32)> {
    let value: mlua::Table = table
        .get(key)
        .map_err(|_| mlua::Error::RuntimeError(format!("line 元素缺 {key}（{{x, y}}）")))?;
    match (
        value.get::<Option<f32>>("x")?,
        value.get::<Option<f32>>("y")?,
    ) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Ok((
            value
                .get::<Option<f32>>(1)?
                .ok_or_else(|| mlua::Error::RuntimeError(format!("{key} 缺 x 坐标")))?,
            value
                .get::<Option<f32>>(2)?
                .ok_or_else(|| mlua::Error::RuntimeError(format!("{key} 缺 y 坐标")))?,
        )),
    }
}

/// 数字字段：没写用 `default`。
fn number(table: &mlua::Table, key: &str, default: f32) -> mlua::Result<f32> {
    Ok(table.get::<Option<f32>>(key)?.unwrap_or(default))
}

/// 必给的正尺寸：先认 `first`（如 `w`），没有再认 `second`（如 `width`）。
fn positive(table: &mlua::Table, first: &str, second: &str) -> mlua::Result<f32> {
    let value = table
        .get::<Option<f32>>(first)?
        .or(table.get::<Option<f32>>(second)?)
        .ok_or_else(|| mlua::Error::RuntimeError(format!("元素缺尺寸 {first}")))?;
    if !(value.is_finite() && value > 0.0) {
        return Err(mlua::Error::RuntimeError(format!(
            "元素的尺寸 {first} 要是正数（现在是 {value}）"
        )));
    }
    Ok(value)
}

/// 可选的正尺寸；没写返回 `None`，写了但 <= 0 / 非有限数报错。
fn optional_positive(table: &mlua::Table, key: &str) -> mlua::Result<Option<f32>> {
    let Some(value) = table.get::<Option<f32>>(key)? else {
        return Ok(None);
    };
    if !(value.is_finite() && value > 0.0) {
        return Err(mlua::Error::RuntimeError(format!(
            "元素的尺寸 {key} 要是正数（现在是 {value}）"
        )));
    }
    Ok(Some(value))
}

fn size(table: &mlua::Table, key: &str, default: f32) -> mlua::Result<f32> {
    check_size(table.get::<Option<f32>>(key)?.unwrap_or(default), key)
}

fn optional_size(table: &mlua::Table, key: &str) -> mlua::Result<Option<f32>> {
    table
        .get::<Option<f32>>(key)?
        .map(|value| check_size(value, key))
        .transpose()
}

fn check_size(value: f32, key: &str) -> mlua::Result<f32> {
    if !value.is_finite() {
        return Err(mlua::Error::RuntimeError(format!("尺寸 {key} 不是有限数")));
    }
    Ok(value.clamp(MIN_SIZE, MAX_SIZE))
}

/// 颜色：`"#RRGGBB"` / `"#AARRGGBB"` / `{ r = …, g = …, b = …, a = … }`。
/// `required` = 没写就报错（形状类元素），否则给 `None`（文字用缺省色）。
fn color(table: &mlua::Table, key: &str, required: bool) -> mlua::Result<Option<Color>> {
    match table.get::<mlua::Value>(key)? {
        mlua::Value::Nil if !required => Ok(None),
        mlua::Value::Nil => Err(mlua::Error::RuntimeError(format!(
            "元素缺颜色 {key}（\"#RRGGBB\" 或 \"#AARRGGBB\"）"
        ))),
        mlua::Value::String(text) => {
            let text = text
                .to_str()
                .map_err(|_| mlua::Error::RuntimeError(format!("颜色 {key} 不是合法 UTF-8")))?;
            parse_hex_color(&text).map(Some).ok_or_else(|| {
                mlua::Error::RuntimeError(format!(
                    "颜色 {key} 写法不认识：{text}（要 \"#RRGGBB\" 或 \"#AARRGGBB\"）"
                ))
            })
        }
        mlua::Value::Table(fields) => {
            let channel = |name: &str, default: u8| -> mlua::Result<u8> {
                Ok(fields
                    .get::<Option<f32>>(name)?
                    .map(|value| value.clamp(0.0, 255.0).round() as u8)
                    .unwrap_or(default))
            };
            Ok(Some(Color::rgba(
                channel("r", 0)?,
                channel("g", 0)?,
                channel("b", 0)?,
                channel("a", 255)?,
            )))
        }
        value => Err(mlua::Error::RuntimeError(format!(
            "颜色 {key} 要是字符串或表，不是 {}",
            value.type_name()
        ))),
    }
}

/// `#RGB` / `#RRGGBB` / `#AARRGGBB`（`#` 可省，大小写都行）。
fn parse_hex_color(text: &str) -> Option<Color> {
    let hex = text.trim().trim_start_matches('#');
    if !hex.is_ascii() {
        return None;
    }
    let parse = |slice: &str| u8::from_str_radix(slice, 16).ok();
    match hex.len() {
        3 => {
            let mut channels = [0u8; 3];
            for (index, channel) in channels.iter_mut().enumerate() {
                let digit = parse(&hex[index..index + 1])?;
                *channel = digit * 17;
            }
            Some(Color::rgba(channels[0], channels[1], channels[2], 255))
        }
        6 => Some(Color::rgba(
            parse(&hex[0..2])?,
            parse(&hex[2..4])?,
            parse(&hex[4..6])?,
            255,
        )),
        8 => Some(Color::rgba(
            parse(&hex[2..4])?,
            parse(&hex[4..6])?,
            parse(&hex[6..8])?,
            parse(&hex[0..2])?,
        )),
        _ => None,
    }
}

/// 脚本窗口上的鼠标 / 键盘动作：UI 线程判定，工人线程再派给脚本（`cloudime.on("window", …)`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    /// 左键单击（按下又抬起，且没拖动过）。
    Click,

    /// 右键单击。
    RightClick,

    /// 鼠标停在与上一次不同的元素上（同一元素里移动不重复派）。
    Hover,

    /// 鼠标离开了窗口。
    Leave,

    /// 滚轮：`event.delta` 是滚动量（一格 = ±120，正数往上 / 往远离用户的方向）。
    Wheel,

    /// 这块窗口拿到了**键盘焦点**（点它、或脚本 `win:focus()`）。
    Focus,

    /// 键盘焦点离开（点了别的窗口、收起 / 销毁、或脚本 `win:focus(false)`）。
    Blur,

    /// 鼠标进入扁平按钮区域。
    MouseEnter,

    /// 鼠标离开扁平按钮区域。
    MouseLeave,

    /// 在扁平按钮上按下鼠标（修饰键在载荷 `ctrl` / `alt` / `shift` 里）。
    MousePressed,

    /// 在扁平按钮上松开鼠标。
    MouseReleased,

    /// 在扁平按钮上单击（没有拖拽、没有修饰键）。
    MouseClicked,

    /// 在扁平按钮上双击（第二次 `mouse_clicked`，没有修饰键）。
    MouseDoubleClicked,

    /// `text_edit` 里的文字或选区变了（`event.control` / `event.text` 一起给）。
    TextChanged,

    /// `text_edit` 按键按下；`event.key` / `event.modifier`。
    KeyPressed,

    /// `text_edit` 获得焦点。
    Focused,

    /// `text_edit` 失去焦点。
    Unfocused,

    /// 滚动条的值变了；`event.value`。
    ValueChanged,
}

impl WindowAction {
    /// 载荷里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::RightClick => "right_click",
            Self::Hover => "hover",
            Self::Leave => "leave",
            Self::Wheel => "wheel",
            Self::Focus => "focus",
            Self::Blur => "blur",
            Self::MouseEnter => "mouse_enter",
            Self::MouseLeave => "mouse_leave",
            Self::MousePressed => "mouse_pressed",
            Self::MouseReleased => "mouse_released",
            Self::MouseClicked => "mouse_clicked",
            Self::MouseDoubleClicked => "mouse_double_clicked",
            Self::TextChanged => "text_changed",
            Self::KeyPressed => "key_pressed",
            Self::Focused => "focused",
            Self::Unfocused => "unfocused",
            Self::ValueChanged => "value_changed",
        }
    }

    /// 事件名 → 动作；布局文件 / 区域的事件表用它校验。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "click" => Self::Click,
            "right_click" => Self::RightClick,
            "hover" => Self::Hover,
            "leave" => Self::Leave,
            "wheel" => Self::Wheel,
            "focus" => Self::Focus,
            "blur" => Self::Blur,
            "mouse_enter" => Self::MouseEnter,
            "mouse_leave" => Self::MouseLeave,
            "mouse_pressed" => Self::MousePressed,
            "mouse_released" => Self::MouseReleased,
            "mouse_clicked" => Self::MouseClicked,
            "mouse_double_clicked" => Self::MouseDoubleClicked,
            "double_click" => Self::MouseDoubleClicked,
            "text_changed" => Self::TextChanged,
            "key_pressed" => Self::KeyPressed,
            "on_key_pressed" | "onKeyPressed" => Self::KeyPressed,
            "focused" => Self::Focused,
            "on_focused" | "onFocused" => Self::Focused,
            "unfocused" => Self::Unfocused,
            "on_unfocused" | "onUnfocused" => Self::Unfocused,
            "value_changed" | "on_value_changed" | "onValueChanged" => Self::ValueChanged,
            _ => return None,
        })
    }
}

/// 一次窗口鼠标 / 键盘事件：UI 线程 → 工人线程（经 `Work` 通道）。
#[derive(Debug, Clone)]
pub struct ScriptWindowEvent {
    /// 哪一块窗口（`cloudime.window.create` 给的编号）。
    pub window: WindowId,

    /// 建窗口时给的名字（日志 / 脚本分支用）。
    pub name: String,

    /// 发生了什么。
    pub action: WindowAction,

    /// 内容区坐标（点）；`Leave` / `Focus` / `Blur` 时是最后一次的位置。
    pub x: f32,

    /// 内容区坐标（点）。
    pub y: f32,

    /// 命中的元素（内容是 1 起的数组，这里也是 1 起）；`Leave` / 没命中时是 `None`。
    pub element: Option<usize>,

    /// 命中的**区域**名（内容里 `regions` 那个 `name`）；没命中 / 没声明区域时是 `None`。
    pub region: Option<String>,

    /// 命中的控件名（布局文件里每个控件的 `name`）；没有名字或没命中根元素时是 `None`。
    pub control: Option<String>,

    /// 区域 / 布局为这个事件声明的处理器名（`events` 表）；没声明是 `None`。
    pub handler: Option<String>,

    /// `text_changed` 时当前编辑框的完整文本；其它事件是 `None`。
    pub text: Option<String>,

    /// `key_pressed` 的虚拟键码；其它事件是 `None`。
    pub key: Option<u32>,

    /// `key_pressed` 的修饰键位（1 shift / 2 ctrl / 4 alt / 8 win，可相加）。
    pub modifier: Option<u8>,

    /// `value_changed` 的新值；其它事件是 `None`。
    pub value: Option<u64>,

    /// 滚轮量（只有 `Wheel` 有）。
    pub delta: Option<i32>,

    /// 鼠标事件发生时的 Ctrl / Alt / Shift 状态。
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 事件的载荷：`{ window = 名字, id = 编号, action = …, x, y, element, region, handler, delta }`。
pub fn event_payload(lua: &mlua::Lua, event: &ScriptWindowEvent) -> mlua::Result<mlua::Table> {
    let payload = lua.create_table()?;
    payload.set("window", event.name.clone())?;
    payload.set("id", event.window)?;
    payload.set("action", event.action.as_str())?;
    payload.set("x", event.x)?;
    payload.set("y", event.y)?;
    payload.set("element", event.element)?;
    payload.set("region", event.region.clone())?;
    payload.set("control", event.control.clone())?;
    payload.set("handler", event.handler.clone())?;
    payload.set("text", event.text.clone())?;
    payload.set("key", event.key)?;
    payload.set("modifier", event.modifier)?;
    payload.set("value", event.value)?;
    payload.set("delta", event.delta)?;
    payload.set("ctrl", event.ctrl)?;
    payload.set("alt", event.alt)?;
    payload.set("shift", event.shift)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建窗口的缺省：面板底色（`background`）与阴影（`shadow`）都开着，与候选窗一致。
    #[test]
    fn windows_default_to_panel_background_and_shadow() {
        let lua = mlua::Lua::new();
        let options = lua.load("return {}").eval::<mlua::Table>().unwrap();
        let spec = parse_window(&options).unwrap();
        assert!(spec.background);
        assert!(spec.shadow);
    }

    /// 颜色三种写法（`#RGB` / `#RRGGBB` / `#AARRGGBB`）与非法写法。
    #[test]
    fn hex_colors_cover_the_three_lengths() {
        assert_eq!(parse_hex_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_hex_color("00ff00"), Some(Color::rgb(0, 255, 0)));
        assert_eq!(
            parse_hex_color("#800000FF"),
            Some(Color::rgba(0, 0, 255, 128)),
            "AARRGGBB"
        );
        assert_eq!(parse_hex_color("#12345"), None);
        assert_eq!(parse_hex_color("gggggg"), None);
    }

    /// 随包测试布局能通过内容解析器，名字也进入 Document。
    #[test]
    fn bundled_json_layout_parses_into_a_document() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../Scripts/lib/test-caret-window.json");
        let lua = mlua::Lua::new();
        let document = load_layout_document(
            &lua,
            &path,
            (Some(500.0), Some(360.0)),
            &mut ImageCache::default(),
        )
        .unwrap_or_else(|error| panic!("{}：{error}", path.display()));
        assert!(
            document
                .control_names
                .iter()
                .any(|control| control.name == "search")
        );
        assert!(
            document
                .control_names
                .iter()
                .any(|control| control.name == "close")
        );
        assert_eq!(document.control_name(3), Some("search"));
        assert_eq!(document.control_name(5), Some("notes"));
        assert_eq!(document.control_name(7), Some("close"));
    }

    /// 嵌套在 `panel.children` 里的 `text_edit`：绝对位置、区域和自动名字都要有。
    #[test]
    fn nested_text_edits_get_absolute_placements_and_regions() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 220, height = 120,
                    { type = "panel", x = 10, y = 20, w = 200, h = 90,
                      children = {
                          { type = "panel", x = 4, y = 6, w = 190, h = 70,
                            children = {
                                { type = "text_edit", name = "inner", x = 8, y = 10,
                                  w = 120, h = 28, text = "inside" }
                            } }
                      } }
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(220.0), Some(120.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        let placements = document.text_edits();
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].path, vec![0, 0, 0]);
        assert_eq!((placements[0].x, placements[0].y), (22.0, 36.0));
        assert!(matches!(
            document.element_at_path(&[0, 0, 0]),
            Some(Element::TextEdit(edit)) if edit.region_name == "inner"
        ));
        assert!(document.regions.iter().any(|region| region.name == "inner"));
    }

    /// 脚本按控件名读写文本：`set_content` 的初始文本进缓存，`set_text` / `get_text` 同步。
    #[test]
    fn text_edits_can_be_read_and_written_by_control_name() {
        let lua = mlua::Lua::new();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let cache = Rc::new(RefCell::new(HashMap::new()));
        let win = handle(
            &lua,
            Kind::Independent,
            1,
            "面板",
            (Some(120.0), Some(40.0)),
            requests.clone(),
            Rc::new(RefCell::new(ImageCache::default())),
            cache.clone(),
        )
        .unwrap();
        lua.globals().set("win", win).unwrap();
        lua.load(
            r#"win:set_content{
                width = 120, height = 40,
                { type = "text_edit", name = "search", x = 0, y = 0, w = 120, h = 40,
                  text = "hello" },
            }"#,
        )
        .exec()
        .unwrap();
        let text: Option<String> = lua.load("return win:get_text('search')").eval().unwrap();
        assert_eq!(text.as_deref(), Some("hello"));

        lua.load("win:set_text('search', 'world')").exec().unwrap();
        let text: Option<String> = lua.load("return win:get_text('search')").eval().unwrap();
        assert_eq!(text.as_deref(), Some("world"));
        assert!(requests.borrow().iter().any(|request| matches!(
            request,
            Request::SetText { name, text, .. } if name == "search" && text == "world"
        )));

        lua.load("win:select_all('search', false)").exec().unwrap();
        lua.load("win:replace('search', 'world', 'earth')")
            .exec()
            .unwrap();
        assert_eq!(
            lua.load("return win:get_text('search')")
                .eval::<Option<String>>()
                .unwrap()
                .as_deref(),
            Some("earth")
        );
        lua.load("win:clear('search')").exec().unwrap();
        assert_eq!(
            lua.load("return win:get_text('search')")
                .eval::<Option<String>>()
                .unwrap()
                .as_deref(),
            Some("")
        );
        let requests = requests.borrow();
        assert!(requests.iter().any(|request| matches!(
            request,
            Request::SelectAll { name, select: false, .. } if name == "search"
        )));
        assert!(requests.iter().any(|request| matches!(
            request,
            Request::ReplaceText { name, find, text, .. }
                if name == "search" && find == "world" && text == "earth"
        )));
        assert!(requests.iter().any(|request| matches!(
            request,
            Request::ClearText { name, .. } if name == "search"
        )));
    }

    /// 新属性名映射到编辑框字段；`font_flag` 只接受 0–15。
    #[test]
    fn text_edit_new_properties_are_parsed() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 120, height = 40,
                    { type = "text_edit", name = "input", x = 0, y = 0, w = 120, h = 40,
                      read_only = true, passkey = true,
                      back_color = "#FF112233", edge_color = "#FF445566", edge_width = 0,
                      text_color = "#FF778899", cursor_color = "#FFAABBCC",
                      sel_back_color = "#FFDDEEFF", sel_text_color = "#FF010203",
                      fill_color = "#FF0A0B0C", font_name = "Consolas",
                      font_size = 20, font_flag = 3, text = "x" },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(120.0), Some(40.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        let Element::TextEdit(edit) = &document.elements[0] else {
            panic!("不是 text_edit");
        };
        assert!(edit.read_only);
        assert!(edit.passkey);
        assert_eq!(edit.fill, Some(Color::rgba(0x11, 0x22, 0x33, 0xFF)));
        assert_eq!(edit.border, Some(Color::rgba(0x44, 0x55, 0x66, 0xFF)));
        assert_eq!(edit.border_width, 0.0);
        assert_eq!(edit.text_color, Some(Color::rgba(0x77, 0x88, 0x99, 0xFF)));
        assert_eq!(edit.caret_color, Some(Color::rgba(0xAA, 0xBB, 0xCC, 0xFF)));
        assert_eq!(
            edit.selection_background,
            Some(Color::rgba(0xDD, 0xEE, 0xFF, 0xFF))
        );
        assert_eq!(
            edit.selection_color,
            Some(Color::rgba(0x01, 0x02, 0x03, 0xFF))
        );
        assert_eq!(edit.svg_tint, Some(Color::rgba(0x0A, 0x0B, 0x0C, 0xFF)));
        assert_eq!(edit.font_name.as_deref(), Some("Consolas"));
        assert_eq!(edit.size, Some(20.0));
        assert_eq!(edit.font_flag, 3);

        let bad = lua
            .load(
                r#"return { width = 10, height = 10,
                    { type = "text_edit", name = "bad", x = 0, y = 0, w = 10, h = 10,
                      font_flag = 16 } }"#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        assert!(
            parse_document(&bad, (Some(10.0), Some(10.0)), &mut ImageCache::default()).is_err()
        );
    }

    /// `text_box`：多行属性、缺省行距 / 自动换行和区域。
    #[test]
    fn text_box_parses_multiline_defaults() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 300, height = 200,
                    { type = "text_box", name = "notes", x = 10, y = 10, w = 280, h = 120,
                      text = "第一行\n第二行", font_name = "Consolas", font_size = 16 },
                    { type = "text_box", name = "code", x = 10, y = 140, w = 280, h = 40,
                      auto_line_wrap = false, line_spacing = 0, text = "一行" },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(300.0), Some(200.0)),
            &mut ImageCache::default(),
        )
        .expect("text_box 读得出来");
        let Element::TextBox(notes) = &document.elements[0] else {
            panic!("不是 text_box");
        };
        assert_eq!(notes.region_name, "notes");
        assert!(notes.auto_line_wrap);
        assert_eq!(notes.line_spacing, 8);
        assert_eq!(notes.font_name.as_deref(), Some("Consolas"));
        assert_eq!(notes.size, Some(16.0));
        assert_eq!(notes.text, "第一行\n第二行");
        assert!(!notes.h_scroll.visible && !notes.v_scroll.visible);

        let Element::TextBox(code) = &document.elements[1] else {
            panic!("不是 text_box");
        };
        assert!(!code.auto_line_wrap);
        assert_eq!(code.line_spacing, 0);
    }

    /// 水平 / 垂直滚动条：值域、区域、事件处理器和 u64 夹取。
    #[test]
    fn scroll_bars_parse_values_and_regions() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 300, height = 200,
                    { type = "hscroll_bar", name = "hours", x = 10, y = 10, w = 200,
                      value = 150, min_value = 0, max_value = 100, step = 7,
                      background_color = "#FF112233", thumb_color = "#FF445566",
                      events = { value_changed = "hours_changed" } },
                    { type = "vscroll_bar", name = "minutes", x = 250, y = 10, h = 150,
                      value = 5, min_value = 10, max_value = 20 },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(300.0), Some(200.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        let Element::ScrollBar(hours) = &document.elements[0] else {
            panic!("不是水平滚动条");
        };
        assert_eq!(hours.axis, ScrollAxis::Horizontal);
        assert_eq!((hours.value, hours.min, hours.max), (100, 0, 100));
        assert_eq!(hours.step, 7);
        assert_eq!(hours.height, 8.0, "水平滚动条高度缺省 8");
        assert_eq!(hours.width, 200.0);
        assert_eq!(hours.background, Some(Color::rgba(0x11, 0x22, 0x33, 0xFF)));
        assert_eq!(hours.thumb, Some(Color::rgba(0x44, 0x55, 0x66, 0xFF)));
        assert!(document.regions.iter().any(|region| region.name == "hours"));
        assert_eq!(
            document
                .regions
                .iter()
                .find(|region| region.name == "hours")
                .and_then(|region| region.handler("value_changed")),
            Some("hours_changed")
        );
        let Element::ScrollBar(minutes) = &document.elements[1] else {
            panic!("不是垂直滚动条");
        };
        assert_eq!(minutes.axis, ScrollAxis::Vertical);
        assert_eq!((minutes.value, minutes.min, minutes.max), (10, 10, 20));
        assert_eq!(minutes.step, 1);
        assert_eq!(minutes.width, 8.0, "垂直滚动条宽度缺省 8");
        assert_eq!(minutes.height, 150.0);

        let bad = lua
            .load(
                r#"return { width = 20, height = 20,
                    { type = "hscroll_bar", name = "bad", x = 0, y = 0, w = 10, h = 10,
                      min_value = 10, max_value = 1 } }"#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        assert!(
            parse_document(&bad, (Some(20.0), Some(20.0)), &mut ImageCache::default()).is_err()
        );

        let zero_step = lua
            .load(
                r#"return { width = 20, height = 20,
                    { type = "hscroll_bar", name = "zero", x = 0, y = 0, w = 10, h = 10,
                      step = 0 } }"#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        assert!(
            parse_document(
                &zero_step,
                (Some(20.0), Some(20.0)),
                &mut ImageCache::default()
            )
            .is_err()
        );

        let defaults = lua
            .load(
                r#"return { width = 20, height = 20,
                    { type = "hscroll_bar", name = "defaults", x = 0, y = 0 } }"#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &defaults,
            (Some(20.0), Some(20.0)),
            &mut ImageCache::default(),
        )
        .unwrap();
        let Element::ScrollBar(bar) = &document.elements[0] else {
            panic!("不是滚动条");
        };
        assert_eq!((bar.width, bar.height), (8.0, 8.0));
    }

    /// 扁平按钮的六个事件名。
    #[test]
    fn flat_button_actions_have_stable_payload_names() {
        for (action, name) in [
            (WindowAction::MouseEnter, "mouse_enter"),
            (WindowAction::MouseLeave, "mouse_leave"),
            (WindowAction::MousePressed, "mouse_pressed"),
            (WindowAction::MouseReleased, "mouse_released"),
            (WindowAction::MouseClicked, "mouse_clicked"),
            (WindowAction::MouseDoubleClicked, "mouse_double_clicked"),
        ] {
            assert_eq!(action.as_str(), name);
            assert_eq!(WindowAction::from_name(name), Some(action));
        }
    }

    /// 动态控件的 `events` 会挂到自动区域上，事件载荷里据此给 `handler`。
    #[test]
    fn dynamic_control_events_land_on_the_auto_region() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 120, height = 48,
                    { type = "flat_svg_button", name = "close", x = 96, y = 8, w = 16, h = 16,
                      svg = [[<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
                                <rect width="16" height="16"/></svg>]],
                      events = { mouse_clicked = "close_window", mouse_enter = "close_hover",
                                 on_key_pressed = "on_key" } },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(120.0), Some(48.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        assert_eq!(document.regions.len(), 1);
        assert_eq!(document.regions[0].name, "close");
        assert_eq!(
            document.regions[0].handler("mouse_clicked"),
            Some("close_window")
        );
        assert_eq!(
            document.regions[0].handler("mouse_enter"),
            Some("close_hover")
        );
        assert_eq!(document.regions[0].handler("mouse_leave"), None);
        assert_eq!(
            document.regions[0].handler("key_pressed"),
            Some("on_key"),
            "on_key_pressed 归一成 key_pressed"
        );

        let bad = lua
            .load(
                r#"
                return {
                    width = 20, height = 20,
                    { type = "text_edit", name = "q", x = 0, y = 0, w = 20, h = 20,
                      events = { no_such_event = "x" } },
                }
            "#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        assert!(
            parse_document(&bad, (Some(20.0), Some(20.0)), &mut ImageCache::default()).is_err()
        );
    }

    /// 图像缓存：同一份文件不重复解码（拿到同一个 `Arc`）；文件改过就重读。
    #[test]
    fn the_image_cache_reuses_unchanged_files_and_notices_rewrites() {
        let dir = std::env::temp_dir().join("cloudime-script-image-cache");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.png");
        Pixmap::new(4, 4).unwrap().save_png(&path).unwrap();

        let mut cache = ImageCache::default();
        let first = cache.load(&path).expect("读得出来");
        let second = cache.load(&path).expect("读得出来");
        assert!(Arc::ptr_eq(&first, &second), "没改过就复用同一张位图");

        // 文件改了（尺寸变了）：必须重读，并把旧的那份从缓存里换掉
        Pixmap::new(5, 5).unwrap().save_png(&path).unwrap();
        let third = cache.load(&path).expect("读得出来");
        assert!(!Arc::ptr_eq(&first, &third), "文件变了要重读");
        assert_eq!((third.width(), third.height()), (5, 5));
    }

    /// `window` 事件的载荷：滚轮量、命中元素、区域、动作名都带上了。
    #[test]
    fn the_window_event_payload_carries_delta_and_element() {
        let lua = mlua::Lua::new();
        let event = ScriptWindowEvent {
            window: 3,
            name: "面板".to_owned(),
            action: WindowAction::Wheel,
            x: 12.5,
            y: 8.0,
            element: Some(2),
            region: Some("plus".to_owned()),
            control: Some("Counter".to_owned()),
            handler: Some("plus_clicked".to_owned()),
            text: Some("12".to_owned()),
            key: Some(0x41),
            modifier: Some(3),
            value: Some(42),
            delta: Some(120),
            ctrl: true,
            alt: false,
            shift: true,
        };
        let payload = event_payload(&lua, &event).unwrap();
        assert_eq!(payload.get::<String>("action").unwrap(), "wheel");
        assert!(payload.get::<bool>("ctrl").unwrap());
        assert!(!payload.get::<bool>("alt").unwrap());
        assert!(payload.get::<bool>("shift").unwrap());
        assert_eq!(payload.get::<String>("window").unwrap(), "面板");
        assert_eq!(payload.get::<String>("region").unwrap(), "plus");
        assert_eq!(payload.get::<String>("control").unwrap(), "Counter");
        assert_eq!(payload.get::<String>("handler").unwrap(), "plus_clicked");
        assert_eq!(payload.get::<String>("text").unwrap(), "12");
        assert_eq!(payload.get::<u32>("key").unwrap(), 0x41);
        assert_eq!(payload.get::<u8>("modifier").unwrap(), 3);
        assert_eq!(payload.get::<u64>("value").unwrap(), 42);
        assert_eq!(payload.get::<u32>("id").unwrap(), 3);
        assert_eq!(payload.get::<i32>("delta").unwrap(), 120);
        assert_eq!(payload.get::<usize>("element").unwrap(), 2);
        assert_eq!(payload.get::<f32>("x").unwrap(), 12.5);
    }

    /// `regions`：名字 / 命中形状 / `drag` 开关都读出来；重名判无效。
    #[test]
    fn regions_are_parsed_with_names_and_drag_flags() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 200, height = 100,
                    regions = {
                        { name = "plus",  x = 0,  y = 0, w = 40, h = 40, radius = 8 },
                        { name = "box",   x = 50, y = 0, w = 100, h = 40, drag = false },
                    },
                    { type = "rect", x = 0, y = 0, w = 200, h = 100, color = "#FFFFFFFF" },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(200.0), Some(100.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        assert_eq!(document.elements.len(), 1);
        assert_eq!(document.regions.len(), 2);
        assert_eq!(document.regions[0].name, "plus");
        assert!(document.regions[0].draggable);
        assert!(document.regions[0].contains(20.0, 20.0), "圆角矩形中间");
        assert!(!document.regions[0].contains(150.0, 20.0));
        assert_eq!(document.regions[1].name, "box");
        assert!(!document.regions[1].draggable, "drag = false");

        // 重名 → 整份内容判无效
        let content = lua
            .load(
                r#"
                return {
                    regions = {
                        { name = "same", x = 0, y = 0, w = 10, h = 10 },
                        { name = "same", x = 20, y = 0, w = 10, h = 10 },
                    },
                }
            "#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        assert!(
            parse_document(
                &content,
                (Some(200.0), Some(100.0)),
                &mut ImageCache::default()
            )
            .is_err(),
            "区域名字重复该报错"
        );
    }

    /// 区域声明的 `combine` / `prune`：合并（并集）与剪除（差集）都进命中范围。
    #[test]
    fn region_combine_and_prune_are_parsed() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    regions = {
                        {
                            name = "union", x = 0, y = 0, w = 100, h = 100,
                            combine = { x = 50, y = 50, w = 100, h = 100 },
                            prune = { { x = 60, y = 60, w = 20, h = 20 } },
                        },
                    },
                    { type = "rect", x = 0, y = 0, w = 10, h = 10, color = "#FF000000" },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(200.0), Some(100.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        let region = &document.regions[0];
        assert_eq!(region.name, "union");
        assert!(region.contains(25.0, 25.0), "原始形状");
        assert!(region.contains(120.0, 60.0), "合并进来的那一块");
        assert!(!region.contains(70.0, 70.0), "剪掉的小块");
        assert!(!region.contains(120.0, 20.0), "两个形状都没盖住");
    }

    /// 区域不必是矩形：圆、带洞的环、任意闭合路径都能声明。
    #[test]
    fn region_shapes_cover_circles_and_paths() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    regions = {
                        { name = "circle", shape = "circle", cx = 50, cy = 50, radius = 20 },
                        {
                            name = "ring", x = 0, y = 0, w = 100, h = 100,
                            prune = { shape = "circle", cx = 50, cy = 50, radius = 10 },
                        },
                        {
                            name = "tri", shape = "path",
                            commands = { {"M", 0, 0}, {"L", 40, 0}, {"L", 0, 40}, {"Z"} },
                        },
                    },
                    { type = "rect", x = 0, y = 0, w = 10, h = 10, color = "#FF000000" },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(200.0), Some(100.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        assert_eq!(document.regions.len(), 3);

        let circle = &document.regions[0];
        assert!(circle.contains(50.0, 50.0));
        assert!(circle.contains(68.0, 50.0), "贴着边也算在圆里");
        assert!(!circle.contains(20.0, 20.0));

        let ring = &document.regions[1];
        assert!(ring.contains(5.0, 5.0), "环的外圈");
        assert!(!ring.contains(50.0, 50.0), "中间挖掉的圆");

        let triangle = &document.regions[2];
        assert!(triangle.contains(10.0, 10.0));
        assert!(!triangle.contains(30.0, 30.0), "斜边外面");
    }

    /// `label` / `text_block` / `static_shape` / `static_image` / `panel` 都能从元素表解析出来。
    #[test]
    fn static_controls_are_parsed_from_the_content_table() {
        let dir = std::env::temp_dir().join("cloudime-script-controls");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let image_path = dir.join("dot.png");
        Pixmap::new(2, 2).unwrap().save_png(&image_path).unwrap();

        let lua = mlua::Lua::new();
        let source = format!(
            r##"
            return {{
                width = 200, height = 120,
                {{ type = "label", x = 4, y = 4, size = 14,
                   runs = {{ {{ text = "云朵", color = "#FF0000FF" }}, {{ text = "输入法" }} }} }},
                {{ type = "text_block", x = 4, y = 24, text = "第一行\n第二行",
                   size = 10, line_spacing = 4, color = "#FF333333" }},
                {{ type = "static_shape", shape = "circle", cx = 20, cy = 60,
                   radius = 8, color = "#FFFF0000" }},
                {{ type = "static_image", x = 40, y = 60, w = 20, h = 10,
                   path = [[{}]] }},
                {{ type = "panel", x = 100, y = 4, w = 90, h = 100,
                   children = {{
                       {{ type = "static_shape", shape = "round_rect", x = 4, y = 4,
                          w = 40, h = 20, radius = 6, color = "#FF00FF00" }},
                   }} }},
            }}
            "##,
            image_path.display()
        );
        let content = lua.load(&source).eval::<mlua::Table>().unwrap();
        let document = parse_document(
            &content,
            (Some(200.0), Some(120.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        assert_eq!(document.elements.len(), 5);

        match &document.elements[0] {
            Element::Label(label) => {
                assert_eq!(label.runs.len(), 2);
                assert_eq!(label.runs[0].color, Color::rgba(0, 0, 255, 255));
                assert_eq!(label.runs[1].color, Color::rgb(0, 0, 0), "缺省黑色");
                assert_eq!(label.slot, FontSlot::Candidate);
            }
            other => panic!("不是 label：{other:?}"),
        }
        match &document.elements[1] {
            Element::TextBlock(block) => {
                assert_eq!(block.text, "第一行\n第二行");
                assert_eq!(block.line_spacing, 4.0);
                assert_eq!(block.color, Color::rgb(0x33, 0x33, 0x33));
                assert_eq!(block.slot, FontSlot::ItemNumber);
            }
            other => panic!("不是 text_block：{other:?}"),
        }
        assert!(matches!(
            &document.elements[2],
            Element::StaticShape(StaticShape::Circle { radius, .. }) if (*radius - 8.0).abs() < 0.01
        ));
        assert!(matches!(
            &document.elements[3],
            Element::StaticImage(image) if (image.width - 20.0).abs() < 0.01
                && (image.height - 10.0).abs() < 0.01
                && image.image.width() == 2
        ));
        match &document.elements[4] {
            Element::Panel(panel) => {
                assert_eq!(panel.children.len(), 1);
                assert_eq!(panel.radius, 6.0);
                assert_eq!(panel.color, Color::rgba(0xE0, 0xE0, 0xE0, 0xFF));
                assert_eq!(panel.border_width, 1.0);
                assert!(panel.fill.is_none(), "面板缺省无填充");
                assert!(matches!(
                    &panel.children[0],
                    Element::StaticShape(StaticShape::RoundRect { radius, .. })
                        if (*radius - 6.0).abs() < 0.01
                ));
            }
            other => panic!("不是 panel：{other:?}"),
        }
    }

    /// `flat_svg_button`：背景尺寸默认取 SVG 画板，区域自动生成，三种状态颜色都读出来。
    #[test]
    fn flat_svg_buttons_parse_artboard_size_colors_and_region() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 100, height = 100,
                    {
                        type = "flat_svg_button", x = 10, y = 10,
                        svg = [[<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20">
                                  <rect x="4" y="4" width="12" height="12"/></svg>]],
                        shape = "round_rect", radius = 3,
                        normal  = { background = "#00000000", svg = "#FF000000" },
                        hovered = { background = "#FFFF3300", svg = "#FFFFFFFF" },
                        pressed = { background = "#FFCC2900", svg = "#FFFFFFFF" },
                    },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(100.0), Some(100.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        let Element::FlatSvgButton(button) = &document.elements[0] else {
            panic!("不是 flat_svg_button：{:?}", document.elements[0]);
        };
        assert_eq!(
            (button.width, button.height),
            (20.0, 20.0),
            "用 SVG 画板尺寸"
        );
        assert_eq!(button.region_name, "flat_svg_button_1");
        assert_eq!(button.state, ButtonState::Normal);
        assert!(
            matches!(button.shape, ButtonShape::RoundRect { radius } if (radius - 3.0).abs() < 0.01)
        );
        assert_eq!(button.normal.background, Color::rgba(0, 0, 0, 0));
        assert_eq!(button.hovered.background, Color::rgba(255, 51, 0, 255));
        assert_eq!(button.hovered.svg, Color::rgb(255, 255, 255));
        assert_eq!(button.pressed.background, Color::rgba(0xCC, 0x29, 0, 255));

        assert_eq!(document.regions.len(), 1, "按钮自动带来一个区域");
        let region = &document.regions[0];
        assert_eq!(region.name, "flat_svg_button_1");
        assert!(!region.draggable);
        assert!(matches!(
            region.shape,
            HitShape::RoundRect { x, y, width, height, radius }
                if (x - 10.0).abs() < 0.01 && (y - 10.0).abs() < 0.01
                    && (width - 20.0).abs() < 0.01 && (height - 20.0).abs() < 0.01
                    && (radius - 3.0).abs() < 0.01
        ));
    }

    /// `text_edit`：默认胶囊尺寸 / 文本区几何 / 自动区域；有 SVG 时文本区让到旁边。
    #[test]
    fn text_edits_parse_capsule_and_text_area_geometry() {
        let lua = mlua::Lua::new();
        let content = lua
            .load(
                r##"
                return {
                    width = 320, height = 100,
                    { type = "text_edit", name = "plain", x = 10, y = 10, w = 200, h = 24,
                      text = "hello" },
                    { type = "text_edit", name = "with_icon", x = 10, y = 50, w = 300, h = 24,
                      svg = [[<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
                                <rect width="32" height="32"/></svg>]],
                      svg_align = "left", svg_color = "#FF00AA00",
                      selection_background = "#FF112233", selection_color = "#FF445566",
                      caret_color = "#FF778899" },
                }
            "##,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let document = parse_document(
            &content,
            (Some(320.0), Some(100.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        assert_eq!(document.regions.len(), 2, "每个 text_edit 自动一个区域");

        let Element::TextEdit(plain) = &document.elements[0] else {
            panic!("不是 text_edit：{:?}", document.elements[0]);
        };
        assert_eq!(plain.region_name, "plain");
        assert_eq!(plain.radius, 12.0, "缺省胶囊：高的一半");
        assert!(plain.fill.is_none());
        assert!(plain.border.is_none(), "缺省用主题角标色");
        assert_eq!(plain.border_width, 1.0);
        assert!(plain.svg.is_none());
        assert_eq!(plain.font, FontSlot::Candidate, "缺省候选项字体");
        assert!(plain.size.is_none(), "字号缺省跟主题");
        assert!(plain.selection_background.is_none());
        assert!(plain.selection_color.is_none());
        assert!(plain.caret_color.is_none());
        assert_eq!(plain.edit_rect(), (12.0, 1.0, 176.0, 22.0));

        let Element::TextEdit(with_icon) = &document.elements[1] else {
            panic!("不是 text_edit：{:?}", document.elements[1]);
        };
        assert!(with_icon.svg.is_some());
        assert_eq!(with_icon.svg_width(), 24.0, "图标按背景高度缩放");
        assert_eq!(with_icon.edit_rect(), (30.0, 1.0, 248.0, 22.0));
        assert_eq!(with_icon.svg_tint, Some(Color::rgba(0, 170, 0, 255)));
        assert_eq!(
            with_icon.selection_background,
            Some(Color::rgba(17, 34, 51, 255))
        );
        assert_eq!(
            with_icon.selection_color,
            Some(Color::rgba(68, 85, 102, 255))
        );
        assert_eq!(with_icon.caret_color, Some(Color::rgba(119, 136, 153, 255)));

        assert!(matches!(
            &document.regions[0],
            Region { name, draggable: false, .. } if name == "plain"
        ));
    }

    /// `cloudime.image.load` 的句柄可以直接放进 `static_image`（生命周期由 Arc 持有）；
    /// 不写 `w` / `h` 时用图片自身尺寸。
    #[test]
    fn static_images_accept_a_memory_handle() {
        use crate::Runtime;

        let dir = std::env::temp_dir().join("cloudime-script-image-handle");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let image_path = dir.join("dot.png");
        Pixmap::new(3, 4).unwrap().save_png(&image_path).unwrap();

        let runtime = Runtime::none();
        let lua = runtime.lua();
        let handle = lua
            .load(format!(
                "return cloudime.image.load([[{}]])",
                image_path.display()
            ))
            .eval::<mlua::AnyUserData>()
            .expect("图像句柄");
        let content = lua
            .load(
                r#"return { width = 40, height = 40,
                    { type = "static_image", x = 0, y = 0 } }"#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        let element: mlua::Table = content.get(1).unwrap();
        element.set("image", handle).unwrap();
        let document = parse_document(
            &content,
            (Some(40.0), Some(40.0)),
            &mut ImageCache::default(),
        )
        .expect("句柄解析得出来");
        match &document.elements[0] {
            Element::StaticImage(image) => {
                assert_eq!((image.image.width(), image.image.height()), (3, 4));
                assert_eq!((image.width, image.height), (3.0, 4.0));
            }
            other => panic!("不是 static_image：{other:?}"),
        }
    }

    /// 图像句柄的变换方法：scale / rotate / flip / to_gray / use_path_mask。
    #[test]
    fn image_handles_expose_transformations() {
        use crate::Runtime;

        let dir = std::env::temp_dir().join("cloudime-script-image-transform");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let image_path = dir.join("dot.png");
        Pixmap::new(4, 2).unwrap().save_png(&image_path).unwrap();

        let runtime = Runtime::none();
        let lua = runtime.lua();
        let handle = lua
            .load(format!(
                "return cloudime.image.load([[{}]])",
                image_path.display()
            ))
            .eval::<mlua::AnyUserData>()
            .expect("图像句柄");
        lua.globals().set("img", handle.clone()).unwrap();
        lua.load(
            r#"
            img:scale(2)                       -- 8 × 4
            img:rotate(90)                     -- 4 × 8
            img:flip("horizontal")
            img:to_gray()
            img:use_path_mask({ { "M", 0, 0 }, { "L", 4, 0 }, { "L", 4, 8 },
                                { "L", 0, 8 }, { "Z" } })
            "#,
        )
        .exec()
        .unwrap();
        let image = handle.borrow::<ImageHandle>().unwrap().snapshot();
        assert_eq!((image.width(), image.height()), (4, 8));
        let pixel = image.pixel(0, 0).unwrap();
        assert_eq!(
            (pixel.red(), pixel.green(), pixel.blue()),
            (pixel.red(), pixel.red(), pixel.red()),
            "to_gray 之后 RGB 相等"
        );
    }

    /// `svg` 元素可以给 `path` 直接读文件；文件不存在时整份内容报错。
    #[test]
    fn svg_elements_can_load_a_file() {
        let dir = std::env::temp_dir().join("cloudime-script-svg-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.svg");
        std::fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#,
        )
        .unwrap();

        let lua = mlua::Lua::new();
        let source = format!(
            r#"return {{ width = 40, height = 40, {{ type = "svg", x = 0, y = 0, size = 20, path = [[{}]] }} }}"#,
            path.display()
        );
        let content = lua.load(&source).eval::<mlua::Table>().unwrap();
        let document = parse_document(
            &content,
            (Some(40.0), Some(40.0)),
            &mut ImageCache::default(),
        )
        .expect("读得出来");
        match &document.elements[0] {
            Element::Svg { source, .. } => assert!(source.contains("<svg")),
            other => panic!("不是 svg 元素：{other:?}"),
        }

        let content = lua
            .load(
                r#"return { width = 40, height = 40,
                    { type = "svg", x = 0, y = 0, size = 20, path = "missing.svg" } }"#,
            )
            .eval::<mlua::Table>()
            .unwrap();
        assert!(
            parse_document(
                &content,
                (Some(40.0), Some(40.0)),
                &mut ImageCache::default()
            )
            .is_err(),
            "文件不存在该报错"
        );
    }

    /// `region_contains` / `region_relation`：在工人线程上查区域镜像（含合并 / 剪除之后）。
    #[test]
    fn region_queries_answer_from_the_script_side_mirror() {
        let lua = mlua::Lua::new();
        let handle = handle(
            &lua,
            Kind::Independent,
            1,
            "面板",
            (Some(200.0), Some(100.0)),
            Rc::new(RefCell::new(Vec::new())),
            Rc::new(RefCell::new(ImageCache::default())),
            Rc::new(RefCell::new(std::collections::HashMap::new())),
        )
        .unwrap();
        lua.globals().set("win", handle).unwrap();
        lua.load(
            r#"
            win:set_content{
                width = 200, height = 100,
                regions = {
                    { name = "a", x = 0, y = 0, w = 40, h = 40 },
                    { name = "b", x = 100, y = 60, w = 40, h = 40 },
                    { name = "big", x = 0, y = 0, w = 100, h = 100 },
                    { name = "small", x = 10, y = 10, w = 20, h = 20 },
                    { name = "same", x = 0, y = 0, w = 40, h = 40 },
                },
            }

            assert(win:region_contains("a", 10, 10) == true)
            assert(win:region_contains("a", 150, 10) == false)
            assert(win:region_contains("missing", 10, 10) == nil)

            assert(win:region_relation("a", "b").relation == "disjoint")
            assert(win:region_relation("big", "small").relation == "contains")
            assert(win:region_relation("big", "small").contains == "a")
            assert(win:region_relation("small", "big").contains == "b")
            assert(win:region_relation("a", "same").relation == "coincide")
            assert(win:region_relation("a", "missing") == nil)

            -- 合并 / 剪除之后镜像同步更新
            win:combine_region("a", { x = 60, y = 0, w = 20, h = 20 })
            assert(win:region_contains("a", 65, 5) == true)
            win:prune_region("a", { x = 0, y = 0, w = 5, h = 5 })
            assert(win:region_contains("a", 2, 2) == false)
            assert(win:region_relation("a", "same").relation == "intersect")
            "#,
        )
        .exec()
        .unwrap();
    }
}
