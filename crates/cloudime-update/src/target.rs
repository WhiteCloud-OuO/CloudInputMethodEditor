/// 这台机器要哪个安装包：索引里资产的 `platform` 与 `cpu`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub platform: &'static str,

    pub cpu: &'static str,
}

impl Target {
    /// Windows：安装包只有 x86_64 一种（arm64 机器上也用它）。
    pub fn current() -> Self {
        Self {
            platform: "windows",
            cpu: "x86_64",
        }
    }
}
