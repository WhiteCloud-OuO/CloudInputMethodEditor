use serde::Deserialize;

/// 一个安装包：只看它是给哪个平台、哪种 CPU 的。
#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub platform: String,

    /// `arm64` / `x86_64`；0.0.1 之前的索引没有这个字段。
    #[serde(default)]
    pub cpu: String,
}
