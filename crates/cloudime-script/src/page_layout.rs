//! 页面树 v2：JSON → 页面树 → 布局 → 现有 `Document` 内容表。
//!
//! 这一步只负责“把 v2 布局展开成现有控件表”，控件字段、图片路径和事件仍交给
//! [`crate::window`] 的同一套解析器。子页面（可重复实例 / 数据绑定）留到后续步骤。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use cloud_ui_core::{
    Align, Axis, Constraints, Edges, GridTrack, Insets, Justify, LayoutSpec, Measure, Node, NodeId,
    NodeKind, PageTree, Position, Size, SizeSpec, layout,
};
use cloudime_render::{ControlName, InstanceInfo};
use mlua::{Lua, Table, Value as LuaValue};
use serde_json::{Map, Number, Value};

use crate::layout_file::{Layout, read_value, resolve_path, to_lua, validate_name};
use crate::runtime::{LayoutMeasureHook, MeasureFont};

/// v2 顶层版本号。
const VERSION: u64 = 2;

/// 子页面最大深度与实例数，防止递归 / 列表把布局撑爆。
const MAX_SUBPAGE_DEPTH: usize = 8;
const MAX_SUBPAGE_INSTANCES: usize = 100;

/// `list_box` 的逻辑项目上限。
const MAX_LIST_BOX_ROWS: usize = 65_535;
/// 一个 `list_box` 实际实例化的节点上限。
const MAX_LIST_BOX_NODES: usize = 100_000;
/// 单个行模板（不含 draw）的节点上限。
const MAX_LIST_BOX_TEMPLATE_NODES: usize = 512;

/// 一份保留下来的 v2 页面树。
///
/// 与一次性的 [`load`] 不同，这个类型保留节点、数据上下文和“路径 → 节点”的索引，
/// 供脚本按路径改可见性或后续重排；每次渲染才把树重新量一遍并展开成 `Document` 内容表。
#[derive(Clone)]
struct PanelOverride {
    draw: Option<Value>,
    regions: Option<Value>,
}

struct ListMetrics {
    heights: Vec<Option<f32>>,
    estimated: f32,
    measured_sum: f32,
    measured_count: usize,
    pins: BTreeSet<usize>,
}

impl Default for ListMetrics {
    fn default() -> Self {
        Self {
            heights: Vec::new(),
            estimated: 24.0,
            measured_sum: 0.0,
            measured_count: 0,
            pins: BTreeSet::new(),
        }
    }
}

impl ListMetrics {
    fn extent(&self, index: usize) -> f32 {
        self.heights
            .get(index)
            .and_then(|height| *height)
            .unwrap_or_else(|| self.estimated.max(1.0))
    }

    fn set(&mut self, index: usize, height: f32) {
        if self.heights.get(index).and_then(|height| *height).is_none() {
            self.measured_sum += height;
            self.measured_count += 1;
            self.estimated = self.measured_sum / self.measured_count as f32;
        }
        if self.heights.len() <= index {
            self.heights.resize(index + 1, None);
        }
        self.heights[index] = Some(height);
    }
}

type ListMetricsMap = Rc<RefCell<HashMap<String, ListMetrics>>>;

#[derive(Clone)]
pub(crate) struct PageLayout {
    value: Arc<Value>,
    base: PathBuf,
    fallback: (Option<f32>, Option<f32>),
    data: Arc<Value>,
    metrics: ListMetricsMap,
    scrolls: HashMap<String, f32>,
    visibility: HashMap<String, bool>,
    overrides: HashMap<String, PanelOverride>,
    measure: Option<LayoutMeasureHook>,
    tree: PageTree,
    payloads: Vec<Option<Value>>,
    instances: HashMap<String, InstanceInfo>,
    /// 实例路径 → 该实例的 item 数据（子页面脚本的 `ctx.item`）。
    instance_items: HashMap<String, Value>,
    paths: BTreeMap<String, NodeId>,
    regions: Option<Value>,
}

impl PageLayout {
    /// 从 v2 JSON 构建页面树。`data` 为空时使用 `page.data`。
    pub(crate) fn build(
        value: &Value,
        base: &Path,
        fallback: (Option<f32>, Option<f32>),
        data: &Value,
        scrolls: &HashMap<String, f32>,
        measure: Option<&LayoutMeasureHook>,
    ) -> mlua::Result<Self> {
        Self::build_shared(
            Arc::new(value.clone()),
            Arc::new(data.clone()),
            base,
            fallback,
            scrolls,
            measure,
            Rc::new(RefCell::new(HashMap::new())),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_shared(
        value: Arc<Value>,
        data: Arc<Value>,
        base: &Path,
        fallback: (Option<f32>, Option<f32>),
        scrolls: &HashMap<String, f32>,
        measure: Option<&LayoutMeasureHook>,
        metrics: ListMetricsMap,
    ) -> mlua::Result<Self> {
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

        let data = if data.is_null() {
            Arc::new(page.get("data").cloned().unwrap_or(Value::Null))
        } else {
            data
        };
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        apply_common(&mut tree.nodes[0], page, fallback)?;
        let mut payloads = vec![None];
        let mut names = HashSet::new();
        let mut instances = HashMap::new();
        let mut instance_items = HashMap::new();
        let mut paths = BTreeMap::new();
        let mut templates = HashMap::new();
        let mut visiting = Vec::new();
        let context = BindContext::root(&data);
        let children = parse_children(
            &mut tree,
            page,
            base,
            &mut payloads,
            &mut names,
            &mut instances,
            &mut instance_items,
            &mut paths,
            scrolls,
            measure,
            &context,
            "",
            "",
            None,
            &mut templates,
            &mut visiting,
            0,
            &metrics,
        )?;
        tree.nodes[0].children = children;

        Ok(Self {
            value: value.clone(),
            base: base.to_path_buf(),
            fallback,
            data,
            metrics,
            scrolls: scrolls.clone(),
            visibility: HashMap::new(),
            overrides: HashMap::new(),
            measure: measure.cloned(),
            tree,
            payloads,
            instances,
            instance_items,
            paths,
            regions: page.get("regions").cloned(),
        })
    }

    /// 用新的数据上下文重建同一份页面树。
    pub(crate) fn with_data(&self, data: &Value) -> mlua::Result<Self> {
        let mut next = Self::build_shared(
            Arc::clone(&self.value),
            Arc::new(data.clone()),
            &self.base,
            self.fallback,
            &self.scrolls,
            self.measure.as_ref(),
            Rc::new(RefCell::new(HashMap::new())),
        )?;
        next.restore_visibility(&self.visibility);
        next.apply_overrides(&self.overrides);
        Ok(next)
    }

    /// 设置某个虚拟列表的滚动偏移（点），并重建可见窗口。
    pub(crate) fn with_scroll(&self, path: &str, offset: f32) -> mlua::Result<Self> {
        let mut scrolls = self.scrolls.clone();
        scrolls.insert(path.to_owned(), offset.max(0.0));
        let mut next = Self::build_shared(
            Arc::clone(&self.value),
            Arc::clone(&self.data),
            &self.base,
            self.fallback,
            &scrolls,
            self.measure.as_ref(),
            Rc::clone(&self.metrics),
        )?;
        next.restore_visibility(&self.visibility);
        next.apply_overrides(&self.overrides);
        Ok(next)
    }

    fn restore_visibility(&mut self, visibility: &HashMap<String, bool>) {
        for (path, visible) in visibility {
            if let Some(&id) = self.paths.get(path) {
                self.tree.nodes[id].visible = *visible;
            }
        }
        self.visibility = visibility.clone();
    }

    fn apply_overrides(&mut self, overrides: &HashMap<String, PanelOverride>) {
        for (path, override_value) in overrides {
            let Some(payload) = self.panel_payload_mut(path) else {
                continue;
            };
            if let Some(draw) = &override_value.draw {
                payload.insert("draw".to_owned(), draw.clone());
            }
            if let Some(regions) = &override_value.regions {
                payload.insert("regions".to_owned(), regions.clone());
            }
        }
        self.overrides = overrides.clone();
    }

    fn remember_panel(&mut self, path: &str) {
        let Some(payload) = self.panel_payload_mut(path) else {
            return;
        };
        let draw = payload.get("draw").cloned();
        let regions = payload.get("regions").cloned();
        self.overrides
            .insert(path.to_owned(), PanelOverride { draw, regions });
    }

    /// 按路径找到自绘面板的 payload（`draw` / `regions` 所在对象）。
    fn panel_payload_mut(&mut self, path: &str) -> Option<&mut Map<String, Value>> {
        let id = *self.paths.get(path)?;
        self.payloads.get_mut(id)?.as_mut()?.as_object_mut()
    }

    /// 替换自绘面板的整份 draw 列表；返回是否找到面板。
    pub(crate) fn set_draw(&mut self, path: &str, draw: Vec<Value>) -> bool {
        let Some(payload) = self.panel_payload_mut(path) else {
            return false;
        };
        payload.insert("draw".to_owned(), Value::Array(draw));
        self.remember_panel(path);
        true
    }

    /// 替换一个具名 draw 元素；name 大小写敏感。
    pub(crate) fn update_draw(&mut self, path: &str, name: &str, element: Value) -> bool {
        let Some(payload) = self.panel_payload_mut(path) else {
            return false;
        };
        let Some(Value::Array(draw)) = payload.get_mut("draw") else {
            return false;
        };
        let Some(slot) = draw
            .iter_mut()
            .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
        else {
            return false;
        };
        *slot = element;
        self.remember_panel(path);
        true
    }

    /// 在一个具名 draw 元素前 / 后插入；`index` 是 draw 数组下标。
    pub(crate) fn insert_draw(&mut self, path: &str, index: usize, element: Value) -> bool {
        let Some(payload) = self.panel_payload_mut(path) else {
            return false;
        };
        let Some(Value::Array(draw)) = payload.get_mut("draw") else {
            return false;
        };
        if index > draw.len() {
            return false;
        }
        draw.insert(index, element);
        self.remember_panel(path);
        true
    }

    /// 删除一个具名 draw 元素。
    pub(crate) fn remove_draw(&mut self, path: &str, name: &str) -> bool {
        let Some(payload) = self.panel_payload_mut(path) else {
            return false;
        };
        let Some(Value::Array(draw)) = payload.get_mut("draw") else {
            return false;
        };
        let Some(index) = draw
            .iter()
            .position(|item| item.get("name").and_then(Value::as_str) == Some(name))
        else {
            return false;
        };
        draw.remove(index);
        self.remember_panel(path);
        true
    }

    /// 更新 / 删除自绘面板的局部区域。
    pub(crate) fn update_region(&mut self, path: &str, name: &str, region: Value) -> bool {
        let Some(payload) = self.panel_payload_mut(path) else {
            return false;
        };
        let Some(Value::Array(regions)) = payload.get_mut("regions") else {
            return false;
        };
        let Some(slot) = regions
            .iter_mut()
            .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
        else {
            return false;
        };
        *slot = region;
        self.remember_panel(path);
        true
    }

    pub(crate) fn remove_region(&mut self, path: &str, name: &str) -> bool {
        let Some(payload) = self.panel_payload_mut(path) else {
            return false;
        };
        let Some(Value::Array(regions)) = payload.get_mut("regions") else {
            return false;
        };
        let Some(index) = regions
            .iter()
            .position(|item| item.get("name").and_then(Value::as_str) == Some(name))
        else {
            return false;
        };
        regions.remove(index);
        self.remember_panel(path);
        true
    }

    /// 页面树 → 布局 → 现有控件表。
    pub(crate) fn render(&self, lua: &Lua) -> mlua::Result<Layout> {
        let constraints = root_constraints(&self.tree.nodes[self.tree.root]);
        let mut measure = PayloadMeasure {
            payloads: &self.payloads,
            measure: self.measure.as_ref(),
        };
        let result = layout(&self.tree, constraints, &mut measure);

        let table = lua.create_table()?;
        table.raw_set("width", result.size.width)?;
        table.raw_set("height", result.size.height)?;
        if let Some(regions) = &self.regions {
            let Value::Array(_) = regions else {
                return Err(runtime("page.regions 必须是数组"));
            };
            table.raw_set("regions", to_lua(lua, regions, &self.base)?)?;
        }

        let mut control_names = Vec::new();
        let mut output_index = 0;
        flatten(
            &self.tree,
            self.tree.root,
            &result.rects,
            &self.payloads,
            lua,
            &self.base,
            &table,
            &mut output_index,
            &mut control_names,
            (0.0, 0.0),
            &mut Vec::new(),
        )?;
        Ok(Layout {
            table,
            control_names,
            instances: self.instances.clone(),
        })
    }

    pub(crate) fn instances(&self) -> &HashMap<String, InstanceInfo> {
        &self.instances
    }

    pub(crate) fn instance_item(&self, path: &str) -> Option<&Value> {
        self.instance_items.get(path)
    }

    /// 返回列表盒路径与“从第 `first` 行开始”的滚动偏移；不是固定行高时返回 `None`。
    pub(crate) fn list_box_window(&self, control: &str, first: usize) -> Option<(String, f32)> {
        let (path, id) = self.paths.iter().find(|(_, id)| {
            self.tree.nodes[**id].kind == NodeKind::ListBox
                && self.payloads[**id]
                    .as_ref()
                    .and_then(Value::as_object)
                    .and_then(|payload| payload.get("name"))
                    .and_then(Value::as_str)
                    == Some(control)
        })?;
        let payload = self.payloads[*id].as_ref()?.as_object()?;
        if payload.get("virtualize").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let gap = payload.get("gap").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let offset = if let Some(height) = payload
            .get("item_height")
            .and_then(Value::as_f64)
            .filter(|height| *height > 0.0)
        {
            first as f32 * (height as f32 + gap)
        } else {
            let metrics = self.metrics.borrow();
            let metrics = metrics.get(path)?;
            let mut offset = 0.0;
            for index in 0..first {
                offset += metrics.extent(index) + gap;
            }
            offset
        };
        Some((path.clone(), offset))
    }

    /// 把窗口外的行固定住；后续窗口重建会继续带上它。
    pub(crate) fn pin_list_box_item(&self, name: &str, index: usize) -> bool {
        let Some(count) = self.list_box_count(name) else {
            return false;
        };
        if index >= count {
            return false;
        }
        self.metrics
            .borrow_mut()
            .entry(name.to_owned())
            .or_default()
            .pins
            .insert(index);
        true
    }

    pub(crate) fn list_box_item_path(&self, name: &str, index: usize) -> Option<String> {
        let count = self.list_box_count(name)?;
        (index < count).then(|| format!("{name}.items[{index}]"))
    }

    pub(crate) fn list_box_scroll(&self, name: &str) -> f32 {
        self.scrolls.get(name).copied().unwrap_or(0.0)
    }

    /// 把指定逻辑行滚到垂直居中位置所需的偏移；固定 / auto 行高都可用。
    pub(crate) fn list_box_scroll_to(&self, name: &str, index: usize) -> Option<f32> {
        let count = self.list_box_count(name)?;
        if index >= count {
            return None;
        }
        let id = *self.paths.get(name)?;
        let payload = self.payloads[id].as_ref()?.as_object()?;
        let gap = payload.get("gap").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let height = payload
            .get("item_height")
            .and_then(Value::as_f64)
            .filter(|height| *height > 0.0)
            .map(|height| height as f32);
        let (top, row_height, total) = if let Some(height) = height {
            (
                index as f32 * (height + gap),
                height,
                height * count as f32 + gap * count.saturating_sub(1) as f32,
            )
        } else {
            let metrics = self.metrics.borrow();
            let metrics = metrics.get(name)?;
            let mut top = 0.0;
            for row in 0..index {
                top += metrics.extent(row) + gap;
            }
            let mut total = 0.0;
            for row in 0..count {
                total += metrics.extent(row);
            }
            total += gap * count.saturating_sub(1) as f32;
            (top, metrics.extent(index), total)
        };
        let edge = payload
            .get("edge_width")
            .and_then(Value::as_f64)
            .unwrap_or(1.0) as f32;
        let content_gap = payload
            .get("the_gap_between_border_and_content")
            .and_then(Value::as_f64)
            .unwrap_or(2.0) as f32;
        let viewport = (payload.get("h").and_then(Value::as_f64).unwrap_or(0.0) as f32
            - edge * 2.0
            - content_gap * 2.0)
            .max(0.0);
        Some((top + row_height * 0.5 - viewport * 0.5).clamp(0.0, (total - viewport).max(0.0)))
    }

    /// 只重排目标列表的可见窗口，并返回该 `list_box` 的 Lua 表。
    pub(crate) fn render_list_window(
        &self,
        lua: &Lua,
        name: &str,
        offset: f32,
    ) -> mlua::Result<(Self, Table)> {
        let next = self.with_scroll(name, offset)?;
        let layout = next.render(lua)?;
        let table = find_list_box_table(&layout.table, name, 0)?
            .ok_or_else(|| runtime(format!("页面树里找不到 list_box：{name}")))?;
        Ok((next, table))
    }

    /// 按节点路径设置可见性；找不到路径返回 `false`。
    ///
    /// 路径同时接受实例下标与实例 key，例如 `rows[1].title` 和 `rows[abc].title`。
    pub(crate) fn set_visible(&mut self, path: &str, visible: bool) -> bool {
        let Some(&id) = self.paths.get(path) else {
            return false;
        };
        self.tree.nodes[id].visible = visible;
        self.visibility.insert(path.to_owned(), visible);
        true
    }

    /// 路径对应节点的实际控件名（内部带实例前缀）；分组节点返回 `None`。
    /// `list_box` 某一项的数据 key；索引越界返回 `None`。
    pub(crate) fn list_box_item_key(&self, name: &str, index: usize) -> Option<Option<String>> {
        let id = *self.paths.get(name)?;
        let payload = self.payloads.get(id)?.as_ref()?.as_object()?;
        if payload.get("type").and_then(Value::as_str) != Some("list_box") {
            return None;
        }
        let repeat = payload.get("repeat").and_then(Value::as_str)?;
        let item = lookup_path(self.data.as_ref(), repeat)?
            .as_array()?
            .get(index)?;
        let key = payload
            .get("key")
            .and_then(Value::as_str)
            .and_then(|key| item.get(key))
            .map(stringify)
            .unwrap_or_else(|| index.to_string());
        Some(Some(key))
    }

    /// `list_box` 的逻辑项目数；找不到控件或不是列表盒返回 `None`。
    pub(crate) fn list_box_count(&self, name: &str) -> Option<usize> {
        let id = *self.paths.get(name)?;
        let payload = self.payloads.get(id)?.as_ref()?.as_object()?;
        if payload.get("type").and_then(Value::as_str) != Some("list_box") {
            return None;
        }
        let Some(repeat) = payload.get("repeat").and_then(Value::as_str) else {
            return Some(0);
        };
        lookup_path(self.data.as_ref(), repeat)
            .and_then(Value::as_array)
            .map(Vec::len)
    }

    pub(crate) fn control_name(&self, path: &str) -> Option<&str> {
        let id = *self.paths.get(path)?;
        match self.tree.nodes[id].kind {
            NodeKind::Control
            | NodeKind::Container
            | NodeKind::ListBox
            | NodeKind::SubpageContainer => self.tree.nodes[id].name.as_deref(),
            _ => None,
        }
    }
}

/// 解析 v2 布局并展开成 `set_content` 能消费的表。
pub(crate) fn load(
    lua: &Lua,
    value: &Value,
    base: &Path,
    fallback: (Option<f32>, Option<f32>),
    data: &Value,
    measure: Option<&LayoutMeasureHook>,
) -> mlua::Result<Layout> {
    PageLayout::build(value, base, fallback, data, &HashMap::new(), measure)?.render(lua)
}

#[allow(clippy::too_many_arguments)]
fn parse_children(
    tree: &mut PageTree,
    object: &Map<String, Value>,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
    instances: &mut HashMap<String, InstanceInfo>,
    instance_items: &mut HashMap<String, Value>,
    paths: &mut BTreeMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    context: &BindContext<'_>,
    prefix: &str,
    path_prefix: &str,
    instance: Option<&InstanceInfo>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    metrics: &ListMetricsMap,
) -> mlua::Result<Vec<NodeId>> {
    let Some(children) = object.get("children") else {
        return Ok(Vec::new());
    };
    let Value::Array(children) = children else {
        return Err(runtime("children 必须是数组"));
    };
    let mut ids = Vec::with_capacity(children.len());
    for child in children {
        ids.push(parse_node(
            tree,
            child,
            base,
            payloads,
            names,
            instances,
            instance_items,
            paths,
            scrolls,
            measure,
            context,
            prefix,
            path_prefix,
            instance,
            templates,
            visiting,
            depth,
            metrics,
        )?);
    }
    Ok(ids)
}

#[allow(clippy::too_many_arguments)]
fn parse_node(
    tree: &mut PageTree,
    value: &Value,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
    instances: &mut HashMap<String, InstanceInfo>,
    instance_items: &mut HashMap<String, Value>,
    paths: &mut BTreeMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    context: &BindContext<'_>,
    prefix: &str,
    path_prefix: &str,
    instance: Option<&InstanceInfo>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    metrics: &ListMetricsMap,
) -> mlua::Result<NodeId> {
    let object = value
        .as_object()
        .ok_or_else(|| runtime("页面树节点必须是对象"))?;
    let kind_name = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| runtime("页面树节点缺 type"))?;
    if kind_name == "subpage" {
        return parse_subpage(
            tree,
            object,
            base,
            payloads,
            names,
            instances,
            instance_items,
            paths,
            scrolls,
            measure,
            context,
            None,
            prefix,
            path_prefix,
            templates,
            visiting,
            depth,
            metrics,
        );
    }
    let kind = match kind_name {
        "stem" => NodeKind::Stem,
        "panel" | "self_drawing_panel" => NodeKind::Container,
        "subpage_container" => NodeKind::SubpageContainer,
        "list_box" => NodeKind::ListBox,
        _ => NodeKind::Control,
    };

    let local_name = object.get("name").and_then(Value::as_str);
    let name = local_name.map(|name| format!("{prefix}{name}"));
    let node_path = local_name.map(|name| join_path(path_prefix, name));
    if let Some(name) = &name {
        validate_name(name).map_err(runtime)?;
        if !names.insert(name.clone()) {
            return Err(runtime(format!("控件名重复：{name}")));
        }
    } else if matches!(
        kind,
        NodeKind::Control | NodeKind::Container | NodeKind::ListBox | NodeKind::SubpageContainer
    ) {
        return Err(runtime("控件 / 容器缺 name"));
    }

    let mut node = Node::new(kind);
    apply_common(&mut node, object, (None, None))?;
    if kind_name == "text_button" {
        let (min_width, min_height) = text_button_min_size(object, measure);
        node.min_width = node.min_width.max(min_width);
        node.min_height = node.min_height.max(min_height);
        if let SizeSpec::Fixed(width) = node.width
            && width < min_width
        {
            node.width = SizeSpec::Fixed(min_width);
        }
        if let SizeSpec::Fixed(height) = node.height
            && height < min_height
        {
            node.height = SizeSpec::Fixed(min_height);
        }
    }
    node.name = name;
    let id = tree.push(node);
    payloads.push(None);
    if let Some(node_path) = &node_path {
        paths.entry(node_path.clone()).or_insert(id);
    }
    if matches!(
        kind,
        NodeKind::Control | NodeKind::Container | NodeKind::ListBox | NodeKind::SubpageContainer
    ) {
        let mut payload = control_payload(object)?;
        if let (Some(name), Some(payload)) = (&tree.nodes[id].name, payload.as_object_mut()) {
            payload.insert("name".to_owned(), Value::String(name.clone()));
        }
        payloads[id] = Some(bind_value(payload, context));
        if let (Some(name), Some(instance), Some(local_name)) =
            (&tree.nodes[id].name, instance, local_name)
        {
            let mut info = instance.clone();
            info.local_name = local_name.to_owned();
            instances.insert(name.clone(), info);
        }
    }
    let children = if kind == NodeKind::ListBox {
        parse_list_box_children(
            tree,
            object,
            base,
            payloads,
            names,
            instances,
            instance_items,
            paths,
            scrolls,
            measure,
            context,
            prefix,
            node_path.as_deref().unwrap_or(path_prefix),
            templates,
            visiting,
            depth,
            metrics,
        )?
    } else {
        parse_children(
            tree,
            object,
            base,
            payloads,
            names,
            instances,
            instance_items,
            paths,
            scrolls,
            measure,
            context,
            prefix,
            node_path.as_deref().unwrap_or(path_prefix),
            instance,
            templates,
            visiting,
            depth,
            metrics,
        )?
    };
    tree.nodes[id].children = children;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn parse_list_box_children(
    tree: &mut PageTree,
    object: &Map<String, Value>,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
    instances: &mut HashMap<String, InstanceInfo>,
    instance_items: &mut HashMap<String, Value>,
    paths: &mut BTreeMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    context: &BindContext<'_>,
    prefix: &str,
    path_prefix: &str,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    metrics: &ListMetricsMap,
) -> mlua::Result<Vec<NodeId>> {
    let subpages = object
        .get("subpage")
        .and_then(Value::as_array)
        .ok_or_else(|| runtime("list_box.subpage 必须是字符串数组"))?;
    let Some(src) = subpages.first().and_then(Value::as_str) else {
        return Err(runtime("list_box.subpage 不能为空"));
    };
    if subpages.len() > 1 {
        tracing::warn!(
            "list_box.subpage 只使用第一个模板，后面的 {} 个已忽略",
            subpages.len() - 1
        );
    }
    if let Some(path) = object.get("repeat").and_then(Value::as_str)
        && let Some(Value::Array(items)) = context.lookup(path)
        && items.len() > MAX_LIST_BOX_ROWS
    {
        return Err(runtime(format!(
            "list_box 项目数 {} 超过上限 {MAX_LIST_BOX_ROWS}",
            items.len()
        )));
    }
    let mut subpage = object.clone();
    subpage.insert("type".to_owned(), Value::String("subpage".to_owned()));
    subpage.insert("src".to_owned(), Value::String(src.to_owned()));
    subpage.insert("name".to_owned(), Value::String("items".to_owned()));
    subpage.insert("list_box".to_owned(), Value::Bool(true));
    subpage.insert(
        "scroll_path".to_owned(),
        Value::String(path_prefix.to_owned()),
    );
    if let Some(offset) = object.get("scroll_offset") {
        subpage.insert("scroll_offset".to_owned(), offset.clone());
    }
    subpage.remove("children");
    subpage.remove("subpage");
    let gap = object
        .get("gap")
        .and_then(Value::as_f64)
        .map(|gap| gap as f32)
        .unwrap_or(0.0)
        .max(0.0);
    let fixed_height = object
        .get("item_height")
        .and_then(Value::as_f64)
        .filter(|height| *height > 0.0);
    let allow_virtual = object
        .get("virtualize")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if allow_virtual {
        let edge_width = object
            .get("edge_width")
            .and_then(Value::as_f64)
            .unwrap_or(1.0) as f32;
        let content_gap = object
            .get("the_gap_between_border_and_content")
            .and_then(Value::as_f64)
            .unwrap_or(2.0) as f32;
        let viewport = (object.get("h").and_then(Value::as_f64).unwrap_or(0.0) as f32
            - edge_width * 2.0
            - content_gap * 2.0)
            .max(1.0);
        let mut virtualize = Map::new();
        virtualize.insert("axis".to_owned(), Value::String("y".to_owned()));
        virtualize.insert("viewport".to_owned(), Value::from(viewport));
        if let Some(height) = fixed_height {
            virtualize.insert("item_height".to_owned(), Value::from(height));
        }
        // 窗口按 8 行一块重建，块内首行最多再走 7 行；overscan 要能覆盖
        // 这 7 行 + 可视区，滚动到块尾时底部才不会空。
        virtualize.insert("overscan".to_owned(), Value::from(8u64));
        virtualize.insert("gap".to_owned(), Value::from(gap));
        subpage.insert("virtualize".to_owned(), Value::Object(virtualize));
    }
    let mut wrappers = Vec::with_capacity(3);
    for state in 0..3u8 {
        let state_value = list_box_state(state);
        let wrapper = if state == 0 {
            parse_subpage(
                tree,
                &subpage,
                base,
                payloads,
                names,
                instances,
                instance_items,
                paths,
                scrolls,
                measure,
                context,
                Some(&state_value),
                prefix,
                path_prefix,
                templates,
                visiting,
                depth,
                metrics,
            )?
        } else {
            let mut variant_names = HashSet::new();
            parse_subpage(
                tree,
                &subpage,
                base,
                payloads,
                &mut variant_names,
                instances,
                instance_items,
                paths,
                scrolls,
                measure,
                context,
                Some(&state_value),
                prefix,
                path_prefix,
                templates,
                visiting,
                depth,
                metrics,
            )?
        };
        tree.nodes[wrapper].layout = LayoutSpec::Flex {
            direction: Axis::Column,
            wrap: false,
            justify: Justify::Start,
            align: Align::Start,
            gap,
        };
        if state > 0 {
            tree.nodes[wrapper].visible = false;
        }
        wrappers.push(wrapper);
    }
    let mut total = 0;
    let mut max_row = 0;
    for (index, wrapper) in wrappers.iter().enumerate() {
        let (variant_total, variant_row) = list_box_tree_stats(tree, *wrapper)?;
        if index == 0 {
            // 状态变体是同一批逻辑行的绘制副本，节点上限按正常态统计。
            total = variant_total;
        }
        max_row = max_row.max(variant_row);
    }
    if total > MAX_LIST_BOX_NODES {
        return Err(runtime(format!(
            "list_box 实例化节点数 {total} 超过上限 {MAX_LIST_BOX_NODES}"
        )));
    }
    if max_row > MAX_LIST_BOX_TEMPLATE_NODES {
        return Err(runtime(format!(
            "list_box 行模板节点数 {max_row} 超过上限 {MAX_LIST_BOX_TEMPLATE_NODES}"
        )));
    }
    Ok(wrappers)
}

fn list_box_state(state: u8) -> Value {
    let mut fields = Map::new();
    fields.insert("item_state".to_owned(), Value::from(state));
    fields.insert("selected".to_owned(), Value::Bool(state == 1));
    fields.insert("focused".to_owned(), Value::Bool(state == 1));
    fields.insert("hovered".to_owned(), Value::Bool(state == 2));
    Value::Object(fields)
}

fn copy_template_rects(
    tree: &PageTree,
    source: NodeId,
    target: NodeId,
    source_rects: &[Option<cloud_ui_core::Rect>],
    target_rects: &mut [Option<cloud_ui_core::Rect>],
) {
    target_rects[target] = source_rects[source];
    let source_children = &tree.nodes[source].children;
    let target_children = &tree.nodes[target].children;
    if source_children.len() != target_children.len() {
        return;
    }
    for (source, target) in source_children.iter().zip(target_children) {
        copy_template_rects(tree, *source, *target, source_rects, target_rects);
    }
}

fn list_box_tree_stats(tree: &PageTree, wrapper: NodeId) -> mlua::Result<(usize, usize)> {
    let mut total = 0;
    let mut max_row = 0;
    for instance in &tree.nodes[wrapper].children {
        let mut stack = vec![*instance];
        let mut row_nodes = 0;
        while let Some(id) = stack.pop() {
            let node = &tree.nodes[id];
            if matches!(node.kind, NodeKind::Subpage | NodeKind::ListBox) {
                return Err(runtime(
                    "list_box 的子页面内部不能再出现 subpage 或 list_box",
                ));
            }
            total += 1;
            row_nodes += 1;
            stack.extend(node.children.iter().copied());
        }
        max_row = max_row.max(row_nodes);
    }
    Ok((total, max_row))
}

#[allow(clippy::too_many_arguments)]
fn virtual_window(
    metrics: &ListMetrics,
    count: usize,
    offset: f32,
    viewport: f32,
    gap: f32,
    overscan: usize,
    include_partial: bool,
    item_height: Option<f32>,
) -> (usize, usize, f32, f32) {
    let mut prefix = 0.0;
    let mut first = None;
    let mut visible_end = 0;
    for index in 0..count {
        let extent = item_height.unwrap_or_else(|| metrics.extent(index));
        if first.is_none()
            && ((include_partial && prefix + extent > offset)
                || (!include_partial && prefix >= offset))
        {
            first = Some((index, prefix));
        }
        if prefix < offset + viewport {
            visible_end = index + 1;
        }
        prefix += extent + gap;
    }
    let (visible_start, visible_prefix) = first.unwrap_or((count, prefix));
    // 列表盒向上滚 / 拖滚动条时，UI 可能比 Server 先停在新位置。窗口带几行
    // 上 overscan，避免当前偏移落在首行上方时顶部整块空白；普通虚拟子页面
    // 仍按原逻辑只取可见往后。
    let start = if include_partial {
        visible_start.saturating_sub(overscan)
    } else {
        visible_start
    };
    let start_prefix = if start == visible_start {
        visible_prefix
    } else {
        let mut prefix = visible_prefix;
        for index in start..visible_start {
            prefix -= item_height.unwrap_or_else(|| metrics.extent(index)) + gap;
        }
        prefix.max(0.0)
    };
    let end = if visible_start >= count {
        count
    } else {
        (visible_end.max(visible_start + 1) + overscan).min(count)
    };
    let total = if count == 0 {
        0.0
    } else {
        (prefix - gap).max(0.0)
    };
    (start, end, start_prefix, total)
}

fn virtual_top(metrics: &ListMetrics, item_height: Option<f32>, gap: f32, index: usize) -> f32 {
    if let Some(height) = item_height {
        index as f32 * (height + gap)
    } else {
        (0..index).map(|row| metrics.extent(row) + gap).sum()
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_virtual_row(
    page: &Map<String, Value>,
    base: &Path,
    context: &BindContext<'_>,
    state: Option<&Value>,
    item: &Value,
    index: usize,
    key_name: Option<&str>,
    wrapper_path: &str,
    local_name: &str,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    axis: Axis,
    metrics: &ListMetricsMap,
) -> mlua::Result<f32> {
    let key = key_name
        .and_then(|key| item.get(key))
        .cloned()
        .unwrap_or_else(|| Value::from(index as u64));
    let context = BindContext::row(context, item, index, key, state);
    let instance_path = format!("{wrapper_path}[{index}]");
    let info = InstanceInfo {
        subpage: local_name.to_owned(),
        instance: instance_path.clone(),
        item_index: index,
        item_key: None,
        local_name: String::new(),
        script: None,
    };
    measure_item_extent(
        page,
        base,
        &context,
        &format!("measure_{index}_"),
        &instance_path,
        &info,
        scrolls,
        measure,
        templates,
        visiting,
        depth,
        axis,
        metrics,
    )
}

/// 子页面脚本固定放在布局目录旁边的 `shared/`，不能借脚本路径越界。
fn shared_script_path(base: &Path, name: &str) -> Option<String> {
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        tracing::warn!(
            script = name,
            "子页面脚本路径必须是 Scripts/shared 下的相对路径"
        );
        return None;
    }
    let path = base.parent()?.join("shared").join(relative);
    if !path.is_file() {
        tracing::warn!(script = %path.display(), "子页面脚本不存在");
        return None;
    }
    Some(path.to_string_lossy().into_owned())
}

#[allow(clippy::too_many_arguments)]
fn parse_subpage(
    tree: &mut PageTree,
    object: &Map<String, Value>,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
    instance_map: &mut HashMap<String, InstanceInfo>,
    instance_items: &mut HashMap<String, Value>,
    paths: &mut BTreeMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    context: &BindContext<'_>,
    state: Option<&Value>,
    prefix: &str,
    path_prefix: &str,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    metrics: &ListMetricsMap,
) -> mlua::Result<NodeId> {
    if depth >= MAX_SUBPAGE_DEPTH {
        return Err(runtime(format!("子页面嵌套超过 {MAX_SUBPAGE_DEPTH} 层")));
    }
    let src = object
        .get("src")
        .and_then(Value::as_str)
        .ok_or_else(|| runtime("subpage 缺 src"))?;
    let path = PathBuf::from(resolve_path(base, src));
    if visiting.iter().any(|visiting| visiting == &path) {
        return Err(runtime(format!("子页面循环引用：{}", path.display())));
    }
    let template = if let Some(template) = templates.get(&path) {
        template.clone()
    } else {
        let template = read_value(&path)?;
        templates.insert(path.clone(), template.clone());
        template
    };
    if template
        .as_object()
        .and_then(|root| root.get("version"))
        .and_then(Value::as_u64)
        != Some(VERSION)
    {
        return Err(runtime(format!(
            "子页面不是 version: 2 页面树：{}",
            path.display()
        )));
    }
    let page = template
        .as_object()
        .and_then(|root| root.get("page"))
        .and_then(Value::as_object)
        .ok_or_else(|| runtime(format!("子页面缺 page：{}", path.display())))?;
    let script = object
        .get("script")
        .or_else(|| page.get("script"))
        .and_then(Value::as_str)
        .and_then(|name| shared_script_path(base, name));

    let local_name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("subpage");
    let instance_prefix = format!("{}{}_", prefix, sanitize_prefix(local_name));
    let wrapper_path = join_path(path_prefix, local_name);
    let repeat = object.get("repeat").and_then(Value::as_str);
    let single;
    let items: &[Value] = match repeat {
        Some(path) => {
            let value = context
                .lookup(path)
                .ok_or_else(|| runtime(format!("subpage.repeat 找不到数据：{path}")))?;
            let Value::Array(items) = value else {
                return Err(runtime(format!("subpage.repeat 必须是数组：{path}")));
            };
            items
        }
        None => {
            single = vec![context.to_value()];
            &single
        }
    };
    let virtual_spec = parse_virtualize(object, page)?;
    let instance_limit = if object.get("list_box").and_then(Value::as_bool) == Some(true) {
        MAX_LIST_BOX_ROWS
    } else {
        MAX_SUBPAGE_INSTANCES
    };
    let max_instances = object
        .get("max_instances")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(instance_limit)
        .min(instance_limit);
    if virtual_spec.is_none() && items.len() > max_instances {
        return Err(runtime(format!(
            "子页面实例数 {} 超过上限 {max_instances}",
            items.len()
        )));
    }
    let required = object
        .get("required")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| runtime("subpage.required 必须是字符串数组"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| runtime("subpage.required 的字段必须是字符串"))
                })
                .collect::<mlua::Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    for (index, item) in items.iter().enumerate() {
        for field in &required {
            if lookup_path(item, field).is_none() {
                return Err(runtime(format!(
                    "子页面 {} 第 {index} 项缺少 required 字段：{field}",
                    path.display()
                )));
            }
        }
    }

    let template_base = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let key_name = object.get("key").and_then(Value::as_str);
    visiting.push(path);
    let scroll_path = object
        .get("scroll_path")
        .and_then(Value::as_str)
        .unwrap_or(&wrapper_path);
    let scroll_offset = scrolls
        .get(scroll_path)
        .copied()
        .or_else(|| {
            object
                .get("scroll_offset")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
        })
        .unwrap_or(0.0)
        .max(0.0);
    let is_list_box = object.get("list_box").and_then(Value::as_bool) == Some(true);
    let (start, end, spacer, start_prefix, total_height, pins) = if let Some(spec) = &virtual_spec {
        let offset = scroll_offset;
        let metrics_key = scroll_path.to_owned();
        let (start, end, start_prefix, total_height) = if let Some(height) = spec.item_height {
            virtual_window(
                &ListMetrics::default(),
                items.len(),
                offset,
                spec.viewport,
                spec.gap,
                spec.overscan,
                is_list_box,
                Some(height),
            )
        } else {
            let window = {
                let mut metrics = metrics.borrow_mut();
                let metrics = metrics.entry(metrics_key.clone()).or_default();
                if metrics.heights.len() < items.len() {
                    metrics.heights.resize(items.len(), None);
                }
                virtual_window(
                    metrics,
                    items.len(),
                    offset,
                    spec.viewport,
                    spec.gap,
                    spec.overscan,
                    is_list_box,
                    None,
                )
            };
            let mut window = window;
            for _ in 0..3 {
                for (index, item) in items.iter().enumerate().take(window.1).skip(window.0) {
                    let needs_measure = {
                        let metrics = metrics.borrow();
                        metrics
                            .get(&metrics_key)
                            .and_then(|metrics| metrics.heights.get(index))
                            .is_none_or(Option::is_none)
                    };
                    if needs_measure {
                        let extent = measure_virtual_row(
                            page,
                            &template_base,
                            context,
                            state,
                            item,
                            index,
                            key_name,
                            &wrapper_path,
                            local_name,
                            scrolls,
                            measure,
                            templates,
                            visiting,
                            depth + 1,
                            spec.axis,
                            metrics,
                        )?;
                        let mut metrics = metrics.borrow_mut();
                        let metrics = metrics.entry(metrics_key.clone()).or_default();
                        metrics.set(index, extent.max(1.0));
                    }
                }
                let next = {
                    let mut metrics = metrics.borrow_mut();
                    let metrics = metrics.entry(metrics_key.clone()).or_default();
                    virtual_window(
                        metrics,
                        items.len(),
                        offset,
                        spec.viewport,
                        spec.gap,
                        spec.overscan,
                        is_list_box,
                        None,
                    )
                };
                window = next;
            }
            window
        };
        let pins = metrics
            .borrow()
            .get(&metrics_key)
            .map(|metrics| metrics.pins.clone())
            .unwrap_or_default();
        if spec.item_height.is_none() {
            for pin in pins.iter().copied() {
                if pin >= items.len() || (start..end).contains(&pin) {
                    continue;
                }
                let needs_measure = {
                    let metrics = metrics.borrow();
                    metrics
                        .get(&metrics_key)
                        .and_then(|metrics| metrics.heights.get(pin))
                        .is_none_or(Option::is_none)
                };
                if needs_measure {
                    let extent = measure_virtual_row(
                        page,
                        &template_base,
                        context,
                        state,
                        &items[pin],
                        pin,
                        key_name,
                        &wrapper_path,
                        local_name,
                        scrolls,
                        measure,
                        templates,
                        visiting,
                        depth + 1,
                        spec.axis,
                        metrics,
                    )?;
                    let mut metrics = metrics.borrow_mut();
                    let metrics = metrics.entry(metrics_key.clone()).or_default();
                    metrics.set(pin, extent.max(1.0));
                }
            }
        }
        let total_height = {
            let windows = metrics.borrow();
            let default = ListMetrics::default();
            let metrics = windows.get(&metrics_key).unwrap_or(&default);
            virtual_window(
                metrics,
                items.len(),
                offset,
                spec.viewport,
                spec.gap,
                spec.overscan,
                is_list_box,
                spec.item_height,
            )
            .3
            .max(total_height)
        };
        let spacer = if is_list_box {
            0.0
        } else if start < items.len() {
            (start_prefix - offset).max(0.0)
        } else {
            0.0
        };
        (start, end, spacer, start_prefix, total_height, pins)
    } else {
        (0, items.len(), 0.0, 0.0, 0.0, BTreeSet::new())
    };

    let mut indices: Vec<usize> = (start..end).collect();
    for pin in pins.iter().copied() {
        if pin < items.len() && !indices.contains(&pin) {
            indices.push(pin);
        }
    }
    let mut instances = Vec::with_capacity(indices.len());
    for index in indices {
        let item = &items[index];
        let key = key_name
            .and_then(|key| item.get(key))
            .cloned()
            .unwrap_or_else(|| Value::from(index as u64));
        let item_key = if key.is_null() {
            None
        } else {
            Some(stringify(&key))
        };
        let instance_path = format!("{wrapper_path}[{index}]");
        let key_aliases = key_paths(&wrapper_path, &key);
        let prefix_key = key_prefix(&key).unwrap_or_else(|| index.to_string());
        let context = BindContext::row(context, item, index, key, state);
        let instance_prefix = format!("{instance_prefix}{prefix_key}_");
        let info = InstanceInfo {
            subpage: local_name.to_owned(),
            instance: instance_path.clone(),
            item_index: index,
            item_key: item_key.clone(),
            local_name: String::new(),
            script: script.clone(),
        };
        instance_items.insert(instance_path.clone(), item.clone());
        let mut instance = Node::new(NodeKind::Stem);
        apply_common(&mut instance, page, (None, None))?;
        instance.name = Some(format!("{instance_prefix}page"));
        instance.item_key = item_key.clone();
        instance.item_index = Some(index);
        if let Some(spec) = &virtual_spec {
            let top = {
                let metrics = metrics.borrow();
                let default = ListMetrics::default();
                let metrics = metrics.get(scroll_path).unwrap_or(&default);
                virtual_top(metrics, spec.item_height, spec.gap, index)
            };
            instance.virtual_top = Some(top);
        }
        instance.visible = object
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let id = tree.push(instance);
        payloads.push(None);
        paths.entry(instance_path.clone()).or_insert(id);
        for path in &key_aliases {
            paths.entry(path.clone()).or_insert(id);
        }
        let children = parse_children(
            tree,
            page,
            &template_base,
            payloads,
            names,
            instance_map,
            instance_items,
            paths,
            scrolls,
            measure,
            &context,
            &instance_prefix,
            &instance_path,
            Some(&info),
            templates,
            visiting,
            depth + 1,
            metrics,
        )?;
        tree.nodes[id].children = children;
        for alias in &key_aliases {
            alias_subtree(paths, &instance_path, alias);
        }
        instances.push(id);
    }
    visiting.pop();

    if spacer > 0.0 {
        let mut gap = Node::new(NodeKind::Stem);
        match virtual_spec.as_ref().map(|spec| spec.axis) {
            Some(Axis::Row) => gap.width = SizeSpec::Fixed(spacer),
            _ => gap.height = SizeSpec::Fixed(spacer),
        }
        let id = tree.push(gap);
        payloads.push(None);
        instances.insert(0, id);
    }

    let mut wrapper = Node::new(NodeKind::Stem);
    wrapper.name = Some(format!(
        "{prefix}{}__instances",
        sanitize_prefix(local_name)
    ));
    let direction = virtual_spec
        .as_ref()
        .map(|spec| spec.axis)
        .unwrap_or(Axis::Row);
    wrapper.layout = LayoutSpec::Flex {
        direction,
        wrap: virtual_spec.is_none(),
        justify: Justify::Start,
        align: Align::Start,
        gap: virtual_spec.as_ref().map(|spec| spec.gap).unwrap_or(0.0),
    };
    if let Some(spec) = &virtual_spec {
        match spec.axis {
            Axis::Row => wrapper.width = SizeSpec::Fixed(spec.viewport),
            Axis::Column => wrapper.height = SizeSpec::Fixed(spec.viewport),
        }
    }
    wrapper.visible = object
        .get("visible")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    wrapper.virtual_start = Some(start);
    wrapper.virtual_count = Some(items.len());
    wrapper.virtual_offset = Some(start_prefix);
    wrapper.virtual_scroll = Some(scroll_offset);
    if virtual_spec.is_some() {
        wrapper.virtual_content_height = Some(total_height);
    }
    wrapper.children = instances;
    let id = tree.push(wrapper);
    payloads.push(None);
    paths.entry(wrapper_path).or_insert(id);
    Ok(id)
}

struct VirtualSpec {
    axis: Axis,
    viewport: f32,
    overscan: usize,
    item_height: Option<f32>,
    gap: f32,
}

fn parse_virtualize(
    object: &Map<String, Value>,
    page: &Map<String, Value>,
) -> mlua::Result<Option<VirtualSpec>> {
    let Some(value) = object.get("virtualize") else {
        return Ok(None);
    };
    let config = match value {
        Value::Bool(true) => None,
        Value::Bool(false) => return Ok(None),
        Value::Object(config) => Some(config),
        _ => return Err(runtime("subpage.virtualize 必须是布尔值或对象")),
    };
    let field = |name: &str| {
        config
            .and_then(|config| config.get(name))
            .or_else(|| object.get(name))
    };
    let axis = match field("axis").and_then(Value::as_str).unwrap_or("y") {
        "x" | "row" => Axis::Row,
        "y" | "column" => Axis::Column,
        other => return Err(runtime(format!("虚拟化 axis 只能是 x / y：{other}"))),
    };
    let viewport = field("viewport")
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .or_else(|| {
            let key = match axis {
                Axis::Row => "width",
                Axis::Column => "height",
            };
            page.get(key)
                .or_else(|| page.get(if key == "width" { "w" } else { "h" }))
                .and_then(Value::as_f64)
                .map(|value| value as f32)
        })
        .ok_or_else(|| runtime("虚拟化子页面缺 viewport（或模板 page 的固定宽 / 高）"))?;
    if viewport <= 0.0 {
        return Err(runtime("虚拟化 viewport 必须大于 0"));
    }
    let item_height = match field("item_height") {
        None => None,
        Some(Value::String(text)) if text == "auto" => None,
        Some(value) => Some(number(value, "item_height")?),
    };
    let overscan = field("overscan").and_then(Value::as_u64).unwrap_or(1) as usize;
    let gap = field("gap")
        .map(|value| number(value, "gap"))
        .transpose()?
        .unwrap_or(0.0)
        .max(0.0);
    Ok(Some(VirtualSpec {
        axis,
        viewport,
        overscan,
        item_height,
        gap,
    }))
}

#[allow(clippy::too_many_arguments)]
fn measure_item_extent(
    page: &Map<String, Value>,
    base: &Path,
    context: &BindContext<'_>,
    prefix: &str,
    instance_path: &str,
    info: &InstanceInfo,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    axis: Axis,
    metrics: &ListMetricsMap,
) -> mlua::Result<f32> {
    let mut tree = PageTree::new(Node::new(NodeKind::Stem));
    apply_common(&mut tree.nodes[0], page, (None, None))?;
    let mut payloads = vec![None];
    let mut names = HashSet::new();
    let mut instances = HashMap::new();
    let mut instance_items = HashMap::new();
    let mut paths = BTreeMap::new();
    let children = parse_children(
        &mut tree,
        page,
        base,
        &mut payloads,
        &mut names,
        &mut instances,
        &mut instance_items,
        &mut paths,
        scrolls,
        measure,
        context,
        prefix,
        instance_path,
        Some(info),
        templates,
        visiting,
        depth,
        metrics,
    )?;
    tree.nodes[0].children = children;
    let constraints = Constraints {
        min_width: 0.0,
        max_width: f32::INFINITY,
        min_height: 0.0,
        max_height: f32::INFINITY,
    };
    let mut payload_measure = PayloadMeasure {
        payloads: &payloads,
        measure,
    };
    let result = layout(&tree, constraints, &mut payload_measure);
    Ok(match axis {
        Axis::Row => result.size.width,
        Axis::Column => result.size.height,
    })
}

struct BindContext<'a> {
    data: &'a Value,
    parent: Option<&'a BindContext<'a>>,
    item: Option<&'a Value>,
    index: Value,
    key: Option<Value>,
    state: Option<&'a Value>,
}

impl<'a> BindContext<'a> {
    fn root(data: &'a Value) -> Self {
        Self {
            data,
            parent: None,
            item: None,
            index: Value::from(0),
            key: None,
            state: None,
        }
    }

    fn row(
        parent: &'a BindContext<'a>,
        item: &'a Value,
        index: usize,
        key: Value,
        state: Option<&'a Value>,
    ) -> Self {
        Self {
            data: parent.data,
            parent: Some(parent),
            item: Some(item),
            index: Value::from(index as u64),
            key: Some(key),
            state,
        }
    }

    /// 只在没有 `repeat` 的子页面里用一次；有重复项时不复制根数据。
    fn to_value(&self) -> Value {
        let mut fields = self.data.as_object().cloned().unwrap_or_default();
        if let Some(parent) = self.parent
            && let Value::Object(parent_fields) = parent.to_value()
        {
            for (name, value) in parent_fields {
                fields.insert(name, value);
            }
        }
        if let Some(item) = self.item {
            fields.insert("item".to_owned(), item.clone());
        }
        fields.insert("index".to_owned(), self.index.clone());
        if let Some(key) = &self.key {
            fields.insert("key".to_owned(), key.clone());
        }
        if let Some(Value::Object(state)) = self.state {
            for (name, value) in state {
                fields.insert(name.clone(), value.clone());
            }
        }
        Value::Object(fields)
    }

    fn lookup(&self, path: &str) -> Option<&Value> {
        let (first, rest) = path
            .split_once('.')
            .map_or((path, ""), |(first, rest)| (first, rest));
        match first {
            "data" => {
                if let Some(parent) = self.parent {
                    if rest.is_empty() {
                        Some(parent.data)
                    } else {
                        parent.lookup(rest)
                    }
                } else if rest.is_empty() {
                    Some(self.data)
                } else {
                    lookup_path(self.data, rest)
                }
            }
            "item" => {
                let item = self.item?;
                if rest.is_empty() {
                    Some(item)
                } else {
                    lookup_path(item, rest)
                }
            }
            "index" => {
                if rest.is_empty() {
                    Some(&self.index)
                } else {
                    None
                }
            }
            "key" => {
                let key = self.key.as_ref()?;
                if rest.is_empty() { Some(key) } else { None }
            }
            _ => {
                if let Some(state) = self.state
                    && let Some(value) = state.as_object().and_then(|state| state.get(first))
                {
                    return if rest.is_empty() {
                        Some(value)
                    } else {
                        lookup_path(value, rest)
                    };
                }
                if let Some(parent) = self.parent {
                    return parent.lookup(path);
                }
                let value = self.data.as_object()?.get(first)?;
                if rest.is_empty() {
                    Some(value)
                } else {
                    lookup_path(value, rest)
                }
            }
        }
    }
}

fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.').filter(|part| !part.is_empty()) {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn bind_value(value: Value, context: &BindContext<'_>) -> Value {
    match value {
        Value::String(text) => Value::String(interpolate(&text, context)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| bind_value(item, context))
                .collect(),
        ),
        Value::Object(mut fields) => {
            if let Some(states) = fields.get("state").and_then(Value::as_object) {
                let item_state = context
                    .lookup("item_state")
                    .map(stringify)
                    .unwrap_or_else(|| "0".to_owned());
                let selected = states
                    .get(&item_state)
                    .or_else(|| states.get("default"))
                    .cloned()
                    .unwrap_or(Value::Null);
                return bind_value(selected, context);
            }
            for (key, value) in &mut fields {
                if matches!(key.as_str(), "name" | "type") {
                    continue;
                }
                *value = bind_value(value.clone(), context);
            }
            Value::Object(fields)
        }
        value => value,
    }
}

fn interpolate(text: &str, context: &BindContext<'_>) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            output.push_str(&rest[start..]);
            return output;
        };
        let path = after[..end].trim();
        if let Some(value) = context.lookup(path) {
            output.push_str(&stringify(value));
        }
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    output
}

fn stringify(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// 有稳定 key 的实例用 key 生成内部控件名前缀，列表重排时控件名不变。
/// 数字 key 与下标无法区分，继续用下标；非 ASCII / 含特殊字符的 key 转成十六进制。
fn key_prefix(key: &Value) -> Option<String> {
    let Value::String(key) = key else {
        return None;
    };
    if key.is_empty() {
        return None;
    }
    let stable = if key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        key.clone()
    } else {
        let mut hex = String::from("k");
        for byte in key.bytes() {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    };
    Some(sanitize_prefix(&stable))
}

fn join_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}.{name}")
    }
}

/// 实例 key 的两种寻址写法：`rows[key]` 与 `rows["key"]`。
/// 数字 key 与下标无法区分，只保留下标路径。
fn key_paths(base: &str, key: &Value) -> Vec<String> {
    let Value::String(key) = key else {
        return Vec::new();
    };
    let mut paths = vec![format!(
        "{base}[\"{}\"]",
        key.replace('\\', "\\\\").replace('"', "\\\"")
    )];
    if !key.is_empty()
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        paths.push(format!("{base}[{key}]"));
    }
    paths
}

/// 把 `base` 子树里已经登记的下标路径复制成 key 路径。
fn alias_subtree(paths: &mut BTreeMap<String, NodeId>, base: &str, alias: &str) {
    let prefix = format!("{base}.");
    let entries: Vec<(String, NodeId)> = paths
        .range(base.to_owned()..)
        .take_while(|(path, _)| path.as_str() == base || path.starts_with(&prefix))
        .map(|(path, id)| (path.clone(), *id))
        .collect();
    for (path, id) in entries {
        let suffix = &path[base.len()..];
        paths.entry(format!("{alias}{suffix}")).or_insert(id);
    }
}

fn sanitize_prefix(name: &str) -> String {
    let mut output: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    if output.is_empty() {
        output.push_str("subpage");
    }
    if output.as_bytes()[0].is_ascii_digit() {
        output.insert(0, 's');
    }
    output
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

fn find_list_box_table(table: &Table, name: &str, depth: usize) -> mlua::Result<Option<Table>> {
    if depth > 16 {
        return Ok(None);
    }
    if table.get::<Option<String>>("type")?.as_deref() == Some("list_box")
        && table.get::<Option<String>>("name")?.as_deref() == Some(name)
    {
        return Ok(Some(table.clone()));
    }
    if let Some(children) = table.get::<Option<Table>>("children")?
        && let Some(found) = find_list_box_table(&children, name, depth + 1)?
    {
        return Ok(Some(found));
    }
    for value in table.clone().sequence_values::<LuaValue>() {
        let LuaValue::Table(child) = value? else {
            continue;
        };
        if let Some(found) = find_list_box_table(&child, name, depth + 1)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
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
    origin: (f32, f32),
    path_prefix: &mut Vec<usize>,
) -> mlua::Result<()> {
    let node = &tree.nodes[id];
    if !node.visible {
        return Ok(());
    }
    if matches!(node.kind, NodeKind::Subpage) {
        return Err(runtime("subpage 要等子页面实例化步骤实现"));
    }
    if node.kind == NodeKind::ListBox {
        return flatten_list_box(
            tree,
            id,
            rects,
            payloads,
            lua,
            base,
            table,
            output_index,
            names,
            origin,
            path_prefix,
        );
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
        object.insert("x".to_owned(), Value::from(rect.x - origin.0));
        object.insert("y".to_owned(), Value::from(rect.y - origin.1));
        object.insert("w".to_owned(), Value::from(rect.width));
        object.insert("h".to_owned(), Value::from(rect.height));
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| runtime("控件缺 name"))?;
        let is_self_panel =
            object.get("type").and_then(Value::as_str) == Some("self_drawing_panel");
        let mut name_path = path_prefix.clone();
        name_path.push(*output_index);
        names.push(ControlName {
            name: name.to_owned(),
            path: name_path,
        });
        let lua_object = to_lua(lua, &Value::Object(object), base)?;
        let LuaValue::Table(lua_object) = lua_object else {
            return Err(runtime("控件 payload 必须生成 Lua 表"));
        };
        table.raw_set(*output_index as i64 + 1, lua_object.clone())?;
        if is_self_panel {
            let children = lua.create_table()?;
            let mut child_index = 0;
            let mut child_prefix = path_prefix.clone();
            child_prefix.push(*output_index);
            for child in &node.children {
                flatten(
                    tree,
                    *child,
                    rects,
                    payloads,
                    lua,
                    base,
                    &children,
                    &mut child_index,
                    names,
                    (rect.x, rect.y),
                    &mut child_prefix,
                )?;
            }
            lua_object.raw_set("children", children)?;
            *output_index += 1;
            return Ok(());
        }
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
            origin,
            path_prefix,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn flatten_list_box(
    tree: &PageTree,
    id: NodeId,
    rects: &[Option<cloud_ui_core::Rect>],
    payloads: &[Option<Value>],
    lua: &Lua,
    base: &Path,
    table: &Table,
    output_index: &mut usize,
    names: &mut Vec<ControlName>,
    origin: (f32, f32),
    path_prefix: &[usize],
) -> mlua::Result<()> {
    let node = &tree.nodes[id];
    let rect = rects
        .get(id)
        .and_then(|rect| *rect)
        .unwrap_or_else(|| cloud_ui_core::Rect::new(0.0, 0.0, 0.0, 0.0));
    let payload = payloads
        .get(id)
        .and_then(Option::as_ref)
        .ok_or_else(|| runtime("列表盒缺 payload"))?;
    let mut object = payload
        .as_object()
        .cloned()
        .ok_or_else(|| runtime("列表盒 payload 必须是对象"))?;
    object.insert("x".to_owned(), Value::from(rect.x - origin.0));
    object.insert("y".to_owned(), Value::from(rect.y - origin.1));
    object.insert("w".to_owned(), Value::from(rect.width));
    object.insert("h".to_owned(), Value::from(rect.height));
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| runtime("列表盒缺 name"))?;
    let mut name_path = path_prefix.to_vec();
    name_path.push(*output_index);
    names.push(ControlName {
        name: name.to_owned(),
        path: name_path,
    });

    let lua_object = to_lua(lua, &Value::Object(object.clone()), base)?;
    let LuaValue::Table(lua_object) = lua_object else {
        return Err(runtime("列表盒 payload 不能转成 Lua 表"));
    };
    let rows = lua.create_table()?;
    let fixed_height = object
        .get("item_height")
        .and_then(Value::as_f64)
        .map(|height| height as f32)
        .filter(|height| *height > 0.0);
    let gap = object
        .get("gap")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0) as f32;
    let mut max_row_width = 0.0_f32;
    let mut content_height = 0.0_f32;
    let mut row_count = 0usize;
    let mut total_count = 0usize;
    let mut window_offset = 0.0f32;
    if let Some(primary) = node.children.first() {
        let wrapper_node = &tree.nodes[*primary];
        let first_index = wrapper_node.virtual_start.unwrap_or(0);
        total_count = wrapper_node.virtual_count.unwrap_or(0);
        let row_offset = wrapper_node.virtual_offset.unwrap_or(0.0);
        window_offset = wrapper_node.virtual_scroll.unwrap_or(0.0);
        let content_top = rect.y;
        // 状态变体的节点没有参与布局，逐行把正常态的子树矩形拷到这三份旁路数组里。
        // 数组只在这里克隆一次；放进行循环会变成 O(行数 × 页树节点数)。
        let mut variant_rects = vec![rects.to_vec(); 3];
        let mut visible_index = 0;
        for instance in &wrapper_node.children {
            let instance_node = &tree.nodes[*instance];
            if !instance_node.visible || instance_node.item_index.is_none() {
                continue;
            }
            let logical_index = instance_node
                .item_index
                .unwrap_or(first_index + visible_index);
            let instance_rect = rects
                .get(*instance)
                .and_then(|rect| *rect)
                .unwrap_or_else(|| cloud_ui_core::Rect::new(0.0, 0.0, 0.0, 0.0));
            let height = instance_rect.height.max(fixed_height.unwrap_or(0.0));
            if height <= 0.0 {
                continue;
            }
            max_row_width = max_row_width.max(instance_rect.width);
            content_height += height;
            let row = lua.create_table()?;
            row.raw_set("index", logical_index)?;
            row.raw_set(
                "top",
                instance_node
                    .virtual_top
                    .unwrap_or(instance_rect.y - content_top + row_offset),
            )?;
            row.raw_set("height", height)?;
            row.raw_set("width", instance_rect.width)?;
            if let Some(key) = &instance_node.item_key {
                row.raw_set("key", key.clone())?;
            }
            let states = lua.create_table()?;
            let mut normal = None;
            for (state, wrapper) in node.children.iter().enumerate().take(3) {
                let Some(variant_instance) = tree.nodes[*wrapper]
                    .children
                    .iter()
                    .find(|id| tree.nodes[**id].item_index == Some(logical_index))
                else {
                    continue;
                };
                let elements = lua.create_table()?;
                let mut element_index = 0;
                let mut row_names = Vec::new();
                let mut row_prefix = path_prefix.to_vec();
                row_prefix.push(*output_index);
                row_prefix.push(logical_index);
                copy_template_rects(
                    tree,
                    *instance,
                    *variant_instance,
                    rects,
                    &mut variant_rects[state],
                );
                for child in &tree.nodes[*variant_instance].children {
                    flatten(
                        tree,
                        *child,
                        &variant_rects[state],
                        payloads,
                        lua,
                        base,
                        &elements,
                        &mut element_index,
                        &mut row_names,
                        (instance_rect.x, instance_rect.y),
                        &mut row_prefix,
                    )?;
                }
                if state == 0 {
                    normal = Some(elements.clone());
                }
                states.raw_set(state as i64 + 1, elements)?;
            }
            if let Some(normal) = normal {
                row.raw_set("elements", normal)?;
            }
            row.raw_set("states", states)?;
            rows.raw_set(visible_index as i64 + 1, row)?;
            visible_index += 1;
        }
        row_count = visible_index;
        if row_count > 1 {
            content_height += gap * (row_count - 1) as f32;
        }
        if let Some(height) = fixed_height {
            let full = height * total_count as f32 + gap * total_count.saturating_sub(1) as f32;
            content_height = content_height.max(full);
        }
        if let Some(full) = wrapper_node.virtual_content_height {
            content_height = content_height.max(full);
        }
    }
    if total_count == 0 {
        total_count = row_count;
    }
    let self_adaption = object
        .get("self_adaption")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if self_adaption && max_row_width > 0.0 {
        let edge_width = object
            .get("edge_width")
            .and_then(Value::as_f64)
            .unwrap_or(1.0) as f32;
        let content_gap = object
            .get("the_gap_between_border_and_content")
            .and_then(Value::as_f64)
            .unwrap_or(2.0) as f32;
        let viewport = (rect.height - edge_width * 2.0 - content_gap * 2.0).max(0.0);
        let scrollbar = content_height > viewport + 0.01;
        let width = max_row_width
            + edge_width * 2.0
            + content_gap * 2.0
            + if scrollbar { 8.0 + 4.0 } else { 0.0 };
        lua_object.raw_set("w", width)?;
    }
    let scroll_offset = if window_offset.abs() > 0.001 {
        window_offset
    } else {
        object
            .get("scroll_offset")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32
    };
    lua_object.raw_set("rows", rows)?;
    lua_object.raw_set("row_count", total_count)?;
    lua_object.raw_set("content_height", content_height)?;
    lua_object.raw_set("scroll_offset", scroll_offset)?;
    table.raw_set(*output_index as i64 + 1, lua_object)?;
    *output_index += 1;
    Ok(())
}

/// 文字 / 图片的粗略固有尺寸。精确字形度量仍由渲染器负责；这里先保证 v2 的 `auto`
/// 文本不会塌成 0。
struct PayloadMeasure<'a> {
    payloads: &'a [Option<Value>],
    measure: Option<&'a LayoutMeasureHook>,
}

impl Measure for PayloadMeasure<'_> {
    fn measure(&mut self, node: NodeId, constraints: Constraints) -> Size {
        let Some(Some(payload)) = self.payloads.get(node) else {
            return Size::ZERO;
        };
        let Some(object) = payload.as_object() else {
            return Size::ZERO;
        };
        if object.get("type").and_then(Value::as_str) == Some("self_drawing_panel")
            && object.get("auto_size").and_then(Value::as_str) == Some("draw")
            && let Some((width, height)) = self.draw_bounds(object)
        {
            return Size::new(
                width.min(constraints.max_width),
                height.min(constraints.max_height),
            );
        }
        if object.get("type").and_then(Value::as_str) == Some("text_button") {
            return self.text_button_extent(object, constraints);
        }
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
            .or_else(|| object.get("font_size"))
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(14.0);
        let line_spacing = object
            .get("line_spacing")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(2.0);
        let font = object
            .get("font")
            .and_then(Value::as_str)
            .and_then(MeasureFont::from_name)
            .unwrap_or(MeasureFont::Candidate);
        if !text.is_empty() {
            if let Some((width, height)) = self
                .measure
                .and_then(|measure| measure(&text, size, line_spacing, font))
            {
                return Size::new(
                    width.min(constraints.max_width),
                    height.min(constraints.max_height),
                );
            }
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

/// `text_button` 的最小尺寸：最宽一行 + 左右各 12、行数 × 行高 + 上下各 6。
fn text_button_min_size(
    object: &Map<String, Value>,
    measure: Option<&LayoutMeasureHook>,
) -> (f32, f32) {
    let text = object.get("text").and_then(Value::as_str).unwrap_or("");
    let size = object
        .get("font")
        .and_then(Value::as_object)
        .and_then(|font| font.get("size"))
        .and_then(Value::as_f64)
        .map(|size| size as f32)
        .unwrap_or(14.0);
    let mut width = 0.0_f32;
    let mut lines = 0.0_f32;
    for line in text.split('\n') {
        lines += 1.0;
        let measured = measure
            .and_then(|measure| measure(line, size, 0.0, MeasureFont::Candidate))
            .map(|(width, _)| width)
            .unwrap_or_else(|| line.chars().count() as f32 * size * 0.6);
        width = width.max(measured);
    }
    (width + 24.0, lines.max(1.0) * size * 1.25 + 12.0)
}

impl PayloadMeasure<'_> {
    /// `text_button` 的固有尺寸：最宽一行 + 水平内边距、行数 × 行高 + 垂直内边距。
    fn text_button_extent(&self, object: &Map<String, Value>, constraints: Constraints) -> Size {
        let (width, height) = text_button_min_size(object, self.measure);
        Size::new(
            width.min(constraints.max_width),
            height.min(constraints.max_height),
        )
    }

    /// 自绘面板 `auto_size = "draw"` 的包围盒；坐标从面板左上角起。
    fn draw_bounds(&self, object: &Map<String, Value>) -> Option<(f32, f32)> {
        let draw = object.get("draw")?.as_array()?;
        let mut max_x = 0.0_f32;
        let mut max_y = 0.0_f32;
        for element in draw {
            let element = element.as_object()?;
            let x = element.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let y = element.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let (width, height) = match element.get("type").and_then(Value::as_str)? {
                "rect" => (
                    element.get("w").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    element.get("h").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                ),
                "line" => {
                    let (fx, fy) = point_json(element.get("from"))?;
                    let (tx, ty) = point_json(element.get("to"))?;
                    let thickness = element
                        .get("thickness")
                        .and_then(Value::as_f64)
                        .unwrap_or(1.0) as f32;
                    (fx.max(tx) + thickness * 0.5, fy.max(ty) + thickness * 0.5)
                }
                "circle" => {
                    let radius = element.get("radius").and_then(Value::as_f64)? as f32;
                    (
                        element.get("cx").and_then(Value::as_f64).unwrap_or(0.0) as f32 + radius,
                        element.get("cy").and_then(Value::as_f64).unwrap_or(0.0) as f32 + radius,
                    )
                }
                "text" => {
                    let text = element.get("text").and_then(Value::as_str).unwrap_or("");
                    let size = element.get("size").and_then(Value::as_f64).unwrap_or(16.0) as f32;
                    let font = element
                        .get("font")
                        .and_then(Value::as_str)
                        .and_then(MeasureFont::from_name)
                        .unwrap_or(MeasureFont::Candidate);
                    if text.is_empty() {
                        (0.0, 0.0)
                    } else if let Some((width, height)) = self
                        .measure
                        .and_then(|measure| measure(text, size, 0.0, font))
                    {
                        (width, height)
                    } else {
                        (text.chars().count() as f32 * size * 0.6, size * 1.25)
                    }
                }
                "svg" => {
                    let size = element.get("size").and_then(Value::as_f64)? as f32;
                    (size, size)
                }
                "img" => (
                    element.get("w").and_then(Value::as_f64)? as f32,
                    element.get("h").and_then(Value::as_f64)? as f32,
                ),
                "path" => {
                    let commands = element.get("commands")?.as_array()?;
                    let (mut px, mut py) = (0.0_f32, 0.0_f32);
                    let mut path_max_x = 0.0_f32;
                    let mut path_max_y = 0.0_f32;
                    for command in commands {
                        let command = command.as_array()?;
                        let verb = command.first()?.as_str()?;
                        let values: Vec<f32> = command
                            .iter()
                            .skip(1)
                            .filter_map(Value::as_f64)
                            .map(|value| value as f32)
                            .collect();
                        match verb.to_ascii_uppercase().as_str() {
                            "M" | "L" if values.len() >= 2 => {
                                px = values[0];
                                py = values[1];
                            }
                            "Q" if values.len() >= 4 => {
                                path_max_x = path_max_x.max(values[0]).max(values[2]);
                                path_max_y = path_max_y.max(values[1]).max(values[3]);
                                px = values[2];
                                py = values[3];
                            }
                            "C" if values.len() >= 6 => {
                                path_max_x =
                                    path_max_x.max(values[0]).max(values[2]).max(values[4]);
                                path_max_y =
                                    path_max_y.max(values[1]).max(values[3]).max(values[5]);
                                px = values[4];
                                py = values[5];
                            }
                            _ => {}
                        }
                        path_max_x = path_max_x.max(px);
                        path_max_y = path_max_y.max(py);
                    }
                    (path_max_x, path_max_y)
                }
                _ => return None,
            };
            max_x = max_x.max(x + width);
            max_y = max_y.max(y + height);
        }
        Some((max_x, max_y))
    }
}

fn point_json(value: Option<&Value>) -> Option<(f32, f32)> {
    let object = value?.as_object()?;
    Some((
        object.get("x").and_then(Value::as_f64)? as f32,
        object.get("y").and_then(Value::as_f64)? as f32,
    ))
}

/// Lua 数据表 → JSON 数据上下文。空表按空数组处理，方便 `items = {}`。
pub(crate) fn lua_to_json(value: LuaValue) -> mlua::Result<Value> {
    match value {
        LuaValue::Nil => Ok(Value::Null),
        LuaValue::Boolean(value) => Ok(Value::Bool(value)),
        LuaValue::Integer(value) => Ok(Value::from(value)),
        LuaValue::Number(value) => Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| runtime("数字不是有限值")),
        LuaValue::String(text) => Ok(Value::String(text.to_str()?.to_owned())),
        LuaValue::Table(table) => table_to_json(table),
        other => Err(runtime(format!(
            "数据上下文只接受字符串 / 数字 / 布尔 / 表，收到 {}",
            other.type_name()
        ))),
    }
}

fn table_to_json(table: Table) -> mlua::Result<Value> {
    let length = table.raw_len();
    let mut array = Vec::with_capacity(length);
    for index in 1..=length {
        let value: LuaValue = table.raw_get(index as i64)?;
        array.push(lua_to_json(value)?);
    }
    let mut object = Map::new();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        match key {
            LuaValue::Integer(index) if index >= 1 && index as usize <= length => continue,
            LuaValue::String(key) => {
                object.insert(key.to_str()?.to_owned(), lua_to_json(value)?);
            }
            other => {
                return Err(runtime(format!(
                    "数据表的键只能是字符串或连续整数，收到 {}",
                    other.type_name()
                )));
            }
        }
    }
    if !object.is_empty() && !array.is_empty() {
        return Err(runtime("数据表不能同时混用数组下标和字符串键"));
    }
    if object.is_empty() {
        Ok(Value::Array(array))
    } else {
        Ok(Value::Object(object))
    }
}

fn runtime(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

#[cfg(test)]
mod tests {
    use super::{PageLayout, load};
    use mlua::Lua;
    use serde_json::json;
    use std::collections::HashMap;

    fn run(value: serde_json::Value) -> mlua::Result<super::Layout> {
        load(
            &Lua::new(),
            &value,
            std::path::Path::new("."),
            (None, None),
            &serde_json::Value::Null,
            None,
        )
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

    #[test]
    fn a_missing_required_item_field_fails_the_whole_layout() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-subpage-required-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let template = dir.join("row.json");
        std::fs::write(&template, r#"{"version":2,"page":{"children":[]}}"#).unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "children": [{
                    "type": "subpage", "name": "rows", "src": "row.json",
                    "repeat": "items", "required": ["title"]
                }]
            }
        });
        let data = json!({ "items": [{ "id": 1 }] });
        let lua = Lua::new();
        let error = load(&lua, &value, &dir, (None, None), &data, None).unwrap_err();
        assert!(error.to_string().contains("required"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subpages_repeat_instances_and_bind_item_fields() {
        let dir = std::env::temp_dir().join(format!("cloudime-subpage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let template = dir.join("row.json");
        std::fs::write(
            &template,
            r##"{
              "version": 2,
              "page": {
                "w": 100, "h": 30,
                "layout": { "type": "linear", "gap": 2 },
                "children": [
                  { "type": "text_block", "name": "title", "w": 60, "h": 20,
                    "text": "{{item.title}}", "size": 14, "color": "#FF000000" },
                  { "type": "text_block", "name": "meta", "w": 30, "h": 20,
                    "text": "{{index}}", "size": 14, "color": "#FF000000" }
                ]
              }
            }"##,
        )
        .unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "w": 100, "h": 100,
                "layout": { "type": "flex", "direction": "column" },
                "children": [{
                    "type": "subpage", "name": "rows", "src": "row.json",
                    "repeat": "items", "key": "id", "max_instances": 10
                }]
            }
        });
        let data = json!({ "items": [
            { "id": 1, "title": "A" },
            { "id": 2, "title": "B" }
        ]});
        let lua = Lua::new();
        let layout = load(&lua, &value, &dir, (None, None), &data, None).unwrap();
        assert_eq!(layout.table.raw_len(), 4);
        let first: mlua::Table = layout.table.get(1).unwrap();
        assert_eq!(first.get::<String>("name").unwrap(), "rows_0_title");
        assert_eq!(first.get::<String>("text").unwrap(), "A");
        let third: mlua::Table = layout.table.get(3).unwrap();
        assert_eq!(third.get::<String>("name").unwrap(), "rows_1_title");
        assert_eq!(third.get::<String>("text").unwrap(), "B");
        let info = layout.instances.get("rows_1_title").expect("实例元数据");
        assert_eq!(info.instance, "rows[1]");
        assert_eq!(info.item_index, 1);
        assert_eq!(info.item_key.as_deref(), Some("2"));
        assert_eq!(info.control_path(), "rows[1].title");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_text_size_uses_the_exact_layout_measure() {
        let value = json!({
            "version": 2,
            "page": {
                "w": 200, "h": 100,
                "children": [{
                    "type": "text_block", "name": "title", "text": "ABC", "size": 20,
                    "color": "#FF000000"
                }]
            }
        });
        let hook: crate::runtime::LayoutMeasureHook = std::rc::Rc::new(
            |_text: &str, _size: f32, _spacing: f32, _font: super::MeasureFont| Some((123.0, 45.0)),
        );
        let page = PageLayout::build(
            &value,
            std::path::Path::new("."),
            (None, None),
            &serde_json::Value::Null,
            &super::HashMap::new(),
            Some(&hook),
        )
        .unwrap();
        let lua = Lua::new();
        let layout = page.render(&lua).unwrap();
        let title: mlua::Table = layout.table.get(1).unwrap();
        assert_eq!(title.get::<f32>("w").unwrap(), 123.0);
        assert_eq!(title.get::<f32>("h").unwrap(), 45.0);
    }

    #[test]
    fn reordering_items_keeps_their_keyed_control_names() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-subpage-keyed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("row.json"),
            r##"{
              "version": 2,
              "page": {
                "w": 100, "h": 20,
                "children": [{
                  "type": "text_block", "name": "title", "w": 100, "h": 20,
                  "text": "{{item.title}}", "size": 14, "color": "#FF000000"
                }]
              }
            }"##,
        )
        .unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "w": 100, "h": 100,
                "children": [{
                    "type": "subpage", "name": "rows", "src": "row.json",
                    "repeat": "items", "key": "id"
                }]
            }
        });
        let first = json!({ "items": [
            { "id": "a", "title": "A" }, { "id": "b", "title": "B" }
        ]});
        let second = json!({ "items": [
            { "id": "b", "title": "B" }, { "id": "a", "title": "A" }
        ]});
        let lua = Lua::new();
        let page =
            PageLayout::build(&value, &dir, (None, None), &first, &HashMap::new(), None).unwrap();
        let names: Vec<_> = page
            .render(&lua)
            .unwrap()
            .control_names
            .into_iter()
            .map(|name| name.name)
            .collect();
        assert_eq!(names, ["rows_a_title", "rows_b_title"]);
        let page = page.with_data(&second).unwrap();
        let names: Vec<_> = page
            .render(&lua)
            .unwrap()
            .control_names
            .into_iter()
            .map(|name| name.name)
            .collect();
        assert_eq!(names, ["rows_b_title", "rows_a_title"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_virtualized_subpage_only_instantiates_the_visible_window() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-subpage-virtual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("row.json"),
            r##"{
              "version": 2,
              "page": {
                "w": 100, "h": 30,
                "children": [{
                  "type": "text_block", "name": "title", "w": 100, "h": 30,
                  "text": "{{item.title}}", "size": 14, "color": "#FF000000"
                }]
              }
            }"##,
        )
        .unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "w": 100, "h": 100,
                "children": [{
                    "type": "subpage", "name": "rows", "src": "row.json",
                    "repeat": "items", "key": "id",
                    "virtualize": { "axis": "y", "viewport": 100, "item_height": 30,
                                    "overscan": 0 }
                }]
            }
        });
        let items: Vec<_> = (0..10)
            .map(|index| json!({ "id": index, "title": format!("row {index}") }))
            .collect();
        let data = json!({ "items": items });
        let lua = Lua::new();
        let page =
            PageLayout::build(&value, &dir, (None, None), &data, &HashMap::new(), None).unwrap();
        assert_eq!(page.render(&lua).unwrap().control_names.len(), 4);
        let page = page.with_scroll("rows", 75.0).unwrap();
        let layout = page.render(&lua).unwrap();
        assert_eq!(layout.control_names.len(), 3);
        assert_eq!(
            layout.control_names.first().map(|name| name.name.as_str()),
            Some("rows_3_title")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_virtualized_subpage_can_scroll_horizontally() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-subpage-virtual-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cell.json"),
            r##"{
              "version": 2,
              "page": {
                "w": 30, "h": 100,
                "children": [{
                  "type": "text_block", "name": "title", "w": 30, "h": 100,
                  "text": "{{item.title}}", "size": 14, "color": "#FF000000"
                }]
              }
            }"##,
        )
        .unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "w": 100, "h": 100,
                "children": [{
                    "type": "subpage", "name": "columns", "src": "cell.json",
                    "repeat": "items", "key": "id",
                    "virtualize": { "axis": "x", "viewport": 100, "item_height": 30,
                                    "overscan": 0 }
                }]
            }
        });
        let items: Vec<_> = (0..10)
            .map(|index| json!({ "id": index, "title": format!("{index}") }))
            .collect();
        let data = json!({ "items": items });
        let lua = Lua::new();
        let page =
            PageLayout::build(&value, &dir, (None, None), &data, &HashMap::new(), None).unwrap();
        assert_eq!(page.render(&lua).unwrap().control_names.len(), 4);
        let page = page.with_scroll("columns", 75.0).unwrap();
        let layout = page.render(&lua).unwrap();
        assert_eq!(layout.control_names.len(), 3);
        assert_eq!(
            layout.control_names.first().map(|name| name.name.as_str()),
            Some("columns_3_title")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_virtualized_subpage_can_use_variable_row_heights() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-subpage-variable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("row.json"),
            r##"{
              "version": 2,
              "page": {
                "w": 100,
                "children": [{
                  "type": "text_block", "name": "title", "w": 100,
                  "text": "{{item.title}}", "size": 14, "color": "#FF000000"
                }]
              }
            }"##,
        )
        .unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "w": 100, "h": 100,
                "children": [{
                    "type": "subpage", "name": "rows", "src": "row.json",
                    "repeat": "items", "key": "id",
                    "virtualize": { "axis": "y", "viewport": 100, "overscan": 0 }
                }]
            }
        });
        let items: Vec<_> = (0..10)
            .map(|index| json!({ "id": index, "title": format!("row {index}") }))
            .collect();
        let data = json!({ "items": items });
        let hook: crate::runtime::LayoutMeasureHook = std::rc::Rc::new(
            |_text: &str, _size: f32, _spacing: f32, _font: super::MeasureFont| Some((100.0, 45.0)),
        );
        let lua = Lua::new();
        let page = PageLayout::build(
            &value,
            &dir,
            (None, None),
            &data,
            &HashMap::new(),
            Some(&hook),
        )
        .unwrap();
        assert_eq!(page.render(&lua).unwrap().control_names.len(), 3);
        let page = page.with_scroll("rows", 70.0).unwrap();
        let layout = page.render(&lua).unwrap();
        assert_eq!(layout.control_names.len(), 2);
        assert_eq!(
            layout.control_names.first().map(|name| name.name.as_str()),
            Some("rows_2_title")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_self_drawing_panel_can_size_itself_from_its_draw_list() {
        let value = json!({
            "version": 2,
            "page": {
                "w": 200, "h": 100,
                "children": [{
                    "type": "self_drawing_panel", "name": "canvas",
                    "auto_size": "draw",
                    "draw": [
                        { "name": "bg", "type": "rect", "x": 0, "y": 0,
                          "w": 80, "h": 40, "color": "#FFFFFFFF" }
                    ]
                }]
            }
        });
        let lua = Lua::new();
        let page = PageLayout::build(
            &value,
            std::path::Path::new("."),
            (None, None),
            &serde_json::Value::Null,
            &HashMap::new(),
            None,
        )
        .unwrap();
        let layout = page.render(&lua).unwrap();
        let canvas: mlua::Table = layout.table.get(1).unwrap();
        assert_eq!(canvas.get::<f32>("w").unwrap(), 80.0);
        assert_eq!(canvas.get::<f32>("h").unwrap(), 40.0);
    }

    #[test]
    fn self_drawing_panel_auto_size_uses_the_layout_measure_hook() {
        let value = json!({
            "version": 2,
            "page": {
                "w": 300, "h": 200,
                "children": [{
                    "type": "self_drawing_panel", "name": "canvas",
                    "auto_size": "draw",
                    "draw": [{
                        "name": "label", "type": "text", "x": 0, "y": 0,
                        "text": "ABC", "size": 20, "color": "#FF000000"
                    }]
                }]
            }
        });
        let hook: crate::runtime::LayoutMeasureHook = std::rc::Rc::new(
            |_text: &str, _size: f32, _spacing: f32, _font: super::MeasureFont| Some((123.0, 25.0)),
        );
        let lua = Lua::new();
        let page = PageLayout::build(
            &value,
            std::path::Path::new("."),
            (None, None),
            &serde_json::Value::Null,
            &HashMap::new(),
            Some(&hook),
        )
        .unwrap();
        let layout = page.render(&lua).unwrap();
        let canvas: mlua::Table = layout.table.get(1).unwrap();
        assert_eq!(canvas.get::<f32>("w").unwrap(), 123.0);
        assert_eq!(canvas.get::<f32>("h").unwrap(), 25.0);
    }

    #[test]
    fn updating_a_draw_element_resizes_an_auto_sized_panel() {
        let value = json!({
            "version": 2,
            "page": {
                "w": 300, "h": 200,
                "children": [{
                    "type": "self_drawing_panel", "name": "canvas",
                    "auto_size": "draw",
                    "draw": [
                        { "name": "bg", "type": "rect", "x": 0, "y": 0,
                          "w": 80, "h": 40, "color": "#FFFFFFFF" }
                    ]
                }]
            }
        });
        let mut page = PageLayout::build(
            &value,
            std::path::Path::new("."),
            (None, None),
            &serde_json::Value::Null,
            &HashMap::new(),
            None,
        )
        .unwrap();
        assert!(page.update_draw(
            "canvas",
            "bg",
            json!({ "name": "bg", "type": "rect", "x": 0, "y": 0,
                    "w": 120, "h": 55, "color": "#FFFFFFFF" }),
        ));
        let lua = Lua::new();
        let layout = page.render(&lua).unwrap();
        let canvas: mlua::Table = layout.table.get(1).unwrap();
        assert_eq!(canvas.get::<f32>("w").unwrap(), 120.0);
        assert_eq!(canvas.get::<f32>("h").unwrap(), 55.0);

        // 后续 set_data / set_scroll 重建页面树时，draw 单元素更新不能丢。
        let page = page.with_data(&serde_json::Value::Null).unwrap();
        let layout = page.render(&lua).unwrap();
        let canvas: mlua::Table = layout.table.get(1).unwrap();
        assert_eq!(canvas.get::<f32>("w").unwrap(), 120.0);
        assert_eq!(canvas.get::<f32>("h").unwrap(), 55.0);
    }

    #[test]
    fn node_paths_accept_both_instance_index_and_key() {
        let dir = std::env::temp_dir().join(format!("cloudime-subpage-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("row.json"),
            r##"{
              "version": 2,
              "page": {
                "w": 100, "h": 20,
                "children": [
                  { "type": "text_block", "name": "title", "w": 100, "h": 20,
                    "text": "{{item.title}}", "size": 14, "color": "#FF000000" }
                ]
              }
            }"##,
        )
        .unwrap();
        let value = json!({
            "version": 2,
            "page": {
                "w": 100, "h": 100,
                "children": [{
                    "type": "subpage", "name": "rows", "src": "row.json",
                    "repeat": "items", "key": "id"
                }]
            }
        });
        let data = json!({ "items": [
            { "id": "a", "title": "A" },
            { "id": "b", "title": "B" }
        ]});
        let mut page =
            PageLayout::build(&value, &dir, (None, None), &data, &HashMap::new(), None).unwrap();
        for path in ["rows[0].title", "rows[a].title", "rows[\"a\"].title"] {
            assert_eq!(page.control_name(path), Some("rows_a_title"), "{path}");
        }
        for path in ["rows[1].title", "rows[b].title", "rows[\"b\"].title"] {
            assert_eq!(page.control_name(path), Some("rows_b_title"), "{path}");
        }
        assert!(page.set_visible("rows[\"b\"].title", false));
        let lua = Lua::new();
        let rendered = page.render(&lua).unwrap();
        assert_eq!(rendered.table.raw_len(), 1);
        assert_eq!(
            rendered
                .control_names
                .first()
                .map(|name| name.name.as_str()),
            Some("rows_a_title")
        );
        let page = page.with_data(&data).unwrap();
        assert_eq!(page.render(&lua).unwrap().table.raw_len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
