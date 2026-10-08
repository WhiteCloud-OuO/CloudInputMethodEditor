//! 脚本发起的 HTTP：一次性线程里跑（与 `cloudime-update` 同一套 reqwest + 单线程 tokio），
//! 派发线程只做「收结果 → 调回调」—— **请求本身不阻塞输入**。
//!
//! 脚本那边是 `cloudime.http_get(url, timeout_ms, callback)` 与
//! `cloudime.http_post(url, body, { timeout_ms = …, headers = { … } }, callback)`：
//! 时限都是**必须给**的响应时间限制，超时才回来的结果由 [`super::Runtime::poll_requests`] 直接丢掉。

use std::time::Duration;

/// 请求方法。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Method {
    Get,
    Post,
}

/// 一次请求的结果。
pub(super) struct Outcome {
    /// HTTP 状态码；连都没连上时 `None`。
    pub status: Option<u16>,

    /// 响应体（按 UTF-8 宽松解码）。
    pub body: String,

    /// 出错的说明（连不上 / 超时 / 读不出体）；成功时 `None`。
    pub error: Option<String>,
}

/// 发一次请求（调用方自己起线程），`timeout` 是脚本给的响应时间限制：
/// 到了还没回来就由 reqwest 掐掉，这里给出 `error`。
pub(super) fn request(
    method: Method,
    url: &str,
    body: &str,
    headers: &[(String, String)],
    timeout: Duration,
) -> Outcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return failed(format!("起 tokio 运行时失败：{error}")),
    };
    runtime.block_on(async {
        let client = match reqwest::Client::builder()
            .timeout(timeout)
            // 脚本自己的请求：只报一个笼统的 UA，没有别的标识
            .user_agent("cloudime-script")
            .build()
        {
            Ok(client) => client,
            Err(error) => return failed(format!("建 HTTP 客户端失败：{error}")),
        };
        let mut builder = match method {
            Method::Get => client.get(url),
            Method::Post => client.post(url).body(body.to_owned()),
        };
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        match builder.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                match response.text().await {
                    Ok(body) => Outcome {
                        status: Some(status),
                        body,
                        error: None,
                    },
                    Err(error) => Outcome {
                        status: Some(status),
                        body: String::new(),
                        error: Some(format!("读响应体失败：{error}")),
                    },
                }
            }
            Err(error) => failed(error.to_string()),
        }
    })
}

/// 表头能不能用（名字 / 值不合规时给脚本一条清楚的错，而不是在后台线程里 panic）。
pub(super) fn check_headers(headers: &[(String, String)]) -> Result<(), String> {
    for (name, value) in headers {
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(format!("表头名字不合法：{name}"));
        }
        if reqwest::header::HeaderValue::from_str(value).is_err() {
            return Err(format!("表头 {name} 的值不合法"));
        }
    }
    Ok(())
}

fn failed(error: String) -> Outcome {
    Outcome {
        status: None,
        body: String::new(),
        error: Some(error),
    }
}
