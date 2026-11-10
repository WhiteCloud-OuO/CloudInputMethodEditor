//! 开发用的最小 TSF UI-less 宿主（Windows only）。

#[cfg(windows)]
mod host;
#[cfg(windows)]
mod text_store;
#[cfg(windows)]
mod uielement_sink;

#[cfg(windows)]
fn main() {
    host::run(std::env::args().any(|arg| arg == "--auto"));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("cloudime-uiless-host 只在 Windows 上运行");
}
