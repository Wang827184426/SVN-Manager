use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 用 windres 把 `icon/app.rc`（引用 `icon/svn_manager.ico`）编成 COFF 目标文件交给链接器，
/// 这样 svn_manager.exe 在桌面、资源管理器、任务栏和标题栏里显示兔子图标。
/// 只有 windows-gnu 需要这样处理（msvc 走 rc.exe）；编不出来时只提示，不中断编译。
///
/// 仓库自带的 MinGW 里 cc1.exe 依赖的 libmpc-3.dll 缺失，windres 默认的 `gcc -E` 预处理器
/// 跑不起来，所以先生成一个「原样吐出 app.rc」的小批处理当预处理器用（app.rc 只有一行，
/// 没有宏，本来就不需要真预处理）；这条路不通再退回 windres 的默认行为。
fn main() {
    println!("cargo:rerun-if-changed=icon/app.rc");
    println!("cargo:rerun-if-changed=icon/svn_manager.ico");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") || !target.contains("gnu") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let icon_dir = manifest.join("icon");
    let script = icon_dir.join("app.rc");
    if !script.is_file() || !icon_dir.join("svn_manager.ico").is_file() {
        println!("cargo:warning=缺少 icon/app.rc 或 icon/svn_manager.ico，exe 不嵌入图标");
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    let resource = out_dir.join("app_icon.o");
    let _ = fs::remove_file(&resource);
    // 批处理必须 @echo off，否则回显的那一行会被 windres 当成 .rc 内容解析
    let stub = out_dir.join("windres_preprocess.cmd");
    let _ = fs::write(&stub, format!("@echo off\r\ntype \"{}\"\r\n", script.display()));
    let mut candidates: Vec<PathBuf> = Vec::new();
    for dir in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        for name in ["windres.exe", "x86_64-w64-mingw32-windres.exe"] {
            candidates.push(dir.join(name));
        }
    }
    for name in ["windres.exe", "x86_64-w64-mingw32-windres.exe"] {
        candidates.push(manifest.join("tools").join("mingw64").join("bin").join(name));
    }
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        // .rc 里的图标路径是相对的，所以工作目录固定在 icon/，输出写到 OUT_DIR
        for fake_preprocessor in [true, false] {
            let mut command = Command::new(&candidate);
            command
                .current_dir(&icon_dir)
                .args(["-J", "rc", "-i", "app.rc", "-O", "coff"]);
            if fake_preprocessor {
                // 路径含空格时 cmd 解析不了，这种机器直接走下面的默认分支
                if no_space(&stub) && no_space(&script) {
                    command.arg(format!("--preprocessor={}", stub.display()));
                } else {
                    continue;
                }
            }
            let built = command
                .arg(&resource)
                .status()
                .map(|status| status.success() && resource.is_file())
                .unwrap_or(false);
            if built {
                println!("cargo:rustc-link-arg={}", resource.display());
                return;
            }
            let _ = fs::remove_file(&resource);
        }
    }
    println!("cargo:warning=windres 未能编译 icon/app.rc，exe 图标未嵌入（请检查 MinGW 的 bin 是否完整）");
}

/// cmd.exe 解析命令行时不认带空格的裸路径
fn no_space(path: &Path) -> bool {
    !path.display().to_string().contains(' ')
}