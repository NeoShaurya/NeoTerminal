# NeoTerminal ⚡

A high-performance, native desktop GUI terminal emulator built in **Rust**, powered by **eframe (egui)** and **portable-pty (ConPTY)** with a bold **Neo-Brutalist** aesthetic.

---

## 📸 Screenshots

| Dark Mode | Light Mode |
|:---:|:---:|
| ![Dark Mode](assets/screenshots/dark_mode.png) | ![Light Mode](assets/screenshots/light_mode.png) |

---

## ✨ Features

- **Neo-Brutalist High-Contrast Aesthetics**:
  - **Dark Mode**: Pure black background (`#000000`), pure white text (`#ffffff`), and a **4px solid white border**.
  - **Light Mode**: Pure white background (`#ffffff`), pure black text (`#000000`), and a **4px solid black border**.
  - Sharp corners, thick geometric dividers, and bold brutalist controls.
  - Seamless one-click instant theme toggle (`[● DARK MODE]` / `[○ LIGHT MODE]`) in the top-right corner.
- **PTY Shell Backend**:
  - Spawns the default system shell (`PowerShell`) via Windows ConPTY using `portable-pty`.
  - Non-blocking background worker thread using `std::sync::mpsc` streaming shell output into the GUI.
  - Reactive GUI repainting on incoming data with `ctx.request_repaint()`.
  - ANSI escape code parser and cleaner for clean, crisp typography.
- **Custom Input Field & Solid Block Cursor**:
  - High-contrast brutalist command dock at the bottom.
  - Custom solid block cursor **`█`** matching the active theme's foreground color.
  - Keystroke navigation: Left/Right caret movement, Backspace, Delete, Home, End.
  - Command history navigation (Up / Down arrow keys).
  - Terminal shortcuts (`Ctrl+C` interrupt signal, `Ctrl+L` clear screen).
- **Easter Eggs**:
  - **`neofetch`**: Intercepted before shell execution to display custom ASCII art for **NeoShaurya** with system statistics and brutalist color swatches.
  - **`sudo contact`**: Intercepted and blocked from execution. Prints `Access Granted. Redirecting to neoshaurya.com/contact...` and opens `https://neoshaurya.com/contact` in the browser.

---

## ⚡ Quick Launch Shortcuts

NeoTerminal integrates directly into Windows as a first-class desktop application:

1. **Global Shortcut Key**:
   - Press **`Ctrl + Alt + T`** anywhere on your PC to launch NeoTerminal instantly.
2. **Windows Run Dialog**:
   - Press **`Win + R`**, type **`neo`**, and hit **Enter**.
3. **Windows Search**:
   - Press **`Win + Q`**, type **`neo`**, and hit **Enter**.

---

## 🛠️ Project Structure

```
NeoTerminal/
├── assets/
│   ├── NeoTerminal.ico            # Windows Application Icon
│   └── screenshots/
│       ├── dark_mode.png          # Dark Mode Preview
│       └── light_mode.png         # Light Mode Preview
├── .cargo/
│   └── config.toml                # Linker & build configuration
├── src/
│   ├── main.rs                    # Core GUI, PTY backend & theme engine
│   └── bin/
│       └── neohotkey.rs           # Global hotkey daemon
├── Cargo.toml                     # Dependencies (eframe, portable-pty)
├── Cargo.lock
└── README.md
```

---

## 🚀 Building from Source

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (1.75+)
- Windows 10 / 11

### Build & Run

Clone the repository and build with Cargo:

```powershell
git clone https://github.com/NeoShaurya/NeoTerminal.git
cd NeoTerminal

# Run in development mode
cargo run

# Build optimized release binary
cargo build --release
```

The compiled standalone executable will be located at:
```
target/release/NeoTerminal.exe
```

---

## 📄 License

MIT © [Shaurya Chandra](https://neoshaurya.com)
