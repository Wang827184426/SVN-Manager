//! 开机自启（仅 Windows）：在 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! 写一个指向当前 exe 的 REG_SZ 值，登录后 Windows 自动拉起本程序；删掉即关闭。
//! 只动 HKCU，不需要管理员权限；显示状态以注册表为准（用户手动删了值也能如实反映），
//! 配置文件里的 `auto_start` 字段只做记录。

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW, RegSetValueExW,
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
    };

    /// 写入的注册表值名（任务管理器 → 启动应用里显示的名字）
    const VALUE_NAME: &str = "SVNManager";
    const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// 打开（必要时创建）Run 键，执行 `f` 后负责关闭
    fn with_run_key<T>(sam: u32, f: impl FnOnce(HKEY) -> Result<T, String>) -> Result<T, String> {
        let subkey = wide(RUN_SUBKEY);
        let mut hkey: HKEY = std::ptr::null_mut();
        let mut created: u32 = 0;
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                0,
                sam,
                std::ptr::null(),
                &mut hkey,
                &mut created,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("打开注册表 Run 键失败（错误码 {rc}）"));
        }
        let result = f(hkey);
        unsafe { RegCloseKey(hkey) };
        result
    }

    pub fn is_enabled() -> bool {
        with_run_key(KEY_QUERY_VALUE, |hkey| {
            let rc = unsafe {
                RegQueryValueExW(
                    hkey,
                    wide(VALUE_NAME).as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
                Ok(rc == ERROR_SUCCESS)
            } else {
                Err(format!("读取注册表失败（错误码 {rc}）"))
            }
        })
        .unwrap_or(false)
    }

    pub fn set_enabled(enable: bool) -> Result<(), String> {
        if enable {
            let exe = std::env::current_exe().map_err(|e| format!("取程序路径失败：{e}"))?;
            // 路径整体加引号：放在带空格的目录（如 Program Files）里也能正确拉起
            let command = format!("\"{}\"", exe.display());
            let data = wide(&command);
            with_run_key(KEY_SET_VALUE, |hkey| {
                let rc = unsafe {
                    RegSetValueExW(
                        hkey,
                        wide(VALUE_NAME).as_ptr(),
                        0,
                        REG_SZ,
                        data.as_ptr().cast(),
                        (data.len() * 2) as u32,
                    )
                };
                if rc == ERROR_SUCCESS {
                    Ok(())
                } else {
                    Err(format!("写入自启注册表值失败（错误码 {rc}）"))
                }
            })
        } else {
            with_run_key(KEY_SET_VALUE, |hkey| {
                let rc = unsafe { RegDeleteValueW(hkey, wide(VALUE_NAME).as_ptr()) };
                match rc {
                    // 值本来就不存在也当成功：目标状态就是「没有」
                    ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
                    _ => Err(format!("删除自启注册表值失败（错误码 {rc}）")),
                }
            })
        }
    }
}

#[cfg(windows)]
pub use imp::{is_enabled, set_enabled};

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_enabled(_enable: bool) -> Result<(), String> {
    Err("开机自启目前只支持 Windows".to_owned())
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::{is_enabled, set_enabled};

    /// 真机演练注册表开关往返：开 → 查得到，关 → 查不到，最后恢复原状态。
    /// 会真的写/删 HKCU Run 键（不需要管理员权限），默认跳过，
    /// 手动跑：`cargo test -- --ignored autostart_roundtrip`
    #[test]
    #[ignore]
    fn autostart_roundtrip() {
        let original = is_enabled();
        set_enabled(true).unwrap();
        assert!(is_enabled(), "写入 Run 键后 is_enabled 应为真");
        set_enabled(false).unwrap();
        assert!(!is_enabled(), "删除 Run 键后 is_enabled 应为假");
        // 别动用户已有的设置：原来开着就恢复开着
        set_enabled(original).unwrap();
        assert_eq!(is_enabled(), original);
    }
}
