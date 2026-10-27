//! `list_box` 的页面树展开：一个模板按数据重复，行宽高进入 `Element::ListBox`。

use cloudime_render::{Color, Document, Element};
use cloudime_script::{Runtime, WindowRequest, mlua};
use serde_json::json;

fn event(runtime: &Runtime, fields: &[(&str, mlua::Value)]) -> mlua::Table {
    let table = runtime.lua().create_table().unwrap();
    for (name, value) in fields {
        table.set(*name, value.clone()).unwrap();
    }
    table
}

fn document_of(requests: &[WindowRequest]) -> &Document {
    requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::SetContent(_, document) => Some(document),
            _ => None,
        })
        .expect("送了内容")
}

#[test]
fn list_box_expands_the_first_subpage_for_every_item() {
    let root = std::env::temp_dir().join("cloudime-script-list-box-layout");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Scripts/layouts")).unwrap();
    std::fs::write(
        root.join("list-box.lua"),
        r#"
cloudime.script{
    name = "列表盒测试",
    description = "list_box 布局测试",
    api = 2,
    trigger_condition = "manual",
    timeout = 20,
    sync = true,
    handover = "return",
    on_error = function(_, message)
        _G.script_error = message
    end,
}

cloudime.on("manual", function()
    local win = cloudime.window.create{
        name = "列表盒测试",
        width = 300,
        height = 200,
        layout = cloudime.layout_path("list-box.json"),
    }
    _G.item_count = win:get_item_count("list")
    _G.selected_index = win:get_selected_index("list")
    _G.scroll_offset = win:get_scroll_offset("list")
    local list = win:list_box("list")
    _G.handle_item_count = list.item_count
    _G.handle_selected = list.selected_index
    _G.handle_offset = list.scroll_offset
    list.selected_index = 1
    _G.handle_written = list.selected_index
    local item = list:get_item(2)
    _G.item_index = item.index
    _G.item_key = item.key
end)
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/layouts/row.json"),
        r##"{
  "version": 2,
  "page": {
    "w": 180,
    "h": 30,
    "children": [
      {
        "name": "title",
        "type": "label",
        "runs": [
          {
            "text": "行",
            "color": {
              "state": {
                "0": "#FF222222",
                "1": "#FFFF0000",
                "2": "#FF0000FF",
                "default": "#FF222222"
              }
            }
          }
        ],
        "size": 14
      },
      {
        "name": "row_edit",
        "type": "text_edit",
        "w": 120,
        "h": 24
      }
    ]
  }
}"##,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/layouts/list-box.json"),
        r##"{
  "version": 2,
  "page": {
    "w": 300,
    "h": 200,
    "data": {
      "items": [
        { "label": "一" },
        { "label": "二" },
        { "label": "三" }
      ]
    },
    "children": [
      {
        "name": "list",
        "type": "list_box",
        "w": 200,
        "h": 100,
        "allow_select": true,
        "auto_center_selected": true,
        "fill": "#FFFFFFFF",
        "subpage": ["row.json"],
        "repeat": "items",
        "gap": 4
      }
    ]
  }
}"##,
    )
    .unwrap();

    let runtime = Runtime::load_file(&root.join("list-box.lua"));
    runtime.set_install_dir(root.clone());
    runtime.dispatch("manual", event(&runtime, &[]), None);
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<String>>("script_error")
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("item_count")
            .unwrap(),
        Some(3)
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("selected_index")
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("scroll_offset")
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("handle_item_count")
            .unwrap(),
        Some(3)
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("handle_selected")
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("handle_offset")
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("handle_written")
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<i64>>("item_index")
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        runtime
            .lua()
            .globals()
            .get::<Option<String>>("item_key")
            .unwrap(),
        Some("2".to_owned())
    );
    let requests = runtime.take_window_requests();
    let document = document_of(&requests);
    let list = document
        .elements
        .iter()
        .find_map(|element| match element {
            Element::ListBox(list) => Some(list),
            _ => None,
        })
        .expect("列表盒元素");
    assert_eq!(list.rows.len(), 3, "三行");
    assert!(list.allow_select);
    assert!(list.auto_center_selected);
    assert!(list.auto_item_gap_averaging);
    assert_eq!(list.content_height, 30.0 * 3.0 + 4.0 * 2.0);
    assert!(list.scrollbar_visible());
    assert!(
        list.rows[0]
            .elements
            .iter()
            .any(|element| matches!(element, Element::Label(_))),
        "行模板里的 label 进了行内容"
    );
    let label_color = |elements: &[Element]| {
        elements
            .iter()
            .find_map(|element| match element {
                Element::Label(label) => Some(label.runs[0].color),
                _ => None,
            })
            .expect("行模板里的 label")
    };
    assert_eq!(
        label_color(&list.rows[0].elements),
        Color::rgb(0x22, 0x22, 0x22)
    );
    assert_eq!(
        label_color(&list.rows[0].selected_elements),
        Color::rgb(0xFF, 0x00, 0x00)
    );
    assert_eq!(
        label_color(&list.rows[0].hovered_elements),
        Color::rgb(0x00, 0x00, 0xFF)
    );
    let row_edit = document
        .text_edits()
        .into_iter()
        .find(|placement| placement.path.len() == 3)
        .expect("行内 text_edit 的路径是 list_box / 行 / 控件");
    assert!(matches!(
        document.element_at_path(&row_edit.path),
        Some(Element::TextEdit(_))
    ));
    assert!(row_edit.clip.is_some(), "行内编辑框带列表盒裁剪矩形");
}

#[test]
fn fixed_height_list_box_only_instantiates_the_visible_window() {
    let root = std::env::temp_dir().join("cloudime-script-list-box-virtual");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Scripts/layouts")).unwrap();
    std::fs::write(
        root.join("virtual.lua"),
        r#"
cloudime.script{
    name = "列表盒虚拟化",
    description = "list_box 虚拟化测试",
    api = 2,
    trigger_condition = "manual",
    timeout = 20,
    sync = true,
    handover = "return",
    on_error = function(_, message) error(message) end,
}
cloudime.on("manual", function()
    local win = cloudime.window.create{
        name = "列表盒虚拟化",
        width = 320,
        height = 140,
        layout = cloudime.layout_path("virtual.json"),
    }
    win:set_scroll("list", 480)
    local list = win:list_box("list")
    local item = list:get_item(90)
    _G.pinned_index = item.index
end)
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/layouts/row.json"),
        r##"{
  "version": 2,
  "page": {
    "w": 280, "h": 24,
    "children": [{
      "type": "label", "name": "title",
      "runs": [{ "text": "{{item.label}}", "color": "#FF222222" }],
      "size": 14
    }]
  }
}"##,
    )
    .unwrap();
    let items: Vec<_> = (0..100)
        .map(|index| json!({ "id": index, "label": format!("row {index}") }))
        .collect();
    let layout = json!({
        "version": 2,
        "page": {
            "w": 320,
            "h": 140,
            "data": { "items": items },
            "children": [{
                "name": "list",
                "type": "list_box",
                "w": 300,
                "h": 130,
                "self_adaption": false,
                "subpage": ["row.json"],
                "repeat": "items",
                "key": "id",
                "item_height": 24
            }]
        }
    });
    std::fs::write(
        root.join("Scripts/layouts/virtual.json"),
        serde_json::to_vec(&layout).unwrap(),
    )
    .unwrap();

    let runtime = Runtime::load_file(&root.join("virtual.lua"));
    runtime.set_install_dir(root.clone());
    runtime.dispatch("manual", event(&runtime, &[]), None);
    let requests = runtime.take_window_requests();
    let document = requests
        .iter()
        .filter_map(|request| match request {
            WindowRequest::SetContent(_, document) => Some(document),
            _ => None,
        })
        .next_back()
        .expect("最后一份内容");
    let list = document
        .elements
        .iter()
        .find_map(|element| match element {
            Element::ListBox(list) => Some(list),
            _ => None,
        })
        .expect("列表盒元素");
    assert_eq!(list.row_count, 100, "逻辑行数保持完整");
    assert!(
        list.rows.len() <= 24,
        "只实例化可见窗口，实际 {} 行",
        list.rows.len()
    );
    let first_visible = list
        .rows
        .iter()
        .find(|row| row.index == 20)
        .expect("第 20 行（480 / 24）仍在窗口内");
    assert_eq!(first_visible.top, 480.0);
    assert!(
        list.rows[0].index < 20,
        "向上留 overscan，窗口首行早于第 20 行"
    );
    assert_eq!(list.scroll_offset, 480.0);
    assert_eq!(list.content_height, 2400.0);
    assert!(
        requests.iter().any(|request| matches!(
            request,
            WindowRequest::SetListWindow { window, .. }
                if window.rows.iter().any(|row| row.index == 90)
        )),
        "get_item(90) 应按需挂载目标行"
    );

    let id = requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::Create(id, _) => Some(*id),
            _ => None,
        })
        .expect("窗口编号");
    let scroll_event = event(
        &runtime,
        &[
            ("id", mlua::Value::Integer(id as i64)),
            (
                "control",
                mlua::Value::String(runtime.lua().create_string("list").unwrap()),
            ),
            (
                "action",
                mlua::Value::String(runtime.lua().create_string("scroll_changed").unwrap()),
            ),
            ("value", mlua::Value::Integer(40)),
        ],
    );
    runtime.dispatch("window", scroll_event, None);
    let updates = runtime.take_window_requests();
    let window = updates
        .iter()
        .find_map(|request| match request {
            WindowRequest::SetListWindow { name, window, .. } if name == "list" => Some(window),
            _ => None,
        })
        .expect("滚动后只发送列表窗口");
    assert!(window.rows.iter().any(|row| row.index == 40));
    assert!(window.rows[0].index < 40, "向上留 overscan");
    assert_eq!(window.row_count, 100);
    assert!(window.rows.len() <= 26, "窗口行 + get_item(90) 的 pin 行");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn auto_height_list_box_measures_only_the_visible_window() {
    let root = std::env::temp_dir().join("cloudime-script-list-box-auto-virtual");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Scripts/layouts")).unwrap();
    std::fs::write(
        root.join("auto.lua"),
        r#"
cloudime.script{
    name = "列表盒自动行高",
    description = "auto virtualize",
    api = 2,
    trigger_condition = "manual",
    timeout = 20,
    sync = true,
    handover = "return",
    on_error = function(_, message) error(message) end,
}
cloudime.on("manual", function()
    cloudime.window.create{
        name = "列表盒自动行高",
        width = 320,
        height = 140,
        layout = cloudime.layout_path("auto.json"),
    }
end)
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/layouts/row.json"),
        r##"{
  "version": 2,
  "page": {
    "w": 280,
    "children": [{
      "type": "label", "name": "title",
      "runs": [{ "text": "{{item.label}}", "color": "#FF222222" }],
      "size": 14
    }]
  }
}"##,
    )
    .unwrap();
    let items: Vec<_> = (0..100)
        .map(|index| json!({ "id": index, "label": format!("row {index}") }))
        .collect();
    let layout = json!({
        "version": 2,
        "page": {
            "w": 320,
            "h": 140,
            "data": { "items": items },
            "children": [{
                "name": "list",
                "type": "list_box",
                "w": 300,
                "h": 130,
                "self_adaption": false,
                "subpage": ["row.json"],
                "repeat": "items",
                "key": "id"
            }]
        }
    });
    std::fs::write(
        root.join("Scripts/layouts/auto.json"),
        serde_json::to_vec(&layout).unwrap(),
    )
    .unwrap();

    let runtime = Runtime::load_file(&root.join("auto.lua"));
    runtime.set_install_dir(root.clone());
    runtime.dispatch("manual", event(&runtime, &[]), None);
    let requests = runtime.take_window_requests();
    let document = requests
        .iter()
        .filter_map(|request| match request {
            WindowRequest::SetContent(_, document) => Some(document),
            _ => None,
        })
        .next_back()
        .expect("内容");
    let list = document
        .elements
        .iter()
        .find_map(|element| match element {
            Element::ListBox(list) => Some(list),
            _ => None,
        })
        .expect("列表盒元素");
    assert_eq!(list.row_count, 100);
    assert!(
        list.rows.len() <= 20,
        "auto 行高也只实例化可见窗口，实际 {} 行",
        list.rows.len()
    );
    assert_eq!(list.rows[0].index, 0);
    assert_eq!(list.rows[0].top, 0.0);
    assert!(list.content_height > 0.0);

    let id = requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::Create(id, _) => Some(*id),
            _ => None,
        })
        .expect("窗口编号");
    let scroll_event = event(
        &runtime,
        &[
            ("id", mlua::Value::Integer(id as i64)),
            (
                "control",
                mlua::Value::String(runtime.lua().create_string("list").unwrap()),
            ),
            (
                "action",
                mlua::Value::String(runtime.lua().create_string("scroll_changed").unwrap()),
            ),
            ("value", mlua::Value::Integer(50)),
        ],
    );
    runtime.dispatch("window", scroll_event, None);
    let updates = runtime.take_window_requests();
    let window = updates
        .iter()
        .find_map(|request| match request {
            WindowRequest::SetListWindow { window, .. } => Some(window),
            _ => None,
        })
        .expect("auto 行高滚动窗口");
    assert!(window.rows.iter().any(|row| row.index == 50));
    assert!(window.rows[0].index < 50, "向上留 overscan");
    assert!(window.rows.len() <= 24);
    let _ = std::fs::remove_dir_all(&root);
}
