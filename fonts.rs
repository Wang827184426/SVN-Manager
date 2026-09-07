use std::path::PathBuf;
use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// 首选字体：各台机器上 PingFang SC 的文件名写法不一（可能带空格、大小写不同），按关键字匹配。
const PREFERRED: &str = "pingfang";

/// 首选字体里算粗体的字重（按文件名关键字判断）。
const BOLD_WORDS: &[&str] = &["bold", "semibold", "medium", "black"];

/// 首选字体没有粗体字面时，兜底使用的中文粗体字体文件名。
const BOLD_FONTS: &[&str] = &[
    "PingFangSC-Semibold.ttf",
    "PingFang SC Semibold.ttf",
    "msyhbd.ttc",
    "Dengb.ttf",
    "simhei.ttf",
];

/// 找不到首选字体时按文件名兜底的中文字体。
const CJK_FONTS: &[&str] = &[
    "PingFang SC.ttf",
    "PingFangSC-Regular.ttf",
    "PingFang-SC-Regular.ttf",
    "msyh.ttc",
    "msyh.ttf",
    "Deng.ttf",
    "simhei.ttf",
    "Microsoft YaHei.ttf",
    "simkai.ttf",
    "STXIHEI.TTF",
    "simsun.ttc",
    "NotoSansSC-Regular.otf",
    "NotoSansCJKsc-Regular.otf",
];

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for name in ["WINDIR", "SystemRoot"] {
        if let Some(root) = std::env::var_os(name) {
            dirs.push(PathBuf::from(root).join("Fonts"));
        }
    }
    dirs.push(PathBuf::from("C:\\Windows\\Fonts"));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
    }
    dirs
}

fn weight_rank(name: &str) -> u8 {
    if name.contains("regular") {
        0
    } else if ["thin", "light", "medium", "bold", "semibold", "black"]
        .iter()
        .any(|word| name.contains(word))
    {
        2
    } else {
        1
    }
}

/// 载入系统中文字体（首选 PingFang SC）并注册给 egui，返回真正用到的字体文件名。
///
/// 这里除了正文与等宽字族，还必须给 `FontFamily::Name("bold")` 建立绑定：
/// epaint 默认只有 Proportional / Monospace 两个字族，排版一个没绑定过的字族会直接 panic
/// （`FontFamily::Name("bold") is not bound to any fonts`），
/// 而 panic 发生在排版过程中，画面会停在最后一帧，看起来就是「按钮能点却没反应」。
pub fn install_cjk(ctx: &Context) -> Option<String> {
    let dirs = font_dirs();
    let mut preferred: Vec<PathBuf> = Vec::new();
    let mut preferred_bold: Vec<PathBuf> = Vec::new();
    for dir in &dirs {
        let Ok(read) = std::fs::read_dir(dir) else {
            continue;
        };
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().to_lowercase();
            let is_font = [".ttf", ".otf", ".ttc"].iter().any(|ext| name.ends_with(ext));
            if !is_font || !name.contains(PREFERRED) {
                continue;
            }
            if BOLD_WORDS.iter().any(|word| name.contains(word)) {
                preferred_bold.push(item.path());
            } else {
                preferred.push(item.path());
            }
        }
    }
    // 同一目录里可能有多款字重，优先 Regular
    preferred.sort_by_key(|path| {
        weight_rank(
            &path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        )
    });
    let mut candidates = preferred;
    for dir in &dirs {
        for file in CJK_FONTS {
            candidates.push(dir.join(file));
        }
    }
    let mut fonts = FontDefinitions::default();
    let mut loaded: Option<String> = None;
    for path in candidates {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.len() < 4096 {
            continue;
        }
        let data = FontData::from_owned(bytes).tweak(egui::FontTweak {
            scale: 1.0,
            y_offset_factor: 0.02,
            ..Default::default()
        });
        fonts.font_data.insert("cjk".to_owned(), Arc::new(data));
        loaded = Some(path.to_string_lossy().into_owned());
        break;
    }
    if loaded.is_some() {
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "cjk".to_owned());
        }
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.push("cjk".to_owned());
        }
    }
    // 粗体字面：先取首选字体的粗体，再退回系统里现成的中文粗体
    let mut bold_candidates = preferred_bold;
    for dir in &dirs {
        for file in BOLD_FONTS {
            bold_candidates.push(dir.join(file));
        }
    }
    let mut bold: Vec<String> = Vec::new();
    for path in bold_candidates {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.len() < 4096 {
            continue;
        }
        let data = FontData::from_owned(bytes).tweak(egui::FontTweak {
            scale: 1.0,
            y_offset_factor: 0.02,
            ..Default::default()
        });
        fonts.font_data.insert("cjk-bold".to_owned(), Arc::new(data));
        bold.push("cjk-bold".to_owned());
        break;
    }
    // 兜底：任何情况下 Name("bold") 都至少指向正文用到的字体
    if let Some(list) = fonts.families.get(&FontFamily::Proportional) {
        bold.extend(list.iter().cloned());
    }
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{FontId, RawInput, RichText};

    /// epaint 遇到没绑定过的字族会在排版时直接 panic（界面就此停在最后一帧，
    /// 表现就是「按钮看着能点，点了没反应」），所以 Name("bold") 必须始终可用。
    #[test]
    fn bold_family_always_lays_out() {
        let ctx = Context::default();
        install_cjk(&ctx);
        let mut width = 0.0;
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            let bold = FontId {
                size: 12.0,
                family: FontFamily::Name("bold".into()),
            };
            width = ui.label(RichText::new("药品目录 HRP.DRUG").font(bold)).rect.width();
        });
        // 测试不渲染纹理，显式丢弃字体增量，否则 epaint 在 Drop 时按未应用增量 panic
        output.textures_delta.clear();
        assert!(width > 0.0, "粗体字族没能排出文字");
    }

    /// 正文与等宽字族同样不能因为换字体而被删掉。
    #[test]
    fn body_and_mono_stay_bound() {
        let ctx = Context::default();
        install_cjk(&ctx);
        let mut widths = [0.0; 2];
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            widths[0] = ui.label("正文").rect.width();
            widths[1] = ui.monospace("mono").rect.width();
        });
        output.textures_delta.clear();
        assert!(widths[0] > 0.0 && widths[1] > 0.0, "{widths:?}");
    }
}