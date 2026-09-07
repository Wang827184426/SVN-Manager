//! 版本更新检查与自动升级。
//!
//! ## 协议（服务端只需要静态文件）
//!
//! 客户端把「设置 → 版本更新 → 服务端地址」配置成服务根目录，例如
//! `http://192.168.1.10:8666`（也可以直接配到 json 文件本身）。程序启动后读取
//!
//! ```text
//! {服务端地址}/latest.json
//! ```
//!
//! 内容（UTF-8 JSON）：
//!
//! ```json
//! {
//!   "version": "1.2.0",
//!   "url": "files/svn_manager_1.2.0.exe",
//!   "notes": "修复了 xxx",
//!   "sha256": "……（可省略，提供后更新脚本会用 certutil 校验）"
//! }
//! ```
//!
//! - `url` 相对 `latest.json` 所在目录拼接；写完整的 http(s) 地址也可以。
//! - 服务端用 `tools/update_server.py` 即可（发布 + 托管），nginx / IIS 等
//!   静态服务器同样适用。
//!
//! ## 更新流程
//!
//! 发现新版本 → 用户确认 → 程序内下载新 exe（日志区可见进度）→ certutil 校验
//! SHA256（服务端提供时）→ 生成收尾 bat（杀残留实例 → 等主程序退出 → 覆盖 →
//! 重启 → 自删）并启动 → 主程序退出，后续交给 bat 完成。
//! 下载与校验不放進 bat：脚本在磁盘上停留越久，越容易被安全软件当成可疑文件查删。
//! bat 由 `start` 拉起，而 start 打开 .bat 等价于 `cmd /K 脚本`：脚本自删之后只是
//! 「返回」（`exit /b`）的话，cmd 会回到已经不存在的脚本上，打印一句
//! 「找不到批处理文件。」并留下一个空着的命令行窗口——所以脚本结尾必须用 `exit`
//! 直接结束 cmd 进程，成功、失败两条路径都是。
//! bat 里的输出全部用 ASCII，避免任何代码页乱码问题。

use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use crate::svn::CREATE_NO_WINDOW;

use serde::{Deserialize, Serialize};

/// 服务端 latest.json 的字段（缺省字段宽松处理，兼容以后扩展）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateManifest {
    /// 服务端最新版本号，如 "1.2.0"（允许 V1.2.0 / v1.2.0 这类前缀）
    pub version: String,
    /// 新版 exe 下载地址（相对或绝对）
    pub url: String,
    /// 更新说明，界面展示用
    #[serde(default)]
    pub notes: String,
    /// 新 exe 的 SHA256（小写十六进制），非空时 bat 里用 certutil 校验
    #[serde(default)]
    pub sha256: String,
    /// 发布时间，界面展示用
    #[serde(default)]
    pub published_at: String,
}

/// 把版本号拆成数字段："V1.2.10" -> [1, 2, 10]。
/// 非 "主.次.修订" 的部分（构建号后缀等）忽略，解析不出任何数字返回空表。
pub fn parse_version(text: &str) -> Vec<u32> {
    text.trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok())
        .collect()
}

/// 远端版本是否比当前版本新（逐段比较，短的补 0：1.2 > 1.1.9，1.2 == 1.2.0）。
pub fn is_newer(remote: &str, current: &str) -> bool {
    let (remote, current) = (parse_version(remote), parse_version(current));
    for index in 0..remote.len().max(current.len()) {
        let r = remote.get(index).copied().unwrap_or(0);
        let c = current.get(index).copied().unwrap_or(0);
        if r != c {
            return r > c;
        }
    }
    false
}

/// 拼接下载地址：绝对 http(s) URL 原样返回，相对路径拼到服务根目录后面。
pub fn join_url(base: &str, url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_owned();
    }
    let base = base.trim().trim_end_matches('/');
    let url = url.trim_start_matches('/');
    format!("{base}/{url}")
}

/// 服务根目录 -> latest.json 地址。地址本身就指向 .json 时原样使用。
fn manifest_url(server: &str) -> String {
    let server = server.trim().trim_end_matches('/');
    if server.ends_with(".json") {
        server.to_owned()
    } else {
        format!("{server}/latest.json")
    }
}

/// 取 URL 里的主机名（剥掉 scheme、userinfo、端口与路径）。
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    if let Some(stripped) = host.strip_prefix('[') {
        // IPv6 字面量 [::1]:8080
        stripped.split(']').next().unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    }
}

/// 更新地址是否指向内网 / 本机。内网地址绝不该走系统代理：
/// 代理客户端（Clash 等）通过 http_proxy 环境变量劫持 curl 后，
/// 内网请求会被转发到远端节点，结果是超时或 502，永远连不到局域网服务器。
fn is_private_host(url: &str) -> bool {
    let host = host_of(url).to_ascii_lowercase();
    if host == "localhost" || host.starts_with("127.") || host == "::1" {
        return true;
    }
    if host.starts_with("192.168.") || host.starts_with("10.") {
        return true;
    }
    // 172.16.0.0 - 172.31.255.255
    if let Some(rest) = host.strip_prefix("172.") {
        if let Ok(second) = rest.split('.').next().unwrap_or("").parse::<u32>() {
            if (16..=31).contains(&second) {
                return true;
            }
        }
    }
    // 不带点的主机名（http://nas:8666 这类）也当内网
    !host.contains('.') && !host.contains(':')
}

/// 找系统自带的 curl.exe（Win10 1803+）。
/// 32 位进程访问 System32 会被重定向到 SysWOW64（同样带 curl），Sysnative 兜底。
fn find_curl() -> Option<PathBuf> {
    for path in [
        PathBuf::from(r"C:\Windows\System32\curl.exe"),
        PathBuf::from(r"C:\Windows\Sysnative\curl.exe"),
    ] {
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// curl 报错里若出现 schannel / (35)，多半是「服务端地址误写成 https、但服务器只讲
/// HTTP」导致 TLS 去握一个明文端口。补一句人话提示，省得用户再排查一轮。
fn enrich_tls_error(stderr: &str) -> String {
    let base = stderr.trim().to_owned();
    if stderr.contains("schannel") || stderr.contains("(35)") {
        format!(
            "{base}\n（TLS 握手失败：更新服务器是普通 HTTP，请确认「服务端地址」是否误写成 https://，应为 http://...）"
        )
    } else {
        base
    }
}

/// 用系统 curl（没有就退回 PowerShell）把 url 下载到 dest。
/// 内网地址绕过系统代理（http_proxy 环境变量会让内网请求连不出去）。
fn fetch(url: &str, dest: &Path) -> Result<(), String> {
    let noproxy = is_private_host(url);
    if let Some(curl) = find_curl() {
        let mut cmd = std::process::Command::new(&curl);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.args(["-fsSL", "--connect-timeout", "10", "--max-time", "120"]);
        if noproxy {
            cmd.arg("--noproxy").arg("*");
        }
        let output = cmd
            .arg("-o")
            .arg(dest)
            .arg(url)
            .output()
            .map_err(|e| format!("无法启动 {}：{e}", curl.display()))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("下载失败（curl 退出码 {}）", output.status)
        } else {
            format!("下载失败：{}", enrich_tls_error(stderr))
        });
    }
    // Invoke-WebRequest 5.1 没有 -NoProxy，清掉默认代理对象即可
    let clear_proxy = if noproxy {
        "[System.Net.WebRequest]::DefaultWebProxy=$null;"
    } else {
        ""
    };
    let mut ps = std::process::Command::new("powershell");
    #[cfg(windows)]
    ps.creation_flags(CREATE_NO_WINDOW);
    let output = ps
        .args(["-NoProfile", "-Command"])
        .arg(format!(
            "$ProgressPreference='SilentlyContinue';{clear_proxy}\
             [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12;\
             Invoke-WebRequest -UseBasicParsing -Uri '{url}' -OutFile '{}'",
            dest.display()
        ))
        .output()
        .map_err(|e| format!("无法启动 powershell：{e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        "下载失败（PowerShell）".to_owned()
    } else {
        format!("下载失败：{stderr}")
    })
}

/// 从服务端拉 latest.json 并解析出最新版本信息。
/// 返回的 manifest.url 已拼成完整下载地址。
pub fn check(server: &str) -> Result<UpdateManifest, String> {
    if server.trim().is_empty() {
        return Err("未配置更新服务端地址".to_owned());
    }
    let url = manifest_url(server);
    let dest = std::env::temp_dir().join("svn_manager_latest.json");
    let _ = std::fs::remove_file(&dest);
    fetch(&url, &dest)?;
    let text = std::fs::read_to_string(&dest)
        .map_err(|e| format!("读取版本信息失败：{e}"))?;
    let _ = std::fs::remove_file(&dest);
    // 用记事本等编辑过的响应可能带 BOM
    let text = text.trim_start_matches('\u{feff}');
    let manifest: UpdateManifest = serde_json::from_str(text)
        .map_err(|e| format!("解析版本信息失败：{e}（{url}）"))?;
    if manifest.version.trim().is_empty() {
        return Err("服务端版本号为空".to_owned());
    }
    if manifest.url.trim().is_empty() {
        return Err("服务端未提供下载地址（url 字段）".to_owned());
    }
    let full = join_url(server, &manifest.url);
    Ok(UpdateManifest { url: full, ..manifest })
}

/// 把 url 下载到 dest（用于新版本 exe），返回下载字节数。
/// 内网地址绕过系统代理（http_proxy 环境变量会让内网请求连不出去）。
/// 下载在程序内完成而不是丢给更新脚本：一来日志区能看到进度与结果，
/// 二来 bat 不用带着「下载几十秒」的可疑特征在磁盘上久留（会被安全软件查删，
/// cmd 逐行读脚本、读到一半文件没了就报「找不到批处理文件」）。
pub fn download(url: &str, dest: &Path) -> Result<u64, String> {
    let noproxy = is_private_host(url);
    if let Some(curl) = find_curl() {
        let mut cmd = std::process::Command::new(&curl);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.args(["-fsSL", "--connect-timeout", "10", "--max-time", "600"]);
        if noproxy {
            cmd.arg("--noproxy").arg("*");
        }
        let output = cmd
            .arg("-o")
            .arg(dest)
            .arg(url)
            .args(["-w", "%{size_download}"])
            .output()
            .map_err(|e| format!("无法启动 {}：{e}", curl.display()))?;
        if output.status.success() {
            let size = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(0);
            return Ok(size);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("下载失败（curl 退出码 {}）", output.status)
        } else {
            format!("下载失败：{}", enrich_tls_error(stderr))
        });
    }
    // Invoke-WebRequest 5.1 没有 -NoProxy，清掉默认代理对象即可
    let clear_proxy = if noproxy {
        "[System.Net.WebRequest]::DefaultWebProxy=$null;"
    } else {
        ""
    };
    let mut ps = std::process::Command::new("powershell");
    #[cfg(windows)]
    ps.creation_flags(CREATE_NO_WINDOW);
    let output = ps
        .args(["-NoProfile", "-Command"])
        .arg(format!(
            "$ProgressPreference='SilentlyContinue';{clear_proxy}\
             [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12;\
             Invoke-WebRequest -UseBasicParsing -Uri '{url}' -OutFile '{}'",
            dest.display()
        ))
        .output()
        .map_err(|e| format!("无法启动 powershell：{e}"))?;
    if output.status.success() {
        return Ok(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        "下载失败（PowerShell）".to_owned()
    } else {
        format!("下载失败：{stderr}")
    })
}

/// 用系统自带的 certutil 算文件 SHA256（64 位小写 hex），用于校验下载的新 exe。
pub fn sha256_of(path: &Path) -> Result<String, String> {
    let mut ct = std::process::Command::new("certutil");
    #[cfg(windows)]
    ct.creation_flags(CREATE_NO_WINDOW);
    let output = ct
        .args(["-hashfile"])
        .arg(path)
        .arg("SHA256")
        .output()
        .map_err(|e| format!("无法启动 certutil：{e}"))?;
    if !output.status.success() {
        return Err(format!("certutil 失败（退出码 {}）", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // hash 行是 64 位 hex（旧版 certutil 里可能带空格），逐行找而不依赖行序
    for line in text.lines() {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() == 64 && compact.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(compact.to_lowercase());
        }
    }
    Err("certutil 输出中没有校验值".to_owned())
}

/// 生成应用更新的收尾 bat：杀残留实例 → 等主程序退出 → 覆盖原 exe
/// （被占用则重试，上限 15 次）→ 删临时文件 → 重启 → bat 自删。
/// 下载与校验已由程序完成，这里只做几秒钟的文件替换，输出全 ASCII。
pub fn build_apply_bat(target: &Path) -> String {
    let target = target.display().to_string();
    let exe_name = target
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("svn_manager.exe")
        .to_owned();
    let download = std::env::temp_dir()
        .join("svn_manager_update.exe")
        .display()
        .to_string();
    format!(
        r#"@echo off
setlocal
title SVN Manager Update
set "TARGET={target}"
set "DOWNLOAD={download}"
set /a TRIES=0

echo Applying SVN Manager update ...
taskkill /f /im "{exe_name}" >nul 2>&1
{WIN_WAIT} /t 2 /nobreak >nul

:copy_retry
copy /y "%DOWNLOAD%" "%TARGET%" >nul 2>&1
if not errorlevel 1 goto copy_ok
set /a TRIES+=1
if %TRIES% GEQ 15 goto copy_fail
echo File is locked, retrying (%TRIES%/15) ...
{WIN_WAIT} /t 2 /nobreak >nul
goto copy_retry

:copy_ok
del "%DOWNLOAD%" >nul 2>&1
start "" "%TARGET%"
del "%~f0" & exit 0

:copy_fail
echo Cannot overwrite "%TARGET%" (file locked).
echo Re-run this script to retry: %~f0
pause
exit 1
"#,
        target = target,
        download = download,
        exe_name = exe_name,
        // 必须写全路径：PATH 里若混进 Git Bash / MinGW 的 usr/bin（从 Git Bash
        // 启动本程序就会），裸写 timeout 会命中 GNU coreutils 的 timeout，
        // 报 "invalid time interval '/t'" 并立刻返回——重试循环会瞬间打完 15 次，
        // 更新必然卡在「文件被占用」。
        WIN_WAIT = r"%SystemRoot%\System32\timeout.exe",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 隐藏窗口（CREATE_NO_WINDOW）后 certutil 仍然要能正常取到哈希，
    /// 否则更新校验会静默失败——这条是真机回归。
    #[test]
    #[cfg(windows)]
    fn sha256_of_works_with_hidden_window() {
        let path = std::env::temp_dir().join("svn_manager_hash_test.bin");
        std::fs::write(&path, b"abc").unwrap();
        let got = sha256_of(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            got,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// 真机演练收尾 bat：用无害替身跑完整一遍「杀进程 → 覆盖 → 重启 → 删除临时
    /// 文件 → 自删」。这是更新链里最脆的一段（早先出过「未找到批处理文件」），
    /// 默认跳过，手动跑：`cargo test -- --ignored apply_bat`
    #[test]
    #[ignore]
    fn apply_bat_replaces_target_and_cleans_up() {
        let dir = std::env::temp_dir().join("svn_manager_bat_probe");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 目标程序用 rundll32 的副本：启动即退出，不会弹界面
        let target = dir.join("fake_target.exe");
        std::fs::copy(r"C:\Windows\System32\rundll32.exe", &target).unwrap();
        // bat 里新 exe 的路径是固定的 temp\svn_manager_update.exe
        let download = std::env::temp_dir().join("svn_manager_update.exe");
        std::fs::write(&download, b"new-binary-bytes").unwrap();

        let bat = dir.join("apply.bat");
        std::fs::write(&bat, build_apply_bat(&target)).unwrap();
        let out = std::process::Command::new("cmd")
            .args(["/C", &bat.to_string_lossy()])
            .output()
            .unwrap();
        // 不看退出码：bat 最后一步是 del "%~f0" 自删，cmd 读不到后续行时
        // 退出码并不总是 0（真实流程里主程序早已退出，这个值无人关心）。
        // 真正要守住的是下面三个效果和「等待命令没被 GNU timeout 抢走」。
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            !stderr.contains("invalid time interval"),
            "等待命令被 GNU coreutils 的 timeout 抢走了（要写全 System32 路径）：{stderr}"
        );
        assert!(!stderr.contains("Cannot overwrite"), "覆盖失败：{stderr}");

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"new-binary-bytes".to_vec(),
            "目标程序未被新版本覆盖"
        );
        assert!(!download.exists(), "下载的临时文件没有被清理");
        // del "%~f0" 在脚本末尾执行，进程结束后文件应已消失（等一小会儿）
        for _ in 0..20 {
            if !bat.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(!bat.exists(), "批处理文件没有自删");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实启动方式是 `start 脚本`，而 start 打开 .bat 等价于 `cmd /K 脚本`：
    /// 脚本自删后若只是返回（`exit /b`），cmd 会回到已经不存在的脚本上打印
    /// 「找不到批处理文件。」并留下一个空窗口。这条按 /K 复现，守住「结尾必须 exit」。
    /// 默认跳过，手动跑：`cargo test -- --ignored apply_bat`
    #[test]
    #[ignore]
    fn apply_bat_ends_the_host_cmd_process() {
        let dir = std::env::temp_dir().join("svn_manager_bat_k");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("fake_target.exe");
        std::fs::copy(r"C:\Windows\System32\rundll32.exe", &target).unwrap();
        std::fs::write(std::env::temp_dir().join("svn_manager_update.exe"), b"new-binary-bytes").unwrap();
        let bat = dir.join("apply.bat");
        std::fs::write(&bat, build_apply_bat(&target)).unwrap();
        // stdin 给空设备：万一脚本又只是返回，cmd 会读完输入直接退出而不是挂住等人按键
        let out = std::process::Command::new("cmd")
            .args(["/K", &bat.to_string_lossy()])
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !text.contains("找不到批处理文件") && !text.to_lowercase().contains("batch file"),
            "脚本自删后 cmd 又回去读已删除的脚本（结尾要用 exit 结束进程）：{text}"
        );
        assert!(!bat.exists(), "批处理文件没有自删");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真机冒烟：默认跳过，需要联网时手动跑，用于验证隐藏窗口没把 curl 弄坏、
    /// 以及内网地址是否真能连通（排查过代理劫持导致 curl 28 的现场）：
    ///
    /// ```text
    /// SVN_UPDATE_TEST_URL=http://192.168.1.251:20700 cargo test -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn check_real_server_smoke() {
        let Ok(server) = std::env::var("SVN_UPDATE_TEST_URL") else {
            return; // 没给地址就当跳过，不算失败
        };
        let info = check(&server).unwrap_or_else(|e| panic!("检查更新失败：{e}"));
        assert!(!info.version.trim().is_empty(), "服务端版本号为空");
        assert!(info.url.starts_with("http"), "下载地址未拼成绝对地址：{}", info.url);
    }

    #[test]
    fn version_segments_are_parsed() {
        assert_eq!(parse_version("1.1.1"), vec![1, 1, 1]);
        assert_eq!(parse_version("V1.1.1"), vec![1, 1, 1]);
        assert_eq!(parse_version("v2.0"), vec![2, 0]);
        assert_eq!(parse_version("正式版 1.2.3"), vec![1, 2, 3]);
        assert_eq!(parse_version("10.20.30"), vec![10, 20, 30]);
        assert!(parse_version("无数字").is_empty());
    }

    #[test]
    fn newer_versions_are_detected() {
        assert!(is_newer("1.1.2", "1.1.1"));
        assert!(is_newer("V1.2.0", "1.1.9"));
        assert!(is_newer("1.2", "1.1.9"), "短的按 0 补齐");
        assert!(is_newer("1.10", "1.9.9"), "数字段按数值比较，不是字符串");
        assert!(!is_newer("1.1.1", "1.1.1"), "相同版本不更新");
        assert!(!is_newer("1.1", "1.1.0"), "补 0 后相等不算新");
        assert!(!is_newer("1.0.9", "1.1.0"));
    }

    #[test]
    fn urls_are_joined() {
        assert_eq!(
            join_url("http://a.b:80/c", "files/x.exe"),
            "http://a.b:80/c/files/x.exe"
        );
        assert_eq!(
            join_url("http://a.b:80/c/", "/files/x.exe"),
            "http://a.b:80/c/files/x.exe"
        );
        assert_eq!(
            join_url("http://a.b/c", "http://c.d/x.exe"),
            "http://c.d/x.exe",
            "绝对地址原样保留"
        );
        assert_eq!(
            join_url("http://a.b/c", "https://c.d/x.exe"),
            "https://c.d/x.exe"
        );
    }

    #[test]
    fn manifest_url_prefers_full_json_path() {
        assert_eq!(manifest_url("http://a.b"), "http://a.b/latest.json");
        assert_eq!(manifest_url("http://a.b/"), "http://a.b/latest.json");
        // 直接配到 json 文件也支持
        assert_eq!(manifest_url("http://a.b/x/ver.json"), "http://a.b/x/ver.json");
    }

    #[test]
    fn check_rejects_empty_server() {
        assert!(check("  ").is_err());
    }

    #[test]
    fn private_hosts_bypass_proxy() {
        // RFC1918 私网与本机地址必须绕过代理
        assert!(is_private_host("http://192.168.1.251:20700/latest.json"));
        assert!(is_private_host("http://10.0.0.2/x.exe"));
        assert!(is_private_host("http://172.16.0.1/"));
        assert!(is_private_host("http://172.31.255.255/"));
        assert!(is_private_host("http://localhost:8666/"));
        assert!(is_private_host("http://127.0.0.1:9000/latest.json"));
        assert!(is_private_host("http://[::1]:8666/latest.json"));
        // 不带点的主机名（http://nas:8666）也当内网
        assert!(is_private_host("http://nas:8666/latest.json"));
        assert!(is_private_host("http://svr/x.exe"));
        // 公网地址照常走系统代理
        assert!(!is_private_host("https://example.com/latest.json"));
        assert!(!is_private_host("http://172.32.0.1/"), "172 段只有 16-31 是私网");
        assert!(!is_private_host("http://11.0.0.1/"));
        assert!(!is_private_host("http://192.169.0.1/"));
    }

    #[test]
    fn tls_error_gets_a_human_hint() {
        let msg = enrich_tls_error(
            "curl: (35) schannel: next InitializeSecurityContext failed: SEC_E_INVALID_TOKEN",
        );
        assert!(
            msg.contains("http://"),
            "schannel 报错应提示把地址改回 http：{msg}"
        );
        // 非 TLS 类错误（如代理超时）不应加 https 提示，避免误导
        let plain = enrich_tls_error("curl: (28) Connection timed out after 10001 milliseconds");
        assert!(
            !plain.contains("TLS 握手失败"),
            "非 TLS 错误不应加 https 提示：{plain}"
        );
    }

    #[test]
    fn apply_bat_contains_the_whole_swap_flow() {
        let bat = build_apply_bat(Path::new(r"D:\tools\svn_manager.exe"));
        // 覆盖目标、杀残留、重试、重启、自删
        assert!(bat.contains(r#"set "TARGET=D:\tools\svn_manager.exe""#));
        assert!(bat.contains(r#"taskkill /f /im "svn_manager.exe""#));
        assert!(bat.contains("goto copy_retry"));
        assert!(bat.contains(r#"start "" "%TARGET%""#));
        assert!(bat.contains(r#"del "%~f0""#));
        // 下载与校验已在程序内完成，bat 里不应再出现
        assert!(!bat.contains("curl"), "下载已在程序内完成");
        assert!(!bat.contains("certutil"), "校验已在程序内完成");
        // 覆盖目标与 echo 输出必须全 ASCII，任何代码页都不会乱码
        assert!(bat.is_ascii(), "bat 输出必须全 ASCII");
    }

    #[test]
    fn sha256_of_matches_known_digest() {
        let path = std::env::temp_dir().join("svn_manager_sha_test.txt");
        std::fs::write(&path, b"hello").expect("写测试文件");
        // sha256("hello") 的公认值
        assert_eq!(
            sha256_of(&path).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let _ = std::fs::remove_file(&path);
        assert!(sha256_of(Path::new(r"C:\surely\not\exist.bin")).is_err());
    }
}
