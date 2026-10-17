//! 脚本窗口：`cloudime.window.create` / `cloudime.window.create_auxiliary` 的请求与句柄。
//!
//! 脚本在**工人线程**上跑，窗口在 **UI 线程**上：两边不能互相调用，所以这里只把脚本要的东西
//! 读成一份**普通数据**（[`Request`]），由 Server 在派发结束后取走、经命令通道交给 UI 线程执行
//! （与 `cloudime.candidate.set_*` / `cloudime.apply_theme` 同一个路子）。
//!
//! 内容是一份声明式的元素表（[`Document`]，坐标一律是**点**）：脚本不直接画，UI 线程拿着它经
//! `Renderer::render_document` 一次出图 —— 这样脚本再怎么写也卡不住 UI 线程，反过来也一样。

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use cloudime_render::{
    Color, Document, Element, FontSlot, HitRect, HitShape, PathCommand, Pixmap, Region, load_image,
};

use crate::mlua;

/// 窗口编号：脚本那一侧只当句柄的一部分，UI 线程按它认窗口。
pub type WindowId = u32;

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

struct CachedImage {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    image: Arc<Pixmap>,
}

impl ImageCache {
    /// 读一张图（缓存命中就不解码）。路径按 Server 的工作目录（安装目录）解析。
    fn load(&mut self, path: &Path) -> mlua::Result<Arc<Pixmap>> {
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
pub(crate) fn handle(
    lua: &mlua::Lua,
    kind: Kind,
    id: WindowId,
    name: &str,
    size: (Option<f32>, Option<f32>),
    requests: Rc<RefCell<Vec<Request>>>,
    images: Rc<RefCell<ImageCache>>,
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
    handle.set(
        "set_content",
        lua.create_function(move |_, (this, content): (mlua::Table, mlua::Table)| {
            let document = parse_document(
                &content,
                fallback_size(&this),
                &mut content_images.borrow_mut(),
            )?;
            *content_regions.borrow_mut() = document.regions.clone();
            content_requests
                .borrow_mut()
                .push(Request::SetContent(id, document));
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
        elements.push(parse_element(&table, images)?);
    }
    Ok(Document {
        width,
        height,
        elements,
        regions: parse_regions(content)?,
    })
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
fn parse_element(table: &mlua::Table, images: &mut ImageCache) -> mlua::Result<Element> {
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
        other => Err(mlua::Error::RuntimeError(format!(
            "元素 type 不认识：{other}（可用：rect / circle / line / text / svg / img / path）"
        ))),
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
            "path 元素的 commands 是空的".to_owned(),
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
        }
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

    /// 滚轮量（只有 `Wheel` 有）。
    pub delta: Option<i32>,
}

/// 事件的载荷：`{ window = 名字, id = 编号, action = …, x, y, element, region, delta }`。
pub fn event_payload(lua: &mlua::Lua, event: &ScriptWindowEvent) -> mlua::Result<mlua::Table> {
    let payload = lua.create_table()?;
    payload.set("window", event.name.clone())?;
    payload.set("id", event.window)?;
    payload.set("action", event.action.as_str())?;
    payload.set("x", event.x)?;
    payload.set("y", event.y)?;
    payload.set("element", event.element)?;
    payload.set("region", event.region.clone())?;
    payload.set("delta", event.delta)?;
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
            delta: Some(120),
        };
        let payload = event_payload(&lua, &event).unwrap();
        assert_eq!(payload.get::<String>("action").unwrap(), "wheel");
        assert_eq!(payload.get::<String>("window").unwrap(), "面板");
        assert_eq!(payload.get::<String>("region").unwrap(), "plus");
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
