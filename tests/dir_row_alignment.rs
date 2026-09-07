//! 诊断：在系统真实中文字体（PingFang / 微软雅黑…）+ egui 默认 emoji 兜底字体的条件下，
//! 量 dir_row 右侧按钮组各按钮的高度与垂直中心，找出「移除 / 更多」与「📂 打开目录」
//! 不在同一水平线上的真正来源（纯结构问题在默认字体下已验证为 0px 落差）。

use egui::{
    Align, Button, Context, DragValue, FontData, FontDefinitions, FontFamily, Frame, Id, Layout,
    Pos2, RawInput, Rect, RichText, ScrollArea, Sense, TextEdit, Ui, UiBuilder, Vec2,
};
use std::path::PathBuf;
use std::sync::Arc;

fn system_cjk_font() -> Option<Vec<u8>> {
    let dirs = vec![
        std::env::var_os("WINDIR").map(PathBuf::from),
        std::env::var_os("SystemRoot").map(PathBuf::from),
        Some(PathBuf::from("C:\\Windows\\Fonts")),
    ];
    let names = [
        "PingFang SC.ttf",
        "msyh.ttc",
        "msyh.ttf",
        "simhei.ttf",
        "Deng.ttf",
        "simsun.ttc",
    ];
    for dir in dirs.into_iter().flatten() {
        let dir = dir.join("Fonts");
        for name in names {
            let Ok(bytes) = std::fs::read(dir.join(name)) else {
                continue;
            };
            if bytes.len() > 4096 {
                return Some(bytes);
            }
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 旧写法：RTL(Center) 里并排放「移除」「更多」两个按钮 + 一个 horizontal（四个功能按钮）
    Nested,
    /// dir_row 现在的写法：六个按钮全部放进同一个 horizontal（一个子 Ui 里对齐）
    SingleRow,
}

/// 尽量贴近真实 dir_row：ScrollArea + row scope + Frame + horizontal，右侧再挂按钮组。
fn row_pass(ctx: &Context, mode: Mode) -> Vec<(&'static str, Rect)> {
    let mut rects = Vec::new();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
            focused: true,
            ..Default::default()
        },
        |ui: &mut Ui| {
            ScrollArea::vertical()
                .id_salt("dir_list")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.scope_builder(
                        UiBuilder::new().id(Id::new(("dir_row", 0))).sense(Sense::click()),
                        |ui| {
                            Frame::new().inner_margin(7.0).show(ui, |ui| {
                                ui.style_mut().interaction.selectable_labels = false;
                                ui.horizontal(|ui| {
                                    ui.label("●");
                                    ui.label("别名");
                                    ui.label("path");
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        // 视觉顺序统一为：打开目录 历史 上传 更新 更多 移除
                                        match mode {
                                            Mode::Nested => {
                                                ui.add_enabled_ui(true, |ui| {
                                                    rects.push(("移除", ui.button("移除").rect));
                                                });
                                                let more = ui.menu_button("更多", |_ui| {});
                                                rects.push(("更多", more.response.rect));
                                                ui.horizontal(|ui| {
                                                    for name in [
                                                        "📂 打开目录",
                                                        "🕘 历史",
                                                        "⬆ 上传",
                                                        "⬇ 更新",
                                                    ] {
                                                        rects.push((name, ui.button(name).rect));
                                                    }
                                                });
                                            }
                                            Mode::SingleRow => {
                                                ui.horizontal(|ui| {
                                                    for name in [
                                                        "📂 打开目录",
                                                        "🕘 历史",
                                                        "⬆ 上传",
                                                        "⬇ 更新",
                                                    ] {
                                                        rects.push((name, ui.button(name).rect));
                                                    }
                                                    let more = ui.menu_button("更多", |_ui| {});
                                                    rects.push(("更多", more.response.rect));
                                                    rects.push((
                                                        "移除",
                                                        ui.add_enabled(true, Button::new("移除"))
                                                            .rect,
                                                    ));
                                                });
                                            }
                                        }
                                    });
                                });
                            });
                        },
                    );
                });
        },
    );
    output.textures_delta.clear();
    rects
}

fn report(tag: &str, rects: &[(&'static str, Rect)]) -> (f32, f32) {
    println!("=== {tag} ===");
    let mut min_c = f32::MAX;
    let mut max_c = f32::MIN;
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;
    for (name, r) in rects {
        println!(
            "  {name:12} y:[{:7.2} .. {:7.2}] 高={:5.2} 中心={:7.2}",
            r.min.y,
            r.max.y,
            r.height(),
            r.center().y
        );
        min_c = min_c.min(r.center().y);
        max_c = max_c.max(r.center().y);
        min_h = min_h.min(r.height());
        max_h = max_h.max(r.height());
    }
    println!("  中心落差 = {:.2}px，高度 {:.2}..{:.2}", max_c - min_c, min_h, max_h);
    (max_c - min_c, max_h - min_h)
}

/// 把系统里第一款可用的中文字体装进 egui，复现实机上的字体兜底；
/// 没找到可用字体时返回 false（测试直接跳过）。
fn install_real_font(ctx: &Context) -> bool {
    let Some(bytes) = system_cjk_font() else {
        return false;
    };
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "cjk".to_owned(),
        Arc::new(FontData::from_owned(bytes).tweak(egui::FontTweak {
            scale: 1.0,
            y_offset_factor: 0.02,
            ..Default::default()
        })),
    );
    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, "cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    true
}

#[test]
fn action_buttons_share_one_center_line() {
    let ctx = Context::default();
    if !install_real_font(&ctx) {
        println!("跳过：未找到系统中文字体，无法复现实机字体兜底");
        return;
    }

    // 预热一帧，避免首帧字体 atlas 尚未生效带来的噪声
    row_pass(&ctx, Mode::SingleRow);

    // 反例：把「移除 / 更多」与四个功能按钮分成两组并列，会错开约半个像素
    let nested = row_pass(&ctx, Mode::Nested);
    let (nested_spread, _) = report("对照：分组并列（旧写法）", &nested);

    // 正例：与 dir_row 一致，六个按钮同在一个 horizontal 里
    let single = row_pass(&ctx, Mode::SingleRow);
    let (spread, height_gap) = report("dir_row 实际写法：同一个 horizontal", &single);

    assert!(
        nested_spread > 0.0,
        "分组并列本该出现落差，实测 {nested_spread:.2}px；若已归零可删掉这个对照"
    );
    assert!(
        spread <= 0.01,
        "「移除 / 更多」与「📂 打开目录」没在同一水平线上，落差 {spread:.2}px：{single:?}"
    );
    assert!(height_gap <= 0.01, "按钮高度不一致：{height_gap:.2}px");
}

/// 复刻提交页 / 历史页顶部的工具条：左边一个「← 返回目录」按钮，
/// 右边是 right_to_left(Center) 子 Ui（里面放转圈 + 操作按钮，历史页还多几个筛选控件）。
/// 这两处的「返回目录」带箭头字符，同样可能走字体兜底，一并纳入回归。
fn header_pass(ctx: &Context, kind: Header) -> Vec<(&'static str, Rect)> {
    let mut rects = Vec::new();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 300.0))),
            focused: true,
            ..Default::default()
        },
        |ui: &mut Ui| {
            ui.horizontal(|ui| {
                rects.push(("← 返回目录", ui.button("← 返回目录").rect));
                ui.label(RichText::new("提交记录：某个目录").size(17.0).strong());
                ui.label(RichText::new("D:\\work\\trunk").size(12.0).weak());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spinner();
                    rects.push((
                        if kind == Header::Commit {
                            "重新读取修改"
                        } else {
                            "刷新"
                        },
                        ui.button(if kind == Header::Commit {
                            "重新读取修改"
                        } else {
                            "刷新"
                        })
                        .rect,
                    ));
                    if kind == Header::History {
                        ui.label(RichText::new("条数").weak().size(12.0));
                        let mut limit = 100;
                        ui.add(DragValue::new(&mut limit).range(1..=2000).speed(5));
                        let mut filter = String::new();
                        ui.add_sized(
                            Vec2::new(200.0, 22.0),
                            TextEdit::singleline(&mut filter).hint_text("过滤"),
                        );
                    }
                });
            });
        },
    );
    output.textures_delta.clear();
    rects
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Header {
    Commit,
    History,
}

/// 提交页 / 历史页顶部那一行：按钮与右侧操作区各在一个 Ui 里，
/// 只要「← 返回目录」的高度和右侧子 Ui 一致，中心线就不会错开。
#[test]
fn window_header_buttons_stay_aligned() {
    let ctx = Context::default();
    if !install_real_font(&ctx) {
        println!("跳过：未找到系统中文字体");
        return;
    }
    header_pass(&ctx, Header::Commit);

    for (tag, kind) in [("提交页顶部行", Header::Commit), ("历史页顶部行", Header::History)] {
        let rects = header_pass(&ctx, kind);
        // 已知落差 1.5px，成因与目录行不同：egui 的水平布局是「边放边定行高」——
        // 放第一个元素时行高还是 0，于是「← 返回目录」贴顶；后面的 17pt 标题把行撑到 22px，
        // 右侧操作区再放时才居中于 22，就差了半个行高差。
        // 不修：两端隔着整屏宽，1.5px 看不出来；要修得把最高的元素挪到行首
        // （会打乱「返回键在最左」的视觉顺序）或硬编码行高，都不划算。
        // 这里只当护栏：不许再恶化。
        let (spread, _) = report(tag, &rects);
        assert!(
            spread <= 2.0,
            "{tag} 的落差已恶化到 {spread:.2}px（原本 1.5px，属已知可接受范围）：{rects:?}"
        );
    }
}
/// 输出面板那一行：左边是「输出记录（N 行）」标签 + 「清空」，右边 RTL 里是
/// 「自动滚动」勾选框 + 「打开配置目录」。两个按钮分处两个 Ui，且都不是行内第一个元素，
/// 同样可能因为行高边走边长而互相错开。
#[test]
fn output_panel_buttons_stay_aligned() {
    let ctx = Context::default();
    if !install_real_font(&ctx) {
        println!("跳过：未找到系统中文字体");
        return;
    }
    let mut rects = Vec::new();
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 300.0))),
            focused: true,
            ..Default::default()
        },
        |ui: &mut Ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("输出记录（12 行）").strong().size(13.0));
                rects.push(("清空", ui.button("清空").rect));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut auto_scroll = true;
                    ui.checkbox(&mut auto_scroll, "自动滚动");
                    rects.push(("打开配置目录", ui.button("打开配置目录").rect));
                });
            });
        },
    );
    output.textures_delta.clear();

    let (spread, _) = report("输出面板行", &rects);
    assert!(
        spread <= 0.01,
        "输出面板两个按钮没在同一水平线上，落差 {spread:.2}px：{rects:?}"
    );
}

/// 这里把各处工具栏用到的按钮文案都量一遍：带图标的（图标在中文字体里没字形时会兜底到
/// emoji 字体、把行高撑大）必须和纯中文按钮一样高，否则别处还会重现同类错位。
#[test]
fn toolbar_buttons_measure_the_same_height() {
    let ctx = Context::default();
    if !install_real_font(&ctx) {
        println!("跳过：未找到系统中文字体");
        return;
    }
    let texts = [
        "移除",
        "更多",
        "设置",
        "刷新",
        "全部上传",
        "重新读取修改",
        "📂 打开目录",
        "🕘 历史",
        "⬆ 上传",
        "⬇ 更新",
        "← 返回目录",
        "✔ 执行 relocate",
    ];
    let mut heights = Vec::new();
    let mut output = ctx.run_ui(
        RawInput {
            // 给足宽度，避免按钮排到换行影响测量
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(6000.0, 400.0))),
            focused: true,
            ..Default::default()
        },
        |ui: &mut Ui| {
            ui.horizontal(|ui| {
                for text in texts {
                    heights.push((text, ui.button(text).rect));
                }
            });
        },
    );
    output.textures_delta.clear();

    report("各处工具栏按钮（同一行内量高度）", &heights);
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;
    for (_, r) in &heights {
        min_h = min_h.min(r.height());
        max_h = max_h.max(r.height());
    }
    assert!(
        max_h - min_h <= 0.01,
        "工具栏按钮高度不一致（{min_h:.2}..{max_h:.2}px），并排时会错开：{heights:?}"
    );
}
