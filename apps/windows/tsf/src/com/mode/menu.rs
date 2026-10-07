//! 任务栏「中 / 英」图标的右键菜单。做法同小狼毫：`TrackPopupMenuEx` 挂在输入框所在窗口上，同步取回点的项。
//! 固定几项：灰显的标题、分隔线、「设置」「查看帮助手册」「重启输入法服务」；中 / 英切换不在这里。

use std::path::{Path, PathBuf};
use std::process::Command;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, MF_GRAYED, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL,
    TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{HSTRING, PCWSTR, w};

use cloudime_platform::protocol::IndicatorCommand;

use crate::com::log::log;
use crate::com::module_path;

/// 灰显标题的命令 id：给 0，点不到也取不回。
const ID_TITLE: u32 = 0;
const ID_SETTINGS: u32 = 1;
const ID_RESTART: u32 = 2;
const ID_TUTORIAL: u32 = 3;

/// 在 `point`（屏幕坐标）弹出菜单，阻塞到用户点了某项或点别处关掉。返回选中项要交给 Server 的命令。
pub(crate) fn track(owner: HWND, point: POINT) -> Option<IndicatorCommand> {
    let menu = unsafe { CreatePopupMenu() }.ok()?;
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED,
            ID_TITLE as usize,
            &HSTRING::from("云朵输入法"),
        )
    };
    let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            ID_SETTINGS as usize,
            &HSTRING::from("设置"),
        )
    };
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            ID_TUTORIAL as usize,
            &HSTRING::from("查看帮助手册"),
        )
    };
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            ID_RESTART as usize,
            &HSTRING::from("重启输入法服务"),
        )
    };
    // 图标在任务栏上，菜单往上弹
    let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN;
    let id = unsafe { TrackPopupMenuEx(menu, flags.0, point.x, point.y, owner, None) };
    let _ = unsafe { DestroyMenu(menu) };
    match id.0 as u32 {
        ID_SETTINGS => Some(IndicatorCommand::OpenSettings),
        ID_RESTART => Some(IndicatorCommand::RestartServer),
        ID_TUTORIAL => {
            open_tutorial();
            None
        }
        _ => None,
    }
}

/// 用系统默认程序打开随包的使用手册（与 DLL 同目录的 `tutorial.md`）。
///
/// 就地打开、不走 Server：那得给协议加一个 `IndicatorCommand` 变体，还要升协议版本、重装 DLL
/// （见 `docs/notes/crate-notes.md` 的协议一节）。打不开只记日志，绝不影响输入法。
fn open_tutorial() {
    let Some(module) = module_path().ok() else {
        log("拿不到 DLL 路径，打不开使用手册");
        return;
    };
    let path = tutorial_path(Path::new(&module.to_string()));
    // 没有默认打开方式（`.md` 在很多机器上就没关联）：直接用记事本，
    // 别让 Windows 先弹一个「你要如何打开这个文件？」。
    if !has_open_association(&path) {
        match Command::new("notepad.exe").arg(&path).spawn() {
            Ok(_) => log("系统没有 .md 的默认打开方式，已用记事本打开使用手册"),
            Err(error) => log(&format!("用记事本打开使用手册失败：{error}")),
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
    // ShellExecuteW 返回值 > 32 才算成功；文件不在时是个小错误码（不会弹对话框）。
    if (code.0 as isize) > 32 {
        log("已打开使用手册");
    } else {
        log(&format!("打开使用手册失败，返回值 {}", code.0 as isize));
    }
}

/// 这个文件类型在系统里有没有「默认打开方式」。问 shell 要关联程序的可执行文件路径，
/// 只问长度（`pszout` 给 null 时它把需要的大小写进 `size`）；没关联时是 0。
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
    // 「要给多大缓冲」是 S_FALSE，也算成功；真没关联时 size 为 0
    queried.is_ok() && size > 1
}

/// 与 DLL 同目录的 `tutorial.md`（安装器把手册装在这里）。
fn tutorial_path(module: &Path) -> PathBuf {
    module.with_file_name("tutorial.md")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{has_open_association, tutorial_path};

    #[test]
    fn tutorial_sits_next_to_the_dll() {
        assert_eq!(
            tutorial_path(Path::new(r"D:\Program Files\CloudIME\cloudime_tsf_x64.dll")),
            Path::new(r"D:\Program Files\CloudIME\tutorial.md")
        );
    }

    #[test]
    fn open_association_is_detected_only_for_known_types() {
        // `.txt` 在 Windows 上一定有默认打开方式（记事本）；没有扩展名的路径一律当成「没关联」。
        // 这两条都不假设 `.md` 有没有关联——那正好是我们要兼容的情况。
        assert!(has_open_association(Path::new(r"C:\Windows\win.ini")));
        assert!(has_open_association(Path::new(
            r"C:\Windows\notepad.exe.txt"
        )));
        assert!(!has_open_association(Path::new(r"C:\Windows")));
        assert!(!has_open_association(Path::new(
            r"C:\Windows\System32\drivers\etc\hosts"
        )));
    }
}
