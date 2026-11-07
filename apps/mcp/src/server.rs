//! rmcp 的 ServerHandler：只暴露三个只读工具，查询放到 blocking 线程。

use std::sync::{Arc, Mutex};

use cloudime_core::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, tool, tool_handler, tool_router};

use crate::data::DictionaryInfoResponse;
use crate::tools::{LookupRequest, LookupResponse, SegmentRequest, SegmentResponse};

/// 云朵只读 MCP Server。
#[derive(Clone)]
pub struct CloudimeServer {
    engine: Arc<Mutex<Engine>>,
    info: Arc<DictionaryInfoResponse>,
    tool_router: ToolRouter<Self>,
}

impl CloudimeServer {
    /// 用已装载的 Engine 与词库信息建服务。
    pub fn new(engine: Engine, info: DictionaryInfoResponse) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
            info: Arc::new(info),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router(router = tool_router)]
impl CloudimeServer {
    /// 返回拼音的所有切分方式和首选切分。
    #[tool(
        name = "cloudime_segment_pinyin",
        description = "切分拼音串，返回所有合法切分、首选切分和末尾未完成部分"
    )]
    pub async fn segment_pinyin(
        &self,
        parameters: Parameters<SegmentRequest>,
    ) -> Result<Json<SegmentResponse>, String> {
        let request = parameters.0;
        let input = request.pinyin;
        let response_input = input.clone();
        let use_jian_pin = request.use_jian_pin;
        let engine = Arc::clone(&self.engine);
        let query = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().map_err(|_| "查询线程状态锁失败".to_owned())?;
            crate::tools::query(&mut engine, &input, use_jian_pin)
        })
        .await
        .map_err(|error| format!("查询线程退出：{error}"))??;
        Ok(Json(crate::tools::build_segment_response(
            &response_input,
            &query,
        )))
    }

    /// 按拼音查询候选。
    #[tool(
        name = "cloudime_lookup_pinyin",
        description = "按拼音查询云朵词库，返回排序后的候选、切分和权重"
    )]
    pub async fn lookup_pinyin(
        &self,
        parameters: Parameters<LookupRequest>,
    ) -> Result<Json<LookupResponse>, String> {
        let request = parameters.0;
        let input = request.pinyin;
        let response_input = input.clone();
        let limit = request.limit;
        let use_jian_pin = request.use_jian_pin;
        let engine = Arc::clone(&self.engine);
        let query = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().map_err(|_| "查询线程状态锁失败".to_owned())?;
            crate::tools::query(&mut engine, &input, use_jian_pin)
        })
        .await
        .map_err(|error| format!("查询线程退出：{error}"))??;
        Ok(Json(crate::tools::build_lookup_response(
            &response_input,
            &query,
            limit,
        )))
    }

    /// 列出当前加载的词库信息。
    #[tool(
        name = "cloudime_dictionary_info",
        description = "列出主词库与已加载第三方词库的名称、条数和许可证"
    )]
    pub async fn dictionary_info(&self) -> Json<DictionaryInfoResponse> {
        Json((*self.info).clone())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CloudimeServer {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("cloudime", env!("CARGO_PKG_VERSION"));
        info.instructions = Some("只读查询云朵输入法的词库：拼音切分、候选与词库信息。".to_owned());
        info
    }
}
