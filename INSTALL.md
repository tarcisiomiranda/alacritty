# Installing Termust

For the Linux binary package, curl installer and local mise install, use the
[installation guide](docs/install.md).

## Build from source

Install stable Rust and the operating system dependencies listed in the
[Alacritty build guide](https://github.com/alacritty/alacritty/blob/master/INSTALL.md#dependencies).
Termust uses the same native rendering and font libraries.

```sh
mise run build:release
mise run install
```

The executable is named `termust`. The main application source directory is
still `alacritty/`, and internal libraries retain their upstream crate names.
For a Cargo-only install from this checkout:

```sh
mise exec -- cargo install --path alacritty --locked
```

The Linux installer also sets up the Termust launcher, icon, shell completions
and compatible Alacritty terminfo entries. It installs into `~/.local` by default.

## Other platforms

Windows builds produce `termust.exe`. The macOS Makefile produces `Termust.app`
and `Termust.dmg`, using the Termust platform icons. These platform packages
have not been tested on their native operating systems in this fork's branding update.

OpenGL ES 2.0 or later is required. Windows requires ConPTY support.
