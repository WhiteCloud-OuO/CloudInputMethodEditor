//! 子页面脚本：创建实例时初始化，事件回到所属实例，并可通过 win:*_at 控制控件。

use cloudime_script::{Runtime, WindowRequest};

#[test]
fn subpage_scripts_receive_created_and_instance_events() {
    let root = std::env::temp_dir().join("cloudime-subpage-script");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Scripts/layouts")).unwrap();
    std::fs::create_dir_all(root.join("Scripts/shared")).unwrap();
    std::fs::write(
        root.join("main.lua"),
        r#"
cloudime.script{
    name = "子页面脚本", description = "subpage script", api = 2,
    trigger_condition = "manual", timeout = 20, sync = true, handover = "return",
    on_error = function(_, message) error(message) end,
}
cloudime.on("manual", function()
    local items = {
        { id = 1, title = "A" },
        { id = 2, title = "B" },
    }
    cloudime.window.create{
        name = "子页面脚本", width = 240, height = 120,
        layout = cloudime.layout_path("main.json"),
        data = { items = items },
    }
end)
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/shared/row.lua"),
        r#"
return {
    created = function(ctx)
        ctx.window:set_text_at(ctx.path("edit"), "created-" .. ctx.index .. "-" .. ctx.item.title)
    end,
    event = function(ctx, event)
        if event.action == "clicked" then
            ctx.window:set_text_at(ctx.path("edit"), "clicked-" .. ctx.index)
            return true
        end
        return false
    end,
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/layouts/row.json"),
        r#"{
  "version": 2,
  "page": {
    "w": 200, "h": 30,
    "children": [{
      "type": "text_edit", "name": "edit", "w": 180, "h": 24,
      "position": "absolute", "inset": { "left": 4, "top": 3 }
    }]
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Scripts/layouts/main.json"),
        r#"{
  "version": 2,
  "page": {
    "w": 240, "h": 120,
    "children": [{
      "type": "list_box", "name": "list", "w": 220, "h": 100,
      "self_adaption": false, "subpage": ["row.json"], "repeat": "items",
      "key": "id", "item_height": 30, "script": "row.lua"
    }]
  }
}"#,
    )
    .unwrap();

    let runtime = Runtime::load_file(&root.join("main.lua"));
    runtime.set_install_dir(root.clone());
    let event = runtime.lua().create_table().unwrap();
    runtime.dispatch("manual", event, None);
    let requests = runtime.take_window_requests();
    let id = requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::Create(id, _) => Some(*id),
            _ => None,
        })
        .expect("窗口创建");
    assert!(
        requests.iter().any(|request| matches!(
            request,
            WindowRequest::SetText { text, .. } if text.starts_with("created-")
        )),
        "created 应通过 win:set_text_at 写入控件"
    );

    let payload = runtime.lua().create_table().unwrap();
    payload.set("id", id).unwrap();
    payload.set("instance", "list.items[0]".to_owned()).unwrap();
    payload.set("action", "clicked".to_owned()).unwrap();
    payload.set("control", "edit".to_owned()).unwrap();
    runtime.dispatch("window", payload, None);
    let updates = runtime.take_window_requests();
    assert!(
        updates.iter().any(|request| matches!(
            request,
            WindowRequest::SetText { text, .. } if text == "clicked-0"
        )),
        "子页面 event 应收到实例事件并写回控件"
    );

    let _ = std::fs::remove_dir_all(&root);
}
