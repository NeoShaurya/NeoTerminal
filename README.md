# NeoTerminal ⚡

> A high-performance native desktop terminal emulator built from scratch in **Rust**, powered by **Windows ConPTY** with an uncompromising, high-contrast **Neo-Brutalist design**.

---

## 📸 Screenshots

| Dark Mode (Multi-Tabs & Status Strip) | Light Mode (High Contrast) |
|:---:|:---:|
| ![Dark Mode](assets/screenshots/dark_mode.png) | ![Light Mode](assets/screenshots/light_mode.png) |

<p align="center">
  <b>Built-in <code>neofetch</code> Easter Egg with Custom ASCII Art & System Specs</b><br><br>
  <img src="assets/screenshots/neofetch.png" alt="NeoTerminal Neofetch" width="95%"/>
</p>

---

## 🛠️ Skills & Technologies Used

| Category | Details |
| :--- | :--- |
| **Core Languages** | **Rust** (100% native codebase) |
| **GUI Framework** | **eframe / egui** (Immediate-mode native GUI, Consolas Bold typography) |
| **Subsystems & APIs** | **Windows ConPTY** (`portable-pty`), **Win32 API** (`kernel32`), Low-Level Keyboard Hooks (`WH_KEYBOARD_LL`) |
| **Architecture** | Multithreaded reader channels (`std::sync::mpsc`), Mutex-guarded PTY writers, Sub-millisecond rendering |
| **Design System** | **Gumroad Neo-Brutalism** (Matching [neoshaurya.com](https://neoshaurya.com): `#ffc900` Cyber Yellow, `#ff90e8` Neon Pink, `#23a094` Mint Teal, `#ff5c00` Flame Orange, uniform 4px borders, hard zero-blur offset drop shadows, tactile mechanical button depression) |
| **Tooling & Build** | **Cargo**, **Rustc**, **MinGW-w64 (w64devkit)**, **PowerShell**, **Git & GitHub** |

---

## ✨ Features

- **Gumroad Neo-Brutalist Aesthetics (Matching [neoshaurya.com](https://neoshaurya.com))**:
  - **Color Palette Tokens**:
    - **Cyber Yellow (`#ffc900`)**: Signature badge accent for the `⚡ NEOTERMINAL` header badge, active tab card, `PS >` prompt dock badge, and active telemetry pills.
    - **Neon Pink (`#ff90e8`)**: Used for the reverse-i-search modal title, Git branch badge, and the electric block cursor **`█`**.
    - **Mint Teal (`#23a094`)**: Live status indicators (`[ ● ONLINE ]`, `[ STATE: IDLE ]`), and the `[ + NEW TAB ]` button.
    - **Flame Orange (`#ff5c00`)**: Real-time high load / busy indicator (`[ STATE: BUSY ● ]`, RAM warning > 80%, close button hover highlight).
    - **Pitch Black (`#000000`) & Crisp White (`#ffffff`)**: High-contrast brutalist surfaces and borders.
  - **Hard Offset Drop Shadows (0 Blur)**:
    - Solid geometric shadow blocks (`4px 4px 0px 0px`, `3px 3px 0px 0px`, and `5px 5px 0px 0px`).
    - Dark mode features high-contrast white and neon accent drop shadows; light mode features pitch-black solid shadows.
  - **Tactile Mechanical Interactions**:
    - **Hover Lift**: Buttons and interactive tiles translate by `(-1px, -1px)` with an expanded drop shadow.
    - **Click-Down Depression**: Buttons and tabs physically depress by `(+3px, +3px)` into the hard shadow on mouse press.
  - **Theme Toggle**:
    - Instant one-click tactile toggle between Dark Mode and Light Mode with zero corner radius (`Rounding::ZERO`) everywhere.
- **Brutalist Multi-Tabs**:
  - Spawn and manage multiple independent ConPTY shell sessions simultaneously.
  - Active tab rendered in Gumroad Cyber Yellow (`#ffc900`) with high-contrast pitch-black text, 4px border, and hard drop shadow.
  - Unread activity indicator (`● NEW`) in Neon Pink (`#ff90e8`) for background tabs with fresh output.
  - Fast tab creation with `[ + NEW TAB ]` in Mint Teal (`#23a094`), individual close buttons (`×`) that turn Flame Orange on hover.
  - Keyboard shortcuts:
    - **`Ctrl + T`**: Open new tab.
    - **`Ctrl + W`**: Close active tab.
    - **`Ctrl + Tab`** / **`Ctrl + Shift + Tab`**: Cycle through tabs.
    - **`Ctrl + 1`** to **`Ctrl + 9`**: Jump directly to tab 1 through 9.
  - Interactive tab renaming: type **`title <name>`** or **`rename <name>`** (e.g., `title Scraper`) to rename the active tab.
- **Brutalist Viewport & Command Dock**:
  - Terminal output enclosed in a brutalist viewport frame with 4px border and 4px offset shadow.
  - Command input dock with solid `[ PS > ]` Cyber Yellow badge and electric pink block cursor **`█`**.
- **Bottom Status Strip (System & Git Telemetry Cards)**:
  - Six modular brutalist cards matching the portfolio card style on [neoshaurya.com](https://neoshaurya.com):
  - **`DIR`**: Current working directory in Cyber Yellow (`#ffc900`).
  - **`GIT`**: Auto-detected active Git branch in Neon Pink (`#ff90e8`).
  - **`RAM`**: Live system memory usage and RAM load percentage (auto-highlights in Flame Orange `#ff5c00` if load exceeds 80%).
  - **`TAB`**: Active and total tab count indicator (`[ TAB: 1/2 ]`).
  - **`STATE`**: Real-time execution status (`[ STATE: IDLE ]` in Mint Teal vs. `[ STATE: BUSY ● ]` in Flame Orange).
  - **`UP`**: Session uptime counter (`[ UP: 12m 45s ]`).
  - Right-aligned shortcut guide (`[ CTRL+R: SEARCH // WIN+Q // CTRL+T ]`).
- **Interactive Fuzzy History Search (`Ctrl + R`)**:
  - Full reverse-i-search powered by a deterministic fuzzy subsequence and prefix ranking engine.
  - Dedicated brutalist overlay dock with 5px drop shadow and Neon Pink title.
  - Reads and auto-syncs persistent Windows PowerShell command history (`ConsoleHost_history.txt`).
  - Live search dock with 4px brutalist border:
    - **`Ctrl + R`**: Toggle interactive history search.
    - **`Type`**: Instant live fuzzy filtering across entire command history.
    - **`Up` / `Down` Arrow Keys** or repeated **`Ctrl + R`**: Cycle through ranked matches with Cyber Yellow highlight selection.
    - **`Enter` / `Tab`**: Accept match and load directly into prompt for editing or execution.
    - **`Esc` / `Ctrl + C`**: Cancel search and return to previous input.
    - **Mouse Interactivity**: Click any match in the results list to load it instantly.
  - Reads and auto-syncs persistent Windows PowerShell command history (`ConsoleHost_history.txt`).
  - Live search dock with 4px brutalist border:
    - **`Ctrl + R`**: Toggle interactive history search.
    - **`Type`**: Instant live fuzzy filtering across entire command history.
    - **`Up` / `Down` Arrow Keys** or repeated **`Ctrl + R`**: Cycle through ranked matches with real-time highlighted selection.
    - **`Enter` / `Tab`**: Accept match and load directly into prompt for editing or execution.
    - **`Esc` / `Ctrl + C`**: Cancel search and return to previous input.
    - **Mouse Interactivity**: Click any match in the results list to load it instantly.
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
│       ├── dark_mode.png          # Dark Mode (Tabs & Status Strip)
│       ├── light_mode.png         # Light Mode Preview
│       └── neofetch.png           # Custom Neofetch Easter Egg
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

The compiled standalone executable is located at:
```
./NeoTerminal.exe
```
(or in `target/release/NeoTerminal.exe` after building)

---

## 📄 License

MIT © [Shaurya Chandra](https://neoshaurya.com)
