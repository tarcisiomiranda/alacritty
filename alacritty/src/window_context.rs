//! Terminal window context.

use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::mem;
#[cfg(not(windows))]
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glutin::config::Config as GlutinConfig;
use glutin::display::GetGlDisplay;
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use glutin::platform::x11::X11GlConfigExt;
use log::error;
use serde_json as json;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, Event as WinitEvent, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey, PhysicalKey};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{CursorIcon, WindowId};

use alacritty_terminal::event::{Event as TerminalEvent, Notify, OnResize};
use alacritty_terminal::event_loop::{EventLoop as PtyEventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Direction;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::NamedColor;

use crate::cli::{ParsedOptions, WindowOptions};
use crate::clipboard::Clipboard;
use crate::close_confirmation::{CloseAction, CloseConfirmation};
use crate::config::UiConfig;
use crate::context_menu::{ContextMenu, MenuAction};
#[cfg(not(windows))]
use crate::daemon::foreground_process_path;
use crate::display::Display;
use crate::display::color::Rgb;
use crate::display::hint::HintState;
use crate::display::window::{ImeInhibitor, Window};
use crate::event::{
    ActionContext, Event, EventProxy, EventType, InlineSearchState, Mouse, SearchState,
    TouchPurpose,
};
use crate::language_settings::LanguageDialogAction;
use crate::localization::Message;
#[cfg(unix)]
use crate::logging::LOG_TARGET_IPC_CONFIG;
use crate::message_bar::MessageBuffer;
use crate::panes::{Axis, PaneAction, PaneNode, PaneRect};
use crate::productivity::{
    PreferencesDialog, ProductivityAction, ProductivityDialog, WorkspaceDialog,
};
use crate::scheduler::{Scheduler, TimerId, Topic};
use crate::session::{
    CommandRecord, MAX_COMMANDS, SavedSession, SavedShell, SavedTab, SessionStore, WorkspaceStore,
};
use crate::shell_integration::ShellIntegration;
use crate::tab_ui::{DialogAction, TabDialog};
use crate::tabs::{TabAction, TabActivity, TabButton, TabId, Tabs};
use crate::{input, renderer};

struct TabContext {
    id: TabId,
    title: String,
    custom_title: Option<String>,
    working_directory: Option<PathBuf>,
    commands: Vec<CommandRecord>,
    captured_commands: Vec<CommandRecord>,
    activity: TabActivity,
    completion: Option<(u64, i32)>,
    replay_last_command: bool,
    pending_replay: Option<CommandRecord>,
    integration: Option<ShellIntegration>,
    original_shell: Option<SavedShell>,
    replay_started: Option<Instant>,
    terminal: Arc<FairMutex<Term<EventProxy>>>,
    cursor_blink_timed_out: bool,
    prev_bell_cmd: Option<Instant>,
    inline_search_state: InlineSearchState,
    search_state: SearchState,
    notifier: Notifier,
    preserve_title: bool,
    hold: bool,
    #[cfg(not(windows))]
    master_fd: RawFd,
    #[cfg(not(windows))]
    shell_pid: u32,
}

impl TabContext {
    fn new(
        display: &Display,
        config: &UiConfig,
        options: WindowOptions,
        proxy: EventLoopProxy<Event>,
        id: TabId,
        shell: Option<tty::Shell>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut pty_config = config.pty_config();
        options.terminal_options.override_pty_config(&mut pty_config);
        if let Some(shell) = shell {
            pty_config.shell = Some(shell);
        }
        let preserve_title = options.window_identity.title.is_some();
        let hold = options.terminal_options.hold;
        let title =
            options.window_identity.title.unwrap_or_else(|| config.window.identity.title.clone());
        let working_directory =
            pty_config.working_directory.clone().or_else(|| std::env::current_dir().ok());
        let integration = ShellIntegration::prepare(&mut pty_config)?;
        let original_shell = integration.as_ref().map(|integration| SavedShell {
            program: integration.original_shell().program().to_owned(),
            args: integration.original_shell().args().to_vec(),
        });
        let event_proxy = EventProxy::for_tab(proxy, display.window.id(), id);
        let terminal = Term::new(config.term_options(), &display.size_info, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));
        let pty = tty::new(&pty_config, display.size_info.into(), display.window.id().into())?;
        #[cfg(not(windows))]
        let master_fd = pty.file().as_raw_fd();
        #[cfg(not(windows))]
        let shell_pid = pty.child().id();
        let event_loop = PtyEventLoop::new(
            Arc::clone(&terminal),
            event_proxy.clone(),
            pty,
            pty_config.drain_on_exit,
            config.debug.ref_test,
        )?;
        let loop_tx = event_loop.channel();
        let _io_thread = event_loop.spawn();
        if config.cursor.style().blinking {
            event_proxy.send_event(TerminalEvent::CursorBlinkingChange.into());
        }
        Ok(Self {
            id,
            title,
            custom_title: None,
            working_directory,
            commands: Vec::new(),
            captured_commands: Vec::new(),
            activity: TabActivity::Idle,
            completion: None,
            replay_last_command: false,
            pending_replay: None,
            integration,
            original_shell,
            replay_started: None,
            terminal,
            notifier: Notifier(loop_tx),
            preserve_title,
            hold,
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
            cursor_blink_timed_out: Default::default(),
            prev_bell_cmd: Default::default(),
            inline_search_state: Default::default(),
            search_state: Default::default(),
        })
    }
}

impl Drop for TabContext {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

impl TabContext {
    fn display_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or(&self.title)
    }

    fn apply_saved(&mut self, saved: SavedTab) {
        self.custom_title = saved.title;
        self.commands = saved.commands;
        self.replay_last_command = saved.replay_last_command;
        if self.replay_last_command
            && self.integration.is_some()
            && self.original_shell == saved.shell
        {
            self.pending_replay = self.commands.last().cloned();
            self.replay_started = Some(Instant::now());
        }
    }

    fn refresh_history(&mut self) -> bool {
        let Some(integration) = &self.integration else { return false };
        let Ok(snapshot) = integration.snapshot() else { return false };
        if let Some(directory) = snapshot.working_directory {
            self.working_directory = Some(directory);
        }
        if !snapshot.ready {
            self.activity = TabActivity::Running;
        } else if snapshot.completion != self.completion {
            self.activity = match snapshot.completion {
                Some((_, 0)) => TabActivity::Success,
                Some(_) => TabActivity::Failed,
                None => TabActivity::Idle,
            };
        }
        if snapshot.ready && self.activity == TabActivity::Running {
            self.activity = TabActivity::Idle;
        }
        self.completion = snapshot.completion;
        let incoming = snapshot.commands;
        let overlap = (0..=self.captured_commands.len().min(incoming.len()))
            .rev()
            .find(|count| {
                self.captured_commands[self.captured_commands.len() - count..] == incoming[..*count]
            })
            .unwrap_or(0);
        self.commands.extend(incoming[overlap..].iter().cloned());
        if self.commands.len() > MAX_COMMANDS {
            self.commands.drain(..self.commands.len() - MAX_COMMANDS);
        }
        self.captured_commands = incoming;
        snapshot.ready
    }

    fn snapshot(&mut self) -> SavedTab {
        self.refresh_history();
        #[cfg(not(windows))]
        if let Ok(directory) = foreground_process_path(self.master_fd, self.shell_pid) {
            self.working_directory = Some(directory);
        }
        SavedTab {
            title: self.custom_title.clone(),
            working_directory: self.working_directory.clone(),
            commands: self.commands.clone(),
            replay_last_command: self.replay_last_command,
            shell: self.original_shell.clone(),
            ..SavedTab::default()
        }
    }
}

struct TerminalTab {
    id: TabId,
    custom_title: Option<String>,
    panes: Tabs<TabContext>,
    layout: PaneNode,
    zoomed: bool,
}

impl std::ops::Deref for TerminalTab {
    type Target = TabContext;

    fn deref(&self) -> &TabContext {
        &self.panes.active
    }
}

impl std::ops::DerefMut for TerminalTab {
    fn deref_mut(&mut self) -> &mut TabContext {
        &mut self.panes.active
    }
}

impl TerminalTab {
    fn single(pane: TabContext) -> Self {
        Self {
            id: pane.id,
            custom_title: None,
            layout: PaneNode::Leaf(pane.id),
            panes: Tabs::new(pane),
            zoomed: false,
        }
    }

    fn display_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or_else(|| self.panes.active.display_title())
    }

    fn snapshot(&mut self) -> SavedTab {
        let ids: Vec<_> = self.panes.iter().map(|pane| pane.id).collect();
        let mut snapshots = self.panes.iter_mut().map(TabContext::snapshot);
        let mut saved = snapshots.next().unwrap();
        saved.title = self.custom_title.clone();
        saved.panes = snapshots.collect();
        saved.active_pane = self.panes.active_index;
        if self.panes.len() > 1 {
            saved.layout =
                Some(self.layout.remap_ids(|id| {
                    ids.iter().position(|candidate| *candidate == id).unwrap() as u64
                }));
        }
        saved
    }

    fn restore(
        display: &Display,
        config: &UiConfig,
        mut saved: SavedTab,
        proxy: &EventLoopProxy<Event>,
        next_id: &mut TabId,
        replay: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let active = saved.active_pane;
        let layout = saved.layout.take();
        let title = saved.title.clone();
        let extra = mem::take(&mut saved.panes);
        let mut restored = Vec::new();
        for saved in std::iter::once(saved).chain(extra) {
            let mut options = WindowOptions::default();
            options.terminal_options.working_directory =
                saved.working_directory.clone().filter(|path| path.is_dir());
            let id = *next_id;
            *next_id += 1;
            let shell = saved
                .shell
                .as_ref()
                .map(|shell| tty::Shell::new(shell.program.clone(), shell.args.clone()));
            let mut pane = TabContext::new(display, config, options, proxy.clone(), id, shell)?;
            pane.apply_saved(saved);
            pane.custom_title = None;
            if !replay {
                pane.pending_replay = None;
            }
            pane.terminal.lock().is_focused = false;
            restored.push(pane);
        }
        let ids: Vec<_> = restored.iter().map(|pane| pane.id).collect();
        let mut restored = restored.into_iter();
        let mut tab = Self::single(restored.next().unwrap());
        for pane in restored {
            tab.panes.push(pane);
        }
        tab.panes.select(active);
        tab.custom_title = title;
        if let Some(layout) = layout {
            tab.layout = layout.remap_ids(|id| ids[id as usize]);
        }
        Ok(tab)
    }

    fn rectangles(&self, size: crate::display::SizeInfo) -> Vec<(TabId, PaneRect)> {
        if self.zoomed {
            vec![(self.panes.active.id, PaneRect {
                x: 0,
                y: 0,
                width: size.columns(),
                height: size.screen_lines(),
            })]
        } else {
            self.layout.layout(size.columns(), size.screen_lines()).panes
        }
    }
}

pub struct WindowContext {
    pub message_buffer: MessageBuffer,
    pub display: Display,
    pub dirty: bool,
    event_queue: Vec<WinitEvent<Event>>,
    tab_actions: Vec<EventType>,
    tabs: Tabs<TerminalTab>,
    pane_drag: Option<Vec<bool>>,
    closing_pane: Option<TabId>,
    tab_dialog_pane: Option<TabId>,
    context_menu_pane: Option<TabId>,
    next_tab_id: TabId,
    recent_closed: Vec<SavedTab>,
    tab_drag: Option<TabId>,
    previewing: bool,
    last_recovery: Instant,
    recovery_snapshot: Option<SavedSession>,
    recovery_failed: bool,
    modifiers: Modifiers,
    mouse: Mouse,
    touch: TouchPurpose,
    pointer_position: PhysicalPosition<f64>,
    tab_mouse_pressed: bool,
    close_confirmation_key: Option<PhysicalKey>,
    close_mouse_release: Option<MouseButton>,
    occluded: bool,
    window_config: ParsedOptions,
    config: Rc<UiConfig>,
}

impl WindowContext {
    /// Create initial window context that does bootstrapping the graphics API we're going to use.
    pub fn initial(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let raw_display_handle = event_loop.display_handle().unwrap().as_raw();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Windows has different order of GL platform initialization compared to any other platform;
        // it requires the window first.
        #[cfg(windows)]
        let window = Window::new(event_loop, &config, &identity, &mut options)?;
        #[cfg(windows)]
        let raw_window_handle = Some(window.raw_window_handle());

        #[cfg(not(windows))]
        let raw_window_handle = None;

        let gl_display = renderer::platform::create_gl_display(
            raw_display_handle,
            raw_window_handle,
            config.debug.prefer_egl,
        )?;
        let gl_config = renderer::platform::pick_gl_config(&gl_display, raw_window_handle)?;

        #[cfg(not(windows))]
        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, &gl_config, raw_window_handle)?;

        let display = Display::new(window, gl_context, &config, false)?;

        let restore = options.terminal_options.command().is_none()
            && options.terminal_options.working_directory.is_none()
            && !options.terminal_options.hold
            && options.window_identity.title.is_none()
            && !config.debug.ref_test;
        Self::new(display, config, options, proxy, restore)
    }

    /// Create additional context with the graphics platform other windows are using.
    pub fn additional(
        gl_config: &GlutinConfig,
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
        config_overrides: ParsedOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let gl_display = gl_config.display();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Check if new window will be opened as a tab.
        // This must be done before `Window::new()`, which unsets `window_tabbing_id`.
        #[cfg(target_os = "macos")]
        let tabbed = options.window_tabbing_id.is_some();
        #[cfg(not(target_os = "macos"))]
        let tabbed = false;

        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let raw_window_handle = window.raw_window_handle();
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, gl_config, Some(raw_window_handle))?;

        let display = Display::new(window, gl_context, &config, tabbed)?;

        let mut window_context = Self::new(display, config, options, proxy, false)?;

        // Set the config overrides at startup.
        //
        // These are already applied to `config`, so no update is necessary.
        window_context.window_config = config_overrides;

        Ok(window_context)
    }

    /// Create a new terminal window context.
    fn new(
        display: Display,
        config: Rc<UiConfig>,
        options: WindowOptions,
        proxy: EventLoopProxy<Event>,
        restore: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let mut restored = Vec::new();
        let mut next_tab_id = 0;
        let mut active = 0;
        let mut notice = None;
        if restore && config.general.restore_session && display.tab_bar.is_some() {
            match Self::load_startup_session(&config) {
                Ok(Some((snapshot, replay))) => {
                    active = snapshot.active;
                    for (index, saved) in snapshot.tabs.into_iter().enumerate() {
                        let tab = match TerminalTab::restore(
                            &display,
                            &config,
                            saved,
                            &proxy,
                            &mut next_tab_id,
                            replay,
                        ) {
                            Ok(tab) => tab,
                            Err(_) => {
                                restored.clear();
                                active = 0;
                                notice = Some(Message::SavedShellFailed);
                                break;
                            },
                        };
                        tab.terminal.lock().is_focused = index == active;
                        restored.push(tab);
                    }
                },
                Ok(None) => (),
                Err(_) => notice = Some(Message::SessionReadFailed),
            }
        }
        let tabs = if restored.is_empty() {
            let id = next_tab_id;
            next_tab_id += 1;
            Tabs::new(TerminalTab::single(TabContext::new(
                &display,
                &config,
                options,
                proxy.clone(),
                id,
                None,
            )?))
        } else {
            let mut iterator = restored.into_iter();
            let mut tabs = Tabs::new(iterator.next().unwrap());
            for tab in iterator {
                tabs.push(tab);
            }
            tabs.select(active);
            tabs
        };
        let mut context = Self {
            display,
            config,
            tabs,
            next_tab_id,
            recent_closed: Vec::new(),
            pane_drag: None,
            closing_pane: None,
            tab_dialog_pane: None,
            context_menu_pane: None,
            tab_drag: None,
            previewing: false,
            last_recovery: Instant::now(),
            recovery_snapshot: None,
            recovery_failed: false,
            message_buffer: Default::default(),
            window_config: Default::default(),
            event_queue: Default::default(),
            tab_actions: Default::default(),
            modifiers: Default::default(),
            occluded: Default::default(),
            mouse: Default::default(),
            touch: Default::default(),
            pointer_position: PhysicalPosition::new(-1., -1.),
            tab_mouse_pressed: false,
            close_confirmation_key: None,
            close_mouse_release: None,
            dirty: true,
        };
        context.display.window.set_title(context.tabs.active.display_title().to_owned());
        context.display.split_active = context.tabs.active.panes.len() > 1;
        context.display.pending_update.dirty = true;
        context.resize_inactive_tabs();
        context.refresh_tab_bar();
        let _ = proxy.send_event(Event::new(EventType::SessionActivity, context.id()));
        if context
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.iter())
            .any(|pane| pane.pending_replay.is_some())
        {
            let _ = proxy.send_event(Event::new(EventType::RestorePending, context.id()));
        }
        if let Some(notice) = notice {
            context.open_tab_dialog(context.tabs.active_index);
            if let Some(dialog) = &mut context.display.tab_dialog {
                dialog.status = Some(notice);
            }
        }
        Ok(context)
    }

    /// Update the terminal window to the latest config.
    pub fn update_config(&mut self, new_config: Rc<UiConfig>) {
        let old_config = mem::replace(&mut self.config, new_config);

        // Apply ipc config if there are overrides.
        if !self.previewing {
            self.config = self.window_config.override_config_rc(self.config.clone());
            if let Some(ProductivityDialog::Preferences(dialog)) =
                &mut self.display.productivity_dialog
            {
                let base = self.config.clone();
                if dialog.original.font != base.font {
                    dialog.original_font_size =
                        base.font.size().scale(self.display.window.scale_factor as f32);
                }
                if dialog.values.language == dialog.original.general.language {
                    dialog.values.language = base.general.language;
                }
                dialog.original = base.clone();
                let mut preview = (*base).clone();
                if dialog.values.preview(&mut preview).is_ok() {
                    self.config = Rc::new(preview);
                }
            }
        }
        let locale = self.config.general.language.resolve();
        if let Some(dialog) = &mut self.display.close_confirmation {
            dialog.locale = locale;
        }
        if let Some(dialog) = &mut self.display.productivity_dialog {
            *dialog.locale_mut() = locale;
            if let ProductivityDialog::Workspaces(dialog) = dialog {
                dialog.startup = self.config.general.startup_session.clone();
            }
        }
        if let Some(dialog) = &mut self.display.tab_dialog {
            dialog.locale = locale;
        }
        if let Some(menu) = &mut self.display.context_menu {
            menu.locale = locale;
            menu.shortcuts = self.config.general.shortcuts.clone();
        }
        if let Some(dialog) = &mut self.display.language_dialog {
            if old_config.general.language != self.config.general.language {
                dialog.selected = self.config.general.language;
                dialog.locale = locale;
            }
        }

        self.display.update_config(&self.config);
        for tab in self.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
            tab.terminal.lock().set_options(self.config.term_options());
            if !tab.preserve_title
                && (!self.config.window.dynamic_title
                    || tab.title == old_config.window.identity.title)
            {
                tab.title = self.config.window.identity.title.clone();
            }
        }

        // Reload cursor if its thickness has changed.
        if (old_config.cursor.thickness() - self.config.cursor.thickness()).abs() > f32::EPSILON {
            self.display.pending_update.set_cursor_dirty();
        }

        if old_config.font != self.config.font {
            let scale_factor = self.display.window.scale_factor as f32;
            // Do not update font size if it has been changed at runtime.
            if self.display.font_size == old_config.font.size().scale(scale_factor) {
                self.display.font_size = self.config.font.size().scale(scale_factor);
            }

            let font = self.config.font.clone().with_size(self.display.font_size);
            self.display.pending_update.set_font(font);
        }

        // Always reload the theme to account for auto-theme switching.
        self.display.window.set_theme(self.config.window.theme());

        // Update display if either padding options or resize increments were changed.
        let window_config = &old_config.window;
        if window_config.padding(1.) != self.config.window.padding(1.)
            || window_config.dynamic_padding != self.config.window.dynamic_padding
            || window_config.resize_increments != self.config.window.resize_increments
        {
            self.display.pending_update.dirty = true;
        }

        // Update title on config reload according to the following table.
        //
        // │cli │ dynamic_title │ current_title == old_config ││ set_title │
        // │ Y  │       _       │              _              ││     N     │
        // │ N  │       Y       │              Y              ││     Y     │
        // │ N  │       Y       │              N              ││     N     │
        // │ N  │       N       │              _              ││     Y     │
        if !self.tabs.active.preserve_title
            && self.tabs.active.custom_title.is_none()
            && (!self.config.window.dynamic_title
                || self.display.window.title() == old_config.window.identity.title)
        {
            self.display.window.set_title(self.config.window.identity.title.clone());
        }

        let opaque = self.config.window_opacity() >= 1.;

        // Disable shadows for transparent windows on macOS.
        #[cfg(target_os = "macos")]
        self.display.window.set_has_shadow(opaque);

        #[cfg(target_os = "macos")]
        self.display.window.set_option_as_alt(self.config.window.option_as_alt());

        // Change opacity and blur state.
        self.display.window.set_transparent(!opaque);
        self.display.window.set_blur(self.config.window.blur);

        // Update hint keys.
        self.display.hint_state.update_alphabet(self.config.hints.alphabet());

        // Update cursor blinking.
        let event = Event::new(TerminalEvent::CursorBlinkingChange.into(), None);
        self.event_queue.push(event.into());

        self.refresh_tab_bar();
        self.dirty = true;
    }

    /// Get reference to the window's configuration.
    fn load_startup_session(config: &UiConfig) -> std::io::Result<Option<(SavedSession, bool)>> {
        if let Some(name) =
            config.general.startup_session.as_deref().filter(|name| !name.trim().is_empty())
        {
            return match WorkspaceStore::default_path() {
                Some(path) => WorkspaceStore::new(path)
                    .load(name)
                    .map(|snapshot| snapshot.map(|snapshot| (snapshot, true))),
                None => Ok(None),
            };
        }
        if config.general.session_autosave {
            if let Some(path) = SessionStore::recovery_path() {
                if let Some(snapshot) = SessionStore::new(path).load()? {
                    return Ok(Some((snapshot, false)));
                }
            }
        }
        Self::load_saved_session().map(|snapshot| snapshot.map(|snapshot| (snapshot, true)))
    }

    fn snapshot(&mut self) -> SavedSession {
        SavedSession {
            version: 1,
            active: self.tabs.active_index,
            tabs: self.tabs.iter_mut().map(TerminalTab::snapshot).collect(),
        }
    }

    fn persist_recovery(&mut self) {
        let autosave = match &self.display.productivity_dialog {
            Some(ProductivityDialog::Preferences(dialog)) => {
                dialog.original.general.session_autosave
            },
            _ => self.config.general.session_autosave,
        };
        if !autosave || !self.tabs.active.terminal.lock().is_focused {
            return;
        }
        let Some(path) = SessionStore::recovery_path() else { return };
        let snapshot = self.snapshot();
        if self.recovery_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        if SessionStore::new(path).save(&snapshot).is_ok() {
            self.recovery_snapshot = Some(snapshot);
            self.recovery_failed = false;
        } else if !self.recovery_failed {
            self.recovery_failed = true;
            log::warn!("Could not save terminal recovery snapshot");
        }
    }

    fn poll_activity(&mut self, scheduler: &mut Scheduler) {
        let mut changed = false;
        let active = self.tabs.active_index;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            let active_pane = tab.panes.active_index;
            for (pane_index, pane) in tab.panes.iter_mut().enumerate() {
                let old = pane.activity;
                let ready = pane.refresh_history();
                if index == active && pane_index == active_pane && ready {
                    pane.activity = TabActivity::Idle;
                }
                changed |= old != pane.activity;
            }
        }
        if changed {
            self.refresh_tab_bar();
        }
        if self.last_recovery.elapsed() >= Duration::from_secs(2) {
            self.persist_recovery();
            self.last_recovery = Instant::now();
        }
        let timer = TimerId::new(Topic::SessionActivity, self.id());
        scheduler.unschedule(timer);
        scheduler.schedule(
            Event::new(EventType::SessionActivity, self.id()),
            Duration::from_millis(500),
            false,
            timer,
        );
    }

    fn select_pane(&mut self, id: TabId, proxy: &EventLoopProxy<Event>, scheduler: &mut Scheduler) {
        let Some(index) = self.tabs.active.panes.iter().position(|pane| pane.id == id) else {
            return;
        };
        if index == self.tabs.active.panes.active_index {
            return;
        }
        let focused = self.tabs.active.terminal.lock().is_focused;
        self.set_tab_focus(false);
        self.tabs.active.panes.select(index);
        self.activate_tab(focused, proxy, scheduler);
        self.resize_inactive_tabs();
    }

    fn pane_action(
        &mut self,
        action: PaneAction,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        if self.display.close_confirmation.is_some() {
            return;
        }
        match action {
            PaneAction::Split(axis) => {
                if self.tabs.active.panes.len() >= 16 {
                    self.session_notice(Message::PaneLimit);
                    return;
                }
                let saved = self.tabs.active.panes.active.snapshot();
                let mut options = WindowOptions::default();
                options.terminal_options.working_directory =
                    saved.working_directory.filter(|path| path.is_dir());
                let shell = saved.shell.map(|shell| tty::Shell::new(shell.program, shell.args));
                let id = self.next_tab_id;
                self.next_tab_id += 1;
                let pane = match TabContext::new(
                    &self.display,
                    &self.config,
                    options,
                    proxy.clone(),
                    id,
                    shell,
                ) {
                    Ok(pane) => pane,
                    Err(_) => {
                        self.session_notice(Message::SessionRestoreFailed);
                        return;
                    },
                };
                let active_id = self.tabs.active.panes.active.id;
                if !self.tabs.active.layout.split(active_id, id, axis) {
                    return;
                }
                pane.terminal.lock().is_focused = false;
                self.tabs.active.panes.push(pane);
                self.tabs.active.zoomed = false;
                self.select_pane(id, proxy, scheduler);
            },
            PaneAction::Focus(direction) => {
                let size = self.display.size_info;
                if let Some(id) = self.tabs.active.layout.navigate(
                    self.tabs.active.panes.active.id,
                    direction,
                    size.columns(),
                    size.screen_lines(),
                ) {
                    self.select_pane(id, proxy, scheduler);
                }
            },
            PaneAction::Zoom => {
                if self.tabs.active.panes.len() > 1 {
                    self.tabs.active.zoomed = !self.tabs.active.zoomed;
                    self.display.pending_update.dirty = true;
                    self.resize_inactive_tabs();
                    self.refresh_tab_bar();
                }
            },
            PaneAction::Close => self.request_close_pane(proxy, scheduler),
        }
    }

    fn request_close_pane(&mut self, proxy: &EventLoopProxy<Event>, scheduler: &mut Scheduler) {
        if self.display.close_confirmation.is_some() {
            return;
        }
        if self.tabs.active.panes.len() == 1 {
            self.request_close_tab(self.tabs.active_index, proxy, scheduler);
            return;
        }
        let id = self.tabs.active.panes.active.id;
        let confirm = match &self.display.productivity_dialog {
            Some(ProductivityDialog::Preferences(dialog)) => {
                dialog.original.general.confirm_tab_close
            },
            _ => self.config.general.confirm_tab_close,
        };
        if !confirm {
            self.close_pane_immediately(id, proxy, scheduler);
            return;
        }
        let title = self.tabs.active.panes.active.display_title().to_owned();
        self.close_context_menu();
        self.close_tab_dialog();
        self.close_language_settings();
        self.close_productivity(true);
        self.cancel_mouse_drag(scheduler);
        let mut dialog = CloseConfirmation::new(id, title, self.config.general.language.resolve());
        dialog.pane = true;
        self.closing_pane = Some(id);
        self.display.close_confirmation = Some(dialog);
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, true);
        self.display.ime.set_preedit(None);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
    }

    fn close_pane_immediately(
        &mut self,
        id: TabId,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        let found = if let Some(pane) =
            self.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()).find(|pane| pane.id == id)
        {
            pane.hold = false;
            pane.terminal.lock().exit();
            true
        } else {
            false
        };
        if found {
            self.route_terminal_event(id, TerminalEvent::Exit, proxy, scheduler);
        }
    }

    fn remove_tab(
        &mut self,
        index: usize,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        let id = self.tabs.get(index).unwrap().id;
        if self.display.close_confirmation.as_ref().is_some_and(|dialog| dialog.tab_id == id)
            && self.closing_pane.is_none()
        {
            self.dismiss_close_confirmation();
        }
        let mut saved = self.tabs.get_mut(index).unwrap().snapshot();
        saved.replay_last_command = false;
        for pane in &mut saved.panes {
            pane.replay_last_command = false;
        }
        if self.recent_closed.len() == 20 {
            self.recent_closed.remove(0);
        }
        self.recent_closed.push(saved);
        self.persist_recovery();
        if self.tabs.len() == 1 {
            self.display.window.hold = false;
            let _ = proxy.send_event(Event::new(TerminalEvent::Exit.into(), self.id()));
            return;
        }
        let focused = self.tabs.active.terminal.lock().is_focused;
        let active = index == self.tabs.active_index;
        if active {
            self.set_tab_focus(false);
        }
        self.tabs.remove(index);
        if self.display.context_menu.as_ref().is_some_and(|menu| menu.tab_id == id) {
            self.close_context_menu();
        }
        if self.display.tab_dialog.as_ref().is_some_and(|dialog| dialog.tab_id == id) {
            self.close_tab_dialog();
        }
        if active {
            self.activate_tab(focused, proxy, scheduler);
        } else {
            self.refresh_tab_bar();
        }
    }

    fn active_pane_size(&self) -> crate::display::SizeInfo {
        let size = self.display.size_info;
        if self.tabs.active.panes.len() == 1 {
            return size;
        }
        let id = self.tabs.active.panes.active.id;
        let rect = self
            .tabs
            .active
            .rectangles(size)
            .into_iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, rect)| rect)
            .unwrap_or(PaneRect { x: 0, y: 0, width: 2, height: 1 });
        let mut size = size.pane(rect.x, rect.y, rect.width, rect.height);
        size.reserve_lines(usize::from(self.tabs.active.search_state.history_index.is_some()));
        size
    }

    fn handle_pane_mouse(
        &mut self,
        event: &WinitEvent<Event>,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) -> bool {
        if let WinitEvent::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. }, ..
        } = event
        {
            self.pointer_position = *position;
        }
        if matches!(event, WinitEvent::WindowEvent { event: WindowEvent::Focused(false), .. }) {
            self.pane_drag = None;
        }
        if self.tabs.active.panes.len() == 1 {
            return false;
        }
        let size = self.display.size_info;
        let (x, y) = self.pointer_cell();
        if let Some(path) = self.pane_drag.clone() {
            match event {
                WinitEvent::WindowEvent { event: WindowEvent::CursorMoved { .. }, .. } => {
                    self.tabs.active.layout.resize_divider(
                        &path,
                        x.min(size.columns()),
                        y.min(size.screen_lines()),
                        size.columns(),
                        size.screen_lines(),
                    );
                    self.resize_inactive_tabs();
                    self.refresh_tab_bar();
                    return true;
                },
                WinitEvent::WindowEvent {
                    event: WindowEvent::MouseInput { state: ElementState::Released, .. },
                    ..
                } => {
                    self.pane_drag = None;
                    return true;
                },
                _ => (),
            }
        }
        if self.pointer_position.x < size.padding_x() as f64
            || self.pointer_position.y < size.padding_y() as f64
            || x >= size.columns()
            || y >= size.screen_lines()
        {
            return false;
        }
        let divider = (!self.tabs.active.zoomed)
            .then(|| self.tabs.active.layout.layout(size.columns(), size.screen_lines()))
            .and_then(|layout| layout.hit_divider(x, y).cloned());
        if let Some(divider) = divider {
            self.display.window.set_mouse_cursor(if divider.axis == Axis::Vertical {
                CursorIcon::ColResize
            } else {
                CursorIcon::RowResize
            });
            match event {
                WinitEvent::WindowEvent {
                    event:
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        },
                    ..
                } => {
                    self.cancel_mouse_drag(scheduler);
                    self.pane_drag = Some(divider.path);
                    return true;
                },
                WinitEvent::WindowEvent {
                    event: WindowEvent::MouseInput { state: ElementState::Released, button, .. },
                    ..
                } => {
                    let pressed = match button {
                        MouseButton::Left => self.mouse.left_button_state,
                        MouseButton::Middle => self.mouse.middle_button_state,
                        MouseButton::Right => self.mouse.right_button_state,
                        _ => ElementState::Released,
                    };
                    return pressed != ElementState::Pressed;
                },
                WinitEvent::WindowEvent {
                    event:
                        WindowEvent::CursorMoved { .. }
                        | WindowEvent::MouseWheel { .. }
                        | WindowEvent::MouseInput { .. },
                    ..
                } => return true,
                _ => (),
            }
        }
        if matches!(event, WinitEvent::WindowEvent {
            event: WindowEvent::MouseInput { state: ElementState::Pressed, .. },
            ..
        }) {
            let target = self
                .tabs
                .active
                .rectangles(size)
                .into_iter()
                .find(|(_, rect)| rect.contains(x, y))
                .map(|(id, _)| id);
            if let Some(id) = target {
                self.select_pane(id, proxy, scheduler);
                self.mouse.x = self.pointer_position.x.max(0.) as usize;
                self.mouse.y = self.pointer_position.y.max(0.) as usize;
            }
        }
        if matches!(event, WinitEvent::WindowEvent { event: WindowEvent::MouseWheel { .. }, .. })
            && !self
                .active_pane_size()
                .contains_point(self.pointer_position.x as usize, self.pointer_position.y as usize)
        {
            return true;
        }
        false
    }

    fn reorder_tab(&mut self, from: usize, to: usize) {
        if self.tabs.move_to(from, to) {
            self.refresh_tab_bar();
        }
    }

    fn open_fresh_tab(
        &mut self,
        mut saved: SavedTab,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) -> bool {
        saved.replay_last_command = false;
        for pane in &mut saved.panes {
            pane.replay_last_command = false;
        }
        match TerminalTab::restore(
            &self.display,
            &self.config,
            saved,
            proxy,
            &mut self.next_tab_id,
            false,
        ) {
            Ok(tab) => {
                self.tabs.push(tab);
                self.select_tab(self.tabs.len() - 1, proxy, scheduler);
                true
            },
            Err(_) => {
                self.session_notice(Message::SessionRestoreFailed);
                false
            },
        }
    }

    fn workspace_names() -> std::io::Result<Vec<String>> {
        match WorkspaceStore::default_path() {
            Some(path) => WorkspaceStore::new(path).list(),
            None => Ok(Vec::new()),
        }
    }

    fn open_workspaces(&mut self) {
        self.dismiss_close_confirmation();
        self.close_productivity(true);
        self.close_context_menu();
        self.close_tab_dialog();
        self.close_language_settings();
        let names = Self::workspace_names();
        let mut dialog = WorkspaceDialog::new(
            self.config.general.language.resolve(),
            names.as_ref().cloned().unwrap_or_default(),
            self.config.general.startup_session.clone(),
        );
        if names.is_err() {
            dialog.status = Some(Message::WorkspaceFailed);
        }
        self.display.productivity_dialog = Some(ProductivityDialog::Workspaces(dialog));
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, false);
        self.display.ime.set_preedit(None);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
    }

    fn open_preferences(&mut self) {
        self.dismiss_close_confirmation();
        self.close_productivity(true);
        self.close_context_menu();
        self.close_tab_dialog();
        self.close_language_settings();
        let mut dialog = PreferencesDialog::new(
            self.config.clone(),
            self.display.font_size,
            Self::workspace_names().unwrap_or_default(),
        );
        dialog.values.font_size =
            self.display.font_size.as_pt() / self.display.window.scale_factor as f32;
        self.display.productivity_dialog = Some(ProductivityDialog::Preferences(Box::new(dialog)));
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, false);
        self.display.ime.set_preedit(None);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
    }

    fn close_productivity(&mut self, restore: bool) {
        let Some(dialog) = self.display.productivity_dialog.take() else { return };
        if let ProductivityDialog::Preferences(dialog) = dialog {
            if restore {
                self.previewing = true;
                self.update_config(dialog.original);
                self.previewing = false;
                self.display.font_size = dialog.original_font_size;
                self.display
                    .pending_update
                    .set_font(self.config.font.clone().with_size(self.display.font_size));
            }
        }
        let vi = self.tabs.active.terminal.lock().mode().contains(TermMode::VI);
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, vi);
        self.display.window.set_mouse_cursor(CursorIcon::Text);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.damage_tracker.next_frame().mark_fully_damaged();
    }

    fn preview_preferences(&mut self) {
        let Some(ProductivityDialog::Preferences(dialog)) = &self.display.productivity_dialog
        else {
            return;
        };
        let mut preview = (*dialog.original).clone();
        if dialog.values.preview(&mut preview).is_err() || preview == *self.config {
            return;
        }
        let font_size = preview.font.size().scale(self.display.window.scale_factor as f32);
        self.previewing = true;
        self.update_config(Rc::new(preview));
        self.previewing = false;
        self.display.font_size = font_size;
        self.display.pending_update.set_font(self.config.font.clone().with_size(font_size));
    }

    pub fn update_settings_config(&mut self, config: Rc<UiConfig>, paths: &[Vec<String>]) {
        self.window_config.clear_settings_overrides(paths);
        self.update_config(config);
    }

    pub fn settings_saved(&mut self, saved: bool) {
        match &mut self.display.productivity_dialog {
            Some(ProductivityDialog::Preferences(dialog)) => {
                dialog.status =
                    Some(if saved { Message::SettingsSaved } else { Message::SettingsSaveFailed });
                if saved {
                    dialog.original_font_size = self.display.font_size;
                }
            },
            Some(ProductivityDialog::Workspaces(dialog)) => {
                dialog.status =
                    Some(if saved { Message::StartupSaved } else { Message::SettingsSaveFailed });
                if saved {
                    dialog.startup = self.config.general.startup_session.clone();
                }
            },
            None => (),
        }
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.window.request_redraw();
    }

    pub fn workspace_removed(&mut self, removed: bool) {
        let names = Self::workspace_names();
        if let Some(ProductivityDialog::Workspaces(dialog)) = &mut self.display.productivity_dialog
        {
            dialog.status =
                Some(if removed { Message::WorkspaceDeleted } else { Message::WorkspaceFailed });
            if let Ok(names) = names {
                dialog.names = names;
            }
            dialog.startup = self.config.general.startup_session.clone();
            if removed {
                dialog.selected = None;
            }
        }
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.window.request_redraw();
    }

    fn workspace_action(&mut self, action: ProductivityAction, proxy: &EventLoopProxy<Event>) {
        let Some(path) = WorkspaceStore::default_path() else { return };
        let store = WorkspaceStore::new(path);
        let mut status = Message::WorkspaceFailed;
        let mut selected = match &self.display.productivity_dialog {
            Some(ProductivityDialog::Workspaces(dialog)) => dialog.selected.clone(),
            _ => None,
        };
        match action {
            ProductivityAction::SaveWorkspace(name) => {
                self.apply_dialog_settings();
                if store.save(&name, &self.snapshot()).is_ok() {
                    status = Message::WorkspaceSaved;
                    selected = Some(name);
                }
            },
            ProductivityAction::RemoveWorkspace(name) => {
                let _ = proxy.send_event(Event::new(EventType::RemoveWorkspace(name), self.id()));
                return;
            },
            ProductivityAction::StartupWorkspace(name) => {
                let _ = proxy
                    .send_event(Event::new(EventType::SetStartupWorkspace(Some(name)), self.id()));
                return;
            },
            _ => (),
        }
        let names = store.list();
        if let Some(ProductivityDialog::Workspaces(dialog)) = &mut self.display.productivity_dialog
        {
            dialog.status = Some(status);
            if let Ok(names) = names {
                dialog.names = names;
            }
            dialog.selected = selected;
        }
    }

    fn handle_productivity_event(
        &mut self,
        event: &WinitEvent<Event>,
        proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
    ) -> bool {
        let Some(dialog) = &mut self.display.productivity_dialog else { return false };
        let size = self.display.size_info;
        let action = match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::KeyboardInput { event, is_synthetic: false, .. },
                ..
            } => {
                if event.state != ElementState::Pressed {
                    return true;
                }
                let modifiers = self.modifiers.state();
                if !dialog.captures_shortcut()
                    && modifiers.control_key()
                    && matches!(&event.logical_key, Key::Character(key) if key.eq_ignore_ascii_case("v"))
                {
                    dialog.paste(&clipboard.load(ClipboardType::Clipboard));
                    None
                } else {
                    dialog.key(event, modifiers)
                }
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                self.pointer_position = *position;
                let column = if position.x < size.padding_x() as f64 {
                    usize::MAX
                } else {
                    ((position.x - size.padding_x() as f64) / size.cell_width() as f64) as usize
                };
                let line = if position.y < size.padding_y() as f64 {
                    usize::MAX
                } else {
                    ((position.y - size.padding_y() as f64) / size.cell_height() as f64) as usize
                };
                self.dirty |= dialog.hover(column, line, size.columns(), size.screen_lines());
                self.display.window.set_mouse_visible(true);
                self.display.window.set_mouse_cursor(
                    if dialog.pointer_cursor(column, line, size.columns(), size.screen_lines()) {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    },
                );
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                self.tab_mouse_pressed = *state == ElementState::Pressed;
                if *state != ElementState::Pressed || *button != MouseButton::Left {
                    return true;
                }
                let x = self.pointer_position.x - size.padding_x() as f64;
                let y = self.pointer_position.y - size.padding_y() as f64;
                if x < 0. || y < 0. {
                    Some(ProductivityAction::Cancel)
                } else {
                    dialog.click(
                        (x / size.cell_width() as f64) as usize,
                        (y / size.cell_height() as f64) as usize,
                        size.columns(),
                        size.screen_lines(),
                    )
                }
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::Ime(winit::event::Ime::Commit(text)),
                ..
            } => {
                dialog.paste(text);
                None
            },
            WinitEvent::WindowEvent { event: WindowEvent::MouseWheel { delta, .. }, .. } => {
                let vertical = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
                    winit::event::MouseScrollDelta::PixelDelta(position) => position.y,
                };
                if vertical != 0. {
                    self.dirty |= dialog.scroll(
                        -vertical.signum() as isize,
                        size.columns(),
                        size.screen_lines(),
                    );
                }
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::Ime(_) | WindowEvent::Touch(_), ..
            } => return true,
            _ => return false,
        };
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        match action {
            Some(ProductivityAction::Cancel) => self.close_productivity(true),
            Some(ProductivityAction::SaveSettings(values)) => {
                self.preview_preferences();
                let _ = proxy
                    .send_event(Event::new(EventType::SetSettings(Box::new(values)), self.id()));
            },
            Some(ProductivityAction::OpenWorkspace(name)) => {
                let snapshot = WorkspaceStore::default_path()
                    .and_then(|path| WorkspaceStore::new(path).load(&name).ok().flatten());
                if let Some(snapshot) = snapshot {
                    self.tab_actions.push(EventType::OpenWorkspace(snapshot));
                } else if let Some(ProductivityDialog::Workspaces(dialog)) =
                    &mut self.display.productivity_dialog
                {
                    dialog.status = Some(Message::WorkspaceFailed);
                }
            },
            Some(action) => self.workspace_action(action, proxy),
            None => self.preview_preferences(),
        }
        true
    }

    fn load_saved_session() -> std::io::Result<Option<SavedSession>> {
        match SessionStore::default_path() {
            Some(path) => SessionStore::new(path).load(),
            None => Ok(None),
        }
    }

    pub fn update_language_config(&mut self, config: Rc<UiConfig>) {
        self.window_config.clear_language_override();
        self.update_config(config);
    }

    pub fn language_saved(&mut self, saved: bool) {
        if let Some(dialog) = &mut self.display.language_dialog {
            dialog.status =
                Some(if saved { Message::LanguageSaved } else { Message::LanguageSaveFailed });
        }
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.window.request_redraw();
    }

    fn close_language_settings(&mut self) {
        if self.display.language_dialog.take().is_none() {
            return;
        };
        let vi_mode = self.tabs.active.terminal.lock().mode().contains(TermMode::VI);
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, vi_mode);
        self.display.window.set_mouse_cursor(CursorIcon::Text);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.damage_tracker.next_frame().mark_fully_damaged();
    }

    fn handle_language_event(
        &mut self,
        event: &WinitEvent<Event>,
        proxy: &EventLoopProxy<Event>,
    ) -> bool {
        let Some(dialog) = &mut self.display.language_dialog else { return false };
        let size = self.display.size_info;
        let action = match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::KeyboardInput { event, is_synthetic: false, .. },
                ..
            } => {
                if event.state != ElementState::Pressed {
                    return true;
                };
                dialog.key(event, self.modifiers.state())
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                self.pointer_position = *position;
                let x = position.x - size.padding_x() as f64;
                let y = position.y - size.padding_y() as f64;
                let column =
                    if x < 0. { usize::MAX } else { (x / size.cell_width() as f64) as usize };
                let line =
                    if y < 0. { usize::MAX } else { (y / size.cell_height() as f64) as usize };
                self.dirty |= dialog.hover(column, line, size.columns(), size.screen_lines());
                self.display.window.set_mouse_visible(true);
                self.display.window.set_mouse_cursor(
                    if dialog.pointer_cursor(column, line, size.columns(), size.screen_lines()) {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    },
                );
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                if *state == ElementState::Released {
                    self.tab_mouse_pressed = false;
                    return true;
                };
                self.tab_mouse_pressed = true;
                if *button != MouseButton::Left {
                    return true;
                };
                let x = self.pointer_position.x - size.padding_x() as f64;
                let y = self.pointer_position.y - size.padding_y() as f64;
                if x < 0. || y < 0. {
                    Some(LanguageDialogAction::Cancel)
                } else {
                    dialog.click(
                        (x / size.cell_width() as f64) as usize,
                        (y / size.cell_height() as f64) as usize,
                        size.columns(),
                        size.screen_lines(),
                    )
                }
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::Ime(_) | WindowEvent::MouseWheel { .. } | WindowEvent::Touch(_),
                ..
            } => return true,
            _ => return false,
        };
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        match action {
            Some(LanguageDialogAction::Save(language)) => {
                let _ = proxy.send_event(Event::new(EventType::SetLanguage(language), self.id()));
            },
            Some(LanguageDialogAction::Cancel) => self.close_language_settings(),
            Some(LanguageDialogAction::Preferences) => self.open_preferences(),
            None => (),
        }
        true
    }

    fn open_tab_dialog(&mut self, index: usize) {
        self.dismiss_close_confirmation();
        if self.display.tab_bar.is_none() {
            return;
        }
        self.close_productivity(true);
        self.close_context_menu();
        self.close_language_settings();
        let Some(tab) = self.tabs.get_mut(index) else { return };
        self.tab_dialog_pane = Some(tab.panes.active.id);
        let mut snapshot = tab.panes.active.snapshot();
        snapshot.title = tab.custom_title.clone();
        self.display.tab_dialog = Some(TabDialog::new(
            tab.id,
            snapshot.title.unwrap_or_default(),
            snapshot.working_directory,
            snapshot.commands,
            snapshot.replay_last_command,
            tab.integration.is_some(),
        ));
        if let Some(dialog) = &mut self.display.tab_dialog {
            dialog.locale = self.config.general.language.resolve();
        }
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, false);
        self.display.ime.set_preedit(None);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
    }

    fn apply_dialog_settings(&mut self) {
        let Some(dialog) = &self.display.tab_dialog else { return };
        let id = dialog.tab_id;
        let title = dialog.name.trim().to_owned();
        let replay = dialog.replay_last_command;
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.custom_title = (!title.is_empty()).then_some(title);
            let Some(tab) = tab.panes.iter_mut().find(|pane| Some(pane.id) == self.tab_dialog_pane)
            else {
                return;
            };
            tab.replay_last_command = replay;
            if !replay {
                tab.pending_replay = None;
                tab.replay_started = None;
            }
        }
        self.display.window.set_title(self.tabs.active.display_title().to_owned());
        self.refresh_tab_bar();
    }

    fn close_tab_dialog(&mut self) {
        self.tab_dialog_pane = None;
        self.display.tab_dialog = None;
        self.display.ime.set_preedit(None);
        self.display.window.set_ime_inhibitor(
            ImeInhibitor::VI,
            self.tabs.active.terminal.lock().mode().contains(TermMode::VI),
        );
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.damage_tracker.next_frame().mark_fully_damaged();
        self.dirty = true;
    }

    fn session_notice(&mut self, notice: Message) {
        if self.display.tab_dialog.is_none() {
            self.open_tab_dialog(self.tabs.active_index);
        }
        if let Some(dialog) = &mut self.display.tab_dialog {
            dialog.status = Some(notice);
        }
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.dirty = true;
    }

    fn save_session(&mut self) {
        self.apply_dialog_settings();
        let Some(path) = SessionStore::default_path() else {
            self.session_notice(Message::StateDirectoryFailed);
            return;
        };
        let snapshot = SavedSession {
            version: 1,
            active: self.tabs.active_index,
            tabs: self.tabs.iter_mut().map(TerminalTab::snapshot).collect(),
        };
        let store = SessionStore::new(path);
        let notice = match store.save(&snapshot) {
            Ok(()) => Message::SessionSaved,
            Err(_) => Message::SessionSaveFailed,
        };
        self.session_notice(notice);
    }

    fn restore_session(&mut self, proxy: &EventLoopProxy<Event>, scheduler: &mut Scheduler) {
        let snapshot = match Self::load_saved_session() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.session_notice(Message::NoSavedSession);
                return;
            },
            Err(_) => {
                self.session_notice(Message::SessionReadFailed);
                return;
            },
        };
        self.restore_snapshot(snapshot, proxy, scheduler);
    }

    fn restore_snapshot(
        &mut self,
        snapshot: SavedSession,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        let mut prepared = Vec::new();
        let base = self.tabs.len();
        for saved in snapshot.tabs {
            match TerminalTab::restore(
                &self.display,
                &self.config,
                saved,
                proxy,
                &mut self.next_tab_id,
                true,
            ) {
                Ok(tab) => prepared.push(tab),
                Err(_) => {
                    self.session_notice(Message::SessionRestoreFailed);
                    return;
                },
            }
        }
        self.close_tab_dialog();
        self.close_productivity(false);
        for tab in prepared {
            self.tabs.push(tab);
        }
        self.select_tab(base + snapshot.active, proxy, scheduler);
        self.poll_session_restore(scheduler);
    }

    fn poll_session_restore(&mut self, scheduler: &mut Scheduler) {
        let timer = TimerId::new(Topic::SessionRestore, self.id());
        scheduler.unschedule(timer);
        for index in 0..self.tabs.len() {
            for pane_index in 0..self.tabs.get(index).unwrap().panes.len() {
                self.replay_pending_command(index, pane_index);
            }
        }
        if self
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.iter())
            .any(|pane| pane.pending_replay.is_some())
        {
            scheduler.schedule(
                Event::new(EventType::RestorePending, self.id()),
                Duration::from_millis(100),
                false,
                timer,
            );
        }
    }

    fn replay_pending_command(&mut self, index: usize, pane_index: usize) {
        let tab = self.tabs.get_mut(index).unwrap().panes.get_mut(pane_index).unwrap();
        if !tab.replay_last_command {
            tab.pending_replay = None;
            tab.replay_started = None;
        }
        if tab.pending_replay.is_none() {
            return;
        }
        if tab.replay_started.is_some_and(|start| start.elapsed() >= Duration::from_secs(60)) {
            tab.pending_replay = None;
            self.session_notice(Message::ShellNotReady);
            return;
        }
        if !tab.refresh_history() {
            return;
        }
        let command = tab.pending_replay.take().unwrap();
        if !command.working_directory.is_dir() {
            self.session_notice(Message::CommandDirectoryMissing);
            return;
        }
        match tab.integration.as_ref().unwrap().replay(&command) {
            Ok(text) => tab.notifier.notify(text.into_bytes()),
            Err(_) => self.session_notice(Message::ReplayFailed),
        }
    }

    fn handle_dialog_event(
        &mut self,
        event: &WinitEvent<Event>,
        clipboard: &mut Clipboard,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) -> bool {
        let Some(dialog) = &mut self.display.tab_dialog else { return false };
        let modifiers = self.modifiers.state();
        let action = match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::KeyboardInput { event, is_synthetic: false, .. },
                ..
            } => {
                if event.state != ElementState::Pressed {
                    return true;
                }
                let key = match &event.logical_key {
                    Key::Character(text) => Some(text.as_str()),
                    _ => None,
                };
                if modifiers.control_key()
                    && modifiers.shift_key()
                    && key.is_some_and(|key| key.eq_ignore_ascii_case("s"))
                {
                    Some(DialogAction::Save)
                } else if modifiers.control_key()
                    && modifiers.shift_key()
                    && key.is_some_and(|key| key.eq_ignore_ascii_case("o"))
                {
                    Some(DialogAction::Restore)
                } else if modifiers.control_key()
                    && key.is_some_and(|key| key.eq_ignore_ascii_case("v"))
                    && dialog.wants_paste()
                {
                    dialog.insert_text(&clipboard.load(ClipboardType::Clipboard));
                    None
                } else {
                    dialog.key(event, modifiers)
                }
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::Ime(winit::event::Ime::Commit(text)),
                ..
            } => {
                if dialog.wants_paste() {
                    dialog.insert_text(text);
                }
                None
            },
            WinitEvent::WindowEvent { event: WindowEvent::Ime(_), .. } => None,
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                self.pointer_position = *position;
                self.display.window.set_mouse_visible(true);
                let size = self.display.size_info;
                let x = self.pointer_position.x - size.padding_x() as f64;
                let y = self.pointer_position.y - size.padding_y() as f64;
                let column =
                    if x < 0. { usize::MAX } else { (x / size.cell_width() as f64) as usize };
                let line =
                    if y < 0. { usize::MAX } else { (y / size.cell_height() as f64) as usize };
                self.dirty |= dialog.hover(column, line, size.columns(), size.screen_lines());
                let input =
                    dialog.elements(size.columns(), size.screen_lines()).iter().any(|element| {
                        element.kind == crate::tab_ui::UiKind::Input
                            && element.rect.contains(column, line)
                    });
                self.display.window.set_mouse_cursor(if input {
                    CursorIcon::Text
                } else if dialog.pointer_cursor(column, line, size.columns(), size.screen_lines()) {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                });
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                self.display.window.set_mouse_visible(true);
                if *state == ElementState::Released {
                    self.tab_mouse_pressed = false;
                    return true;
                }
                self.tab_mouse_pressed = true;
                if *button != MouseButton::Left {
                    return true;
                }
                let size = self.display.size_info;
                let x = self.pointer_position.x - size.padding_x() as f64;
                let y = self.pointer_position.y - size.padding_y() as f64;
                if x < 0. || y < 0. {
                    Some(DialogAction::Cancel)
                } else {
                    dialog.click(
                        (x / size.cell_width() as f64) as usize,
                        (y / size.cell_height() as f64) as usize,
                        size.columns(),
                        size.screen_lines(),
                    )
                }
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseWheel { .. } | WindowEvent::Touch(_),
                ..
            } => return true,
            _ => return false,
        };
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        match action {
            Some(DialogAction::Apply) => {
                self.apply_dialog_settings();
                self.close_tab_dialog();
            },
            Some(DialogAction::Save) => self.save_session(),
            Some(DialogAction::Restore) => self.restore_session(proxy, scheduler),
            Some(DialogAction::Cancel) => self.close_tab_dialog(),
            None => (),
        }
        true
    }

    fn refresh_tab_bar(&mut self) {
        if let Some(bar) = &mut self.display.tab_bar {
            bar.titles = self.tabs.iter().map(|tab| tab.display_title().to_owned()).collect();
            bar.active = self.tabs.active_index;
            bar.activities = self
                .tabs
                .iter()
                .map(|tab| {
                    let states: Vec<_> = tab.panes.iter().map(|pane| pane.activity).collect();
                    if states.contains(&TabActivity::Running) {
                        TabActivity::Running
                    } else if states.contains(&TabActivity::Failed) {
                        TabActivity::Failed
                    } else if states.contains(&TabActivity::Success) {
                        TabActivity::Success
                    } else {
                        TabActivity::Idle
                    }
                })
                .collect();
            self.dirty = true;
            self.display.damage_tracker.frame().mark_fully_damaged();
        }
    }

    fn consume_close_input(&mut self, event: &WinitEvent<Event>) -> bool {
        match event {
            WinitEvent::WindowEvent { event: WindowEvent::KeyboardInput { event, .. }, .. }
                if self.close_confirmation_key == Some(event.physical_key) =>
            {
                if event.state == ElementState::Released {
                    self.close_confirmation_key = None;
                }
                true
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state: ElementState::Released, button, .. },
                ..
            } if self.close_mouse_release == Some(*button) => {
                self.close_mouse_release = None;
                self.tab_mouse_pressed = false;
                true
            },
            WinitEvent::WindowEvent { event: WindowEvent::Focused(false), .. } => {
                self.close_confirmation_key = None;
                self.close_mouse_release = None;
                false
            },
            _ => false,
        }
    }

    fn dismiss_close_confirmation(&mut self) {
        self.closing_pane = None;
        if self.display.close_confirmation.take().is_none() {
            return;
        }
        let vi = self.tabs.active.terminal.lock().mode().contains(TermMode::VI);
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, vi);
        self.display.window.set_mouse_cursor(CursorIcon::Text);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.damage_tracker.next_frame().mark_fully_damaged();
    }

    fn close_tab_immediately(
        &mut self,
        id: TabId,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        let index = self.tabs.iter().position(|tab| tab.id == id);
        if let Some(index) = index {
            self.remove_tab(index, proxy, scheduler);
        }
    }

    fn request_close_tab(
        &mut self,
        index: usize,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        if self.display.close_confirmation.is_some() {
            return;
        }
        let Some(tab) = self.tabs.get(index) else { return };
        let id = tab.id;
        let title = tab.display_title().to_owned();
        self.closing_pane = None;
        let confirm = match &self.display.productivity_dialog {
            Some(ProductivityDialog::Preferences(dialog)) => {
                dialog.original.general.confirm_tab_close
            },
            _ => self.config.general.confirm_tab_close,
        };
        if !confirm {
            self.close_tab_immediately(id, proxy, scheduler);
            return;
        }
        self.close_context_menu();
        self.close_tab_dialog();
        self.close_language_settings();
        self.close_productivity(true);
        self.cancel_mouse_drag(scheduler);
        self.display.close_confirmation =
            Some(CloseConfirmation::new(id, title, self.config.general.language.resolve()));
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, true);
        self.display.ime.set_preedit(None);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
    }

    fn handle_close_confirmation(
        &mut self,
        event: &WinitEvent<Event>,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) -> bool {
        let Some(dialog) = &mut self.display.close_confirmation else { return false };
        let size = self.display.size_info;
        let action = match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::KeyboardInput { event, is_synthetic: false, .. },
                ..
            } => {
                if event.state != ElementState::Pressed || event.repeat {
                    return true;
                }
                let action = dialog.key(event, self.modifiers.state());
                if action.is_some() {
                    self.close_confirmation_key = Some(event.physical_key);
                }
                action
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                self.pointer_position = *position;
                let column = if position.x < size.padding_x() as f64 {
                    usize::MAX
                } else {
                    ((position.x - size.padding_x() as f64) / size.cell_width() as f64) as usize
                };
                let line = if position.y < size.padding_y() as f64 {
                    usize::MAX
                } else {
                    ((position.y - size.padding_y() as f64) / size.cell_height() as f64) as usize
                };
                self.dirty |= dialog.hover(column, line, size.columns(), size.screen_lines());
                self.display.window.set_mouse_visible(true);
                self.display.window.set_mouse_cursor(
                    if dialog.pointer_cursor(column, line, size.columns(), size.screen_lines()) {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    },
                );
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                self.tab_mouse_pressed = *state == ElementState::Pressed;
                if *state != ElementState::Pressed || *button != MouseButton::Left {
                    return true;
                }
                let x = self.pointer_position.x - size.padding_x() as f64;
                let y = self.pointer_position.y - size.padding_y() as f64;
                let action = if x < 0. || y < 0. {
                    Some(CloseAction::Cancel)
                } else {
                    dialog.click(
                        (x / size.cell_width() as f64) as usize,
                        (y / size.cell_height() as f64) as usize,
                        size.columns(),
                        size.screen_lines(),
                    )
                };
                if action.is_some() {
                    self.close_mouse_release = Some(*button);
                }
                action
            },
            WinitEvent::WindowEvent {
                event:
                    WindowEvent::Ime(_)
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::Touch(_)
                    | WindowEvent::DroppedFile(_)
                    | WindowEvent::HoveredFile(_)
                    | WindowEvent::HoveredFileCancelled,
                ..
            } => return true,
            WinitEvent::UserEvent(Event { payload: EventType::ClipboardPaste, .. }) => return true,
            _ => return false,
        };
        let id = dialog.tab_id;
        let pane = self.closing_pane;
        if let Some(action) = action {
            self.dismiss_close_confirmation();
            if action == CloseAction::Confirm {
                if pane.is_some() {
                    self.close_pane_immediately(id, proxy, scheduler);
                } else {
                    self.close_tab_immediately(id, proxy, scheduler);
                }
            }
        }
        true
    }

    fn create_tab(
        &mut self,
        options: WindowOptions,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        match TabContext::new(&self.display, &self.config, options, proxy.clone(), id, None) {
            Ok(tab) => {
                tab.terminal.lock().is_focused = false;
                self.tabs.push(TerminalTab::single(tab));
                self.select_tab(self.tabs.len() - 1, proxy, scheduler);
            },
            Err(err) => error!("Could not open tab: {err}"),
        }
    }

    fn cancel_tab_timers(&self, scheduler: &mut Scheduler) {
        for topic in [
            Topic::SelectionScrolling,
            Topic::DelayedSearch,
            Topic::BlinkCursor,
            Topic::BlinkTimeout,
        ] {
            scheduler.unschedule(TimerId::new(topic, self.id()));
        }
    }

    fn select_tab(
        &mut self,
        index: usize,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        if index >= self.tabs.len() || index == self.tabs.active_index {
            return;
        }
        let focused = self.tabs.active.terminal.lock().is_focused;
        self.set_tab_focus(false);
        self.tabs.select(index);
        if matches!(self.tabs.active.activity, TabActivity::Success | TabActivity::Failed) {
            self.tabs.active.activity = TabActivity::Idle;
        }
        self.activate_tab(focused, proxy, scheduler);
    }

    fn set_tab_focus(&mut self, focused: bool) {
        let tab = &mut self.tabs.active.panes.active;
        let mut terminal = tab.terminal.lock();
        if terminal.is_focused != focused && terminal.mode().contains(TermMode::FOCUS_IN_OUT) {
            tab.notifier.notify(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }
        terminal.is_focused = focused;
    }

    fn activate_tab(
        &mut self,
        focused: bool,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        self.cancel_tab_timers(scheduler);
        self.pane_drag = None;
        self.display.split_active = self.tabs.active.panes.len() > 1;
        self.set_tab_focus(focused);
        self.display.window.set_title(self.tabs.active.display_title().to_owned());
        self.display.cursor_hidden = false;
        self.display.highlighted_hint = None;
        self.display.vi_highlighted_hint = None;
        self.display.hint_state = HintState::new(self.config.hints.alphabet());
        self.display.ime.set_preedit(None);
        self.display.window.set_ime_inhibitor(
            ImeInhibitor::VI,
            self.display.close_confirmation.is_some()
                || self.tabs.active.terminal.lock().mode().contains(TermMode::VI),
        );
        self.mouse.left_button_state = ElementState::Released;
        self.mouse.middle_button_state = ElementState::Released;
        self.mouse.right_button_state = ElementState::Released;
        self.touch = Default::default();
        self.display.pending_update.dirty = true;
        self.refresh_tab_bar();
        let event_proxy =
            EventProxy::for_tab(proxy.clone(), self.id(), self.tabs.active.panes.active.id);
        event_proxy.send_event(TerminalEvent::CursorBlinkingChange.into());
        if self.tabs.active.search_state.history_index.is_some() {
            let _ = proxy.send_event(
                Event::new(EventType::SearchNext, self.id())
                    .with_tab(Some(self.tabs.active.panes.active.id)),
            );
        }
    }

    fn resize_inactive_tabs(&mut self) {
        let size = self.display.size_info;
        let padding = self.config.window.padding(self.display.window.scale_factor as f32);
        for tab in self.tabs.iter_mut() {
            let mut full = crate::display::SizeInfo::new(
                size.width(),
                size.height(),
                size.cell_width(),
                size.cell_height(),
                padding.0,
                padding.1,
                self.config.window.dynamic_padding,
            );
            if self.display.tab_bar.is_some() {
                full.reserve_top_lines(crate::tabs::TabBar::ROWS);
            }
            let message_lines = self.message_buffer.message().map_or(0, |message| {
                message.text_with_locale(&full, self.config.general.language.resolve()).len()
            });
            full.reserve_lines(message_lines);
            let rectangles = tab.rectangles(full);
            for pane in tab.panes.iter_mut() {
                let rect = rectangles.iter().find(|(id, _)| *id == pane.id).map(|(_, rect)| *rect);
                let Some(rect) = rect else {
                    continue;
                };
                let mut pane_size = full.pane(rect.x, rect.y, rect.width, rect.height);
                pane_size.reserve_lines(usize::from(pane.search_state.history_index.is_some()));
                let mut terminal = pane.terminal.lock();
                if terminal.columns() != pane_size.columns()
                    || terminal.screen_lines() != pane_size.screen_lines()
                {
                    pane.notifier.on_resize(pane_size.into());
                    terminal.resize(pane_size);
                    pane.search_state.clear_focused_match();
                }
            }
        }
    }

    fn route_terminal_event(
        &mut self,
        id: TabId,
        event: TerminalEvent,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) -> Option<TerminalEvent> {
        let (index, pane_index) = self.tabs.iter().enumerate().find_map(|(index, tab)| {
            tab.panes.iter().position(|pane| pane.id == id).map(|pane| (index, pane))
        })?;
        if matches!(event, TerminalEvent::Wakeup) {
            self.replay_pending_command(index, pane_index);
        }
        let active =
            index == self.tabs.active_index && pane_index == self.tabs.active.panes.active_index;
        if matches!(event, TerminalEvent::Exit) {
            if self.tabs.get(index).unwrap().panes.get(pane_index).unwrap().hold {
                return None;
            }
            if self.closing_pane == Some(id) {
                self.dismiss_close_confirmation();
            }
            if self.tab_dialog_pane == Some(id) {
                self.close_tab_dialog();
            }
            if self.context_menu_pane == Some(id) {
                self.close_context_menu();
            }
            if self.tabs.get(index).unwrap().panes.len() == 1 {
                self.remove_tab(index, proxy, scheduler);
            } else {
                let focused = self.tabs.active.terminal.lock().is_focused;
                if active {
                    self.set_tab_focus(false);
                }
                let tab = self.tabs.get_mut(index).unwrap();
                tab.layout.remove(id);
                tab.panes.remove(pane_index);
                tab.zoomed = false;
                if index == self.tabs.active_index {
                    self.activate_tab(focused, proxy, scheduler);
                }
                self.resize_inactive_tabs();
                self.refresh_tab_bar();
            }
            return None;
        }
        match &event {
            TerminalEvent::Title(title) => {
                let tab = self.tabs.get_mut(index).unwrap().panes.get_mut(pane_index).unwrap();
                if !tab.preserve_title && self.config.window.dynamic_title {
                    tab.title = title.clone();
                    self.refresh_tab_bar();
                }
            },
            TerminalEvent::ResetTitle => {
                let tab = self.tabs.get_mut(index).unwrap().panes.get_mut(pane_index).unwrap();
                if !tab.preserve_title && self.config.window.dynamic_title {
                    tab.title = self.config.window.identity.title.clone();
                    self.refresh_tab_bar();
                }
            },
            _ => (),
        }
        if active {
            if matches!(event, TerminalEvent::Wakeup) {
                self.dirty = true;
            }
            return Some(event);
        }
        if index == self.tabs.active_index {
            self.dirty = true;
        }
        let tab = self.tabs.get_mut(index).unwrap().panes.get_mut(pane_index).unwrap();
        match event {
            TerminalEvent::PtyWrite(text) => tab.notifier.notify(text.into_bytes()),
            TerminalEvent::ColorRequest(index, format) => {
                let color = match tab.terminal.lock().colors()[index] {
                    Some(color) => Rgb(color),
                    None if index == NamedColor::Cursor as usize => return None,
                    None => self.display.colors[index],
                };
                tab.notifier.notify(format(color.0).into_bytes());
            },
            TerminalEvent::TextAreaSizeRequest(format) => {
                let mut size: alacritty_terminal::event::WindowSize = self.display.size_info.into();
                let terminal = tab.terminal.lock();
                size.num_lines = terminal.screen_lines() as u16;
                size.num_cols = terminal.columns() as u16;
                tab.notifier.notify(format(size).into_bytes());
            },
            TerminalEvent::Bell => {
                let urgency = tab.terminal.lock().mode().contains(TermMode::URGENCY_HINTS);
                if urgency && !self.tabs.active.terminal.lock().is_focused {
                    self.display.window.set_urgent(true);
                }
            },
            _ => (),
        }
        None
    }

    fn cancel_mouse_drag(&mut self, scheduler: &mut Scheduler) {
        if let Some(bar) = &mut self.display.tab_bar {
            self.dirty |= bar.hover(None);
        }
        self.tab_drag = None;
        self.pane_drag = None;
        self.mouse.left_button_state = ElementState::Released;
        self.mouse.middle_button_state = ElementState::Released;
        self.mouse.right_button_state = ElementState::Released;
        scheduler.unschedule(TimerId::new(Topic::SelectionScrolling, self.id()));
    }

    fn pointer_cell(&self) -> (usize, usize) {
        let size = self.display.size_info;
        let column = ((self.pointer_position.x - size.padding_x() as f64)
            / size.cell_width() as f64)
            .max(0.) as usize;
        let line = ((self.pointer_position.y - size.padding_y() as f64) / size.cell_height() as f64)
            .max(0.) as usize;
        (column, line)
    }

    fn open_context_menu(&mut self, index: usize) {
        self.dismiss_close_confirmation();
        let Some(tab) = self.tabs.get(index) else { return };
        let id = tab.id;
        self.context_menu_pane = Some(tab.panes.active.id);
        let copy_enabled =
            tab.terminal.lock().selection_to_string().is_some_and(|text| !text.is_empty());
        let (column, line) = self.pointer_cell();
        let mut menu = ContextMenu::new(id, column, line);
        menu.copy_enabled = copy_enabled;
        menu.shortcuts = self.config.general.shortcuts.clone();
        menu.locale = self.config.general.language.resolve();
        self.display.context_menu = Some(menu);
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, true);
        self.display.ime.set_preedit(None);
        self.display.window.set_mouse_cursor(CursorIcon::Default);
        self.mouse.inside_text_area = false;
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
    }

    fn close_context_menu(&mut self) {
        self.context_menu_pane = None;
        if self.display.context_menu.take().is_none() {
            return;
        };
        let vi_mode = self.tabs.active.terminal.lock().mode().contains(TermMode::VI);
        self.display.window.set_ime_inhibitor(ImeInhibitor::VI, vi_mode);
        self.display.window.set_mouse_cursor(CursorIcon::Text);
        self.dirty = true;
        self.display.damage_tracker.frame().mark_fully_damaged();
        self.display.damage_tracker.next_frame().mark_fully_damaged();
    }

    fn open_context_menu_from_event(&mut self, event: &WinitEvent<Event>) -> bool {
        if self.display.tab_bar.is_none() {
            return false;
        };
        let WinitEvent::WindowEvent {
            event: WindowEvent::MouseInput { state: ElementState::Pressed, button, .. },
            ..
        } = event
        else {
            return false;
        };
        let size = self.display.size_info;
        let top =
            self.pointer_position.y >= 0. && self.pointer_position.y < size.padding_y() as f64;
        let (column, _) = self.pointer_cell();
        let hit = top
            .then(|| self.display.tab_bar.as_ref().unwrap().hit(column, size.columns()))
            .flatten();
        let toolbar = *button == MouseButton::Left && hit == Some(TabButton::Menu);
        if *button != MouseButton::Right && !toolbar {
            return false;
        };
        if !top && !self.modifiers.state().shift_key() {
            let terminal = self.tabs.active.terminal.lock();
            if terminal.mode().intersects(TermMode::MOUSE_MODE)
                && !terminal.mode().contains(TermMode::VI)
            {
                return false;
            };
        }
        let index = match hit {
            Some(TabButton::Select(index) | TabButton::Close(index)) => index,
            _ => self.tabs.active_index,
        };
        self.tab_mouse_pressed = true;
        self.open_context_menu(index);
        true
    }

    fn handle_context_menu_event(&mut self, event: &WinitEvent<Event>) -> bool {
        if self.display.context_menu.is_none() {
            return false;
        };
        let size = self.display.size_info;
        let (mut column, mut line) = self.pointer_cell();
        if self.pointer_position.x < size.padding_x() as f64 {
            column = usize::MAX;
        }
        if self.pointer_position.y < size.padding_y() as f64 {
            line = usize::MAX;
        }
        let action = match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::KeyboardInput { event, is_synthetic: false, .. },
                ..
            } => {
                if event.state != ElementState::Pressed {
                    return true;
                };
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    self.close_context_menu();
                    return true;
                }
                let result = self.display.context_menu.as_mut().unwrap().key(
                    event,
                    size.columns(),
                    size.screen_lines(),
                );
                self.dirty = true;
                result
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                self.pointer_position = *position;
                let (mut column, mut line) = self.pointer_cell();
                if position.x < size.padding_x() as f64 {
                    column = usize::MAX;
                }
                if position.y < size.padding_y() as f64 {
                    line = usize::MAX;
                }
                let menu = self.display.context_menu.as_mut().unwrap();
                self.dirty |= menu.hover(column, line, size.columns(), size.screen_lines());
                self.display.window.set_mouse_visible(true);
                let scroll_control =
                    menu.decorations(size.columns(), size.screen_lines()).iter().any(|element| {
                        element.kind == crate::tab_ui::UiKind::MenuItem
                            && !element.disabled
                            && element.rect.contains(column, line)
                    });
                self.display.window.set_mouse_cursor(
                    if menu.selected.is_some() || scroll_control {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    },
                );
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                if *state == ElementState::Released {
                    self.tab_mouse_pressed = false;
                    return true;
                }
                self.tab_mouse_pressed = true;
                let menu = self.display.context_menu.as_mut().unwrap();
                if *button == MouseButton::Left
                    && menu.hit_scroll(column, line, size.columns(), size.screen_lines())
                {
                    self.dirty = true;
                    return true;
                }
                let action = if *button == MouseButton::Left {
                    menu.click(column, line, size.columns(), size.screen_lines())
                } else {
                    None
                };
                if action.is_none() {
                    if *button != MouseButton::Left
                        || !menu.layout(size.columns(), size.screen_lines()).contains(column, line)
                    {
                        self.close_context_menu();
                    }
                    return true;
                }
                action
            },
            WinitEvent::WindowEvent { event: WindowEvent::Focused(false), .. } => {
                self.close_context_menu();
                return false;
            },
            WinitEvent::WindowEvent { event: WindowEvent::MouseWheel { delta, .. }, .. } => {
                let vertical = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
                    winit::event::MouseScrollDelta::PixelDelta(position) => position.y,
                };
                if vertical != 0. {
                    self.dirty |= self.display.context_menu.as_mut().unwrap().scroll(
                        -vertical.signum() as isize,
                        size.columns(),
                        size.screen_lines(),
                    );
                }
                return true;
            },
            WinitEvent::WindowEvent {
                event: WindowEvent::Touch(_) | WindowEvent::Ime(_), ..
            } => return true,
            _ => return false,
        };
        let Some(action) = action else { return true };
        let id = self.display.context_menu.as_ref().unwrap().tab_id;
        self.close_context_menu();
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else { return true };
        match action {
            MenuAction::NewTab => {
                let mut options = WindowOptions::default();
                options.terminal_options.working_directory =
                    self.tabs.get_mut(index).unwrap().panes.active.snapshot().working_directory;
                self.tab_actions.push(EventType::CreateTab(options));
            },
            MenuAction::Settings => self.open_preferences(),
            MenuAction::SplitVertical
            | MenuAction::SplitHorizontal
            | MenuAction::PaneZoom
            | MenuAction::ClosePane => {
                self.tab_actions.push(EventType::SelectTab(TabAction::Index(index)));
                self.tab_actions.push(EventType::Pane(match action {
                    MenuAction::SplitVertical => PaneAction::Split(Axis::Vertical),
                    MenuAction::SplitHorizontal => PaneAction::Split(Axis::Horizontal),
                    MenuAction::PaneZoom => PaneAction::Zoom,
                    _ => PaneAction::Close,
                }));
            },
            MenuAction::Duplicate => self.tab_actions.push(EventType::DuplicateTab(Some(index))),
            MenuAction::ReopenClosed => self.tab_actions.push(EventType::ReopenClosedTab),
            MenuAction::Workspaces => self.open_workspaces(),
            MenuAction::Rename => self.tab_actions.push(EventType::RenameTab(Some(index))),
            MenuAction::Save => self.tab_actions.push(EventType::SaveSession),
            MenuAction::Restore => self.tab_actions.push(EventType::RestoreSession),
            MenuAction::Close => self.tab_actions.push(EventType::CloseTab(Some(index))),
            MenuAction::Copy | MenuAction::Paste => {
                self.tab_actions.push(EventType::SelectTab(TabAction::Index(index)));
                self.tab_actions.push(if action == MenuAction::Copy {
                    EventType::ClipboardCopy
                } else {
                    EventType::ClipboardPaste
                });
            },
        }
        true
    }

    fn handle_tab_mouse(&mut self, event: &WinitEvent<Event>) -> bool {
        if self.display.tab_bar.is_none() {
            return false;
        }
        if let WinitEvent::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. }, ..
        } = event
        {
            self.pointer_position = *position;
            let size = self.display.size_info;
            let x = position.x - size.padding_x() as f64;
            let hit = if x >= 0. && position.y >= 0. && position.y < size.padding_y() as f64 {
                self.display
                    .tab_bar
                    .as_ref()
                    .unwrap()
                    .hit((x / size.cell_width() as f64) as usize, size.columns())
            } else {
                None
            };
            self.dirty |= self.display.tab_bar.as_mut().unwrap().hover(hit);
            self.display.window.set_mouse_visible(true);
            if self.tab_mouse_pressed {
                if let Some(id) = self.tab_drag {
                    let size = self.display.size_info;
                    let x = position.x - size.padding_x() as f64;
                    if x >= 0. && position.y >= 0. && position.y < size.padding_y() as f64 {
                        let hit = self
                            .display
                            .tab_bar
                            .as_ref()
                            .unwrap()
                            .hit((x / size.cell_width() as f64) as usize, size.columns());
                        if let Some(TabButton::Select(to) | TabButton::Close(to)) = hit {
                            let from = self.tabs.iter().position(|tab| tab.id == id);
                            if let Some(from) = from {
                                self.reorder_tab(from, to);
                            }
                        }
                    }
                    return true;
                }
            }
            if position.y >= 0.
                && position.y < self.display.size_info.padding_y() as f64
                && self.mouse.left_button_state != ElementState::Pressed
            {
                self.mouse.inside_text_area = false;
                self.display.window.set_mouse_cursor(if hit.is_some() {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                });
                return true;
            }
        }
        if matches!(event, WinitEvent::WindowEvent { event: WindowEvent::CursorLeft { .. }, .. }) {
            self.dirty |= self.display.tab_bar.as_mut().unwrap().hover(None);
        }
        let top = self.pointer_position.y >= 0.
            && self.pointer_position.y < self.display.size_info.padding_y() as f64;
        match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } if (*state == ElementState::Pressed && top) || self.tab_mouse_pressed => {
                self.display.window.set_mouse_visible(true);
                if *state == ElementState::Released {
                    self.tab_mouse_pressed = false;
                    self.tab_drag = None;
                    return true;
                }
                self.tab_mouse_pressed = true;
                let size = self.display.size_info;
                let x = self.pointer_position.x - size.padding_x() as f64;
                if x < 0. || !top {
                    return true;
                }
                let column = (x / size.cell_width() as f64) as usize;
                let bar = self.display.tab_bar.as_ref().unwrap();
                let action = match (bar.hit(column, size.columns()), button) {
                    (Some(TabButton::Create), MouseButton::Left) => {
                        let mut options = WindowOptions::default();
                        #[cfg(not(windows))]
                        {
                            options.terminal_options.working_directory = foreground_process_path(
                                self.tabs.active.master_fd,
                                self.tabs.active.shell_pid,
                            )
                            .ok();
                        }
                        Some(EventType::CreateTab(options))
                    },
                    (Some(TabButton::Close(index)), MouseButton::Left)
                    | (
                        Some(TabButton::Select(index) | TabButton::Close(index)),
                        MouseButton::Middle,
                    ) => Some(EventType::CloseTab(Some(index))),
                    (Some(TabButton::Menu), MouseButton::Left) => None,
                    (Some(TabButton::Select(index)), MouseButton::Left) => {
                        self.tab_drag = self.tabs.get(index).map(|tab| tab.id);
                        Some(EventType::SelectTab(TabAction::Index(index)))
                    },
                    _ => None,
                };
                if let Some(action) = action {
                    self.tab_actions.push(action);
                }
                true
            },
            WinitEvent::WindowEvent { event: WindowEvent::MouseWheel { .. }, .. } if top => true,
            _ => false,
        }
    }

    #[cfg(unix)]
    pub fn config(&self) -> &UiConfig {
        &self.config
    }

    /// Clear the window config overrides.
    #[cfg(unix)]
    pub fn reset_window_config(&mut self, config: Rc<UiConfig>) {
        // Clear previous window errors.
        self.message_buffer.remove_target(LOG_TARGET_IPC_CONFIG);

        self.window_config.clear();

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Add new window config overrides.
    #[cfg(unix)]
    pub fn add_window_config(&mut self, config: Rc<UiConfig>, options: &ParsedOptions) {
        // Clear previous window errors.
        self.message_buffer.remove_target(LOG_TARGET_IPC_CONFIG);

        self.window_config.extend_from_slice(options);

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Draw the window.
    pub fn draw(&mut self, scheduler: &mut Scheduler) {
        self.display.window.requested_redraw = false;

        if self.occluded {
            return;
        }

        self.dirty = false;

        // Force the display to process any pending display update.
        self.display.process_renderer_update();

        // Request immediate re-draw if visual bell animation is not finished yet.
        if !self.display.visual_bell.completed() {
            // We can get an OS redraw which bypasses alacritty's frame throttling, thus
            // marking the window as dirty when we don't have frame yet.
            if self.display.window.has_frame {
                self.display.window.request_redraw();
            } else {
                self.dirty = true;
            }
        }

        // Redraw the window.
        let size = self.display.size_info;
        let tab = &mut self.tabs.active;
        if tab.panes.len() == 1 {
            let pane = &mut tab.panes.active;
            self.display.draw(
                pane.terminal.lock(),
                scheduler,
                &self.message_buffer,
                &self.config,
                &mut pane.search_state,
            );
        } else {
            let rectangles = tab.rectangles(size);
            let focused_id = tab.panes.active.id;
            let dividers = if tab.zoomed {
                Vec::new()
            } else {
                tab.layout
                    .layout(size.columns(), size.screen_lines())
                    .dividers
                    .into_iter()
                    .map(|divider| {
                        let rect = divider.rect;
                        crate::renderer::rects::RenderRect::new(
                            size.padding_x() + rect.x as f32 * size.cell_width(),
                            size.padding_y() + rect.y as f32 * size.cell_height(),
                            rect.width as f32 * size.cell_width(),
                            rect.height as f32 * size.cell_height(),
                            self.config.colors.footer_bar_background(),
                            1.,
                        )
                    })
                    .collect()
            };
            let panes = tab
                .panes
                .iter_mut()
                .filter_map(|pane| {
                    let rect =
                        rectangles.iter().find(|(id, _)| *id == pane.id).map(|(_, rect)| rect)?;
                    if rect.width == 0 || rect.height == 0 {
                        return None;
                    }
                    let mut pane_size = size.pane(rect.x, rect.y, rect.width, rect.height);
                    pane_size.reserve_lines(usize::from(pane.search_state.history_index.is_some()));
                    Some(crate::display::PaneRender {
                        terminal: pane.terminal.lock(),
                        search_state: &mut pane.search_state,
                        size: pane_size,
                        focused: pane.id == focused_id,
                    })
                })
                .collect();
            self.display.draw_panes(panes, scheduler, &self.message_buffer, &self.config, dividers);
        }
    }

    /// Process events for this terminal window.
    fn process_tab_actions(
        &mut self,
        #[cfg(target_os = "macos")] event_loop: &ActiveEventLoop,
        event_proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
        scheduler: &mut Scheduler,
    ) {
        for action in mem::take(&mut self.tab_actions) {
            self.process_event(
                #[cfg(target_os = "macos")]
                event_loop,
                event_proxy,
                clipboard,
                scheduler,
                WinitEvent::UserEvent(Event::new(action, self.id())),
            );
        }
        if self.display.pending_update.dirty {
            self.process_event(
                #[cfg(target_os = "macos")]
                event_loop,
                event_proxy,
                clipboard,
                scheduler,
                WinitEvent::UserEvent(Event::new(TerminalEvent::Wakeup.into(), self.id())),
            );
        }
    }

    fn process_event(
        &mut self,
        #[cfg(target_os = "macos")] event_loop: &ActiveEventLoop,
        event_proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
        scheduler: &mut Scheduler,
        event: WinitEvent<Event>,
    ) {
        if self.consume_close_input(&event) {
            return;
        }
        if self.display.tab_dialog.is_some()
            || self.display.context_menu.is_some()
            || self.display.language_dialog.is_some()
            || self.display.productivity_dialog.is_some()
            || self.display.close_confirmation.is_some()
        {
            self.cancel_mouse_drag(scheduler);
            if matches!(event, WinitEvent::UserEvent(Event { payload: EventType::Scroll(_), .. })) {
                return;
            }
        }
        if let WinitEvent::UserEvent(Event { tab_id: Some(id), .. }) = &event {
            if *id != self.tabs.active.panes.active.id {
                return;
            }
        }
        if matches!(&event, WinitEvent::WindowEvent { event: WindowEvent::CloseRequested, .. }) {
            self.persist_recovery();
        }
        let event = match event {
            WinitEvent::UserEvent(Event { payload: EventType::SessionActivity, .. }) => {
                self.poll_activity(scheduler);
                return;
            },
            WinitEvent::UserEvent(Event {
                payload: EventType::OpenWorkspace(snapshot), ..
            }) => {
                self.restore_snapshot(snapshot, event_proxy, scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::OpenWorkspaces, .. }) => {
                self.open_workspaces();
                self.cancel_mouse_drag(scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::DuplicateTab(index), .. }) => {
                if let Some(tab) = self.tabs.get_mut(index.unwrap_or(self.tabs.active_index)) {
                    let mut snapshot = tab.snapshot();
                    snapshot.replay_last_command = false;
                    self.open_fresh_tab(snapshot, event_proxy, scheduler);
                }
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::ReopenClosedTab, .. }) => {
                if let Some(snapshot) = self.recent_closed.last().cloned() {
                    if self.open_fresh_tab(snapshot, event_proxy, scheduler) {
                        self.recent_closed.pop();
                    }
                } else {
                    self.session_notice(Message::NoClosedTabs);
                }
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::MoveTab(previous), .. }) => {
                let from = self.tabs.active_index;
                let to = if previous {
                    from.saturating_sub(1)
                } else {
                    (from + 1).min(self.tabs.len() - 1)
                };
                self.reorder_tab(from, to);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::RestorePending, .. }) => {
                self.poll_session_restore(scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::Pane(action), .. }) => {
                self.pane_action(action, event_proxy, scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::OpenSettings, .. }) => {
                self.open_preferences();
                self.cancel_mouse_drag(scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::RenameTab(index), .. }) => {
                self.open_tab_dialog(index.unwrap_or(self.tabs.active_index));
                self.cancel_mouse_drag(scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::SaveSession, .. }) => {
                self.save_session();
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::RestoreSession, .. }) => {
                self.restore_session(event_proxy, scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::CreateTab(options), .. }) => {
                self.create_tab(options, event_proxy, scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::SelectTab(action), .. }) => {
                self.select_tab(self.tabs.target(action), event_proxy, scheduler);
                return;
            },
            WinitEvent::UserEvent(Event { payload: EventType::CloseTab(index), .. }) => {
                self.request_close_tab(
                    index.unwrap_or(self.tabs.active_index),
                    event_proxy,
                    scheduler,
                );
                return;
            },
            WinitEvent::UserEvent(Event {
                payload: EventType::TabTerminal(id, payload), ..
            }) => match self.route_terminal_event(id, payload, event_proxy, scheduler) {
                Some(payload) => WinitEvent::UserEvent(Event::new(payload.into(), self.id())),
                None => return,
            },
            event => event,
        };
        if self.handle_close_confirmation(&event, event_proxy, scheduler) {
            return;
        }
        if self.handle_productivity_event(&event, event_proxy, clipboard) {
            return;
        }
        if self.handle_language_event(&event, event_proxy) {
            return;
        }
        if self.handle_dialog_event(&event, clipboard, event_proxy, scheduler) {
            return;
        }
        if self.display.context_menu.is_none()
            && self.handle_pane_mouse(&event, event_proxy, scheduler)
        {
            return;
        }
        if self.handle_context_menu_event(&event) || self.open_context_menu_from_event(&event) {
            self.cancel_mouse_drag(scheduler);
            self.process_tab_actions(
                #[cfg(target_os = "macos")]
                event_loop,
                event_proxy,
                clipboard,
                scheduler,
            );
            return;
        }
        if self.handle_tab_mouse(&event) {
            self.process_tab_actions(
                #[cfg(target_os = "macos")]
                event_loop,
                event_proxy,
                clipboard,
                scheduler,
            );
            return;
        }
        let full_size = self.display.size_info;
        self.display.size_info = self.active_pane_size();
        let group_title = self.tabs.active.custom_title.is_some();
        let pane = &mut self.tabs.active.panes.active;
        let mut terminal = pane.terminal.lock();

        let old_is_searching = pane.search_state.history_index.is_some();

        let context = ActionContext {
            cursor_blink_timed_out: &mut pane.cursor_blink_timed_out,
            prev_bell_cmd: &mut pane.prev_bell_cmd,
            message_buffer: &mut self.message_buffer,
            inline_search_state: &mut pane.inline_search_state,
            search_state: &mut pane.search_state,
            modifiers: &mut self.modifiers,
            notifier: &mut pane.notifier,
            display: &mut self.display,
            window_size: full_size,
            mouse: &mut self.mouse,
            touch: &mut self.touch,
            dirty: &mut self.dirty,
            occluded: &mut self.occluded,
            terminal: &mut terminal,
            #[cfg(not(windows))]
            master_fd: pane.master_fd,
            #[cfg(not(windows))]
            shell_pid: pane.shell_pid,
            preserve_title: pane.preserve_title || group_title,
            tab_id: pane.id,
            tab_actions: &mut self.tab_actions,
            config: &self.config,
            event_proxy,
            #[cfg(target_os = "macos")]
            event_loop,
            clipboard,
            scheduler,
        };
        let mut processor = input::Processor::new(context);

        processor.handle_event(event);
        self.display.size_info = full_size;

        // Process DisplayUpdate events.
        if self.display.pending_update.dirty {
            Self::submit_display_update(
                &mut terminal,
                &mut self.display,
                &mut pane.notifier,
                &self.message_buffer,
                &mut pane.search_state,
                old_is_searching,
                &self.config,
            );
            self.dirty = true;
        }

        drop(terminal);
        self.resize_inactive_tabs();
        self.process_tab_actions(
            #[cfg(target_os = "macos")]
            event_loop,
            event_proxy,
            clipboard,
            scheduler,
        );
    }

    pub fn handle_event(
        &mut self,
        #[cfg(target_os = "macos")] event_loop: &ActiveEventLoop,
        event_proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
        scheduler: &mut Scheduler,
        event: WinitEvent<Event>,
    ) {
        match event {
            WinitEvent::AboutToWait
            | WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                if self.event_queue.is_empty() {
                    return;
                }
            },
            event => {
                self.event_queue.push(event);
                return;
            },
        }
        for queued_event in mem::take(&mut self.event_queue) {
            self.process_event(
                #[cfg(target_os = "macos")]
                event_loop,
                event_proxy,
                clipboard,
                scheduler,
                queued_event,
            );
        }
        if self.display.pending_update.dirty {
            self.process_event(
                #[cfg(target_os = "macos")]
                event_loop,
                event_proxy,
                clipboard,
                scheduler,
                WinitEvent::UserEvent(Event::new(TerminalEvent::Wakeup.into(), self.id())),
            );
        }
        if (self.dirty || self.mouse.hint_highlight_dirty)
            && self.display.tab_dialog.is_none()
            && self.display.context_menu.is_none()
            && self.display.language_dialog.is_none()
            && self.display.productivity_dialog.is_none()
            && self.display.close_confirmation.is_none()
        {
            let full_size = self.display.size_info;
            self.display.size_info = self.active_pane_size();
            let terminal = self.tabs.active.terminal.lock();
            self.dirty |= self.display.update_highlighted_hints(
                &terminal,
                &self.config,
                &self.mouse,
                self.modifiers.state(),
            );
            self.display.size_info = full_size;
            self.mouse.hint_highlight_dirty = false;
        }

        // Don't call `request_redraw` when event is `RedrawRequested` since the `dirty` flag
        // represents the current frame, but redraw is for the next frame.
        if self.dirty
            && self.display.window.has_frame
            && !self.occluded
            && !matches!(event, WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. })
        {
            self.display.window.request_redraw();
        }
    }

    /// ID of this terminal context.
    pub fn id(&self) -> WindowId {
        self.display.window.id()
    }

    /// Write the ref test results to the disk.
    pub fn write_ref_test_results(&self) {
        // Dump grid state.
        let mut grid = self.tabs.active.terminal.lock().grid().clone();
        grid.initialize_all();
        grid.truncate();

        let serialized_grid = json::to_string(&grid).expect("serialize grid");

        let size_info = &self.display.size_info;
        let size = TermSize::new(size_info.columns(), size_info.screen_lines());
        let serialized_size = json::to_string(&size).expect("serialize size");

        let serialized_config = format!("{{\"history_size\":{}}}", grid.history_size());

        File::create("./grid.json")
            .and_then(|mut f| f.write_all(serialized_grid.as_bytes()))
            .expect("write grid.json");

        File::create("./size.json")
            .and_then(|mut f| f.write_all(serialized_size.as_bytes()))
            .expect("write size.json");

        File::create("./config.json")
            .and_then(|mut f| f.write_all(serialized_config.as_bytes()))
            .expect("write config.json");
    }

    /// Submit the pending changes to the `Display`.
    fn submit_display_update(
        terminal: &mut Term<EventProxy>,
        display: &mut Display,
        notifier: &mut Notifier,
        message_buffer: &MessageBuffer,
        search_state: &mut SearchState,
        old_is_searching: bool,
        config: &UiConfig,
    ) {
        // Compute cursor positions before resize.
        let num_lines = terminal.screen_lines();
        let cursor_at_bottom = terminal.grid().cursor.point.line + 1 == num_lines;
        let origin_at_bottom = if terminal.mode().contains(TermMode::VI) {
            terminal.vi_mode_cursor.point.line == num_lines - 1
        } else {
            search_state.direction == Direction::Left
        };

        display.handle_update(terminal, notifier, message_buffer, search_state, config);

        let new_is_searching = search_state.history_index.is_some();
        if !old_is_searching && new_is_searching {
            // Scroll on search start to make sure origin is visible with minimal viewport motion.
            let display_offset = terminal.grid().display_offset();
            if display_offset == 0 && cursor_at_bottom && !origin_at_bottom {
                terminal.scroll_display(Scroll::Delta(1));
            } else if display_offset != 0 && origin_at_bottom {
                terminal.scroll_display(Scroll::Delta(-1));
            }
        }
    }
}
