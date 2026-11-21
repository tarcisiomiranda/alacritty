# Split terminal panes

Each tab can contain up to sixteen independent terminals. Right-click a terminal
and choose **Split right / Dividir à direita** or **Split below / Dividir abaixo**.
The new shell inherits the focused pane's working directory and supported shell
profile. Existing programs continue running in their own panes.

| Shortcut | Action |
| --- | --- |
| Ctrl+Shift+E | Split to the right |
| Ctrl+Shift+J | Split below |
| Ctrl+Alt+Left / Right / Up / Down | Focus the neighboring pane |
| Ctrl+Shift+Z | Maximize or restore the focused pane |
| Ctrl+Alt+W | Close the focused pane |
| Ctrl+Shift+W | Close the whole tab, including its panes |

On macOS use Shift+Cmd+E, Shift+Cmd+J and Shift+Cmd+Z to split and maximize,
Option+Cmd+arrows to move the focus, Option+Cmd+W to close a pane and Cmd+W to
close the whole tab.

Click inside a pane to focus it. A subtle one-pixel line separates panes; it does
not reserve a whole text row or column. Drag the line or its invisible surrounding
margin to change the proportions.
Maximizing temporarily hides neighboring panes; their processes continue running.
Restoring returns to the previous arrangement. Each pane has independent output,
selection, search, scrollback, shell state and command history. The blue border
identifies the focused pane. Search with **Ctrl+Shift+F** stays inside that pane.

Pane closure uses the same configurable confirmation as tabs: **Y / Enter**
confirms, **N / Escape** cancels. Closing a pane expands its neighbor. Exiting
its shell closes only that pane; the final pane closes its tab. A whole-tab close
requests one confirmation for all terminals in that tab.

## Move a pane

Drag the small circle in a pane's upper-right corner to move that live terminal.
The pointer becomes an open hand over the circle and a closed hand while dragging
onto a valid destination. No header or extra terminal row is added. In a tab with
one terminal, the circle appears when you hover over its upper-right corner.

Drop near the left, right, top or bottom edge of another pane to dock there. The
highlight previews the destination. To move across tabs, hold the circle over the
target tab to reveal its panes, then choose an edge; dropping directly on the tab
adds the terminal to the right of its focused pane. Drop on **+** to detach a pane
into a new tab. Moving the final pane out of a tab removes that empty tab.

Shells, running commands, SSH connections, scrollback and search state travel with
the pane. **Escape**, loss of window focus or an invalid drop cancels the move.
Normal selection, application mouse input and separator resizing keep their
existing behavior. Saved layouts include the resulting arrangement.

## Saved layouts

**Save all tabs** and named **Workspaces** preserve each tab's split tree,
divider proportions, focused pane, names, working directories, supported shell
profiles, recent commands and replay preferences. Legacy snapshots with one
terminal per tab remain readable. Saved pane IDs are local indices and are mapped
to new live session identities on restoration.

Reopening a saved workspace adds fresh shells alongside existing tabs. Processes,
terminal output and shell variables are not restored. Command replay remains an
explicit choice for each pane: focus it, open the tab settings, select the replay
checkbox and save the layout. The tab name applies to the whole tab; the directory,
recent commands and replay checkbox refer to the pane focused when settings open.

Automatic recovery saves the same layout when enabled and always suppresses
command replay. Duplicating or reopening a closed tab preserves its layout and
directories, with replay disabled. Temporary pane maximization is not saved.

The configurable action names are `SplitVertical`, `SplitHorizontal`,
`FocusPaneLeft`, `FocusPaneRight`, `FocusPaneUp`, `FocusPaneDown`,
`TogglePaneZoom` and `ClosePane`. Existing shortcuts, including
**Ctrl+Shift+O** for restoring sessions, retain their behavior. Change common
shortcuts in **Settings → Shortcuts**; changes apply when saved.

## Validation

```sh
mise exec -- cargo test --workspace
mise exec -- cargo build -p termust
mise exec -- python3 -B scripts/test-panes.py
```

The isolated X11 regression exercises real shell identities and directories,
nested splits, keyboard and mouse focus, resizing, pixel clipping, search,
maximization, saved workspaces, confirmations and natural process exits.
