# Install Termust

Termust is a terminal based on Alacritty. It has tabs, split panes,
saved sessions, menus and settings in English and Brazilian Portuguese.

Use one command on Linux x86_64 and macOS Apple Silicon:

```sh
curl -fsSL https://f005.backblazeb2.com/file/termust/install.sh | bash
```

The installer detects the platform and downloads a native binary.

## In-app updates

Install this release once with the command above and start the new Termust process.
On Linux x86_64 and macOS Apple Silicon, Termust checks for updates in the background
once a day. When an update is available, a message points you to **Settings → Terminal**.
You can also choose **Check now** there at any time. Select **Install update** to
download the displayed build. The installer verifies its SHA256 and keeps your open
windows and shells running.

Start a new Termust process to use the installed build. On Linux, open **New Window**
from the desktop launcher. On macOS, you can run `open -n ~/Applications/Termust.app`
to open it alongside your current session.

To turn off automatic checks, clear the checkbox under **Settings → Terminal** and save,
or set `check_updates = false` in the `[general]` section of your configuration.
Installs from Cargo or a system package manager use their original update method.

## macOS Apple Silicon

Open `~/Applications/Termust.app` from Finder after installation.

The installer verifies the downloaded archive and app signature and installs it
without sudo or a Rust toolchain. Re-running the command updates the verified
Termust app through an atomic launcher link and preserves the previous bundle
at `~/Applications/.termust/previous.app`. Use `open -n ~/Applications/Termust.app`
to open the updated app alongside existing windows. Unrecognized apps and links
outside the managed releases are rejected. The app is ad hoc signed. On macOS,
`--prefix` selects the app directory, and `--check` verifies the app.

Termust starts the shell assigned to your macOS account as an interactive login
shell. For Zsh, this includes `.zprofile` and `.zshrc`, where your prompt theme can
be configured. Set `[terminal.shell]` in your Termust configuration to override
the account shell.

To build and publish the native binary from an Apple Silicon Mac, install Xcode
Command Line Tools, a native Rust toolchain and the B2 CLI, then authorize B2
and run:

```sh
cd ~/srv/termust
bash scripts/build-macos-release.sh --source-dir "$PWD"
```

The release script compiles and signs the app, uploads a checksum-addressed
archive, verifies installation and updates `latest-macos-arm64.json`. Both
platforms continue using the same `install.sh`; installation requires no Rust.

## Linux installation

The canonical download bucket is `termust`, dedicated to this project and its
releases. Previous download addresses remain available for existing users and
pinned releases. New releases are verified at the canonical location and both
previous locations.

The Linux installer installs `~/.local/bin/termust` and a user desktop entry named
**Termust**. After updating, right-click the Termust launcher and choose **New Window**,
or run `~/.local/bin/termust` to start the installed build. Clicking a running
app in the dock can return you to a window opened before the update.
The system package in `/usr/bin/alacritty` and your existing configuration and
saved sessions are kept. Existing terminal processes continue running.

If the installer finds its own previous **Alacritty Workspace** installation,
it verifies the receipt, executable version and launcher paths before migration.
The old `alacritty` command then opens Termust, and the previous fork launcher
is removed from the application menu. The previous binary,
launcher and receipt are backed up under
`share/doc/termust/legacy-alacritty/VERSION` inside the installation prefix.
Other Alacritty installations and customized launchers are kept. Using
`--no-desktop` skips this migration.

Termust starts new shells with `TERM=xterm-256color`, a widely available terminal
type on SSH servers. Set `TERM` under `[env]` in your Termust configuration to
choose another type.

When `tic` is available, the installer also adds the `alacritty` and
`alacritty-direct` entries to `~/.terminfo` for applications or configurations
that use them. Other user terminfo entries are kept, including when you choose a
different installation prefix.

The public package is for **Linux x86_64 with glibc**. It is built with Rust
1.89 in a Debian 11 container, with a generic x86-64 CPU target. Its measured
minimum glibc version is **2.29**, down from 2.43 in the earlier Fedora build.
It still needs fontconfig, FreeType, xkbcommon and X11 or Wayland with OpenGL.
Bash and Python 3 are required; remote installation also needs curl and CA
certificates. Rust is not needed to install the downloaded package.

The release checks run on Ubuntu 22.04 and 24.04, Debian 12 and 13, and Fedora
43 and 44. Each environment checks installation, repeat installation, PATH,
receipt integrity, the desktop launcher, terminfo and a real graphical terminal
running a shell. These tests use X11 with software rendering; they do not cover
every GPU driver or Wayland desktop. The Linux package targets x86_64 with glibc.
The macOS package targets Apple Silicon.

Before replacement, the installer checks SHA256, package metadata, glibc,
runtime libraries and the new executable. Missing dependencies produce a
suggested package-manager command for the detected distro. It never runs sudo
or installs system packages automatically.

Run the same command again to update. The installer prints the latest package for
your platform, then reports whether it installed an update or found the same build.
The release and its required glibc version
are listed in the public `release.json` file inside its version directory.
The version directory also contains `compatibility.json` with test results.
Publishing is blocked if that report does not match the package and installer.

The installer compares application versions and, when available, build timestamps.
It refuses older builds unless you pass `--allow-downgrade`. An older package
without a timestamp also needs that option when the installed build has one.
An identical release repairs integration files without discarding the previous
rollback copy. Installs into the same prefix are locked to prevent overlap.

To choose a user prefix or an earlier Termust version:

```sh
curl -fsSL https://f005.backblazeb2.com/file/termust/install.sh | bash -s -- --prefix "$HOME/.local"
curl -fsSL https://f005.backblazeb2.com/file/termust/install.sh | bash -s -- --version VERSION
```

For a release from before the Termust rename, use its original installer at
`releases/VERSION/install.sh` under the same download URL. Its executable and
launcher keep the Alacritty Workspace name.

Run `termust` from your shell or open **Termust** in the application menu.
Ensure `~/.local/bin` is in your PATH before `/usr/bin`. The desktop entry uses
the full installed path, so it does not depend on the desktop session PATH.

## Check and restore an installation

Check which executable the shell and desktop will use:

```sh
curl -fsSL https://f005.backblazeb2.com/file/termust/install.sh | bash -s -- --check
```

The check does not change files. It reports other Termust copies, PATH priority,
receipt mismatches and desktop launcher conflicts. For a custom `--prefix`, add
its `bin` directory first in PATH and its `share` directory to `XDG_DATA_DIRS`.
The installer does not edit shell startup files or delete unknown installations.
After an update, the Linux installer and `--check` report any older process they
can identify for the same installation. Its tabs keep using the old build. The
printed launch command starts a separate process from the installed executable.

Before replacing an existing executable, the installer saves one previous copy
of the binary and managed integration files under `share/termust/rollback` in
the prefix. Its manifest includes checksums. Restore it without a download:

```sh
bash ~/.local/share/doc/termust/install.sh --rollback
```

You can also download the current installer and pass `--rollback`. Use the same
`--prefix` as the installation. Rollback validates all backup files and runs the
previous executable before replacing anything. Configuration, sessions and
running terminals are preserved. If saving the receipt fails during an update,
the previous executable and receipt are restored. A fresh install may finish
with an integration warning; fix the reported permissions and rerun it.

## Build and install this checkout

With the Rust compiler and the normal Alacritty build dependencies installed:

```sh
mise trust
mise run install
```

This builds an optimized binary, packages the desktop integration and installs
that local archive with the same checksum and compatibility checks.

## Release commands

```sh
mise run test:install
mise run test:release-tools
mise run package:portable
mise run test:distros
mise run publish
mise run release:verify
mise run install:verify
```

The portable build needs devrig plus Docker or Podman. The build image is pinned
by digest; its cache is separate from the host toolchain. `mise run package`
and `mise run install` still build for the current machine. Public publishing
only accepts a portable package with a matching successful distro report.

Publishing uses the existing B2 CLI account. Credentials stay in the CLI cache;
they are not needed by the downloaded installer. `distribution.toml` holds only
the public bucket name and download URL. Each release has a fixed version path,
checksums and metadata. The publisher verifies public downloads before changing
the latest pointer. Reusing a version with different bytes is rejected.

The optimized binary, license, desktop entry, icon, terminfo and shell completions
are in the archive. Use `/usr/bin/alacritty` if you want to open the Fedora
package directly.
