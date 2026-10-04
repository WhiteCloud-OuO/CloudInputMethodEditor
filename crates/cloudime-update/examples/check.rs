//! 手动走一遍检查更新：`cargo run -p cloudime-update --example check -- <当前版本> [stable|beta]`。
//! `CLOUDIME_UPDATE_INDEX` 可以指向本地起的静态服务（签名照验）。

use std::time::Duration;

use cloudime_platform::{UpdateChannel, UpdateConfig};
use cloudime_update::Checker;

fn main() {
    let mut args = std::env::args().skip(1);
    let current = args.next().unwrap_or_else(|| "0.0.1".to_owned());
    let channel = match args.next().as_deref() {
        Some("beta") => UpdateChannel::Beta,
        _ => UpdateChannel::Stable,
    };
    let config = UpdateConfig {
        check: true,
        channel,
    };
    let state = std::env::temp_dir().join(format!("cloudime-update-{}.json", std::process::id()));
    let checker = Checker::new(state.clone(), &current);
    checker.poll(&config);
    while checker.checking() {
        std::thread::sleep(Duration::from_millis(100));
    }
    match checker.available(&config) {
        Some(found) => println!(
            "{current} → {} ({}, {})",
            found.version, found.channel, found.date
        ),
        None if checker.checked_at() > 0 => println!("{current}：已是最新"),
        None => println!("{current}：没查成（开发版、断网或验签失败）"),
    }
    let _ = std::fs::remove_file(state);
}
