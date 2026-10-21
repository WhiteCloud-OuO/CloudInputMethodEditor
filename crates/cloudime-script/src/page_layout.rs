//! 页面树 v2：JSON → 页面树 → 布局 → 现有 `Document` 内容表。
//!
//! 这一步只负责“把 v2 布局展开成现有控件表”，控件字段、图片路径和事件仍交给
//! [`crate::window`] 的同一套解析器。子页面（可重复实例 / 数据绑定）留到后续步骤。

use std::collections::HashSet;
use std::path::Path;

use cloud_ui_core::{
    Align, Axis, Constraints, Edges, GridTrack, Insets, Justify, LayoutSpec, Measure, Node, NodeId,
    NodeKind, PageTree, Position, Size, SizeSpec, layout,
};
use cloudime_render::ControlName;
use mlua::{Lua, Table};
use serde_json::{Map, Value};

use crate::layout_file::{Layout, to_lua, validate_name};

/// v2 顶层版本号。
const VERSION: u64 = 2;

/// 解析 v2 布局并展开成 `set_content` 能消费的表。
pub(crate) fn load(
    lua: &Lua,
    value: &Value,
    base: &Path,
    fallback: (Option<f32>, Option<f32>),
) -> mlua::Result<Layout> {
    let root = value
        .as_object()
        .ok_or_else(|| runtime("v2 布局根必须是对象"))?;
    if root.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(runtime("只支持 version: 2 的页面树"));
    }
    let page = root
        .get("page")
        .and_then(Value::as_object)
        .ok_or_else(|| runtime("v2 布局缺 page 对象"))?;

    let mut tree = PageTree::new(Node::new(NodeKind::Page));
    apply_common(&mut tree.nodes[0], page, fallback)?;
    let mut payloads = vec![None];
    let mut names = HashSet::new();
    let children = parse_children(&mut tree, page, base, &mut payloads, &mut names)?;
    tree.nodes[0].children = children;

    let constraints = root_constraints(&tree.nodes[0]);
    let mut measure = PayloadMeasure {
        payloads: &payloads,
    };
    let result = layout(&tree, constraints, &mut measure);

    let table = lua.create_table()?;
    table.raw_set("width", result.size.width)?;
    table.raw_set("height", result.size.height)?;
    if let Some(regions) = page.get("regions") {
        let Value::Array(_) = regions else {
            return Err(runtime("page.regions 必须是数组"));
        };
        table.raw_set("regions", to_lua(lua, regions, base)?)?;
    }

    let mut control_names = Vec::new();
    let mut output_index = 0;
    flatten(
        &tree,
        tree.root,
        &result.rects,
        &payloads,
        lua,
        base,
        &table,
        &mut output_index,
        &mut control_names,
    )?;
    Ok(Layout {
        table,
        control_names,
    })
}

fn parse_children(
    tree: &mut PageTree,
    object: &Map<String, Value>,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
) -> mlua::Result<Vec<NodeId>> {
    let Some(children) = object.get("children") else {
        return Ok(Vec::new());
    };
    let Value::Array(children) = children else {
        return Err(runtime("children 必须是数组"));
    };
    let mut ids = Vec::with_capacity(children.len());
    for child in children {
        ids.push(parse_node(tree, child, base, payloads, names)?);
    }
    Ok(ids)
}

fn parse_node(
    tree: &mut PageTree,
    value: &Value,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
) -> mlua::Result<NodeId> {
    let object = value
        .as_object()
        .ok_or_else(|| runtime("页面树节点必须是对象"))?;
    let kind_name = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| runtime("页面树节点缺 type"))?;
    let kind = match kind_name {
        "stem" => NodeKind::Stem,
        "subpage" => NodeKind::Subpage,
        "panel" => NodeKind::Container,
        _ => NodeKind::Control,
    };

    if let Some(name) = object.get("name").and_then(Value::as_str) {
        validate_name(name).map_err(runtime)?;
        if !names.insert(name.to_owned()) {
            return Err(runtime(format!("控件名重复：{name}")));
        }
    } else if matches!(kind, NodeKind::Control | NodeKind::Container) {
        return Err(runtime("控件 / 容器缺 name"));
    }

    let mut node = Node::new(kind);
    apply_common(&mut node, object, (None, None))?;
    let id = tree.push(node);
    payloads.push(None);
    if matches!(kind, NodeKind::Control | NodeKind::Container) {
        payloads[id] = Some(control_payload(object)?);
    }
    if matches!(kind, NodeKind::Subpage) {
        // 子页面在下一步实现；先保留节点，让解析阶段能给出明确错误。
        payloads[id] = None;
    }
    let children = parse_children(tree, object, base, payloads, names)?;
    tree.nodes[id].children = children;
    Ok(id)
}

fn control_payload(object: &Map<String, Value>) -> mlua::Result<Value> {
    let mut payload = object.clone();
    for key in [
        "layout",
        "padding",
        "margin",
        "position",
        "inset",
        "left",
        "top",
        "right",
        "bottom",
        "visible",
        "min_width",
        "max_width",
        "min_height",
        "max_height",
        "grow",
        "shrink",
        "align_self",
        "grid_column",
        "grid_row",
        "column_span",
        "row_span",
        "children",
        "width",
        "height",
    ] {
        payload.remove(key);
    }
    Ok(Value::Object(payload))
}

fn apply_common(
    node: &mut Node,
    object: &Map<String, Value>,
    fallback: (Option<f32>, Option<f32>),
) -> mlua::Result<()> {
    node.name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    node.layout = parse_layout(object.get("layout"))?;
    node.padding = parse_edges(object.get("padding"))?;
    node.margin = parse_edges(object.get("margin"))?;
    node.width = object
        .get("w")
        .or_else(|| object.get("width"))
        .map(parse_size)
        .transpose()?
        .or_else(|| fallback.0.map(SizeSpec::Fixed))
        .unwrap_or(SizeSpec::Auto);
    node.height = object
        .get("h")
        .or_else(|| object.get("height"))
        .map(parse_size)
        .transpose()?
        .or_else(|| fallback.1.map(SizeSpec::Fixed))
        .unwrap_or(SizeSpec::Auto);
    node.min_width = number_or(object.get("min_width"), 0.0)?;
    node.max_width = number_or(object.get("max_width"), f32::INFINITY)?;
    node.min_height = number_or(object.get("min_height"), 0.0)?;
    node.max_height = number_or(object.get("max_height"), f32::INFINITY)?;
    node.grow = number_or(object.get("grow"), 0.0)?;
    node.shrink = number_or(object.get("shrink"), 1.0)?;
    node.align_self = object
        .get("align_self")
        .and_then(Value::as_str)
        .map(parse_align)
        .transpose()?;
    node.visible = object
        .get("visible")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    node.position = match object.get("position").and_then(Value::as_str) {
        Some("absolute") => Position::Absolute,
        _ => Position::Flow,
    };
    node.insets = parse_insets(object)?;
    node.grid_column = optional_index(object.get("grid_column"))?;
    node.grid_row = optional_index(object.get("grid_row"))?;
    node.column_span = object
        .get("column_span")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    node.row_span = object
        .get("row_span")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    Ok(())
}

fn parse_layout(value: Option<&Value>) -> mlua::Result<LayoutSpec> {
    let Some(value) = value else {
        return Ok(LayoutSpec::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| runtime("layout 必须是对象"))?;
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("flex");
    match kind {
        "linear" => Ok(LayoutSpec::Linear {
            gap: number_or(object.get("gap"), 0.0)?,
        }),
        "flex" => Ok(LayoutSpec::Flex {
            direction: parse_direction(object.get("direction"))?,
            wrap: object.get("wrap").and_then(Value::as_bool).unwrap_or(false),
            justify: object
                .get("justify")
                .and_then(Value::as_str)
                .map(parse_justify)
                .transpose()?
                .unwrap_or(Justify::Start),
            align: object
                .get("align")
                .and_then(Value::as_str)
                .map(parse_align)
                .transpose()?
                .unwrap_or(Align::Start),
            gap: number_or(object.get("gap"), 0.0)?,
        }),
        "grid" => Ok(LayoutSpec::Grid {
            columns: parse_tracks(object.get("columns"))?,
            rows: parse_tracks(object.get("rows"))?,
            gap: number_or(object.get("gap"), 0.0)?,
        }),
        "responsive" => Ok(LayoutSpec::Responsive {
            gap: number_or(object.get("gap"), 0.0)?,
            min_item_width: object
                .get("min_item_width")
                .map(|value| number(value, "min_item_width"))
                .transpose()?,
        }),
        other => Err(runtime(format!("未知 layout.type：{other}"))),
    }
}

fn parse_direction(value: Option<&Value>) -> mlua::Result<Axis> {
    match value.and_then(Value::as_str).unwrap_or("row") {
        "row" => Ok(Axis::Row),
        "column" => Ok(Axis::Column),
        other => Err(runtime(format!("direction 只能是 row / column：{other}"))),
    }
}

fn parse_justify(value: &str) -> mlua::Result<Justify> {
    match value {
        "start" => Ok(Justify::Start),
        "center" => Ok(Justify::Center),
        "end" => Ok(Justify::End),
        "space_between" | "space-between" => Ok(Justify::SpaceBetween),
        "space_around" | "space-around" => Ok(Justify::SpaceAround),
        "space_evenly" | "space-evenly" => Ok(Justify::SpaceEvenly),
        other => Err(runtime(format!("未知 justify：{other}"))),
    }
}

fn parse_align(value: &str) -> mlua::Result<Align> {
    match value {
        "start" => Ok(Align::Start),
        "center" => Ok(Align::Center),
        "end" => Ok(Align::End),
        "stretch" => Ok(Align::Stretch),
        other => Err(runtime(format!("未知 align：{other}"))),
    }
}

fn parse_tracks(value: Option<&Value>) -> mlua::Result<Vec<GridTrack>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(runtime("grid 的 columns / rows 必须是数组"));
    };
    items
        .iter()
        .map(|item| match item {
            Value::Number(number) => Ok(GridTrack::Fixed(as_f32(number, "轨道")?)),
            Value::String(text) if text == "auto" => Ok(GridTrack::Auto),
            Value::String(text) if text.ends_with('%') => {
                let percent = text[..text.len() - 1]
                    .parse::<f32>()
                    .map_err(|_| runtime(format!("百分比轨道不合法：{text}")))?;
                Ok(GridTrack::Percent(percent / 100.0))
            }
            Value::String(text) if text.ends_with("fr") => {
                let fraction = text[..text.len() - 2]
                    .parse::<f32>()
                    .map_err(|_| runtime(format!("fr 轨道不合法：{text}")))?;
                Ok(GridTrack::Fraction(fraction))
            }
            _ => Err(runtime("轨道只能是数字 / auto / 百分比 / fr")),
        })
        .collect()
}

fn parse_size(value: &Value) -> mlua::Result<SizeSpec> {
    match value {
        Value::Number(number) => Ok(SizeSpec::Fixed(as_f32(number, "尺寸")?)),
        Value::String(text) if text == "auto" => Ok(SizeSpec::Auto),
        Value::String(text) if text.ends_with('%') => {
            let percent = text[..text.len() - 1]
                .parse::<f32>()
                .map_err(|_| runtime(format!("百分比尺寸不合法：{text}")))?;
            Ok(SizeSpec::Percent(percent / 100.0))
        }
        _ => Err(runtime("尺寸只能是数字 / auto / 百分比")),
    }
}

fn parse_edges(value: Option<&Value>) -> mlua::Result<Edges> {
    let Some(value) = value else {
        return Ok(Edges::default());
    };
    if let Some(value) = value.as_f64() {
        return Ok(Edges::all(value as f32));
    }
    let object = value
        .as_object()
        .ok_or_else(|| runtime("padding / margin 必须是数字或对象"))?;
    Ok(Edges {
        top: number_or(object.get("top"), 0.0)?,
        right: number_or(object.get("right"), 0.0)?,
        bottom: number_or(object.get("bottom"), 0.0)?,
        left: number_or(object.get("left"), 0.0)?,
    })
}

fn parse_insets(object: &Map<String, Value>) -> mlua::Result<Insets> {
    let inset = object.get("inset").and_then(Value::as_object);
    let get = |name: &str| {
        object
            .get(name)
            .or_else(|| inset.and_then(|inset| inset.get(name)))
            .map(|value| number(value, name))
            .transpose()
    };
    Ok(Insets {
        top: get("top")?,
        right: get("right")?,
        bottom: get("bottom")?,
        left: get("left")?,
    })
}

fn optional_index(value: Option<&Value>) -> mlua::Result<Option<usize>> {
    value
        .map(|value| {
            value
                .as_u64()
                .map(|value| value as usize)
                .ok_or_else(|| runtime("grid_column / grid_row 必须是非负整数"))
        })
        .transpose()
}

fn number_or(value: Option<&Value>, default: f32) -> mlua::Result<f32> {
    value
        .map(|value| number(value, "数字"))
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn number(value: &Value, field: &str) -> mlua::Result<f32> {
    value
        .as_f64()
        .map(|value| value as f32)
        .ok_or_else(|| runtime(format!("{field} 必须是数字")))
}

fn as_f32(value: &serde_json::Number, field: &str) -> mlua::Result<f32> {
    value
        .as_f64()
        .map(|value| value as f32)
        .ok_or_else(|| runtime(format!("{field} 必须是数字")))
}

fn root_constraints(node: &Node) -> Constraints {
    let width = match node.width {
        SizeSpec::Fixed(width) => width,
        SizeSpec::Percent(_) | SizeSpec::Auto => f32::INFINITY,
    };
    let height = match node.height {
        SizeSpec::Fixed(height) => height,
        SizeSpec::Percent(_) | SizeSpec::Auto => f32::INFINITY,
    };
    Constraints {
        min_width: if width.is_finite() { width } else { 0.0 },
        max_width: width,
        min_height: if height.is_finite() { height } else { 0.0 },
        max_height: height,
    }
}

#[allow(clippy::too_many_arguments)]
fn flatten(
    tree: &PageTree,
    id: NodeId,
    rects: &[Option<cloud_ui_core::Rect>],
    payloads: &[Option<Value>],
    lua: &Lua,
    base: &Path,
    table: &Table,
    output_index: &mut usize,
    names: &mut Vec<ControlName>,
) -> mlua::Result<()> {
    let node = &tree.nodes[id];
    if !node.visible {
        return Ok(());
    }
    if matches!(node.kind, NodeKind::Subpage) {
        return Err(runtime("subpage 要等子页面实例化步骤实现"));
    }
    if matches!(node.kind, NodeKind::Control | NodeKind::Container)
        && let Some(payload) = payloads.get(id).and_then(Option::as_ref)
    {
        let rect = rects
            .get(id)
            .and_then(|rect| *rect)
            .unwrap_or_else(|| cloud_ui_core::Rect::new(0.0, 0.0, 0.0, 0.0));
        let mut object = payload
            .as_object()
            .cloned()
            .ok_or_else(|| runtime("控件 payload 必须是对象"))?;
        object.insert("x".to_owned(), Value::from(rect.x));
        object.insert("y".to_owned(), Value::from(rect.y));
        object.insert("w".to_owned(), Value::from(rect.width));
        object.insert("h".to_owned(), Value::from(rect.height));
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| runtime("控件缺 name"))?;
        names.push(ControlName {
            name: name.to_owned(),
            path: vec![*output_index],
        });
        table.raw_set(
            *output_index as i64 + 1,
            to_lua(lua, &Value::Object(object), base)?,
        )?;
        *output_index += 1;
    }
    for child in &node.children {
        flatten(
            tree,
            *child,
            rects,
            payloads,
            lua,
            base,
            table,
            output_index,
            names,
        )?;
    }
    Ok(())
}

/// 文字 / 图片的粗略固有尺寸。精确字形度量仍由渲染器负责；这里先保证 v2 的 `auto`
/// 文本不会塌成 0。
struct PayloadMeasure<'a> {
    payloads: &'a [Option<Value>],
}

impl Measure for PayloadMeasure<'_> {
    fn measure(&mut self, node: NodeId, constraints: Constraints) -> Size {
        let Some(Some(payload)) = self.payloads.get(node) else {
            return Size::ZERO;
        };
        let Some(object) = payload.as_object() else {
            return Size::ZERO;
        };
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                object.get("runs").and_then(Value::as_array).map(|runs| {
                    runs.iter()
                        .filter_map(|run| run.get("text").and_then(Value::as_str))
                        .collect::<String>()
                })
            })
            .unwrap_or_default();
        let size = object
            .get("size")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(14.0);
        if !text.is_empty() {
            let lines = text.lines().count().max(1) as f32;
            let width = text
                .lines()
                .map(|line| line.chars().count() as f32 * size * 0.6)
                .fold(0.0, f32::max);
            return Size::new(
                width.min(constraints.max_width),
                (lines * size * 1.2).min(constraints.max_height),
            );
        }
        Size::ZERO
    }
}

fn runtime(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

#[cfg(test)]
mod tests {
    use super::load;
    use mlua::Lua;
    use serde_json::json;

    fn run(value: serde_json::Value) -> mlua::Result<super::Layout> {
        load(&Lua::new(), &value, std::path::Path::new("."), (None, None))
    }

    #[test]
    fn a_control_without_a_name_fails_the_whole_layout() {
        let error = run(json!({
            "version": 2,
            "page": {
                "w": 100,
                "h": 100,
                "children": [{ "type": "rect", "w": 10, "h": 10 }]
            }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("缺 name"), "{error}");
    }

    #[test]
    fn an_unknown_layout_type_fails_the_whole_layout() {
        let error = run(json!({
            "version": 2,
            "page": {
                "w": 100,
                "h": 100,
                "layout": { "type": "masonry" },
                "children": []
            }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("未知 layout.type"), "{error}");
    }
}
