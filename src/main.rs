#![windows_subsystem = "windows"]

use eframe::egui::{
    self, Align, Color32, FontId, Key, Layout, Margin, RichText, Rounding, ScrollArea, Stroke,
    Vec2, Visuals,
};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

fn render_status_badge(
    ui: &mut egui::Ui,
    label: &str,
    is_inverted: bool,
    bg_color: Color32,
    fg_color: Color32,
    border_color: Color32,
    dark_mode: bool,
) {
    let (badge_bg, badge_fg) = if is_inverted {
        if dark_mode {
            (Color32::WHITE, Color32::BLACK)
        } else {
            (Color32::BLACK, Color32::WHITE)
        }
    } else {
        (bg_color, fg_color)
    };

    egui::Frame::none()
        .fill(badge_bg)
        .stroke(Stroke::new(4.0_f32, border_color))
        .rounding(Rounding::ZERO)
        .inner_margin(Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .font(FontId::monospace(12.0))
                    .strong()
                    .color(badge_fg),
            );
        });
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

        Ok(Self {
            _id: id,
            title,
            terminal_output: String::new(),
            ansi_leftover: String::new(),
            input_text: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_idx: None,
            has_unread: false,
            last_activity: Instant::now(),
            writer,
            rx,
            _master: pair.master,
            _child: child,
        })
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

        // Store into command history
        self.history.push(cmd.clone());

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
            (Color32::from_rgb(0, 0, 0), Color32::from_rgb(255, 255, 255))
        } else {
            (Color32::from_rgb(255, 255, 255), Color32::from_rgb(0, 0, 0))
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

        // Neo-Brutalist Theme Colors:
        // Dark Mode: Pure black background, pure white text, 4px white border
        // Light Mode: Pure white background, pure black text, 4px black border
        let (bg_color, fg_color, border_color) = if self.dark_mode {
            (
                Color32::from_rgb(0, 0, 0),
                Color32::from_rgb(255, 255, 255),
                Color32::from_rgb(255, 255, 255),
            )
        } else {
            (
                Color32::from_rgb(255, 255, 255),
                Color32::from_rgb(0, 0, 0),
                Color32::from_rgb(0, 0, 0),
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
                    egui::Event::Paste(text) => {
                        let filtered = text
                            .replace("\r\n", " ")
                            .replace('\n', " ")
                            .replace('\r', " ");
                        let byte_pos = tab
                            .input_text
                            .char_indices()
                            .nth(tab.cursor_pos)
                            .map(|(idx, _)| idx)
                            .unwrap_or(tab.input_text.len());
                        tab.input_text.insert_str(byte_pos, &filtered);
                        tab.cursor_pos += filtered.chars().count();
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => match key {
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
                // Header: Title & Theme Toggle
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("NEOTERMINAL")
                            .font(FontId::monospace(19.0))
                            .strong()
                            .color(fg_color),
                    );
                    ui.label(
                        RichText::new("// POWERSHELL CONPTY")
                            .font(FontId::monospace(13.0))
                            .strong()
                            .color(fg_color),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Neo-Brutalist Theme Toggle Button (Only toggle button, no checkmark)
                        let (btn_bg, btn_fg) = if self.dark_mode {
                            (Color32::WHITE, Color32::BLACK)
                        } else {
                            (Color32::BLACK, Color32::WHITE)
                        };

                        let toggle_label = if self.dark_mode {
                            "● DARK MODE"
                        } else {
                            "○ LIGHT MODE"
                        };

                        let toggle_btn = egui::Button::new(
                            RichText::new(toggle_label)
                                .font(FontId::monospace(13.0))
                                .strong()
                                .color(btn_fg),
                        )
                        .fill(btn_bg)
                        .stroke(Stroke::new(4.0_f32, border_color))
                        .rounding(Rounding::ZERO);

                        if ui.add(toggle_btn).clicked() {
                            self.dark_mode = !self.dark_mode;
                            self.apply_theme(ctx);
                        }
                    });
                });

                ui.add_space(8.0);
                // 4px Neo-Brutalist Divider
                let (rect_div1, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 4.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect_div1, Rounding::ZERO, border_color);
                ui.add_space(8.0);

                // Neo-Brutalist Tab Bar
                let mut switch_to_tab: Option<usize> = None;
                let mut close_tab_idx: Option<usize> = None;
                let mut create_new_tab = false;

                ui.horizontal(|ui| {
                    for (idx, tab) in self.tabs.iter().enumerate() {
                        let is_active = idx == self.active_tab;
                        let (tab_bg, tab_fg) = if is_active {
                            if self.dark_mode {
                                (Color32::WHITE, Color32::BLACK)
                            } else {
                                (Color32::BLACK, Color32::WHITE)
                            }
                        } else {
                            (bg_color, fg_color)
                        };

                        let unread_marker = if tab.has_unread && !is_active { " ●" } else { "" };
                        let tab_label = format!(" {:02}: {}{} ", idx + 1, tab.title, unread_marker);

                        egui::Frame::none()
                            .fill(tab_bg)
                            .stroke(Stroke::new(4.0_f32, border_color))
                            .rounding(Rounding::ZERO)
                            .inner_margin(Margin::symmetric(8.0, 5.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let title_resp = ui.add(
                                        egui::Label::new(
                                            RichText::new(tab_label)
                                                .font(FontId::monospace(12.5))
                                                .strong()
                                                .color(tab_fg),
                                        )
                                        .sense(egui::Sense::click()),
                                    );
                                    if title_resp.clicked() {
                                        switch_to_tab = Some(idx);
                                    }

                                    if self.tabs.len() > 1 {
                                        ui.add_space(2.0);
                                        let close_resp = ui.add(
                                            egui::Label::new(
                                                RichText::new("×")
                                                    .font(FontId::monospace(13.5))
                                                    .strong()
                                                    .color(tab_fg),
                                            )
                                            .sense(egui::Sense::click()),
                                        );
                                        if close_resp.clicked() {
                                            close_tab_idx = Some(idx);
                                        }
                                    }
                                });
                            });

                        ui.add_space(4.0);
                    }

                    // "+ NEW TAB" button
                    let add_btn = egui::Button::new(
                        RichText::new(" + NEW TAB ")
                            .font(FontId::monospace(12.0))
                            .strong()
                            .color(fg_color),
                    )
                    .fill(bg_color)
                    .stroke(Stroke::new(4.0_f32, border_color))
                    .rounding(Rounding::ZERO);

                    if ui.add(add_btn).clicked() {
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

                ui.add_space(8.0);
                // 4px Neo-Brutalist Divider below Tab Bar
                let (rect_div_tabs, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 4.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect_div_tabs, Rounding::ZERO, border_color);
                ui.add_space(8.0);

                // Scrolling Terminal Output Area
                let available_height = (ui.available_height() - 108.0_f32).max(80.0_f32);

                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    ScrollArea::vertical()
                        .max_height(available_height)
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&active_tab.terminal_output)
                                        .font(FontId::monospace(15.0))
                                        .strong()
                                        .color(fg_color),
                                )
                                .wrap_mode(egui::TextWrapMode::Wrap),
                            );
                        });
                }

                ui.add_space(8.0);
                // 4px Neo-Brutalist Divider above Input Field
                let (rect_div2, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 4.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect_div2, Rounding::ZERO, border_color);
                ui.add_space(8.0);

                if let Some(active_tab) = self.tabs.get_mut(self.active_tab) {
                    // Input Field with Solid Block Cursor █ Matching Theme Text Color
                    egui::Frame::none()
                        .fill(bg_color)
                        .stroke(Stroke::new(4.0_f32, border_color))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("PS >")
                                        .font(FontId::monospace(15.5))
                                        .strong()
                                        .color(fg_color),
                                );

                                let byte_pos = active_tab
                                    .input_text
                                    .char_indices()
                                    .nth(active_tab.cursor_pos)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(active_tab.input_text.len());

                                let before = &active_tab.input_text[..byte_pos];
                                let after = &active_tab.input_text[byte_pos..];

                                // Solid block cursor █ in current theme's text color
                                let input_line = format!("{before}█{after}");

                                ui.label(
                                    RichText::new(input_line)
                                        .font(FontId::monospace(15.5))
                                        .strong()
                                        .color(fg_color),
                                );
                            });
                        });
                }

                ui.add_space(8.0);
                // 4px Neo-Brutalist Divider above Bottom Status Strip
                let (rect_div3, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 4.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect_div3, Rounding::ZERO, border_color);
                ui.add_space(8.0);

                // Bottom Status Strip (System & Git Telemetry)
                ui.horizontal(|ui| {
                    // 1. Directory Badge
                    let dir_name = get_current_dir_name();
                    render_status_badge(
                        ui,
                        &format!("DIR: {dir_name}"),
                        false,
                        bg_color,
                        fg_color,
                        border_color,
                        self.dark_mode,
                    );
                    ui.add_space(4.0);

                    // 2. Git Branch Badge
                    if let Some(branch) = get_git_branch_info() {
                        render_status_badge(
                            ui,
                            &format!("GIT: {branch}"),
                            true,
                            bg_color,
                            fg_color,
                            border_color,
                            self.dark_mode,
                        );
                        ui.add_space(4.0);
                    }

                    // 3. System Memory Badge
                    if let Some((load, used_gb, total_gb)) = get_system_memory_info() {
                        render_status_badge(
                            ui,
                            &format!("RAM: {used_gb:.1}G/{total_gb:.0}G ({load}%)"),
                            false,
                            bg_color,
                            fg_color,
                            border_color,
                            self.dark_mode,
                        );
                        ui.add_space(4.0);
                    }

                    // 4. Tab Status Badge
                    let total_tabs = self.tabs.len();
                    let current_tab_num = self.active_tab + 1;
                    render_status_badge(
                        ui,
                        &format!("TAB: {current_tab_num}/{total_tabs}"),
                        false,
                        bg_color,
                        fg_color,
                        border_color,
                        self.dark_mode,
                    );
                    ui.add_space(4.0);

                    // 5. Active Tab Execution State Badge
                    let is_busy = self
                        .tabs
                        .get(self.active_tab)
                        .map(|t| t.last_activity.elapsed() < Duration::from_millis(800))
                        .unwrap_or(false);
                    let (state_label, state_invert) = if is_busy {
                        ("STATE: BUSY ●", true)
                    } else {
                        ("STATE: IDLE", false)
                    };
                    render_status_badge(
                        ui,
                        state_label,
                        state_invert,
                        bg_color,
                        fg_color,
                        border_color,
                        self.dark_mode,
                    );
                    ui.add_space(4.0);

                    // 6. Uptime Badge
                    let up_secs = self.start_time.elapsed().as_secs();
                    let up_mins = up_secs / 60;
                    let up_rem_secs = up_secs % 60;
                    render_status_badge(
                        ui,
                        &format!("UP: {up_mins}m {up_rem_secs:02}s"),
                        false,
                        bg_color,
                        fg_color,
                        border_color,
                        self.dark_mode,
                    );

                    // Right-aligned hotkey pill
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        render_status_badge(
                            ui,
                            "WIN+Q // CTRL+ALT+T",
                            false,
                            bg_color,
                            fg_color,
                            border_color,
                            self.dark_mode,
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
}

