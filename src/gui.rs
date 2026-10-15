//! Giao diện đơn giản: chạy lại chính chương trình này ở chế độ dòng lệnh (tiến trình con),
//! hiện log và dừng bằng cách đóng stdin của tiến trình con để nó tự dọn dẹp (gỡ hosts/PAC).
//!
//! Giao diện theo design system Prism. egui không làm mờ nền được nên các lớp "kính"
//! dùng màu đục `bg-elevated` (phương án dự phòng của Prism khi không có blur).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Shadow, Stroke, Vec2};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Biến môi trường báo cho tiến trình con biết: dừng khi stdin bị đóng.
pub const ENV: &str = "STEAM_UNBLOCK_GUI";
const MAX_LOG: usize = 2000;

/// Bảng màu Prism (tokens.json), theo giao diện sáng/tối.
struct Pal {
    canvas: Color32,
    elevated: Color32,
    sunken: Color32,
    separator: Color32,
    ink: Color32,
    ink2: Color32,
    ink3: Color32,
    accent: Color32,
    accent_pressed: Color32,
    accent_tint: Color32,
    on_accent: Color32,
    success: Color32,
    warning: Color32,
    danger: Color32,
    shadow: Color32,
}

const fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

const LIGHT: Pal = Pal {
    canvas: hex(0xeef1f5),
    elevated: hex(0xffffff),
    sunken: hex(0xd9dee5),
    separator: Color32::from_rgba_premultiplied(2, 2, 3, 26), // rgba(20,22,26,.10)
    ink: hex(0x14161a),
    ink2: hex(0x4d535c),
    ink3: hex(0x5f6670),
    accent: hex(0x0066d6),
    accent_pressed: hex(0x0052ad),
    accent_tint: Color32::from_rgba_premultiplied(0, 12, 26, 31), // rgba(0,102,214,.12)
    on_accent: hex(0xffffff),
    success: hex(0x177a33),
    warning: hex(0xb25e00),
    danger: hex(0xc42b1c),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 20),
};

const DARK: Pal = Pal {
    canvas: hex(0x0c0e12),
    elevated: hex(0x17191f),
    sunken: hex(0x05060a),
    separator: Color32::from_rgba_premultiplied(26, 26, 26, 26), // rgba(255,255,255,.10)
    ink: hex(0xf2f4f7),
    ink2: hex(0xa9afb8),
    ink3: hex(0x878e98),
    accent: hex(0x4ba3ff),
    accent_pressed: hex(0x2f8cf0),
    accent_tint: Color32::from_rgba_premultiplied(14, 29, 46, 46), // rgba(75,163,255,.18)
    on_accent: hex(0x04121f),
    success: hex(0x30d158),
    warning: hex(0xffb340),
    danger: hex(0xff6961),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 90),
};

fn pal(ui: &egui::Ui) -> &'static Pal {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

// radius-*, space-* của Prism (px)
const RADIUS_SM: u8 = 10;
const RADIUS_LG: u8 = 20;
const RADIUS_PILL: u8 = 255;
const SPACE_2: f32 = 8.0;
const SPACE_3: f32 = 12.0;
const SPACE_4: f32 = 16.0;
const SPACE_5: f32 = 20.0;
const SPACE_6: f32 = 24.0;

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Browser,
    Client,
}

struct Job {
    child: Child,
    label: &'static str,
    /// Dịch vụ chạy lâu (proxy/hosts), khác với lệnh chạy một lần (check/unhosts)
    service: bool,
}

struct App {
    mode: Mode,
    job: Option<Job>,
    log: Arc<Mutex<Vec<String>>>,
    admin: bool,
}

pub fn run() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 560.0])
            .with_min_inner_size([460.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Steam Unblock",
        opts,
        Box::new(|cc| {
            load_fonts(&cc.egui_ctx);
            apply_style(&cc.egui_ctx);
            Ok(Box::new(App {
                mode: Mode::Browser,
                job: None,
                log: Default::default(),
                admin: is_admin(),
            }))
        }),
    )
}

fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Prism dùng Inter, dự phòng font hệ thống (Segoe UI trên Windows); font mặc định của egui
/// lại thiếu dấu tiếng Việt, nên lấy font của hệ điều hành.
fn load_fonts(ctx: &egui::Context) {
    const SANS: &[&str] = &[
        r"C:\Windows\Fonts\segoeui.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    ];
    const SEMIBOLD: &[&str] = &[
        r"C:\Windows\Fonts\seguisb.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
    ];
    const MONO: &[&str] = &[
        r"C:\Windows\Fonts\consola.ttf",
        "/System/Library/Fonts/Menlo.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, paths: &[&str], family: FontFamily| {
        let data = paths.iter().find_map(|p| std::fs::read(p).ok())?;
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(data)));
        fonts.families.entry(family).or_default().insert(0, name.into());
        Some(())
    };
    let sans = add("sys-sans", SANS, FontFamily::Proportional).is_some();
    add("sys-mono", MONO, FontFamily::Monospace);
    // Không có font đậm thì dùng font thường; mono thiếu dấu thì lấy dấu từ font sans.
    let mut bold = fonts.families[&FontFamily::Proportional].clone();
    if let Some(data) = SEMIBOLD.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("sys-semibold".into(), Arc::new(egui::FontData::from_owned(data)));
        bold.insert(0, "sys-semibold".into());
    }
    fonts.families.insert(semibold(), bold);
    if sans {
        fonts.families.entry(FontFamily::Monospace).or_default().push("sys-sans".into());
    }
    ctx.set_fonts(fonts);
}

/// Áp token Prism vào style của egui, cho cả giao diện sáng và tối (theo hệ điều hành).
fn apply_style(ctx: &egui::Context) {
    for (theme, p) in [(egui::Theme::Light, &LIGHT), (egui::Theme::Dark, &DARK)] {
        let mut v = if theme == egui::Theme::Dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = p.canvas;
        v.window_fill = p.elevated;
        v.extreme_bg_color = p.sunken;
        v.faint_bg_color = p.canvas;
        v.override_text_color = Some(p.ink);
        v.hyperlink_color = p.accent;
        v.warn_fg_color = p.warning;
        v.error_fg_color = p.danger;
        v.selection.bg_fill = p.accent_tint;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.window_stroke = Stroke::new(1.0, p.separator);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.corner_radius = CornerRadius::same(RADIUS_SM);
            w.bg_stroke = Stroke::NONE;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
        v.widgets.inactive.weak_bg_fill = p.accent_tint;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.accent);
        v.widgets.hovered.weak_bg_fill = p.accent_tint;
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.accent);
        v.widgets.active.weak_bg_fill = p.accent_tint;
        v.widgets.active.fg_stroke = Stroke::new(1.0, p.accent_pressed);
        v.widgets.hovered.expansion = 1.0;
        v.widgets.active.expansion = -1.0; // nhấn: co lại một chút
        ctx.set_visuals_of(theme, v);
    }
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles = [
            (Heading, FontId::new(22.0, semibold())), // title-2
            (Body, FontId::proportional(15.0)),       // body
            (Button, FontId::proportional(14.0)),     // callout
            (Small, FontId::proportional(13.0)),      // footnote
            (Monospace, FontId::monospace(13.0)),     // code
        ]
        .into();
        s.spacing.item_spacing = Vec2::new(SPACE_2, SPACE_2);
        s.spacing.button_padding = Vec2::new(SPACE_4, 7.0);
        s.spacing.interact_size.y = 34.0;
        s.animation_time = 0.18; // duration-fast
    });
}

/// Thẻ nền đục, bo `radius-lg`, đổ bóng `shadow-glass`.
fn card(p: &Pal) -> egui::Frame {
    egui::Frame::new()
        .fill(p.elevated)
        .stroke(Stroke::new(1.0, p.separator))
        .corner_radius(CornerRadius::same(RADIUS_LG))
        .inner_margin(Margin::same(SPACE_5 as i8))
        .shadow(Shadow { offset: [0, 8], blur: 24, spread: 0, color: p.shadow })
}

/// Nút chính: nền `accent`, chữ `on-accent`, dạng viên thuốc.
fn primary(p: &Pal, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(p.on_accent).family(semibold()))
        .fill(p.accent)
        .corner_radius(CornerRadius::same(RADIUS_PILL))
        .min_size(Vec2::new(120.0, 36.0))
}

/// Nút phụ: nền `accent-tint`, chữ `accent`.
fn secondary(p: &Pal, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(p.accent))
        .fill(p.accent_tint)
        .corner_radius(CornerRadius::same(RADIUS_PILL))
        .min_size(Vec2::new(0.0, 36.0))
}

/// Segmented control của Prism: rãnh `bg-sunken`, ô được chọn trượt sang bằng animation.
fn segmented(ui: &mut egui::Ui, value: &mut Mode, options: &[(Mode, &str)], enabled: bool) {
    let p = pal(ui);
    let height = 36.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let n = options.len() as f32;
    let seg_w = (rect.width() - 6.0) / n;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(RADIUS_PILL), p.sunken);

    let idx = options.iter().position(|(m, _)| m == value).unwrap_or(0) as f32;
    let x = ui.ctx().animate_value_with_time(ui.id().with("seg"), idx, 0.18);
    let thumb = egui::Rect::from_min_size(rect.min + Vec2::new(3.0 + x * seg_w, 3.0), Vec2::new(seg_w, height - 6.0));
    painter.add(Shadow { offset: [0, 1], blur: 4, spread: 0, color: p.shadow }.as_shape(thumb, CornerRadius::same(RADIUS_PILL)));
    painter.rect_filled(thumb, CornerRadius::same(RADIUS_PILL), p.elevated);

    for (i, (mode, label)) in options.iter().enumerate() {
        let r = egui::Rect::from_min_size(rect.min + Vec2::new(3.0 + i as f32 * seg_w, 3.0), Vec2::new(seg_w, height - 6.0));
        let resp = ui.interact(r, ui.id().with(("seg", i)), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
        if resp.clicked() {
            *value = *mode;
        }
        let selected = mode == value;
        let color = if !enabled && !selected { p.ink3 } else if selected { p.ink } else { p.ink2 };
        let family = if selected { semibold() } else { FontFamily::Proportional };
        painter.text(r.center(), egui::Align2::CENTER_CENTER, *label, FontId::new(14.0, family), color);
        if enabled && !selected {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
}

/// Nhãn trạng thái: chấm màu + chữ (Prism: không chỉ dùng màu để báo trạng thái).
fn status(ui: &mut egui::Ui, color: Color32, text: &str) {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(text.into(), FontId::proportional(13.0), p.ink);
    let size = Vec2::new(SPACE_3 + 8.0 + 6.0 + galley.size().x + SPACE_3, 30.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(RADIUS_PILL), p.elevated, Stroke::new(1.0, p.separator), egui::StrokeKind::Inside);
    painter.circle_filled(egui::pos2(rect.left() + SPACE_3 + 4.0, rect.center().y), 4.0, color);
    painter.galley(egui::pos2(rect.left() + SPACE_3 + 14.0, rect.center().y - galley.size().y / 2.0), galley, p.ink);
}

impl App {
    fn push(&self, line: impl Into<String>) {
        let mut log = self.log.lock().unwrap();
        log.push(line.into());
        let n = log.len();
        if n > MAX_LOG {
            log.drain(..n - MAX_LOG);
        }
    }

    fn start(&mut self, ctx: &egui::Context, args: &[&str], label: &'static str, service: bool) {
        let mut cmd = Command::new(std::env::current_exe().unwrap_or_else(|_| "steam-unblock".into()));
        cmd.args(args)
            .env(ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        no_window(&mut cmd);
        match cmd.spawn() {
            Ok(mut child) => {
                self.push(format!("── {label} ──"));
                pipe(child.stdout.take(), self.log.clone(), ctx.clone());
                pipe(child.stderr.take(), self.log.clone(), ctx.clone());
                self.job = Some(Job { child, label, service });
            }
            Err(e) => self.push(format!("Không chạy được: {e}")),
        }
    }

    /// Đóng stdin để tiến trình con tự dừng và dọn dẹp; quá 5 giây thì buộc tắt.
    fn stop(&mut self) {
        let Some(mut job) = self.job.take() else { return };
        drop(job.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = job.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = job.child.kill();
        let _ = job.child.wait();
        self.push("Đã buộc dừng (có thể cần bấm \"Gỡ hosts\").");
    }

    fn controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = pal(ui);
        let busy = self.job.is_some();
        let running = self.job.as_ref().filter(|j| j.service).map(|j| j.label);

        ui.label(RichText::new("Chế độ").size(13.0).color(p.ink2));
        segmented(ui, &mut self.mode, &[(Mode::Browser, "Trình duyệt"), (Mode::Client, "Steam client")], !busy);
        ui.add_space(SPACE_2);
        let hint = match self.mode {
            Mode::Browser => "Tự đặt PAC cho hệ thống, chỉ tên miền Steam đi qua proxy. Mở lại trình duyệt sau khi bắt đầu.",
            Mode::Client => "Thoát Steam hoàn toàn (kể cả ở khay hệ thống), bấm Bắt đầu rồi mở lại Steam.",
        };
        ui.label(RichText::new(hint).size(13.0).color(p.ink2));

        if self.mode == Mode::Client && !self.admin && cfg!(windows) && !busy {
            ui.add_space(SPACE_2);
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠ Cần quyền Administrator").size(13.0).color(p.warning));
                let btn = egui::Button::new(RichText::new("Chạy lại bằng Administrator").size(13.0).color(p.accent))
                    .fill(p.accent_tint)
                    .corner_radius(CornerRadius::same(RADIUS_PILL))
                    .min_size(Vec2::new(0.0, 28.0));
                if ui.add(btn).clicked() {
                    if relaunch_admin() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        self.push("Không mở lại được bằng quyền Administrator.");
                    }
                }
            });
        }

        ui.add_space(SPACE_4);
        ui.horizontal(|ui| {
            if running.is_some() {
                let stop = egui::Button::new(RichText::new("Dừng").color(p.ink).family(semibold()))
                    .fill(p.sunken)
                    .corner_radius(CornerRadius::same(RADIUS_PILL))
                    .min_size(Vec2::new(120.0, 36.0));
                if ui.add(stop).clicked() {
                    self.stop();
                }
            } else {
                if ui.add_enabled(!busy, primary(p, "Bắt đầu")).clicked() {
                    match self.mode {
                        Mode::Browser => self.start(ctx, &["proxy", "--system"], "Trình duyệt", true),
                        Mode::Client => self.start(ctx, &["hosts"], "Steam client", true),
                    }
                }
                if ui.add_enabled(!busy, secondary(p, "Kiểm tra")).on_hover_text("Kiểm tra Steam đang bị chặn kiểu gì").clicked() {
                    self.start(ctx, &["check"], "Kiểm tra", false);
                }
                if ui.add_enabled(!busy, secondary(p, "Gỡ hosts")).on_hover_text("Gỡ tên miền Steam khỏi file hosts nếu lần trước thoát không sạch").clicked() {
                    self.start(ctx, &["unhosts"], "Gỡ hosts", false);
                }
                if busy {
                    ui.add(egui::Spinner::new().color(p.accent));
                }
            }
        });
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Nhật ký").size(15.0).family(semibold()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clear = egui::Button::new(RichText::new("Xoá").size(13.0).color(p.accent)).frame(false);
                if ui.add(clear).clicked() {
                    self.log.lock().unwrap().clear();
                }
            });
        });
        ui.add_space(SPACE_2);
        egui::Frame::new()
            .fill(p.canvas)
            .corner_radius(CornerRadius::same(RADIUS_LG - 8)) // đồng tâm: radius-lg − khoảng lùi
            .inner_margin(Margin::same(SPACE_3 as i8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                    let log = self.log.lock().unwrap();
                    if log.is_empty() {
                        ui.label(RichText::new("Chưa có gì. Bấm Bắt đầu hoặc Kiểm tra.").size(13.0).color(p.ink3));
                    }
                    for line in log.iter() {
                        let color = if line.starts_with("──") {
                            p.ink
                        } else if line.contains("lỗi") || line.starts_with("Không") {
                            p.danger
                        } else {
                            p.ink2
                        };
                        ui.label(RichText::new(line).monospace().color(color));
                    }
                });
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        // Tiến trình con đã tự thoát?
        if let Some(job) = &mut self.job {
            if let Ok(Some(status)) = job.child.try_wait() {
                let msg = if status.success() { "Đã xong.".into() } else { format!("Đã dừng ({status}).") };
                self.job = None;
                self.push(msg);
            } else {
                ctx.request_repaint_after(Duration::from_millis(300));
            }
        }

        let frame = egui::Frame::new().fill(pal(root).canvas).inner_margin(Margin::same(SPACE_6 as i8));
        egui::CentralPanel::default().frame(frame).show(root, |ui| {
            let p = pal(ui);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Steam Unblock");
                    ui.label(RichText::new("Vào Steam khi bị nhà mạng chặn").size(13.0).color(p.ink2));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| match &self.job {
                    Some(j) if j.service => status(ui, p.success, &format!("Đang chạy · {}", j.label)),
                    Some(j) => status(ui, p.accent, &format!("{}…", j.label)),
                    None => status(ui, p.ink3, "Đã dừng"),
                });
            });
            ui.add_space(SPACE_4);
            card(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.controls(ui, ctx);
            });
            ui.add_space(SPACE_4);
            card(p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());
                self.log_view(ui);
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
    }
}

/// Đọc từng dòng từ tiến trình con vào log.
fn pipe(src: Option<impl Read + Send + 'static>, log: Arc<Mutex<Vec<String>>>, ctx: egui::Context) {
    let Some(src) = src else { return };
    std::thread::spawn(move || {
        for line in BufReader::new(src).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let mut log = log.lock().unwrap();
            log.push(line);
            let n = log.len();
            if n > MAX_LOG {
                log.drain(..n - MAX_LOG);
            }
            drop(log);
            ctx.request_repaint();
        }
    });
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_: &mut Command) {}

#[cfg(windows)]
fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(not(windows))]
fn is_admin() -> bool {
    Command::new("id").arg("-u").output().map(|o| o.stdout.starts_with(b"0")).unwrap_or(false)
}

#[cfg(windows)]
fn relaunch_admin() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.display().to_string().replace('\'', "''");
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-Command", &format!("Start-Process -FilePath '{exe}' -Verb RunAs")]);
    no_window(&mut cmd);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

#[cfg(not(windows))]
fn relaunch_admin() -> bool {
    false
}

/// Khi được mở bằng cách nhấp đúp (cửa sổ console chỉ có mỗi tiến trình này) thì ẩn console đi.
#[cfg(windows)]
pub fn hide_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 4];
    unsafe {
        if GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 {
            FreeConsole();
        }
    }
}

#[cfg(not(windows))]
pub fn hide_own_console() {}
