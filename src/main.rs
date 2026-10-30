#![windows_subsystem = "windows"]

use eframe::egui::{
    self, pos2, Align, Color32, FontId, Key, Layout, Margin, Rect, RichText, Rounding, ScrollArea,
    Stroke, UiBuilder, Vec2, Visuals,
};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Gumroad Neo-Brutalist Design Tokens (matching neoshaurya.com)
pub const ACCENT_PINK: Color32 = Color32::from_rgb(255, 144, 232); // #ff90e8
pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(255, 201, 0); // #ffc900
pub const ACCENT_TEAL: Color32 = Color32::from_rgb(35, 160, 148); // #23a094
pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 92, 0); // #ff5c00
pub const PITCH_BLACK: Color32 = Color32::from_rgb(0, 0, 0); // #000000
pub const CRISP_WHITE: Color32 = Color32::from_rgb(255, 255, 255); // #ffffff

#[cfg(target_os = "windows")]
#[repr(C)]
struct MEMORYSTATUSEX {
    dw_length: u32,
    dw_memory_load: u32,
    ull_total_phys: u64,
    ull_avail_phys: u64,
    ull_total_page_file: u64,
    ull_avail_page_file: u64,
    ull_total_virtual: u64,
    ull_avail_virtual: u64,
    ull_avail_extended_virtual: u64,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn SetErrorMode(uMode: u32) -> u32;
    fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> i32;
}

fn get_system_memory_info() -> Option<(u32, f32, f32)> {
    #[cfg(target_os = "windows")]
    {
        let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        mem.dw_length = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 {
            let used_gb =
                (mem.ull_total_phys - mem.ull_avail_phys) as f32 / (1024.0 * 1024.0 * 1024.0);
            let total_gb = mem.ull_total_phys as f32 / (1024.0 * 1024.0 * 1024.0);
            return Some((mem.dw_memory_load, used_gb, total_gb));
        }
    }
    None
}

fn get_git_branch_info() -> Option<String> {
    if let Ok(mut current) = std::env::current_dir() {
        loop {
            let head_path = current.join(".git").join("HEAD");
            if head_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&head_path) {
                    let trimmed = content.trim();
                    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                        return Some(branch.to_string());
                    } else if trimmed.len() >= 7 {
                        return Some(trimmed[..7].to_string());
                    }
                }
                return Some("HEAD".to_string());
            }
            if !current.pop() {
                break;
            }
        }
    }
    None
}

fn get_current_dir_name() -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(name) = cwd.file_name() {
            return name.to_string_lossy().to_string();
        }
    }
    "~".to_string()
}

/// Fuzzy match score between a search pattern and a candidate string
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(0);
    }
    let p_lower = pattern.to_lowercase();
    let c_lower = candidate.to_lowercase();

    // Exact match
    if p_lower == c_lower {
        return Some(10000);
    }

    // Exact substring match
    if let Some(pos) = c_lower.find(&p_lower) {
        let prefix_bonus = if pos == 0 { 2000 } else { 1000 };
        let score = prefix_bonus - (pos as i32) * 20 - (candidate.len() as i32);
        return Some(score);
    }

    // Subsequence match: all chars of pattern appear in order
    let mut p_chars = p_lower.chars().peekable();
    let mut score = 100;
    let mut consecutive = 0;
    let mut prev_matched = false;

    for ch in c_lower.chars() {
        if let Some(&p) = p_chars.peek() {
            if ch == p {
                p_chars.next();
                score += 15 + (if prev_matched { consecutive * 10 } else { 0 });
                consecutive += 1;
                prev_matched = true;
            } else {
                prev_matched = false;
                consecutive = 0;
            }
        }
    }

    if p_chars.peek().is_none() {
        Some(score - (candidate.len() as i32))
    } else {
        None
    }
}

/// Load persistent command history from PSReadLine history file if present
fn load_persistent_history() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let path = std::path::PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("PowerShell")
                .join("PSReadLine")
                .join("ConsoleHost_history.txt");
            if let Ok(content) = std::fs::read_to_string(path) {
                let mut history = Vec::new();
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        if history.last().map(|s: &String| s.as_str()) != Some(trimmed) {
                            history.push(trimmed.to_string());
                        }
                    }
                }
                return history;
            }
        }
    }
    Vec::new()
}

/// Append newly executed command to PSReadLine persistent history
fn append_persistent_history(cmd: &str) {
    #[cfg(target_os = "windows")]
    {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = std::path::PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("PowerShell")
                .join("PSReadLine");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("ConsoleHost_history.txt");
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{}", trimmed);
            }
        }
    }
}

/// Renders a tactile Neo-Brutalist button matching neoshaurya.com:
/// - 4px solid border
/// - Solid hard drop shadow (0 blur)
/// - Hover interaction: lifts by (-1px, -1px) with shadow expanding to (shadow_size + 1)
/// - Active (click-down) interaction: depresses by (shadow_size - 1) into the shadow
fn render_brutal_button(
    ui: &mut egui::Ui,
    text: &str,
    bg: Color32,
    fg: Color32,
    border_color: Color32,
    shadow_color: Color32,
    shadow_size: f32,
    font_size: f32,
) -> egui::Response {
    let font_id = FontId::monospace(font_size);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, fg);
    let pad_x = 10.0_f32;
    let pad_y = 6.0_f32;
    let content_size = Vec2::new(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
    let total_size = content_size + Vec2::new(shadow_size, shadow_size);

    let (rect, response) = ui.allocate_exact_size(total_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let is_hovered = response.hovered();
        let is_down = response.is_pointer_button_down_on();

        let (fg_offset, current_shadow_offset) = if is_down {
            (
                Vec2::new(shadow_size - 1.0, shadow_size - 1.0),
                Vec2::new(shadow_size, shadow_size),
            )
        } else if is_hovered {
            (
                Vec2::new(-1.0, -1.0),
                Vec2::new(shadow_size + 1.0, shadow_size + 1.0),
            )
        } else {
            (
                Vec2::ZERO,
                Vec2::new(shadow_size, shadow_size),
            )
        };

        let base_min = rect.min;
        let shadow_rect = Rect::from_min_size(base_min + current_shadow_offset, content_size);
        let fg_rect = Rect::from_min_size(base_min + fg_offset, content_size);

        // 1. Paint hard drop shadow
        ui.painter().rect_filled(shadow_rect, Rounding::ZERO, shadow_color);
        // 2. Paint foreground button card
        ui.painter().rect_filled(fg_rect, Rounding::ZERO, bg);
        // 3. Paint 4px brutalist stroke
        ui.painter().rect_stroke(fg_rect, Rounding::ZERO, Stroke::new(4.0_f32, border_color));
        // 4. Paint monospace text
        let text_pos = fg_rect.min + Vec2::new(pad_x, pad_y);
        ui.painter().galley(text_pos, galley, fg);
    }

    response
}

/// Renders a solid Neo-Brutalist badge with a 4px border and hard offset drop shadow
fn render_brutal_badge(
    ui: &mut egui::Ui,
    text: &str,
    bg: Color32,
    fg: Color32,
    border_color: Color32,
    shadow_color: Color32,
    shadow_size: f32,
    font_size: f32,
) {
    let font_id = FontId::monospace(font_size);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, fg);
    let pad_x = 8.0_f32;
    let pad_y = 4.0_f32;
    let content_size = Vec2::new(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
    let total_size = content_size + Vec2::new(shadow_size, shadow_size);

    let (rect, _) = ui.allocate_exact_size(total_size, egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let base_min = rect.min;
        let shadow_rect = Rect::from_min_size(base_min + Vec2::new(shadow_size, shadow_size), content_size);
        let fg_rect = Rect::from_min_size(base_min, content_size);

        ui.painter().rect_filled(shadow_rect, Rounding::ZERO, shadow_color);
        ui.painter().rect_filled(fg_rect, Rounding::ZERO, bg);
        ui.painter().rect_stroke(fg_rect, Rounding::ZERO, Stroke::new(4.0_f32, border_color));
        let text_pos = fg_rect.min + Vec2::new(pad_x, pad_y);
        ui.painter().galley(text_pos, galley, fg);
    }
}

/// Renders a Neo-Brutalist tab tile with active accent highlighting, unread indicator,
/// close button, hover lift, and active depression.
/// Returns (clicked_select_tab, clicked_close_tab).
fn render_brutal_tab(
    ui: &mut egui::Ui,
    idx: usize,
    title: &str,
    is_active: bool,
    has_unread: bool,
    can_close: bool,
    dark_mode: bool,
    border_color: Color32,
    shadow_color: Color32,
) -> (bool, bool) {
    let font_id = FontId::monospace(12.5);
    let title_text = format!(" {:02}: {} ", idx + 1, title);

    let (tab_bg, tab_fg, shadow_c) = if is_active {
        (
            ACCENT_YELLOW,
            PITCH_BLACK,
            if dark_mode { ACCENT_TEAL } else { PITCH_BLACK },
        )
    } else if dark_mode {
        (PITCH_BLACK, CRISP_WHITE, shadow_color)
    } else {
        (CRISP_WHITE, PITCH_BLACK, shadow_color)
    };

    let title_galley = ui.painter().layout_no_wrap(title_text, font_id.clone(), tab_fg);

    let unread_galley = if has_unread && !is_active {
        Some(ui.painter().layout_no_wrap(" ● NEW".to_string(), font_id.clone(), ACCENT_PINK))
    } else {
        None
    };

    let close_galley = if can_close {
        Some(ui.painter().layout_no_wrap(" × ".to_string(), FontId::monospace(14.0), tab_fg))
    } else {
        None
    };

    let pad_x = 8.0_f32;
    let pad_y = 5.0_f32;
    let mut inner_w = title_galley.size().x;
    if let Some(ref ug) = unread_galley {
        inner_w += ug.size().x;
    }
    if let Some(ref cg) = close_galley {
        inner_w += cg.size().x + 2.0;
    }
    let inner_h = title_galley.size().y.max(close_galley.as_ref().map_or(0.0, |g| g.size().y));
    let content_size = Vec2::new(inner_w + pad_x * 2.0, inner_h + pad_y * 2.0);
    let shadow_size = if is_active { 4.0_f32 } else { 3.0_f32 };
    let total_size = content_size + Vec2::new(shadow_size, shadow_size);

    let (rect, response) = ui.allocate_exact_size(total_size, egui::Sense::click());
    let mut clicked_tab = false;
    let mut clicked_close = false;

    if ui.is_rect_visible(rect) {
        let is_hovered = response.hovered();
        let is_down = response.is_pointer_button_down_on();

        let (fg_offset, current_shadow_offset) = if is_down {
            (
                Vec2::new(shadow_size - 1.0, shadow_size - 1.0),
                Vec2::new(shadow_size, shadow_size),
            )
        } else if is_hovered {
            (
                Vec2::new(-1.0, -1.0),
                Vec2::new(shadow_size + 1.0, shadow_size + 1.0),
            )
        } else {
            (
                Vec2::ZERO,
                Vec2::new(shadow_size, shadow_size),
            )
        };

        let base_min = rect.min;
        let shadow_rect = Rect::from_min_size(base_min + current_shadow_offset, content_size);
        let fg_rect = Rect::from_min_size(base_min + fg_offset, content_size);

        ui.painter().rect_filled(shadow_rect, Rounding::ZERO, shadow_c);
        ui.painter().rect_filled(fg_rect, Rounding::ZERO, tab_bg);
        ui.painter().rect_stroke(fg_rect, Rounding::ZERO, Stroke::new(4.0_f32, border_color));

        let mut curr_x = fg_rect.min.x + pad_x;
        let text_y = fg_rect.min.y + pad_y;
        ui.painter().galley(pos2(curr_x, text_y), title_galley.clone(), tab_fg);
        curr_x += title_galley.size().x;

        if let Some(ug) = unread_galley {
            ui.painter().galley(pos2(curr_x, text_y), ug.clone(), ACCENT_PINK);
            curr_x += ug.size().x;
        }

        if let Some(cg) = close_galley {
            curr_x += 2.0;
            let close_rect = Rect::from_min_size(pos2(curr_x, text_y), cg.size());
            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            let close_hovered = pointer_pos.map_or(false, |p| close_rect.contains(p));

            let close_fg = if close_hovered {
                ACCENT_ORANGE
            } else {
                tab_fg
            };
            ui.painter().galley(pos2(curr_x, text_y), cg, close_fg);

            if response.clicked() {
                if close_hovered {
                    clicked_close = true;
                } else {
                    clicked_tab = true;
                }
            }
        } else if response.clicked() {
            clicked_tab = true;
        }
    }

    (clicked_tab, clicked_close)
}

/// Renders a full-width Neo-Brutalist card container with a 4px border and solid drop shadow
fn render_brutal_card<R>(
    ui: &mut egui::Ui,
    height: f32,
    bg: Color32,
    border_color: Color32,
    shadow_color: Color32,
    shadow_size: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let available_w = ui.available_width();
    let content_w = (available_w - shadow_size).max(10.0);
    let total_size = Vec2::new(available_w, height + shadow_size);

    let (rect, _) = ui.allocate_exact_size(total_size, egui::Sense::hover());

    let base_min = rect.min;
    let fg_rect = Rect::from_min_size(base_min, Vec2::new(content_w, height));
    let shadow_rect = Rect::from_min_size(base_min + Vec2::new(shadow_size, shadow_size), Vec2::new(content_w, height));

    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(shadow_rect, Rounding::ZERO, shadow_color);
        ui.painter().rect_filled(fg_rect, Rounding::ZERO, bg);
        ui.painter().rect_stroke(fg_rect, Rounding::ZERO, Stroke::new(4.0_f32, border_color));
    }

    let inner_rect = fg_rect.shrink2(Vec2::new(8.0, 6.0));
    ui.allocate_new_ui(UiBuilder::new().max_rect(inner_rect), |child_ui| {
        content(child_ui)
    }).inner
}

/// Renders a 4px thick brutalist horizontal divider line
fn render_brutal_divider(ui: &mut egui::Ui, border_color: Color32) {
    ui.add_space(6.0);
    let (rect_div, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 4.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect_div, Rounding::ZERO, border_color);
    ui.add_space(6.0);
}

/// Filter ANSI escape sequences and normalize control characters across buffer chunks
fn clean_terminal_output(raw: &str, leftover: &mut String) -> String {
    let combined = format!("{}{}", leftover, raw);
    leftover.clear();

    let bytes = combined.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b == 0x1b {
            if i + 1 >= len {
                *leftover = String::from_utf8_lossy(&bytes[i..]).to_string();
                break;
            }
            let next = bytes[i + 1];
            if next == b'[' {
                let mut j = i + 2;
                let mut found = false;
                while j < len {
                    if (0x40..=0x7e).contains(&bytes[j]) {
                        found = true;
                        i = j + 1;
                        break;
                    }
                    j += 1;
                }
                if !found {
                    *leftover = String::from_utf8_lossy(&bytes[i..]).to_string();
                    break;
                }
            } else if next == b']' {
                let mut j = i + 2;
                let mut found = false;
                while j < len {
                    if bytes[j] == 0x07 {
                        found = true;
                        i = j + 1;
                        break;
                    }
                    if bytes[j] == 0x1b && j + 1 < len && bytes[j + 1] == b'\\' {
                        found = true;
                        i = j + 2;
                        break;
                    }
                    j += 1;
                }
                if !found {
                    *leftover = String::from_utf8_lossy(&bytes[i..]).to_string();
                    break;
                }
            } else if next == b'(' || next == b')' || next == b'*' || next == b'+' {
                if i + 2 < len {
                    i += 3;
                } else {
                    *leftover = String::from_utf8_lossy(&bytes[i..]).to_string();
                    break;
                }
            } else {
                i += 2;
            }
        } else if b == b'\r' {
            if i + 1 >= len {
                *leftover = "\r".to_string();
                break;
            }
            if bytes[i + 1] == b'\n' {
                i += 1;
            } else {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                i += 1;
            }
        } else if b == 0x08 {
            if out.chars().last() != Some('\n') {
                out.pop();
            }
            i += 1;
        } else {
            let s = &combined[i..];
            if let Some(ch) = s.chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        }
    }

    out
}

/// Custom ASCII art for 'NeoShaurya' easter egg
fn get_neofetch_art() -> String {
    r#"
  _   _             ____  _                                
 | \ | | ___  ___  / ___|| |__   __ _ _   _ _ __ _   _  __ _ 
 |  \| |/ _ \/ _ \ \___ \| '_ \ / _` | | | | '__| | | |/ _` |
 | |\  |  __/ (_) | ___) | | | | (_| | |_| | |  | |_| | (_| |
 |_| \_|\___|\___/ |____/|_| |_|\__,_|\__,_|_|   \__, |\__,_|
                                                 |___/       
 ------------------------------------------------------------
 OS:       NeoTerminal OS (Windows x86_64)
 Host:     NeoShaurya Workstation
 Kernel:   NeoCore v1.0.0-brutal
 Uptime:   42 mins
 Shell:    PowerShell 5.1 / ConPTY Native
 Terminal: NeoTerminal (eframe / egui)
 Theme:    Neo-Brutalist High-Contrast
 Palette:  ███ ███ ███ ███ ███ ███ ███ ███
 Creator:  Shaurya
 Status:   SYSTEMS OPERATIONAL // ALL CHECKS PASSED
 ------------------------------------------------------------
"#
    .to_string()
}

/// Create a sleek Neo-Brutalist application icon (64x64 RGBA)
fn create_app_icon() -> egui::IconData {
    let width = 64u32;
    let height = 64u32;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);

    for y in 0..height {
        for x in 0..width {
            // 4px outer brutalist border
            let is_border = x < 4 || x >= width - 4 || y < 4 || y >= height - 4;

            // Chevron '>' prompt
            let is_chevron = if y >= 18 && y <= 46 {
                let x_target = if y <= 32 {
                    16 + (y - 18)
                } else {
                    30 - (y - 32)
                };
                x >= x_target.saturating_sub(2) && x <= x_target + 2
            } else {
                false
            };

            // Solid block cursor '█'
            let is_block_cursor = x >= 36 && x <= 48 && y >= 20 && y <= 44;

            if is_border || is_chevron || is_block_cursor {
                // Pure white pixel
                rgba.extend_from_slice(&[255, 255, 255, 255]);
            } else {
                // Pure black pixel
                rgba.extend_from_slice(&[0, 0, 0, 255]);
            }
        }
    }

    egui::IconData {
        rgba,
        width,
        height,
    }
}

struct TerminalTab {
    _id: usize,
    title: String,
    terminal_output: String,
    ansi_leftover: String,
    input_text: String,
    cursor_pos: usize,
    history: Vec<String>,
    history_idx: Option<usize>,
    has_unread: bool,
    search_active: bool,
    search_query: String,
    search_selected_idx: usize,
    last_activity: Instant,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    rx: Receiver<String>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl TerminalTab {
    fn new(id: usize, title: String, egui_ctx: egui::Context) -> Result<Self, Box<dyn std::error::Error>> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // On Windows, spawn PowerShell directly with inherited environment
        #[cfg(target_os = "windows")]
        let cmd = {
            let mut c = CommandBuilder::new("powershell.exe");
            c.args([
                "-NoLogo",
                "-NoExit",
                "-Command",
                "Remove-Module PSReadLine -ErrorAction SilentlyContinue",
            ]);
            let mut current_path = String::new();
            for (k, v) in std::env::vars_os() {
                if k.to_string_lossy().eq_ignore_ascii_case("PATH") {
                    current_path = v.to_string_lossy().to_string();
                } else {
                    c.env(k, v);
                }
            }
            // Ensure APPDATA\npm and USERPROFILE\.cargo\bin are in PATH so scraper tools and cargo are always reachable
            if let Ok(appdata) = std::env::var("APPDATA") {
                let npm_dir = format!("{}\\npm", appdata);
                if !current_path.to_lowercase().contains(&npm_dir.to_lowercase()) {
                    if !current_path.is_empty() && !current_path.ends_with(';') {
                        current_path.push(';');
                    }
                    current_path.push_str(&npm_dir);
                }
            }
            if let Ok(userprofile) = std::env::var("USERPROFILE") {
                let cargo_dir = format!("{}\\.cargo\\bin", userprofile);
                if !current_path.to_lowercase().contains(&cargo_dir.to_lowercase()) {
                    if !current_path.is_empty() && !current_path.ends_with(';') {
                        current_path.push(';');
                    }
                    current_path.push_str(&cargo_dir);
                }
            }
            c.env("PATH", current_path);

            if let Ok(home) = std::env::var("USERPROFILE") {
                c.cwd(home);
            }
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let shell_cmd = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
            let mut c = CommandBuilder::new(shell_cmd);
            if let Ok(dir) = std::env::current_dir() {
                c.cwd(dir);
            }
            c
        };

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let writer = Arc::new(Mutex::new(writer));

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        // Background thread using std::sync::mpsc to read shell output and send it to the GUI
        let (tx, rx) = channel::<String>();
        let ctx = egui_ctx.clone();

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // Shell exited / EOF
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        if tx.send(text).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                    Err(_) => break,
                }
            }
        });

        let history = load_persistent_history();

        Ok(Self {
            _id: id,
            title,
            terminal_output: String::new(),
            ansi_leftover: String::new(),
            input_text: String::new(),
            cursor_pos: 0,
            history,
            history_idx: None,
            has_unread: false,
            search_active: false,
            search_query: String::new(),
            search_selected_idx: 0,
            last_activity: Instant::now(),
            writer,
            rx,
            _master: pair.master,
            _child: child,
        })
    }

    fn get_search_matches(&self) -> Vec<String> {
        let query = self.search_query.trim();
        let mut candidates: Vec<String> = Vec::new();

        for cmd in self.history.iter().rev() {
            let trimmed = cmd.trim();
            if !trimmed.is_empty() && !candidates.iter().any(|c| c == trimmed) {
                candidates.push(trimmed.to_string());
            }
        }

        if query.is_empty() {
            candidates.truncate(5);
            return candidates;
        }

        let mut scored: Vec<(i32, String)> = Vec::new();
        for cmd in candidates {
            if let Some(score) = fuzzy_match(query, &cmd) {
                scored.push((score, cmd));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().take(5).map(|(_, s)| s).collect()
    }

    fn submit_command(&mut self, ctx: &egui::Context) {
        self.last_activity = Instant::now();
        let cmd = std::mem::take(&mut self.input_text);
        self.cursor_pos = 0;
        self.history_idx = None;

        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            // Send empty newline (Carriage Return) to shell
            if let Ok(mut w) = self.writer.lock() {
                let _ = w.write_all(b"\r");
                let _ = w.flush();
            }
            return;
        }

        // Store into command history and persistent file
        if self.history.last().map(|s| s.as_str()) != Some(&cmd) {
            self.history.push(cmd.clone());
        }
        append_persistent_history(&cmd);

        // Custom tab rename command: 'title <name>' or 'rename <name>'
        let lower = trimmed.to_lowercase();
        if lower.starts_with("title ") || lower.starts_with("rename ") {
            if let Some(space_idx) = trimmed.find(' ') {
                let new_title = trimmed[space_idx + 1..].trim().to_uppercase();
                if !new_title.is_empty() {
                    self.title = new_title;
                    self.terminal_output
                        .push_str(&format!("PS > {}\nTab renamed to '{}'\n\n", cmd, self.title));
                    return;
                }
            }
        }

        // Easter Egg 1: Intercept 'neofetch' to print custom ASCII art for 'NeoShaurya'
        if trimmed.eq_ignore_ascii_case("neofetch") {
            self.terminal_output.push_str(&format!("PS > {}\n", cmd));
            self.terminal_output.push_str(&get_neofetch_art());
            self.terminal_output.push('\n');
            return;
        }

        // Easter Egg 2: Intercept 'sudo contact' to print 'Access Granted...' and block execution
        if trimmed.eq_ignore_ascii_case("sudo contact") {
            self.terminal_output.push_str(&format!("PS > {}\n", cmd));
            self.terminal_output
                .push_str("Access Granted. Redirecting to neoshaurya.com/contact...\n\n");
            // Open URL in default browser
            ctx.open_url(egui::OpenUrl::same_tab("https://neoshaurya.com/contact"));
            return;
        }

        // Send normal command to shell thread using Carriage Return \r for interactive execution
        if let Ok(mut w) = self.writer.lock() {
            let payload = format!("{}\r", trimmed);
            let _ = w.write_all(payload.as_bytes());
            let _ = w.flush();
        }
    }
}

struct NeoTerminalApp {
    dark_mode: bool,
    tabs: Vec<TerminalTab>,
    active_tab: usize,
    next_tab_id: usize,
    start_time: Instant,
}

impl NeoTerminalApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, Box<dyn std::error::Error>> {
        // Load bold native monospace font for punchy Neo-Brutalist typography
        let mut fonts = egui::FontDefinitions::default();
        if let Ok(font_bytes) = std::fs::read("C:\\Windows\\Fonts\\consolab.ttf") {
            fonts.font_data.insert(
                "consolas_bold".to_owned(),
                egui::FontData::from_owned(font_bytes),
            );
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "consolas_bold".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "consolas_bold".to_owned());
        }
        cc.egui_ctx.set_fonts(fonts);

        let initial_tab = TerminalTab::new(1, "SHELL 01".to_string(), cc.egui_ctx.clone())?;

        let app = Self {
            dark_mode: true,
            tabs: vec![initial_tab],
            active_tab: 0,
            next_tab_id: 2,
            start_time: Instant::now(),
        };

        app.apply_theme(&cc.egui_ctx);
        Ok(app)
    }

    fn new_tab(&mut self, ctx: &egui::Context) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let title = format!("SHELL {:02}", id);
        match TerminalTab::new(id, title, ctx.clone()) {
            Ok(tab) => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
            Err(e) => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.terminal_output
                        .push_str(&format!("\n[ERROR] Failed to spawn new tab: {:?}\n", e));
                }
            }
        }
    }

    fn close_tab(&mut self, index: usize, ctx: &egui::Context) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() > 1 {
            self.tabs.remove(index);
            if self.active_tab >= self.tabs.len() {
                self.active_tab = self.tabs.len() - 1;
            } else if self.active_tab > index {
                self.active_tab -= 1;
            }
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.has_unread = false;
            }
        } else {
            // Replace only remaining tab with a fresh one
            let id = self.next_tab_id;
            self.next_tab_id += 1;
            let title = format!("SHELL {:02}", id);
            if let Ok(new_tab) = TerminalTab::new(id, title, ctx.clone()) {
                self.tabs = vec![new_tab];
                self.active_tab = 0;
            } else if let Some(tab) = self.tabs.get_mut(0) {
                tab.terminal_output.clear();
            }
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let (bg_color, fg_color) = if self.dark_mode {
            (PITCH_BLACK, CRISP_WHITE)
        } else {
            (CRISP_WHITE, PITCH_BLACK)
        };

        let mut visuals = if self.dark_mode {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        visuals.panel_fill = bg_color;
        visuals.window_fill = bg_color;
        visuals.override_text_color = Some(fg_color);
        visuals.extreme_bg_color = bg_color;
        visuals.window_rounding = Rounding::ZERO;
        visuals.widgets.noninteractive.rounding = Rounding::ZERO;
        visuals.widgets.inactive.rounding = Rounding::ZERO;
        visuals.widgets.hovered.rounding = Rounding::ZERO;
        visuals.widgets.active.rounding = Rounding::ZERO;
        visuals.widgets.open.rounding = Rounding::ZERO;
        ctx.set_visuals(visuals);
    }
}

impl eframe::App for NeoTerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Ensure at least one tab exists and active_tab is in bounds
        if self.tabs.is_empty() {
            self.new_tab(ctx);
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        }

        // Drain incoming shell output for all tabs
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            let mut received = false;
            while let Ok(msg) = tab.rx.try_recv() {
                let cleaned = clean_terminal_output(&msg, &mut tab.ansi_leftover);
                tab.terminal_output.push_str(&cleaned);
                received = true;
                tab.last_activity = Instant::now();

                // Cap output to prevent unbounded memory growth (UTF-8 character boundary safe)
                if tab.terminal_output.len() > 100_000 {
                    let excess = tab.terminal_output.len() - 100_000;
                    if let Some((i, _)) = tab
                        .terminal_output
                        .char_indices()
                        .find(|(i, _)| *i >= excess)
                    {
                        tab.terminal_output.drain(..i);
                    }
                }
            }
            if received && idx != self.active_tab {
                tab.has_unread = true;
            }
        }

        // Gumroad Neo-Brutalist Theme Colors (matching neoshaurya.com):
        // Dark Mode: Pure pitch black surface, crisp white text & borders, solid white/accent shadows
        // Light Mode: Crisp white surface, pitch black text & borders, solid pitch black shadows
        let (bg_color, fg_color, border_color, shadow_color) = if self.dark_mode {
            (
                PITCH_BLACK,
                CRISP_WHITE,
                CRISP_WHITE,
                CRISP_WHITE,
            )
        } else {
            (
                CRISP_WHITE,
                PITCH_BLACK,
                PITCH_BLACK,
                PITCH_BLACK,
            )
        };

        self.apply_theme(ctx);

        // Keystroke Capture
        let mut action_new_tab = false;
        let mut action_close_tab = false;
        let mut action_next_tab = false;
        let mut action_prev_tab = false;
        let mut action_select_tab: Option<usize> = None;

        ctx.input(|i| {
            if self.tabs.is_empty() {
                return;
            }
            let active_idx = self.active_tab.min(self.tabs.len() - 1);
            let tab = &mut self.tabs[active_idx];

            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        if tab.search_active {
                            for ch in text.chars() {
                                if !ch.is_control() {
                                    tab.search_query.push(ch);
                                    tab.search_selected_idx = 0;
                                }
                            }
                        } else {
                            for ch in text.chars() {
                                if !ch.is_control() {
                                    let byte_pos = tab
                                        .input_text
                                        .char_indices()
                                        .nth(tab.cursor_pos)
                                        .map(|(idx, _)| idx)
                                        .unwrap_or(tab.input_text.len());
                                    tab.input_text.insert(byte_pos, ch);
                                    tab.cursor_pos += 1;
                                }
                            }
                        }
                    }
                    egui::Event::Paste(text) => {
                        let filtered = text
                            .replace("\r\n", " ")
                            .replace('\n', " ")
                            .replace('\r', " ");
                        if tab.search_active {
                            tab.search_query.push_str(&filtered);
                            tab.search_selected_idx = 0;
                        } else {
                            let byte_pos = tab
                                .input_text
                                .char_indices()
                                .nth(tab.cursor_pos)
                                .map(|(idx, _)| idx)
                                .unwrap_or(tab.input_text.len());
                            tab.input_text.insert_str(byte_pos, &filtered);
                            tab.cursor_pos += filtered.chars().count();
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if tab.search_active {
                            match key {
                                Key::Escape => {
                                    tab.search_active = false;
                                }
                                Key::C if modifiers.ctrl => {
                                    tab.search_active = false;
                                }
                                Key::R if modifiers.ctrl => {
                                    let matches = tab.get_search_matches();
                                    if !matches.is_empty() {
                                        tab.search_selected_idx =
                                            (tab.search_selected_idx + 1) % matches.len();
                                    }
                                }
                                Key::Enter | Key::Tab => {
                                    let matches = tab.get_search_matches();
                                    if !matches.is_empty() {
                                        let sel =
                                            tab.search_selected_idx.min(matches.len() - 1);
                                        tab.input_text = matches[sel].clone();
                                        tab.cursor_pos = tab.input_text.chars().count();
                                    }
                                    tab.search_active = false;
                                }
                                Key::Backspace => {
                                    tab.search_query.pop();
                                    tab.search_selected_idx = 0;
                                }
                                Key::ArrowUp => {
                                    if tab.search_selected_idx > 0 {
                                        tab.search_selected_idx -= 1;
                                    }
                                }
                                Key::ArrowDown => {
                                    let matches = tab.get_search_matches();
                                    if !matches.is_empty()
                                        && tab.search_selected_idx + 1 < matches.len()
                                    {
                                        tab.search_selected_idx += 1;
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            match key {
                                Key::R if modifiers.ctrl => {
                                    tab.search_active = true;
                                    tab.search_query.clear();
                                    tab.search_selected_idx = 0;
                                }
                                Key::Enter => {
                                    tab.submit_command(ctx);
                                }
                                Key::Backspace => {
                                    if tab.cursor_pos > 0 && !tab.input_text.is_empty() {
                                        tab.cursor_pos -= 1;
                                        let byte_pos = tab
                                            .input_text
                                            .char_indices()
                                            .nth(tab.cursor_pos)
                                            .map(|(idx, _)| idx)
                                            .unwrap_or(0);
                                        tab.input_text.remove(byte_pos);
                                    }
                                }
                                Key::Delete => {
                                    if tab.cursor_pos < tab.input_text.chars().count() {
                                        let byte_pos = tab
                                            .input_text
                                            .char_indices()
                                            .nth(tab.cursor_pos)
                                            .map(|(idx, _)| idx)
                                            .unwrap_or(tab.input_text.len());
                                        tab.input_text.remove(byte_pos);
                                    }
                                }
                                Key::ArrowLeft => {
                                    if tab.cursor_pos > 0 {
                                        tab.cursor_pos -= 1;
                                    }
                                }
                                Key::ArrowRight => {
                                    if tab.cursor_pos < tab.input_text.chars().count() {
                                        tab.cursor_pos += 1;
                                    }
                                }
                                Key::Home => {
                                    tab.cursor_pos = 0;
                                }
                                Key::End => {
                                    tab.cursor_pos = tab.input_text.chars().count();
                                }
                                Key::ArrowUp => {
                                    if !tab.history.is_empty() {
                                        let next_idx = match tab.history_idx {
                                            None => tab.history.len() - 1,
                                            Some(idx) if idx > 0 => idx - 1,
                                            Some(idx) => idx,
                                        };
                                        tab.history_idx = Some(next_idx);
                                        tab.input_text = tab.history[next_idx].clone();
                                        tab.cursor_pos = tab.input_text.chars().count();
                                    }
                                }
                                Key::ArrowDown => {
                                    if let Some(idx) = tab.history_idx {
                                        if idx + 1 < tab.history.len() {
                                            tab.history_idx = Some(idx + 1);
                                            tab.input_text = tab.history[idx + 1].clone();
                                            tab.cursor_pos = tab.input_text.chars().count();
                                        } else {
                                            tab.history_idx = None;
                                            tab.input_text.clear();
                                            tab.cursor_pos = 0;
                                        }
                                    }
                                }
                                Key::C if modifiers.ctrl => {
                                    tab.terminal_output
                                        .push_str(&format!("PS > {}^C\n", tab.input_text));
                                    tab.input_text.clear();
                                    tab.cursor_pos = 0;
                                    if let Ok(mut w) = tab.writer.lock() {
                                        let _ = w.write_all(b"\x03");
                                        let _ = w.flush();
                                    }
                                }
                                Key::L if modifiers.ctrl => {
                                    tab.terminal_output.clear();
                                }
                                Key::T if modifiers.ctrl => {
                                    action_new_tab = true;
                                }
                                Key::W if modifiers.ctrl => {
                                    action_close_tab = true;
                                }
                                Key::Tab if modifiers.ctrl => {
                                    if modifiers.shift {
                                        action_prev_tab = true;
                                    } else {
                                        action_next_tab = true;
                                    }
                                }
                                Key::Num1 if modifiers.ctrl => { action_select_tab = Some(0); }
                                Key::Num2 if modifiers.ctrl => { action_select_tab = Some(1); }
                                Key::Num3 if modifiers.ctrl => { action_select_tab = Some(2); }
                                Key::Num4 if modifiers.ctrl => { action_select_tab = Some(3); }
                                Key::Num5 if modifiers.ctrl => { action_select_tab = Some(4); }
                                Key::Num6 if modifiers.ctrl => { action_select_tab = Some(5); }
                                Key::Num7 if modifiers.ctrl => { action_select_tab = Some(6); }
                                Key::Num8 if modifiers.ctrl => { action_select_tab = Some(7); }
                                Key::Num9 if modifiers.ctrl => { action_select_tab = Some(8); }
                                _ => {}
                            }
                        }
                    },
                    _ => {}
                }
            }
        });

        if action_new_tab {
            self.new_tab(ctx);
        } else if action_close_tab {
            self.close_tab(self.active_tab, ctx);
        } else if action_next_tab && !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.tabs[self.active_tab].has_unread = false;
        } else if action_prev_tab && !self.tabs.is_empty() {
            self.active_tab = if self.active_tab == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab - 1
            };
            self.tabs[self.active_tab].has_unread = false;
        } else if let Some(target) = action_select_tab {
            if target < self.tabs.len() {
                self.active_tab = target;
                self.tabs[self.active_tab].has_unread = false;
            }
        }

        // 4px Neo-Brutalist Outer Border & Layout
        let border_stroke = Stroke::new(4.0_f32, border_color);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(bg_color)
                    .stroke(border_stroke)
                    .inner_margin(Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                // Header: Title, Engine Pill, Live Indicator, & Theme Toggle Button
                ui.horizontal(|ui| {
                    render_brutal_badge(
                        ui,
                        "⚡ NEOTERMINAL",
                        ACCENT_YELLOW,
                        PITCH_BLACK,
                        border_color,
                        shadow_color,
                        4.0,
                        14.5,
                    );
                    ui.add_space(4.0);

                    render_brutal_badge(
                        ui,
                        "POWERSHELL CONPTY",
                        bg_color,
                        fg_color,
                        border_color,
                        shadow_color,
                        3.0,
                        11.5,
                    );
                    ui.add_space(4.0);

                    render_brutal_badge(
                        ui,
                        "● ONLINE",
                        ACCENT_TEAL,
                        PITCH_BLACK,
                        border_color,
                        shadow_color,
                        3.0,
                        11.5,
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (toggle_label, btn_bg, btn_fg, btn_shadow) = if self.dark_mode {
                            ("● DARK MODE", CRISP_WHITE, PITCH_BLACK, ACCENT_YELLOW)
                        } else {
                            ("○ LIGHT MODE", PITCH_BLACK, CRISP_WHITE, PITCH_BLACK)
                        };

                        if render_brutal_button(
                            ui,
                            toggle_label,
                            btn_bg,
                            btn_fg,
                            border_color,
                            btn_shadow,
                            3.0,
                            12.0,
                        )
                        .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                            self.apply_theme(ctx);
                        }
                    });
                });

                render_brutal_divider(ui, border_color);

                // Neo-Brutalist Tab Bar
                let mut switch_to_tab: Option<usize> = None;
                let mut close_tab_idx: Option<usize> = None;
                let mut create_new_tab = false;

                ui.horizontal(|ui| {
                    for (idx, tab) in self.tabs.iter().enumerate() {
                        let is_active = idx == self.active_tab;
                        let (clicked_tab, clicked_close) = render_brutal_tab(
                            ui,
                            idx,
                            &tab.title,
                            is_active,
                            tab.has_unread,
                            self.tabs.len() > 1,
                            self.dark_mode,
                            border_color,
                            shadow_color,
                        );
                        if clicked_tab {
                            switch_to_tab = Some(idx);
                        }
                        if clicked_close {
                            close_tab_idx = Some(idx);
                        }
                        ui.add_space(4.0);
                    }

                    if render_brutal_button(
                        ui,
                        " + NEW TAB ",
                        ACCENT_TEAL,
                        PITCH_BLACK,
                        border_color,
                        shadow_color,
                        3.0,
                        12.0,
                    )
                    .clicked()
                    {
                        create_new_tab = true;
                    }
                });

                if let Some(target) = switch_to_tab {
                    if target < self.tabs.len() {
                        self.active_tab = target;
                        self.tabs[self.active_tab].has_unread = false;
                    }
                }
                if let Some(target) = close_tab_idx {
                    self.close_tab(target, ctx);
                }
                if create_new_tab {
                    self.new_tab(ctx);
                }

                render_brutal_divider(ui, border_color);

                // Scrolling Terminal Output Area in a Brutalist Screen Box
                let is_searching = self
                    .tabs
                    .get(self.active_tab)
                    .map_or(false, |t| t.search_active);
                let bottom_reserve = if is_searching { 325.0_f32 } else { 140.0_f32 };
                let viewport_h = (ui.available_height() - bottom_reserve).max(80.0_f32);

                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    render_brutal_card(
                        ui,
                        viewport_h,
                        bg_color,
                        border_color,
                        shadow_color,
                        4.0,
                        |ui| {
                            ScrollArea::vertical()
                                .max_height(viewport_h - 14.0)
                                .auto_shrink([false, false])
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&active_tab.terminal_output)
                                                .font(FontId::monospace(14.5))
                                                .strong()
                                                .color(fg_color),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                    );
                                });
                        },
                    );
                }

                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    if active_tab.search_active {
                        render_brutal_divider(ui, border_color);

                        let matches = active_tab.get_search_matches();
                        let total_matches = matches.len();
                        let sel_idx = if total_matches > 0 {
                            active_tab.search_selected_idx.min(total_matches - 1)
                        } else {
                            0
                        };

                        render_brutal_card(
                            ui,
                            165.0,
                            bg_color,
                            border_color,
                            if self.dark_mode { ACCENT_PINK } else { border_color },
                            5.0,
                            |ui| {
                                ui.horizontal(|ui| {
                                    render_brutal_badge(
                                        ui,
                                        "REVERSE-i-SEARCH >",
                                        ACCENT_PINK,
                                        PITCH_BLACK,
                                        border_color,
                                        shadow_color,
                                        2.0,
                                        13.0,
                                    );
                                    ui.add_space(4.0);

                                    let query_display = format!("{}█", active_tab.search_query);
                                    ui.label(
                                        RichText::new(query_display)
                                            .font(FontId::monospace(14.0))
                                            .strong()
                                            .color(if active_tab.search_query.is_empty() {
                                                ACCENT_YELLOW
                                            } else {
                                                fg_color
                                            }),
                                    );

                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let (count_label, count_bg, count_fg) = if total_matches > 0 {
                                            (
                                                format!(
                                                    "[ MATCH {:02}/{:02} ]",
                                                    sel_idx + 1,
                                                    total_matches
                                                ),
                                                ACCENT_YELLOW,
                                                PITCH_BLACK,
                                            )
                                        } else {
                                            (
                                                "[ NO MATCHES ]".to_string(),
                                                ACCENT_ORANGE,
                                                PITCH_BLACK,
                                            )
                                        };
                                        render_brutal_badge(
                                            ui,
                                            &count_label,
                                            count_bg,
                                            count_fg,
                                            border_color,
                                            shadow_color,
                                            2.0,
                                            11.5,
                                        );
                                    });
                                });

                                if !matches.is_empty() {
                                    ui.add_space(6.0);
                                    let mut clicked_idx: Option<usize> = None;
                                    ScrollArea::vertical().max_height(105.0).show(ui, |ui| {
                                        for (m_idx, match_item) in matches.iter().enumerate() {
                                            let is_selected = m_idx == sel_idx;
                                            let (row_bg, row_fg) = if is_selected {
                                                (ACCENT_YELLOW, PITCH_BLACK)
                                            } else {
                                                (bg_color, fg_color)
                                            };

                                            let prefix = if is_selected { "> " } else { "  " };
                                            let row_text =
                                                format!("{}{:02}: {}", prefix, m_idx + 1, match_item);

                                            egui::Frame::none()
                                                .fill(row_bg)
                                                .stroke(if is_selected {
                                                    Stroke::new(3.0_f32, border_color)
                                                } else {
                                                    Stroke::NONE
                                                })
                                                .rounding(Rounding::ZERO)
                                                .inner_margin(Margin::symmetric(8.0, 3.5))
                                                .show(ui, |ui| {
                                                    let resp = ui.add(
                                                        egui::Label::new(
                                                            RichText::new(row_text)
                                                                .font(FontId::monospace(12.5))
                                                                .strong()
                                                                .color(row_fg),
                                                        )
                                                        .sense(egui::Sense::click()),
                                                    );
                                                    if resp.clicked() {
                                                        clicked_idx = Some(m_idx);
                                                    }
                                                });
                                            ui.add_space(2.0);
                                        }
                                    });

                                    if let Some(c_idx) = clicked_idx {
                                        active_tab.input_text = matches[c_idx].clone();
                                        active_tab.cursor_pos =
                                            active_tab.input_text.chars().count();
                                        active_tab.search_active = false;
                                    }
                                }
                            },
                        );
                    }
                }

                render_brutal_divider(ui, border_color);

                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    // Command Dock with Gumroad Yellow Prompt Badge and Electric Pink Cursor
                    render_brutal_card(
                        ui,
                        46.0,
                        bg_color,
                        border_color,
                        shadow_color,
                        4.0,
                        |ui| {
                            ui.horizontal(|ui| {
                                render_brutal_badge(
                                    ui,
                                    "PS >",
                                    ACCENT_YELLOW,
                                    PITCH_BLACK,
                                    border_color,
                                    shadow_color,
                                    2.0,
                                    14.0,
                                );
                                ui.add_space(6.0);

                                let byte_pos = active_tab
                                    .input_text
                                    .char_indices()
                                    .nth(active_tab.cursor_pos)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(active_tab.input_text.len());

                                let before = &active_tab.input_text[..byte_pos];
                                let after = &active_tab.input_text[byte_pos..];

                                ui.label(
                                    RichText::new(before)
                                        .font(FontId::monospace(15.0))
                                        .strong()
                                        .color(fg_color),
                                );
                                ui.label(
                                    RichText::new("█")
                                        .font(FontId::monospace(15.0))
                                        .strong()
                                        .color(ACCENT_PINK),
                                );
                                ui.label(
                                    RichText::new(after)
                                        .font(FontId::monospace(15.0))
                                        .strong()
                                        .color(fg_color),
                                );
                            });
                        },
                    );
                }

                render_brutal_divider(ui, border_color);

                // Bottom Status Strip (System & Git Telemetry Cards matching neoshaurya.com)
                ui.horizontal(|ui| {
                    // 1. Directory Badge (Yellow)
                    let dir_name = get_current_dir_name();
                    render_brutal_badge(
                        ui,
                        &format!("DIR: {dir_name}"),
                        ACCENT_YELLOW,
                        PITCH_BLACK,
                        border_color,
                        shadow_color,
                        3.0,
                        12.0,
                    );
                    ui.add_space(4.0);

                    // 2. Git Branch Badge (Pink)
                    if let Some(branch) = get_git_branch_info() {
                        render_brutal_badge(
                            ui,
                            &format!("GIT: {branch}"),
                            ACCENT_PINK,
                            PITCH_BLACK,
                            border_color,
                            shadow_color,
                            3.0,
                            12.0,
                        );
                        ui.add_space(4.0);
                    }

                    // 3. System Memory Badge (Dynamic Warning if >80%)
                    if let Some((load, used_gb, total_gb)) = get_system_memory_info() {
                        let (ram_bg, ram_fg) = if load > 80 {
                            (ACCENT_ORANGE, PITCH_BLACK)
                        } else {
                            (bg_color, fg_color)
                        };
                        render_brutal_badge(
                            ui,
                            &format!("RAM: {used_gb:.1}G/{total_gb:.0}G ({load}%)"),
                            ram_bg,
                            ram_fg,
                            border_color,
                            shadow_color,
                            3.0,
                            12.0,
                        );
                        ui.add_space(4.0);
                    }

                    // 4. Tab Status Badge
                    let total_tabs = self.tabs.len();
                    let current_tab_num = self.active_tab + 1;
                    render_brutal_badge(
                        ui,
                        &format!("TAB: {current_tab_num}/{total_tabs}"),
                        bg_color,
                        fg_color,
                        border_color,
                        shadow_color,
                        3.0,
                        12.0,
                    );
                    ui.add_space(4.0);

                    // 5. Active Tab Execution State Badge (Teal IDLE / Orange BUSY)
                    let is_busy = self
                        .tabs
                        .get(self.active_tab)
                        .map(|t| t.last_activity.elapsed() < Duration::from_millis(800))
                        .unwrap_or(false);
                    let (state_label, state_bg) = if is_busy {
                        ("STATE: BUSY ●", ACCENT_ORANGE)
                    } else {
                        ("STATE: IDLE", ACCENT_TEAL)
                    };
                    render_brutal_badge(
                        ui,
                        state_label,
                        state_bg,
                        PITCH_BLACK,
                        border_color,
                        shadow_color,
                        3.0,
                        12.0,
                    );
                    ui.add_space(4.0);

                    // 6. Uptime Badge
                    let up_secs = self.start_time.elapsed().as_secs();
                    let up_mins = up_secs / 60;
                    let up_rem_secs = up_secs % 60;
                    render_brutal_badge(
                        ui,
                        &format!("UP: {up_mins}m {up_rem_secs:02}s"),
                        bg_color,
                        fg_color,
                        border_color,
                        shadow_color,
                        3.0,
                        12.0,
                    );

                    // Right-aligned hotkey pill
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (hint_text, hint_bg, hint_fg) = if is_searching {
                            ("ESC: CANCEL // ENTER: SELECT", ACCENT_PINK, PITCH_BLACK)
                        } else {
                            ("CTRL+R: SEARCH // WIN+Q // CTRL+T", bg_color, fg_color)
                        };
                        render_brutal_badge(
                            ui,
                            hint_text,
                            hint_bg,
                            hint_fg,
                            border_color,
                            shadow_color,
                            3.0,
                            11.5,
                        );
                    });
                });
            });
    }
}

fn main() -> eframe::Result {
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::write("C:\\Users\\shaur\\neo_panic.log", format!("PANIC: {:?}", info));
    }));

    // Suppress system modal error boxes
    #[cfg(target_os = "windows")]
    unsafe {
        SetErrorMode(0x0001 | 0x0002); // SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX
    }

    let icon_data = create_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0_f32, 640.0_f32])
            .with_min_inner_size([500.0_f32, 360.0_f32])
            .with_title("NeoTerminal")
            .with_icon(icon_data),
        ..Default::default()
    };

    eframe::run_native(
        "NeoTerminal",
        options,
        Box::new(|cc| {
            let app = match NeoTerminalApp::new(cc) {
                Ok(a) => a,
                Err(e) => {
                    let _ = std::fs::write("C:\\Users\\shaur\\neo_panic.log", format!("INIT ERROR: {:?}", e));
                    panic!("Init error: {:?}", e);
                }
            };
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_terminal_output_ansi_csi() {
        let mut leftover = String::new();
        let raw = "\x1b[32mHello \x1b[1;34mWorld\x1b[0m\r\n";
        let cleaned = clean_terminal_output(raw, &mut leftover);
        assert_eq!(cleaned, "Hello World\n");
        assert!(leftover.is_empty());
    }

    #[test]
    fn test_clean_terminal_output_osc() {
        let mut leftover = String::new();
        let raw = "\x1b]0;Title\x07Text\r\n";
        let cleaned = clean_terminal_output(raw, &mut leftover);
        assert_eq!(cleaned, "Text\n");
        assert!(leftover.is_empty());
    }

    #[test]
    fn test_clean_terminal_output_crlf_split() {
        let mut leftover = String::new();
        let chunk1 = "[OpenClaw Sync] Scraping repositories...\r";
        let cleaned1 = clean_terminal_output(chunk1, &mut leftover);
        assert_eq!(cleaned1, "[OpenClaw Sync] Scraping repositories...");
        assert_eq!(leftover, "\r");

        let chunk2 = "\n[OpenClaw Sync] Found 3 repositories\r\n";
        let cleaned2 = clean_terminal_output(chunk2, &mut leftover);
        assert_eq!(cleaned2, "\n[OpenClaw Sync] Found 3 repositories\n");
        assert!(leftover.is_empty());
    }

    #[test]
    fn test_clean_terminal_output_bare_cr_progress() {
        let mut leftover = String::new();
        let raw = "Step 1\rStep 2\r\n";
        let cleaned = clean_terminal_output(raw, &mut leftover);
        assert_eq!(cleaned, "Step 1\nStep 2\n");
        assert!(leftover.is_empty());
    }

    #[test]
    fn test_clean_terminal_output_backspace() {
        let mut leftover = String::new();
        let raw = "abc\x08d";
        let cleaned = clean_terminal_output(raw, &mut leftover);
        assert_eq!(cleaned, "abd");
    }

    #[test]
    fn test_clean_terminal_output_split_escape() {
        let mut leftover = String::new();
        let chunk1 = "Hello \x1b[";
        let cleaned1 = clean_terminal_output(chunk1, &mut leftover);
        assert_eq!(cleaned1, "Hello ");
        assert_eq!(leftover, "\x1b[");

        let chunk2 = "32mWorld";
        let cleaned2 = clean_terminal_output(chunk2, &mut leftover);
        assert_eq!(cleaned2, "World");
        assert!(leftover.is_empty());
    }

    #[test]
    fn test_clean_terminal_output_utf8() {
        let mut leftover = String::new();
        let raw = "Status: █ SYSTEM ONLINE — OK\r\n";
        let cleaned = clean_terminal_output(raw, &mut leftover);
        assert_eq!(cleaned, "Status: █ SYSTEM ONLINE — OK\n");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_backend_sync_projects_pty() {
        use std::time::{Duration, Instant};

        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 32,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty failed");

        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.args([
            "-NoLogo",
            "-NoExit",
            "-Command",
            "Remove-Module PSReadLine -ErrorAction SilentlyContinue",
        ]);

        let mut current_path = String::new();
        for (k, v) in std::env::vars_os() {
            if k.to_string_lossy().eq_ignore_ascii_case("PATH") {
                current_path = v.to_string_lossy().to_string();
            } else {
                cmd.env(k, v);
            }
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            let npm_dir = format!("{}\\npm", appdata);
            if !current_path.to_lowercase().contains(&npm_dir.to_lowercase()) {
                if !current_path.is_empty() && !current_path.ends_with(';') {
                    current_path.push(';');
                }
                current_path.push_str(&npm_dir);
            }
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let cargo_dir = format!("{}\\.cargo\\bin", userprofile);
            if !current_path.to_lowercase().contains(&cargo_dir.to_lowercase()) {
                if !current_path.is_empty() && !current_path.ends_with(';') {
                    current_path.push(';');
                }
                current_path.push_str(&cargo_dir);
            }
        }
        cmd.env("PATH", current_path);

        if let Ok(home) = std::env::var("USERPROFILE") {
            cmd.cwd(home);
        }

        let mut reader = pair.master.try_clone_reader().expect("reader clone failed");
        let mut writer = pair.master.take_writer().expect("take_writer failed");
        let _child = pair.slave.spawn_command(cmd).expect("spawn_command failed");
        drop(pair.slave);

        // Keep master alive
        let _master = pair.master;

        // Allow PowerShell to initialize
        std::thread::sleep(Duration::from_millis(1500));

        // Read initial prompt
        let mut buf = [0u8; 4096];
        let _ = reader.read(&mut buf);

        // Send sync-projects
        writer
            .write_all(b"sync-projects\r")
            .expect("write failed");
        writer.flush().expect("flush failed");

        let mut accumulated = String::new();
        let mut leftover = String::new();
        let start = Instant::now();

        // Read until we see sync completion or timeout
        while start.elapsed() < Duration::from_secs(15) {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    let cleaned = clean_terminal_output(&chunk, &mut leftover);
                    accumulated.push_str(&cleaned);
                    if accumulated.contains("[OpenClaw Sync]")
                        && (accumulated.contains("All projects already up to date")
                            || accumulated.contains("Sync pipeline completed"))
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        println!("Test Accumulated Output:\n{}", accumulated);
        assert!(
            accumulated.contains("[OpenClaw Sync]"),
            "Output must contain OpenClaw Sync header"
        );
        assert!(
            accumulated.contains("Starting GitHub -> Portfolio Sync Pipeline"),
            "Output must show sync pipeline start"
        );
    }

    #[test]
    fn test_tab_management_logic() {
        let mut tab_count = 3;
        let mut active = 2; // Active is the 3rd tab (index 2)

        // Close tab 0 (earlier tab)
        let closed = 0;
        tab_count -= 1;
        if active > closed {
            active -= 1;
        }
        assert_eq!(active, 1);
        assert_eq!(tab_count, 2);

        // Now active is 1 (the last tab of 2). Close tab 1 (the active tab).
        let _closed = 1;
        tab_count -= 1;
        if active >= tab_count {
            active = tab_count - 1;
        }
        assert_eq!(active, 0);
        assert_eq!(tab_count, 1);

        // Test title parsing logic
        let cmd = "title Scraper Engine";
        let trimmed = cmd.trim();
        let space_idx = trimmed.find(' ').unwrap();
        let new_title = trimmed[space_idx + 1..].trim().to_uppercase();
        assert_eq!(new_title, "SCRAPER ENGINE");
    }

    #[test]
    fn test_system_telemetry_functions() {
        // Test memory info retrieval
        if let Some((load, used_gb, total_gb)) = get_system_memory_info() {
            assert!(load <= 100, "Memory load should be <= 100%");
            assert!(total_gb > 0.0, "Total RAM should be greater than 0");
            assert!(used_gb <= total_gb, "Used RAM should be <= Total RAM");
        }

        // Test current directory retrieval
        let dir = get_current_dir_name();
        assert!(!dir.is_empty(), "Directory name should not be empty");

        // Test git branch retrieval within a git repo
        if let Some(branch) = get_git_branch_info() {
            assert!(!branch.is_empty(), "Branch name should not be empty");
        }
    }

    #[test]
    fn test_fuzzy_match_algorithm() {
        // Exact match
        assert_eq!(fuzzy_match("git status", "git status"), Some(10000));
        assert_eq!(fuzzy_match("GIT STATUS", "git status"), Some(10000));

        // Substring match
        let score_prefix = fuzzy_match("sync", "sync-projects").expect("should match prefix");
        let score_mid = fuzzy_match("proj", "sync-projects").expect("should match substring");
        assert!(score_prefix > score_mid, "Prefix match should score higher");

        // Subsequence match
        let score_subseq = fuzzy_match("sp", "sync-projects").expect("should match subsequence");
        assert!(score_subseq > 0);

        // Non-matching
        assert_eq!(fuzzy_match("xyz", "sync-projects"), None);
    }

    #[test]
    fn test_history_search_matches() {
        let history = vec![
            "git status".to_string(),
            "sync-portfolio".to_string(),
            "cargo build --release".to_string(),
            "sync-projects".to_string(),
        ];

        // Empty query returns most recent items
        let mut candidates = Vec::new();
        for cmd in history.iter().rev() {
            candidates.push(cmd.clone());
        }
        assert_eq!(candidates[0], "sync-projects");

        // "sync" query matches both sync commands with "sync-projects" ranked first
        let q = "sync";
        let mut scored: Vec<(i32, String)> = Vec::new();
        for cmd in candidates {
            if let Some(score) = fuzzy_match(q, &cmd) {
                scored.push((score, cmd));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let results: Vec<String> = scored.into_iter().map(|(_, s)| s).collect();
        assert!(results.contains(&"sync-projects".to_string()));
        assert!(results.contains(&"sync-portfolio".to_string()));
        assert!(!results.contains(&"git status".to_string()));
    }
}

