//! 状态条「工具」按钮的弹出菜单：列 exe 旁 `tools\tools.list` 里登记的工具，点了就起它。
//!
//! 清单每行 `工具短路径=菜单项名称`（如 `cwt.exe=词库转换工具`），短路径相对 `tools\` 目录；
//! 空行与 `#` 开头忽略；文件不在的项跳过（记一条日志）。清单读不到 / 一个可用项都没有就什么都不弹。

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, MF_SEPARATOR, MF_STRING, PostMessageW,
    SW_SHOWNORMAL, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR, w};

/// 清单文件名（在 `tools\` 下）。
const LIST_FILE: &str = "tools.list";

/// 「查看帮助手册」的菜单命令 id；工具项用 1 起，避开一段距离。
const HELP_COMMAND: usize = 0x7000;

/// 一个菜单里能起的工具。
struct Tool {
    /// 文件完整路径。
    path: PathBuf,

    /// 菜单项名称。
    name: String,
}

impl Tool {
    /// exe 旁的 `tools\` 目录。
    fn dir() -> Option<PathBuf> {
        Some(std::env::current_exe().ok()?.parent()?.join("tools"))
    }

    /// 读清单，挑出文件确实在的项。
    fn load() -> Vec<Self> {
        let Some(dir) = Self::dir() else {
            return Vec::new();
        };
        let list = dir.join(LIST_FILE);
        let Ok(text) = std::fs::read_to_string(&list) else {
            tracing::debug!(path = %list.display(), "工具清单不在，「工具」菜单没内容");
            return Vec::new();
        };
        parse(&text, &dir)
            .into_iter()
            .filter(|(path, _)| {
                if path.is_file() {
                    return true;
                }
                tracing::warn!(path = %path.display(), "工具清单里的文件不在，跳过");
                false
            })
            .map(|(path, name)| Self { path, name })
            .collect()
    }

    /// 起这个工具，工作目录设成它所在的 `tools\`（与双击一致）。
    ///
    /// 控制台程序（自己就是 `cmd /k` 起：程序跑完控制台留着，看得见输出、还能接着敲命令——
    /// `cwt.exe` 这类不给参数只会打用法，一闪而过什么也看不到）；窗口程序直接起，不要多余的黑框。
    fn launch(&self) {
        let dir = self.path.parent();
        let spawned = if is_console(&self.path) {
            let mut command = std::process::Command::new("cmd");
            command.arg("/k").arg(&self.path);
            if let Some(dir) = dir {
                command.current_dir(dir);
            }
            command.spawn()
        } else {
            let mut command = std::process::Command::new(&self.path);
            if let Some(dir) = dir {
                command.current_dir(dir);
            }
            command.spawn()
        };
        if let Err(error) = spawned {
            tracing::warn!(%error, path = %self.path.display(), "起工具失败");
        }
    }
}

/// PE 可选头里的子系统编号：3 = 控制台、2 = 窗口。
const SUBSYSTEM_CONSOLE: u16 = 3;

/// 这个文件是不是控制台程序：`.bat` / `.cmd` 一律算；`.exe` 看 PE 头的子系统。
/// 读不出来就当窗口程序（直接起；宁可少一个黑框，也不给窗口程序套一层命令行）。
fn is_console(path: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("bat") => true,
        Some(extension) if extension.eq_ignore_ascii_case("cmd") => true,
        Some(extension) if extension.eq_ignore_ascii_case("exe") => {
            pe_subsystem(path) == Some(SUBSYSTEM_CONSOLE)
        }
        _ => false,
    }
}

/// 读 PE 头里的 `Subsystem`；不是 PE / 读不了返回 `None`。
fn pe_subsystem(path: &Path) -> Option<u16> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut offset = [0u8; 4];
    // DOS 头 0x3C 处是 PE 头偏移。
    file.seek(SeekFrom::Start(0x3c)).ok()?;
    file.read_exact(&mut offset).ok()?;
    let pe = u32::from_le_bytes(offset);
    // PE 签名(4) + COFF 头(20) + 可选头里的 Subsystem（PE32 与 PE32+ 都在可选头偏移 68）。
    file.seek(SeekFrom::Start(u64::from(pe) + 4 + 20 + 68))
        .ok()?;
    let mut subsystem = [0u8; 2];
    file.read_exact(&mut subsystem).ok()?;
    Some(u16::from_le_bytes(subsystem))
}

/// 拆清单文本：`短路径=名称` 的行；短路径按 `dir` 拼成完整路径。
fn parse(text: &str, dir: &Path) -> Vec<(PathBuf, String)> {
    let mut items = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((path, name)) = line.split_once('=') else {
            tracing::warn!(line, "工具清单这行看不懂（要 `短路径=名称`），跳过");
            continue;
        };
        let (path, name) = (path.trim(), name.trim());
        if path.is_empty() || name.is_empty() {
            continue;
        }
        items.push((dir.join(path), name.to_owned()));
    }
    items
}

/// 在 `owner`（状态条窗口）上弹「工具」菜单；选了工具就起它，选「查看帮助手册」就开手册。
///
/// 工具清单为空时菜单里仍保留「查看帮助手册」。
pub(super) fn show_menu(owner: HWND) {
    let tools = Tool::load();
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        tracing::warn!("建弹出菜单失败");
        return;
    };
    for (index, tool) in tools.iter().enumerate() {
        let name = wide_z(&tool.name);
        // 命令 id 从 1 起（`TrackPopupMenu` 返回 0 表示没选）。
        if let Err(error) =
            unsafe { AppendMenuW(menu, MF_STRING, index + 1, PCWSTR(name.as_ptr())) }
        {
            tracing::warn!(%error, "往菜单里加项失败");
            let _ = unsafe { DestroyMenu(menu) };
            return;
        }
    }
    if !tools.is_empty() {
        let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
    }
    let help = wide_z("查看帮助手册");
    if let Err(error) = unsafe { AppendMenuW(menu, MF_STRING, HELP_COMMAND, PCWSTR(help.as_ptr())) }
    {
        tracing::warn!(%error, "往菜单里加帮助项失败");
        let _ = unsafe { DestroyMenu(menu) };
        return;
    }
    let mut point = POINT::default();
    let _ = unsafe { GetCursorPos(&mut point) };
    // 弹出前先把前台权抢过来，菜单才会在点别处时消失（状态条本身是 NOACTIVATE 的置顶窗）。
    let _ = unsafe { SetForegroundWindow(owner) };
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            Some(0),
            owner,
            None,
        )
    };
    // 老规矩：菜单收场后给窗口补一条空消息。
    let _ = unsafe { PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0)) };
    let _ = unsafe { DestroyMenu(menu) };
    let command = command.0;
    if command <= 0 {
        return;
    }
    let command = command as usize;
    if command == HELP_COMMAND {
        open_tutorial();
    } else if let Some(tool) = tools.get(command - 1) {
        tool.launch();
    }
}

/// 用系统默认程序打开随包的使用手册（安装目录根部的 `tutorial.md`）。
///
/// `.md` 在很多机器上没有默认打开方式：没有就直接用记事本，别先弹「你要如何打开」。
fn open_tutorial() {
    let Some(root) = cloudime_platform::resources::bundled_root() else {
        tracing::warn!("找不到随包资源根目录，打不开帮助手册");
        return;
    };
    let path = root.join("tutorial.md");
    if !path.is_file() {
        tracing::warn!(path = %path.display(), "帮助手册不在，跳过");
        return;
    }
    if !has_open_association(&path) {
        if let Err(error) = std::process::Command::new("notepad.exe").arg(&path).spawn() {
            tracing::warn!(%error, path = %path.display(), "用记事本打开帮助手册失败");
        }
        return;
    }
    let file = HSTRING::from(path.as_os_str());
    let workdir = path
        .parent()
        .map(|dir| HSTRING::from(dir.as_os_str()))
        .unwrap_or_default();
    let code = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR(workdir.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW 返回值 > 32 才算成功。
    if (code.0 as isize) <= 32 {
        tracing::warn!(code = code.0 as isize, path = %path.display(), "打开帮助手册失败");
    }
}

/// 这个文件类型在系统里有没有「默认打开方式」。
fn has_open_association(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    let extension = HSTRING::from(format!(".{extension}"));
    let mut size = 0u32;
    let queried = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            PCWSTR(extension.as_ptr()),
            PCWSTR::null(),
            None,
            &mut size,
        )
    };
    // 「要给多大缓冲」是 S_FALSE，也算成功；真没关联时 size 为 0。
    queried.is_ok() && size > 1
}

/// 以 0 结尾的宽字符串。
fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lines_are_split_into_path_and_name() {
        let text = "# 注释\n\ncwt.exe=词库转换工具\nfoo.exe=另一个工具=带等号\n坏的没有等号\n";
        let parsed = parse(text, std::path::Path::new("tools"));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].1, "词库转换工具");
        assert!(parsed[0].0.ends_with("cwt.exe"));
        // 名字里再带等号：只按第一个等号切
        assert_eq!(parsed[1].1, "另一个工具=带等号");
    }

    #[test]
    fn wide_z_terminates() {
        assert_eq!(wide_z("ab"), [0x61, 0x62, 0]);
    }

    #[test]
    fn console_tools_are_recognized() {
        // 测试二进制本身就是控制台程序
        let exe = std::env::current_exe().unwrap();
        assert_eq!(pe_subsystem(&exe), Some(SUBSYSTEM_CONSOLE));
        assert!(is_console(&exe));
        assert!(is_console(Path::new("a.bat")));
        assert!(is_console(Path::new("a.CMD")));
        assert!(!is_console(Path::new("a.png")));
    }
}
