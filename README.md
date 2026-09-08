# 🦈 SharkManager

**Thunar-like file manager built with Rust + GTK 4**

SharkManager is a fast, lightweight, Thunar-inspired file manager for Linux, written in pure Rust using `gtk4-rs`. It aims to be familiar to Xfce/Thunar users while leveraging modern GTK4 and safe Rust.

![License](https://img.shields.io/badge/license-GPL--3.0-blue)
![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)
![GTK4](https://img.shields.io/badge/GTK-4.22-green)

---

## ✨ Features

### 📁 Thunar-like UX
- **Sidebar** — Places (Home, Documents, Downloads, Pictures, Videos, Desktop, Music, Trash, File System), **Bookmarks** (empty by default with plain folder icons like Nautilus; icon-only, right-click to rename/remove/add; stored in `~/.config/sharkmanager/bookmarks`), and **Devices** (auto-detected `/media`, `/mnt`, `/run/media`)
- **Breadcrumbs** — clickable path bar in the header center (`/` → `home` → `matko` → …) with active segment highlight, plus **Ctrl+L** to edit raw path
- **HeaderBar** — Nautilus-style: sidebar toggle, Back / Forward, centered search (toggle, `Ctrl+F`), view toggle, sort menu, new folder
- **Status pill** — floating item/selection count overlay (Nautilus-style)
- **Tabs** — Nautilus-style `AdwTabBar`/`AdwTabView` strip under the header (drag-reorder, middle-click close, `Ctrl+T`/`Ctrl+W`)

### 🗂 Two View Modes
- **Icon View** (`FlowBox`) — 48px icons, thumbnails for images via `gdk-pixbuf` (96px scaled), 2-line wrapped labels, hover/selection styling
- **List View** (`ListBox` table) — columns: Name (icon+name), Size (`human_size`), Modified (`chrono`), Type (mime), hidden/symlink dimming

### 🔍 Search & Filtering
- Instant substring filter (case-insensitive) via header `SearchEntry`
- Show hidden files toggle (`Ctrl+H` + button)

### ⚡ Navigation
- Full **history** (Back `Alt+Left`, Forward `Alt+Right`, Up `Alt+Up`, Home)
- Double-click to open dirs/files
- Path entry (`Ctrl+L`, `Escape` to cancel, `Enter` to navigate)
- Selection tracking (FlowBox + ListBox multiple, status update)

### 🛠 File Operations
- **New Folder** (`Ctrl+Shift+N` / header button) — dialog with duplicate check
- **New File** (`Ctrl+N`)
- **Rename** (`F2` / context menu) — with exists check
- **Trash** (`Delete` / context menu) — via `trash` crate (moves to `~/.local/share/Trash`)
- **Delete Permanently** (`Shift+Delete` / context menu) — warning dialog
- **Cut / Copy / Paste** (`Ctrl+X/C/V` + context menu) — clipboard in-memory, `copy_recursive` for dirs, `unique_name` to avoid collisions
- **Open** (double-click, Enter, context menu) — `open::that` → `xdg-open`
- **Open in Terminal** (background menu) — tries `xdg-terminal-exec`, `kitty`, `konsole`, `gnome-terminal`
- **Properties** — Name, Path, Type, Size, Modified, Permissions (octal), Symlink, Item count for dirs
- **Context menus** — per-file (`Open`, `Cut/Copy/Rename/Trash/Delete/Properties`) and background (`New Folder/File/Paste/Terminal/Refresh`)

### ⌨️ Keyboard Shortcuts
| Shortcut | Action |
|---|---|
| `Alt+Left` / `Alt+Right` | Back / Forward |
| `Alt+Up` / `Backspace` | Up |
| `Ctrl+L` | Edit location |
| `Ctrl+H` | Toggle hidden |
| `F5` | Refresh |
| `F2` | Rename |
| `Delete` | Trash |
| `Shift+Delete` | Permanent delete |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / Cut / Paste |
| `Ctrl+A` | Select all |
| `Ctrl+N` | New file |
| `Ctrl+Shift+N` | New folder |

---

## 🖥 Screenshots

> Run the app to see: sidebar on left (210px), Nautilus-style header (search center) with tab strip below, FlowBox icon grid or ListBox table, floating status pill.

---

## 📦 Installation

### NixOS (recommended)

```bash
# Clone
git clone https://github.com/example/sharkmanager
cd sharkmanager

# Dev shell (provides gtk4, pkg-config, rust, etc.)
nix-shell          # or nix develop

# Build & run
cargo run --release
# Or via flake
nix run
```

### Non-Nix (Ubuntu, Fedora, Arch)

Install GTK4 dev packages:

```bash
# Ubuntu/Debian
sudo apt install libgtk-4-dev libglib2.0-dev libcairo2-dev libpango1.0-dev libgdk-pixbuf-2.0-dev libgraphene-1.0-dev pkg-config

# Fedora
sudo dnf install gtk4-devel glib2-devel cairo-devel pango-devel gdk-pixbuf2-devel graphene-devel pkgconf

# Arch
sudo pacman -S gtk4 glib2 cairo pango gdk-pixbuf2 graphene pkgconf
```

Then:

```bash
cargo build --release
./target/release/sharkmanager
# or
cargo run --release
```

### Cargo install (once published)

```bash
cargo install sharkmanager
```

---

## 🧊 NixOS / Flakes integration

SharkManager ships **two** ready-made Nix integrations.

### Option A — Add to your system flake (recommended)

A template `nixos-flake.nix` is provided. As **root**, copy it to `/etc/nixos/flake.nix`
(it references your existing `configuration.nix` + `hardware-configuration.nix` and just adds
`sharkmanager` to `environment.systemPackages`):

```bash
sudo cp nixos-flake.nix /etc/nixos/flake.nix
sudo nixos-rebuild switch --flake /etc/nixos#nixos
```

> The template pins `nixpkgs` to `nixos-unstable`. To match your current 26.11 system
> exactly, change the `nixpkgs.url` to `github:NixOS/nixpkgs/nixos-26.11`.

### Option B — Channel-based config (no flake conversion)

If you'd rather not convert your whole system to flakes, use `default.nix`:

```nix
# in configuration.nix
environment.systemPackages = [
  (pkgs.callPackage /mnt/ssd/My-Files/Projects/sharkmanager/default.nix {})
];
```

This builds with **your** system's `nixpkgs`, so there's zero version drift.

### Option C — Just run it (standalone flake)

```bash
nix run                 # builds + launches
nix build              # produces ./result/bin/sharkmanager
```

The package is wrapped with `wrapGAppsHook4` so GTK4 themes, icons and GSettings schemas work.

---

## 🚀 Usage

```bash
# Inside nix-shell / after build
cargo run
# or directly
./target/release/sharkmanager

# The window opens at $HOME. Navigate, double-click, right-click for menus.
```

`.desktop` file is provided at `data/sharkmanager.desktop` for app launcher integration:

```bash
# Install locally
mkdir -p ~/.local/share/applications
cp data/sharkmanager.desktop ~/.local/share/applications/
update-desktop-database ~/.local/share/applications
```

---

## 🏗 Project Structure

```
sharkmanager/
├── Cargo.toml          # gtk4 0.9, gio, gdk4, pango, chrono, mime_guess, dirs, trash, open
├── Cargo.lock
├── flake.nix           # Nix devShell + package (rustPlatform.buildRustPackage, wrapGAppsHook4)
├── nixos-flake.nix     # system flake template (copy to /etc/nixos/flake.nix)
├── default.nix         # classic channel-based package for callPackage
├── shell.nix           # legacy nix-shell
├── src/
│   ├── main.rs         # 1700+ lines: AppState, build_ui, navigation, breadcrumbs, views, menus, dialogs
│   ├── file_entry.rs   # FileEntry, load_directory (dirs-first sorting), human_size, format_time, mime/icon guess
│   └── operations.rs   # trash, delete, rename, create, copy_recursive, unique_name, open
└── data/
    └── sharkmanager.desktop
```

Key modules (`src/main.rs:1`):

- `AppState` (`src/main.rs:25`) — current path, history stack, show_hidden, view_mode, search, clipboard, selected set
- `navigate_to` (`src/main.rs:886`) — canonicalize + history truncate (max 100)
- `rebuild_breadcrumbs` (`src/main.rs:905`) — Button row with `›` separators
- `build_icon_tile` (`src/main.rs:960`) — image thumbnail vs symbolic icon
- `build_list_row` (`src/main.rs:1002`) — hbox with 4 columns
- `build_sidebar` (`src/main.rs:1075`) — Places/Bookmarks/Devices
- Dialogs: `prompt_new_folder` (`src/main.rs:1450`), `prompt_rename` (`src/main.rs:1550`), `confirm_and_trash` (`src/main.rs:1600`), `show_properties` (`src/main.rs:1660`)

---

## 🎨 Theming

Uses system GTK4 theme + CSS provider (`src/main.rs:45`):

```css
.shark-sidebar, .shark-breadcrumb button.active, .shark-file-card:hover, .shark-list-row.selected, etc.
```

Respects `GTK_THEME` and `gsettings-desktop-schemas`.

---

## 🔧 Development

```bash
nix-shell --run "cargo check"
nix-shell --run "cargo clippy"
nix-shell --run "cargo fmt"
nix-shell --run "cargo run"   # needs $DISPLAY / Wayland
```

Environment on NixOS sets:

```bash
XDG_DATA_DIRS=$XDG_DATA_DIRS:/nix/store/.../share/gsettings-schemas/...
```

See `flake.nix:22`, `shell.nix:10`.

---

## 🗺 Roadmap (Thunar parity)

- [ ] Drag & drop (move/copy)
- [ ] Bulk rename
- [ ] Custom actions (`uca.xml` like Thunar)
- [ ] Split view / dual pane
- [ ] Tree view in sidebar
- [ ] Volume monitor via `g_volume_monitor` (USB automount)
- [ ] Thumbnail cache (`~/.cache/thumbnails`)
- [ ] Trash view (`trash:///` via gio)
- [ ] Search recursion
- [ ] Migrate deprecated `Dialog/MessageDialog` → `AlertDialog`

---

## 📄 License

GPL-3.0-or-later — like Thunar. See `LICENSE` (to be added).

---

Made with 🦈 and `gtk4-rs`.
