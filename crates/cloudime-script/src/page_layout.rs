//! 页面树 v2：JSON → 页面树 → 布局 → 现有 `Document` 内容表。
//!
//! 这一步只负责“把 v2 布局展开成现有控件表”，控件字段、图片路径和事件仍交给
//! [`crate::window`] 的同一套解析器。子页面（可重复实例 / 数据绑定）留到后续步骤。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// 一份保留下来的 v2 页面树。
///
/// 与一次性的 [`load`] 不同，这个类型保留节点、数据上下文和“路径 → 节点”的索引，
/// 供脚本按路径改可见性或后续重排；每次渲染才把树重新量一遍并展开成 `Document` 内容表。
pub(crate) struct PageLayout {
    value: Value,
    base: PathBuf,
    fallback: (Option<f32>, Option<f32>),
    data: Value,
    scrolls: HashMap<String, f32>,
    measure: Option<LayoutMeasureHook>,
    tree: PageTree,
    payloads: Vec<Option<Value>>,
    instances: HashMap<String, InstanceInfo>,
    paths: HashMap<String, NodeId>,
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
            page.get("data").cloned().unwrap_or(Value::Null)
        } else {
            data.clone()
        };
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        apply_common(&mut tree.nodes[0], page, fallback)?;
        let mut payloads = vec![None];
        let mut names = HashSet::new();
        let mut instances = HashMap::new();
        let mut paths = HashMap::new();
        let mut templates = HashMap::new();
        let mut visiting = Vec::new();
        let children = parse_children(
            &mut tree,
            page,
            base,
            &mut payloads,
            &mut names,
            &mut instances,
            &mut paths,
            scrolls,
            measure,
            &data,
            "",
            "",
            None,
            &mut templates,
            &mut visiting,
            0,
        )?;
        tree.nodes[0].children = children;

        Ok(Self {
            value: value.clone(),
            base: base.to_path_buf(),
            fallback,
            data,
            scrolls: scrolls.clone(),
            measure: measure.cloned(),
            tree,
            payloads,
            instances,
            paths,
            regions: page.get("regions").cloned(),
        })
    }

    /// 用新的数据上下文重建同一份页面树。
    pub(crate) fn with_data(&self, data: &Value) -> mlua::Result<Self> {
        Self::build(
            &self.value,
            &self.base,
            self.fallback,
            data,
            &self.scrolls,
            self.measure.as_ref(),
        )
    }

    /// 设置某个虚拟列表的滚动偏移（点），并重建可见窗口。
    pub(crate) fn with_scroll(&self, path: &str, offset: f32) -> mlua::Result<Self> {
        let mut scrolls = self.scrolls.clone();
        scrolls.insert(path.to_owned(), offset.max(0.0));
        Self::build(
            &self.value,
            &self.base,
            self.fallback,
            &self.data,
            &scrolls,
            self.measure.as_ref(),
        )
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
        )?;
        Ok(Layout {
            table,
            control_names,
            instances: self.instances.clone(),
        })
    }

    /// 按节点路径设置可见性；找不到路径返回 `false`。
    ///
    /// 路径同时接受实例下标与实例 key，例如 `rows[1].title` 和 `rows[abc].title`。
    pub(crate) fn set_visible(&mut self, path: &str, visible: bool) -> bool {
        let Some(&id) = self.paths.get(path) else {
            return false;
        };
        self.tree.nodes[id].visible = visible;
        true
    }

    /// 路径对应节点的实际控件名（内部带实例前缀）；分组节点返回 `None`。
    pub(crate) fn control_name(&self, path: &str) -> Option<&str> {
        let id = *self.paths.get(path)?;
        match self.tree.nodes[id].kind {
            NodeKind::Control | NodeKind::Container => self.tree.nodes[id].name.as_deref(),
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
    paths: &mut HashMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    data: &Value,
    prefix: &str,
    path_prefix: &str,
    instance: Option<&InstanceInfo>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
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
            paths,
            scrolls,
            measure,
            data,
            prefix,
            path_prefix,
            instance,
            templates,
            visiting,
            depth,
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
    paths: &mut HashMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    data: &Value,
    prefix: &str,
    path_prefix: &str,
    instance: Option<&InstanceInfo>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
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
            paths,
            scrolls,
            measure,
            data,
            prefix,
            path_prefix,
            templates,
            visiting,
            depth,
        );
    }
    let kind = match kind_name {
        "stem" => NodeKind::Stem,
        "panel" => NodeKind::Container,
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
    } else if matches!(kind, NodeKind::Control | NodeKind::Container) {
        return Err(runtime("控件 / 容器缺 name"));
    }

    let mut node = Node::new(kind);
    apply_common(&mut node, object, (None, None))?;
    node.name = name;
    let id = tree.push(node);
    payloads.push(None);
    if let Some(node_path) = &node_path {
        paths.entry(node_path.clone()).or_insert(id);
    }
    if matches!(kind, NodeKind::Control | NodeKind::Container) {
        let mut payload = control_payload(object)?;
        if let (Some(name), Some(payload)) = (&tree.nodes[id].name, payload.as_object_mut()) {
            payload.insert("name".to_owned(), Value::String(name.clone()));
        }
        payloads[id] = Some(bind_value(payload, data));
        if let (Some(name), Some(instance), Some(local_name)) =
            (&tree.nodes[id].name, instance, local_name)
        {
            let mut info = instance.clone();
            info.local_name = local_name.to_owned();
            instances.insert(name.clone(), info);
        }
    }
    let children = parse_children(
        tree,
        object,
        base,
        payloads,
        names,
        instances,
        paths,
        scrolls,
        measure,
        data,
        prefix,
        node_path.as_deref().unwrap_or(path_prefix),
        instance,
        templates,
        visiting,
        depth,
    )?;
    tree.nodes[id].children = children;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn parse_subpage(
    tree: &mut PageTree,
    object: &Map<String, Value>,
    base: &Path,
    payloads: &mut Vec<Option<Value>>,
    names: &mut HashSet<String>,
    instance_map: &mut HashMap<String, InstanceInfo>,
    paths: &mut HashMap<String, NodeId>,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    data: &Value,
    prefix: &str,
    path_prefix: &str,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
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

    let local_name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("subpage");
    let instance_prefix = format!("{}{}_", prefix, sanitize_prefix(local_name));
    let wrapper_path = join_path(path_prefix, local_name);
    let repeat = object.get("repeat").and_then(Value::as_str);
    let items = match repeat {
        Some(path) => {
            let value = lookup_path(data, path)
                .ok_or_else(|| runtime(format!("subpage.repeat 找不到数据：{path}")))?;
            let Value::Array(items) = value else {
                return Err(runtime(format!("subpage.repeat 必须是数组：{path}")));
            };
            items.clone()
        }
        None => vec![data.clone()],
    };
    let virtual_spec = parse_virtualize(object, page)?;
    let max_instances = object
        .get("max_instances")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(MAX_SUBPAGE_INSTANCES)
        .min(MAX_SUBPAGE_INSTANCES);
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
    let (start, end, spacer) = if let Some(spec) = &virtual_spec {
        let offset = scrolls.get(&wrapper_path).copied().unwrap_or(0.0).max(0.0);
        let mut prefix = 0.0;
        let mut first = None;
        let mut visible_end = 0;
        for (index, item) in items.iter().enumerate() {
            let extent = match spec.item_height {
                Some(height) => height,
                None => {
                    let key = key_name
                        .and_then(|key| item.get(key))
                        .cloned()
                        .unwrap_or_else(|| Value::from(index as u64));
                    let context = make_context(data, item, index, key);
                    let instance_path = format!("{wrapper_path}[{index}]");
                    let info = InstanceInfo {
                        subpage: local_name.to_owned(),
                        instance: instance_path.clone(),
                        item_index: index,
                        item_key: None,
                        local_name: String::new(),
                    };
                    measure_item_extent(
                        page,
                        &template_base,
                        &context,
                        &format!("measure_{index}_"),
                        &instance_path,
                        &info,
                        scrolls,
                        measure,
                        templates,
                        visiting,
                        depth + 1,
                        spec.axis,
                    )?
                }
            };
            if first.is_none() && prefix >= offset {
                first = Some((index, prefix));
            }
            if prefix < offset + spec.viewport {
                visible_end = index + 1;
            }
            prefix += extent;
        }
        let (start, start_prefix) = first.unwrap_or((items.len(), prefix));
        let end = if start >= items.len() {
            items.len()
        } else {
            (visible_end.max(start + 1) + spec.overscan).min(items.len())
        };
        let spacer = if start < items.len() {
            (start_prefix - offset).max(0.0)
        } else {
            0.0
        };
        (start, end, spacer)
    } else {
        (0, items.len(), 0.0)
    };

    let mut instances = Vec::with_capacity(end.saturating_sub(start) + 1);
    for (index, item) in items.iter().enumerate().take(end).skip(start) {
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
        let context = make_context(data, item, index, key);
        let instance_prefix = format!("{instance_prefix}{prefix_key}_");
        let info = InstanceInfo {
            subpage: local_name.to_owned(),
            instance: instance_path.clone(),
            item_index: index,
            item_key,
            local_name: String::new(),
        };
        let mut instance = Node::new(NodeKind::Stem);
        apply_common(&mut instance, page, (None, None))?;
        instance.name = Some(format!("{instance_prefix}page"));
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
    context: &Value,
    prefix: &str,
    instance_path: &str,
    info: &InstanceInfo,
    scrolls: &HashMap<String, f32>,
    measure: Option<&LayoutMeasureHook>,
    templates: &mut HashMap<PathBuf, Value>,
    visiting: &mut Vec<PathBuf>,
    depth: usize,
    axis: Axis,
) -> mlua::Result<f32> {
    let mut tree = PageTree::new(Node::new(NodeKind::Stem));
    apply_common(&mut tree.nodes[0], page, (None, None))?;
    let mut payloads = vec![None];
    let mut names = HashSet::new();
    let mut instances = HashMap::new();
    let mut paths = HashMap::new();
    let children = parse_children(
        &mut tree,
        page,
        base,
        &mut payloads,
        &mut names,
        &mut instances,
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

fn make_context(data: &Value, item: &Value, index: usize, key: Value) -> Value {
    let mut context = Map::new();
    context.insert("item".to_owned(), item.clone());
    context.insert("index".to_owned(), Value::from(index as u64));
    context.insert("key".to_owned(), key);
    context.insert("data".to_owned(), data.clone());
    if let Value::Object(fields) = data {
        for (name, value) in fields {
            context.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    Value::Object(context)
}

fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.').filter(|part| !part.is_empty()) {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn bind_value(value: Value, context: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(interpolate(&text, context)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| bind_value(item, context))
                .collect(),
        ),
        Value::Object(mut fields) => {
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

fn interpolate(text: &str, context: &Value) -> String {
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
        if let Some(value) = lookup_path(context, path) {
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
fn alias_subtree(paths: &mut HashMap<String, NodeId>, base: &str, alias: &str) {
    let prefix = format!("{base}.");
    let entries: Vec<(String, NodeId)> = paths
        .iter()
        .filter(|(path, _)| path.as_str() == base || path.starts_with(&prefix))
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
