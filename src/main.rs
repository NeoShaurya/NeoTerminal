#![windows_subsystem = "windows"]

use eframe::egui::{
    self, Align, Color32, FontId, Key, Layout, Margin, RichText, Rounding, ScrollArea, Stroke,
    Vec2, Visuals,
};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn SetErrorMode(uMode: u32) -> u32;
}

/// Filter ANSI escape sequences and normalize control characters
fn clean_terminal_output(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next();
                    // consume parameter and intermediate bytes until terminating char
                    while let Some(&b) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&b) {
                            break;
                        }
                    }
                    continue;
                } else if next == ']' {
                    // OSC sequence: consume until BEL (\x07) or ST (\x1b\)
                    chars.next();
                    while let Some(b) = chars.next() {
                        if b == '\x07' {
                            break;
                        }
                        if b == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                    continue;
                }
            }
        } else if c == '\r' {
            // Drop carriage return (\r); standard \n handles line wrapping
            continue;
        } else if c == '\x08' {
            // Backspace character
            out.pop();
        } else {
            out.push(c);
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

struct NeoTerminalApp {
    dark_mode: bool,
    terminal_output: String,
    input_text: String,
    cursor_pos: usize,
    history: Vec<String>,
    history_idx: Option<usize>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    rx: Receiver<String>,
}

impl NeoTerminalApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, Box<dyn std::error::Error>> {
        // Backend: Use portable-pty to spawn the default system shell
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // On Windows, invoke PowerShell through the native console host to ensure clean DLL initialization
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = CommandBuilder::new("cmd.exe");
            c.args(["/k", "powershell.exe -NoLogo"]);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let shell_cmd = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
            CommandBuilder::new(shell_cmd)
        };

        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }

        let _child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let writer = Arc::new(Mutex::new(writer));

        // Background thread using std::sync::mpsc to read shell output and send it to the GUI
        let (tx, rx) = channel::<String>();
        let egui_ctx = cc.egui_ctx.clone();

        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // Shell exited / EOF
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        if tx.send(text).is_err() {
                            break;
                        }
                        egui_ctx.request_repaint();
                    }
                    Err(_) => break,
                }
            }
        });

        let app = Self {
            dark_mode: true,
            terminal_output: String::new(),
            input_text: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_idx: None,
            writer,
            rx,
        };

        app.apply_theme(&cc.egui_ctx);
        Ok(app)
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

    fn submit_command(&mut self, ctx: &egui::Context) {
        let cmd = std::mem::take(&mut self.input_text);
        self.cursor_pos = 0;
        self.history_idx = None;

        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            // Send empty newline to shell
            if let Ok(mut w) = self.writer.lock() {
                let _ = writeln!(w);
                let _ = w.flush();
            }
            return;
        }

        // Store into command history
        self.history.push(cmd.clone());

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

        // Send normal command to shell thread
        if let Ok(mut w) = self.writer.lock() {
            let _ = writeln!(w, "{}", cmd);
            let _ = w.flush();
        }
    }
}

impl eframe::App for NeoTerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Read any incoming shell output
        while let Ok(msg) = self.rx.try_recv() {
            let cleaned = clean_terminal_output(&msg);
            self.terminal_output.push_str(&cleaned);

            // Cap output to prevent unbounded memory growth
            if self.terminal_output.len() > 100_000 {
                let excess = self.terminal_output.len() - 100_000;
                self.terminal_output.drain(..excess);
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
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            if !ch.is_control() {
                                let byte_pos = self
                                    .input_text
                                    .char_indices()
                                    .nth(self.cursor_pos)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(self.input_text.len());
                                self.input_text.insert(byte_pos, ch);
                                self.cursor_pos += 1;
                            }
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => match key {
                        Key::Enter => {
                            self.submit_command(ctx);
                        }
                        Key::Backspace => {
                            if self.cursor_pos > 0 && !self.input_text.is_empty() {
                                self.cursor_pos -= 1;
                                let byte_pos = self
                                    .input_text
                                    .char_indices()
                                    .nth(self.cursor_pos)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(0);
                                self.input_text.remove(byte_pos);
                            }
                        }
                        Key::Delete => {
                            if self.cursor_pos < self.input_text.chars().count() {
                                let byte_pos = self
                                    .input_text
                                    .char_indices()
                                    .nth(self.cursor_pos)
                                    .map(|(idx, _)| idx)
                                    .unwrap_or(self.input_text.len());
                                self.input_text.remove(byte_pos);
                            }
                        }
                        Key::ArrowLeft => {
                            if self.cursor_pos > 0 {
                                self.cursor_pos -= 1;
                            }
                        }
                        Key::ArrowRight => {
                            if self.cursor_pos < self.input_text.chars().count() {
                                self.cursor_pos += 1;
                            }
                        }
                        Key::Home => {
                            self.cursor_pos = 0;
                        }
                        Key::End => {
                            self.cursor_pos = self.input_text.chars().count();
                        }
                        Key::ArrowUp => {
                            if !self.history.is_empty() {
                                let next_idx = match self.history_idx {
                                    None => self.history.len() - 1,
                                    Some(idx) if idx > 0 => idx - 1,
                                    Some(idx) => idx,
                                };
                                self.history_idx = Some(next_idx);
                                self.input_text = self.history[next_idx].clone();
                                self.cursor_pos = self.input_text.chars().count();
                            }
                        }
                        Key::ArrowDown => {
                            if let Some(idx) = self.history_idx {
                                if idx + 1 < self.history.len() {
                                    self.history_idx = Some(idx + 1);
                                    self.input_text = self.history[idx + 1].clone();
                                    self.cursor_pos = self.input_text.chars().count();
                                } else {
                                    self.history_idx = None;
                                    self.input_text.clear();
                                    self.cursor_pos = 0;
                                }
                            }
                        }
                        Key::C if modifiers.ctrl => {
                            self.terminal_output
                                .push_str(&format!("PS > {}^C\n", self.input_text));
                            self.input_text.clear();
                            self.cursor_pos = 0;
                            if let Ok(mut w) = self.writer.lock() {
                                let _ = w.write_all(b"\x03");
                                let _ = w.flush();
                            }
                        }
                        Key::L if modifiers.ctrl => {
                            self.terminal_output.clear();
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        });

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
                            .font(FontId::monospace(18.0))
                            .strong()
                            .color(fg_color),
                    );
                    ui.label(
                        RichText::new("// POWERSHELL CONPTY")
                            .font(FontId::monospace(12.0))
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
                        .stroke(Stroke::new(2.0_f32, border_color))
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

                // Scrolling Terminal Output Area
                let available_height = ui.available_height() - 56.0;

                ScrollArea::vertical()
                    .max_height(available_height)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.terminal_output)
                                    .font(FontId::monospace(14.0))
                                    .color(fg_color),
                            )
                            .wrap_mode(egui::TextWrapMode::Wrap),
                        );
                    });

                ui.add_space(8.0);
                // 4px Neo-Brutalist Divider above Input Field
                let (rect_div2, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 4.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect_div2, Rounding::ZERO, border_color);
                ui.add_space(8.0);

                // Input Field with Solid Block Cursor █ Matching Theme Text Color
                egui::Frame::none()
                    .fill(bg_color)
                    .stroke(Stroke::new(2.0_f32, border_color))
                    .inner_margin(Margin::symmetric(10.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("PS >")
                                    .font(FontId::monospace(15.0))
                                    .strong()
                                    .color(fg_color),
                            );

                            let byte_pos = self
                                .input_text
                                .char_indices()
                                .nth(self.cursor_pos)
                                .map(|(idx, _)| idx)
                                .unwrap_or(self.input_text.len());

                            let before = &self.input_text[..byte_pos];
                            let after = &self.input_text[byte_pos..];

                            // Solid block cursor █ in current theme's text color
                            let input_line = format!("{before}█{after}");

                            ui.label(
                                RichText::new(input_line)
                                    .font(FontId::monospace(15.0))
                                    .strong()
                                    .color(fg_color),
                            );
                        });
                    });
            });
    }
}

fn main() -> eframe::Result {
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
            let app = NeoTerminalApp::new(cc).expect("Failed to initialize NeoTerminal backend");
            Ok(Box::new(app))
        }),
    )
}
