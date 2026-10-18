# Settings and working environments

Open **Settings / Configurações** from the right-click menu or press **Ctrl+,**.
The unified preferences dialog opens directly, with pages for **Appearance**,
**Terminal**, **Tabs**, **Sessions**, **Language** and **Shortcuts**. All pages
support mouse and keyboard navigation, including compact windows.

Wide windows show categories in a sidebar; compact windows use two rows of
categories. The content scrolls independently while Save and Cancel stay visible.
Use the wheel, the arrow controls, or PageUp/PageDown to scroll. Tab and Shift+Tab
move focus and reveal the next control. The selected category has an accent strip;
a focus outline shows the control that Enter will activate.

Shared controls adapt to a light or dark terminal background. Inputs, radio
choices, checkboxes and destructive actions have distinct visual styles. The
right-click menu groups tab, pane, clipboard, session and closing actions;
shortcuts appear in a separate column. Compact menus scroll with the wheel,
arrow controls and keyboard navigation. Clicking a disabled item or separator
keeps the menu open; Escape or clicking outside dismisses it.

Appearance controls edit the font family, point size, opacity and a complete
dark or light palette. **Keep current colors** preserves the configured theme.
Changes preview in the current window. **Cancel**, Escape or clicking outside
restores its prior appearance, including an existing temporary font zoom.
**Save** applies the settings to every open window and persists them
in the primary TOML configuration. Comments, imports, unrelated settings,
font styles and permissions are retained. Saving replaces only corresponding
CLI and IPC overrides. Unsupported or unwritable files remain intact.
**Restore defaults** previews the application's default preferences and the
dark palette; it still requires Save to persist them.

Terminal controls set scrollback capacity, hiding the pointer while typing and
copying selections to the clipboard. Scrollback capacity changes only on Save,
so canceling a preview cannot discard existing history. Language offers Automatic,
English and Brazilian Portuguese in the same dialog.

## Editing shortcuts

The Shortcuts page shows two actions at a time. Select an action, press the new
combination and choose **Save**. Use Ctrl, Alt or Super with a key, or a function
key. Existing bindings, terminal modes and configured hint shortcuts are checked
for conflicts. A conflict keeps the previous binding and displays an explanation.
Escape cancels capture; **Cancel** discards unsaved changes. Shortcuts apply only
on Save. **Disable** removes shortcuts for the selected action; Restore defaults
stages the standard combinations again.

Edits are stored individually under `general.shortcuts`. Imported settings,
comments and custom `keyboard.bindings` remain in the original files. An override
replaces bindings for its named action; unrelated actions keep their bindings.
An empty string disables the action's shortcuts. Removing its override restores
the original bindings. For example:

```toml
[general.shortcuts]
SplitVertical = "Ctrl+Alt+E"
CreateNewTab = "Ctrl+Shift+T"
```

The editor covers tab creation, duplication, reopening and reordering; pane
splitting, navigation, maximization and closure; workspaces, settings and session
saving. Other actions remain available through `keyboard.bindings`.

## Named workspaces

Press **Ctrl+Shift+P** or choose **Workspaces / Ambientes de trabalho** from the
context menu. Enter a name and click **Save current tabs / Salvar abas atuais**
to create a working environment. Saving an existing name updates that layout.
Each workspace stores ordered tabs, their active tab, custom names, working
directories, shell profiles, recent command records and replay preferences.
Tabs with [split panes](panes.md) also retain their arrangement, proportions and
focused pane.

Select a saved workspace to open its tabs alongside the current sessions.
The sessions already running in the window continue unchanged. **Use at
startup / Usar ao iniciar** enables restoration and selects that workspace for
a normal launch. The preferences Sessions page can select another workspace or
return to **Last saved session / Última sessão salva**. An explicit command,
directory, title or hold option still starts its requested session.

**Remove workspace / Remover ambiente** requires a second confirmation click.
Workspace names may contain Unicode, use up to 80 characters, and are never
interpreted as filenames. Up to 32 environments share the private, bounded
`workspaces.json` index beside the existing `tabs.json` manual snapshot.
Malformed state files are preserved and produce a visible error.

## Automatic recovery

The Sessions page has separate controls to restore tabs at startup and save
recovery snapshots automatically. Recovery is off by default. When enabled,
the focused window saves changes approximately every two seconds to a private
`recovery.json` beside the manual snapshot. Closing a tab updates recovery;
closing the last tab retains its layout for the next launch. Other windows do
not overwrite the focused window's recovery in the background.

Startup selects an explicit named workspace first. Otherwise, with recovery
enabled it uses the recovery snapshot when present, then falls back to the
manually saved session. Recovery creates fresh shells and always suppresses
command replay. The saved per-tab replay choices remain available for explicit
manual or named restoration. A recovery snapshot never replaces `tabs.json` or
the named environments. Recovery cannot restore running processes, scrollback
or unsaved shell variables.

```toml
[general]
restore_session = true
session_autosave = false
startup_session = "Backend"
```

`restore_session` defaults to `true`. Set it to `false` to start without
restoring tabs. `session_autosave` defaults to `false`. An absent or empty
`startup_session` selects the last saved session or optional recovery;
the GUI writes an explicit empty string to override an imported preference.

## Confirming tab closure

Closing a tab through Ctrl+Shift+W, its × button, middle-click or the context
menu shows a confirmation by default. The dialog identifies the requested
tab, including an inactive tab or the final tab. Press **Y** or **Enter** to
confirm; press **N** or **Escape**, click Cancel, or click outside to cancel.
Canceling keeps the running session and its working directory intact.

The **Tabs** preferences page includes **Confirm before closing a tab /
Confirmar antes de fechar uma aba**. Turn it off and save to restore immediate
tab closure. The setting is `general.confirm_tab_close`, defaults to `true`,
and uses the same safe configuration persistence as other preferences.
An unsaved preference preview does not change close behavior. Natural shell
exits and whole-window close actions retain their existing behavior.

## Tab organization and activity

Drag a tab to reorder it or use **Ctrl+Shift+PageUp / PageDown**. Reordering
preserves the running shell, its terminal state and active identity.
**Ctrl+Shift+D** duplicates the active tab using a fresh shell in its working
directory. **Ctrl+Shift+Alt+T** reopens the most recently closed tab. Up to 20
closed tabs are available for the current window. Duplicating and reopening
retain custom names and recent command records, disable replay, and never
reexecute a saved command.

Supported interactive Bash, Zsh and Fish shells provide activity markers:
**●** for a running command, **✓** for successful completion in a background
tab, and **!** for failure. Selecting the tab acknowledges its completion
marker. Activity works for history-excluded commands without capturing their
text. Custom commands and unsupported shell profiles do not fabricate shell
status. Process state and command output remain independent of these markers.

The configurable actions are `DuplicateTab`, `ReopenClosedTab`, `MoveTabLeft`,
`MoveTabRight`, `OpenWorkspaces` and `OpenSettings`. Bind them through the
Shortcuts preferences page or the existing `keyboard.bindings` table.

## Validation

```sh
mise exec -- cargo test --workspace
mise exec -- cargo build -p alacritty
mise exec -- python3 -B scripts/test-productivity.py
mise exec -- python3 -B scripts/test-close-confirmation.py
```

The graphical regression uses an isolated X11 display and temporary user
directories. It verifies preview/cancel, configuration preservation and
restart, workspace startup selection, tab identities and directories, and
independent recovery without command replay. Existing tab, session, UI and
language regressions remain available in `scripts/`.
