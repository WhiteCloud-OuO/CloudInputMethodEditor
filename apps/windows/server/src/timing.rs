//! 只读分段打点：环境变量 `CLOUDIME_TIMING` 存在时才输出，平时不改变行为也不写日志。
//!
//! 与 TSF DLL 共用同一个开关（DLL 与 Server 都从宿主进程继承环境）；打点统一带
//! `cloudime_timing` target，方便在日志里筛。

use std::sync::OnceLock;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CLOUDIME_TIMING").is_some())
}
