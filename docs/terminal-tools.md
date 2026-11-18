# Tabs, panes and daily work

Termust adds tools for daily work in one window. It is an independent fork of Alacritty. Alacritty upstream has different goals and does not accept native tabs or
split panes.

I used Terminator before. It was slow on my computer. Terminator uses Python.
I liked the idea of a terminal written in Rust, so I tried Alacritty. I wanted
to keep the tabs and split panes that I use every day. This is why I started
these changes. This is my own experience, not a speed test.

## Tabs

Each tab has its own shell, history and working folder. You can give it a name,
move it with the mouse, make a copy and reopen a closed tab. Long names use an
ellipsis. The close button has space around it.

![Three tabs in one window](images/terminal-tools/tabs.png)

## Split panes

Split a tab to the right or below. Each pane has its own shell. Move focus with
the keyboard or mouse. Drag the border to change the size. You can also make
one pane fill the tab and then go back to the full layout.

![Three shells in one tab](images/terminal-tools/split-panes.png)

## Right-click menu

The menu gives quick access to tabs, panes, saved sessions and settings. It
also works with the keyboard. In a small window, it can scroll.

On Linux, **Settings → Applications** lists installed IDEs/editors and file managers.
Choose a preferred app in each category and save. The menu then offers actions such
as **Open in VS Code**, **Open in Zed**, **Open in Nautilus** or **Open in Dolphin**.
The first two menu actions open the preferred editor and file manager, before
all tab actions. They show the installed application icons, including VS Code and
Nautilus. PNG and SVG icons are loaded through the optional system GdkPixbuf
library; generic editor and folder icons are used when an app icon is unavailable.

They open the current folder of the pane that owns the menu without sending a
command to its shell. Automatic uses the first detected editor and the system
file manager. Applications installed through desktop launchers, including exported
Flatpak launchers, are detected when GIO is available. Reopen settings after
installing an application. A saved app that is no longer installed stays visible
as unavailable; choose another app or Automatic.

The same preferences can be set in the terminal configuration:

```toml
[general]
editor = "code"
file_manager = "nautilus"
```

Supported editor IDs are `auto`, `code`, `zed`, `codium`, `code-insiders`, `kate`
and `sublime`. File manager IDs are `auto`, `nautilus`, `dolphin`, `nemo`, `thunar`,
`caja` and `pcmanfm`. The registered desktop application or executable must be installed.

![Right-click menu](images/terminal-tools/context-menu.png)

## Tab names and saved commands

The tab dialog shows the name, working folder and recent commands. It can
save all tabs. Running the last command again is off by default. Turn it on
only for the tabs where you want this behavior.

![Tab name, working folder and command options](images/terminal-tools/tab-settings.png)

## Close confirmation

Before a tab closes, you can check its name. Press **Y** or **Enter** to close
it. Press **N** or **Escape** to cancel. You can turn this option off in settings.

![Close confirmation](images/terminal-tools/close-confirmation.png)

## Workspaces and sessions

Save a group of tabs and panes with a name, such as **Daily work**. Open it
later with its folders and layout. Session settings also control startup and
recovery.

![Saved workspace](images/terminal-tools/workspaces.png)

![Session settings](images/terminal-tools/settings-sessions.png)

## Settings and themes

Change settings with a live preview. Save keeps the change. Cancel brings back
the old value. Tabs, menus and settings use the terminal colors. Light and dark
themes both work.

![Appearance settings](images/terminal-tools/settings-appearance.png)

Compact Termust settings keep numeric controls and Save/Cancel buttons small,
with a visible keyboard legend at the bottom. These examples use the updated
interface at narrow window sizes.

![Compact Termust settings](images/terminal-tools/settings-compact-termust.png)

![Compact Termust settings in Portuguese](images/terminal-tools/settings-compact-termust-pt.png)

![Light theme](images/terminal-tools/settings-light.png)

![Split panes in a light theme](images/terminal-tools/split-panes-light.png)

## Personal icons

Open **Settings → Appearance → Application icon** to choose the original Termust
icon, a cat, a dog, a fox or a robot. Click a card or use Tab and Enter to preview
your choice. Save applies it to all Termust windows and remembers it after restart.
Cancel restores the saved choice. **Restore original icon** changes only the icon;
press Save to keep that change.

On Linux, Save updates the current user's `Termust.desktop` launcher while keeping
its command, application identity and other fields. The icon files live in the user
data directory and survive release updates. Existing panel or pinned launchers may
need to be pinned again. Runtime window previews work on X11; the current Wayland
backend uses the saved launcher icon instead. KDE, GNOME, Cinnamon and XFCE shell
integration still needs validation in their actual sessions.

On Windows, window icons update live and Save updates a per-user Start menu
shortcut. Existing pinned shortcuts may need to be pinned again. On macOS, the
chosen icon appears in the Dock while Termust is running; Finder and the closed
application keep the original icon. The installed executable and macOS bundle
resources are unchanged.

The setting is also available as `window.icon`, with values `termust`, `cat`, `dog`,
`fox` and `robot`. Presets are bundled and work offline. Personal photos and animated
companions are not part of this version.

![Personal icon picker](images/terminal-tools/icon-personalization.en-dpi1-cat-preview.png)

## Language

Choose the system language, English or Brazilian Portuguese. The choice is
saved. It changes the interface, while shell output keeps its own language.

![Language settings in English](images/terminal-tools/settings-language.png)

![Language settings in Brazilian Portuguese](images/terminal-tools/settings-language-pt.png)

## Shortcuts

See actions and their keys together in a scrollable list. Search by action name
or key, or press Ctrl+F to search. Click a row to change its key, then Save.
The editor checks conflicts, and Save/Cancel stay visible while you scroll.

![Searchable shortcut list](images/terminal-tools/shortcut-list-en-wide.png)

![Compact shortcut list in Portuguese](images/terminal-tools/shortcut-list-pt-compact.png)

See [tabs and saved sessions](tabs.md), [split panes](panes.md) and
[settings and workspaces](productivity.md) for the keys and full details.

## Tests

Some screenshots were made before the Termust rename; the compact settings examples show the updated Termust interface. They come from the actual app on Linux/X11, using a temporary
Xvfb display and sample files. They do not show private sessions. The Rust
workspace tests pass on stable Rust and Rust 1.85.0. Graphical tests cover tabs,
panes, sessions, menus, settings, language and close confirmation. This is not a
speed comparison with Terminator. Windows and macOS have not been tested here.
