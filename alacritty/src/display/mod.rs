//! The display subsystem including window management, font rasterization, and
//! GPU drawing.

use std::cmp;
use std::fmt::{self, Formatter};
use std::mem::{self, ManuallyDrop};
use std::num::NonZeroU32;
use std::ops::Deref;
use std::time::{Duration, Instant};

use glutin::config::GetGlConfig;
use glutin::context::{NotCurrentContext, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::error::ErrorKind;
use glutin::prelude::*;
use glutin::surface::{Surface, SwapInterval, WindowSurface};

use log::{debug, info};
use parking_lot::MutexGuard;
use serde::{Deserialize, Serialize};
use winit::dpi::PhysicalSize;
use winit::keyboard::ModifiersState;
use winit::raw_window_handle::RawWindowHandle;
use winit::window::CursorIcon;

use crossfont::{Rasterize, Rasterizer, Size as FontSize};
use unicode_width::UnicodeWidthChar;

use alacritty_terminal::event::{EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions as TermDimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{
    self, LineDamageBounds, MIN_COLUMNS, MIN_SCREEN_LINES, Term, TermDamage, TermMode,
};
use alacritty_terminal::vte::ansi::{CursorShape, NamedColor};

use crate::config::UiConfig;
use crate::config::debug::RendererPreference;
use crate::config::font::Font;
use crate::config::window::Dimensions;
#[cfg(not(windows))]
use crate::config::window::StartupMode;
use crate::display::bell::VisualBell;
use crate::display::color::{List, Rgb};
use crate::display::content::{RenderableContent, RenderableCursor};
use crate::display::cursor::IntoRects;
use crate::display::damage::{DamageTracker, damage_y_to_viewport_y};
use crate::display::hint::{HintMatch, HintState};
use crate::display::meter::Meter;
use crate::display::window::Window;
use crate::event::{Event, EventType, Mouse, SearchState};
use crate::message_bar::{MessageBuffer, MessageType};
use crate::renderer::rects::{RenderLine, RenderLines, RenderRect};
use crate::renderer::{self, GlyphCache, Renderer, platform};
use crate::scheduler::{Scheduler, TimerId, Topic};
use crate::string::{ShortenDirection, StrShortener};

pub mod color;
pub mod content;
pub mod cursor;
pub mod hint;
pub mod window;

mod bell;
mod damage;
mod meter;

/// The character used to shorten the visible text like uri preview or search regex.
const SHORTENER: char = '…';

/// Color which is used to highlight damaged rects when debugging.
const DAMAGE_RECT_COLOR: Rgb = Rgb::new(255, 0, 255);

#[derive(Debug)]
pub enum Error {
    /// Error with window management.
    Window(window::Error),

    /// Error dealing with fonts.
    Font(crossfont::Error),

    /// Error in renderer.
    Render(renderer::Error),

    /// Error during context operations.
    Context(glutin::error::Error),
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Window(err) => err.source(),
            Error::Font(err) => err.source(),
            Error::Render(err) => err.source(),
            Error::Context(err) => err.source(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::Window(err) => err.fmt(f),
            Error::Font(err) => err.fmt(f),
            Error::Render(err) => err.fmt(f),
            Error::Context(err) => err.fmt(f),
        }
    }
}

impl From<window::Error> for Error {
    fn from(val: window::Error) -> Self {
        Error::Window(val)
    }
}

impl From<crossfont::Error> for Error {
    fn from(val: crossfont::Error) -> Self {
        Error::Font(val)
    }
}

impl From<renderer::Error> for Error {
    fn from(val: renderer::Error) -> Self {
        Error::Render(val)
    }
}

impl From<glutin::error::Error> for Error {
    fn from(val: glutin::error::Error) -> Self {
        Error::Context(val)
    }
}

/// Terminal size info.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub struct SizeInfo<T = f32> {
    /// Terminal window width.
    width: T,

    /// Terminal window height.
    height: T,

    /// Width of individual cell.
    cell_width: T,

    /// Height of individual cell.
    cell_height: T,

    /// Horizontal window padding.
    padding_x: T,
    right_padding_x: T,

    /// Vertical window padding.
    padding_y: T,
    bottom_padding_y: T,

    /// Number of lines in the viewport.
    screen_lines: usize,

    /// Number of columns in the viewport.
    columns: usize,
}

impl From<SizeInfo<f32>> for SizeInfo<u32> {
    fn from(size_info: SizeInfo<f32>) -> Self {
        Self {
            width: size_info.width as u32,
            height: size_info.height as u32,
            cell_width: size_info.cell_width as u32,
            cell_height: size_info.cell_height as u32,
            padding_x: size_info.padding_x as u32,
            right_padding_x: size_info.right_padding_x as u32,
            padding_y: size_info.padding_y as u32,
            bottom_padding_y: size_info.bottom_padding_y as u32,
            screen_lines: size_info.screen_lines,
            columns: size_info.columns,
        }
    }
}

impl From<SizeInfo<f32>> for WindowSize {
    fn from(size_info: SizeInfo<f32>) -> Self {
        Self {
            num_cols: size_info.columns() as u16,
            num_lines: size_info.screen_lines() as u16,
            cell_width: size_info.cell_width() as u16,
            cell_height: size_info.cell_height() as u16,
        }
    }
}

impl<T: Clone + Copy> SizeInfo<T> {
    #[inline]
    pub fn width(&self) -> T {
        self.width
    }

    #[inline]
    pub fn height(&self) -> T {
        self.height
    }

    #[inline]
    pub fn cell_width(&self) -> T {
        self.cell_width
    }

    #[inline]
    pub fn cell_height(&self) -> T {
        self.cell_height
    }

    #[inline]
    pub fn padding_x(&self) -> T {
        self.padding_x
    }

    #[inline]
    pub fn right_padding_x(&self) -> T {
        self.right_padding_x
    }

    pub fn bottom_padding_y(&self) -> T {
        self.bottom_padding_y
    }

    #[inline]
    pub fn padding_y(&self) -> T {
        self.padding_y
    }
}

impl SizeInfo<f32> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        width: f32,
        height: f32,
        cell_width: f32,
        cell_height: f32,
        mut padding_x: f32,
        mut padding_y: f32,
        dynamic_padding: bool,
    ) -> SizeInfo {
        if dynamic_padding {
            padding_x = Self::dynamic_padding(padding_x.floor(), width, cell_width);
            padding_y = Self::dynamic_padding(padding_y.floor(), height, cell_height);
        }

        let lines = (height - 2. * padding_y) / cell_height;
        let screen_lines = cmp::max(lines as usize, MIN_SCREEN_LINES);

        let columns = (width - 2. * padding_x) / cell_width;
        let columns = cmp::max(columns as usize, MIN_COLUMNS);

        SizeInfo {
            width,
            height,
            cell_width,
            cell_height,
            padding_x: padding_x.floor(),
            right_padding_x: padding_x.floor(),
            padding_y: padding_y.floor(),
            bottom_padding_y: padding_y.floor(),
            screen_lines,
            columns,
        }
    }

    pub fn reserve_top_lines(&mut self, count: usize) {
        self.padding_y += self.cell_height * count as f32;
        self.reserve_lines(count);
    }

    pub fn pane(&self, x: usize, y: usize, columns: usize, lines: usize) -> Self {
        let padding_x = self.padding_x + x as f32 * self.cell_width;
        let padding_y = self.padding_y + y as f32 * self.cell_height;
        let right_padding_x = (self.width - padding_x - columns as f32 * self.cell_width).max(0.);
        let bottom_padding_y = (self.height - padding_y - lines as f32 * self.cell_height).max(0.);
        let columns = columns.max(MIN_COLUMNS);
        let screen_lines = lines.max(MIN_SCREEN_LINES);
        Self {
            padding_x,
            padding_y,
            right_padding_x,
            bottom_padding_y,
            columns,
            screen_lines,
            ..*self
        }
    }

    #[inline]
    pub fn reserve_lines(&mut self, count: usize) {
        self.screen_lines = cmp::max(self.screen_lines.saturating_sub(count), MIN_SCREEN_LINES);
    }

    /// Check if coordinates are inside the terminal grid.
    ///
    /// The padding, message bar or search are not counted as part of the grid.
    #[inline]
    pub fn contains_point(&self, x: usize, y: usize) -> bool {
        x <= (self.padding_x + self.columns as f32 * self.cell_width) as usize
            && x > self.padding_x as usize
            && y <= (self.padding_y + self.screen_lines as f32 * self.cell_height) as usize
            && y > self.padding_y as usize
    }

    /// Calculate padding to spread it evenly around the terminal content.
    #[inline]
    fn dynamic_padding(padding: f32, dimension: f32, cell_dimension: f32) -> f32 {
        padding + ((dimension - 2. * padding) % cell_dimension) / 2.
    }
}

impl TermDimensions for SizeInfo {
    #[inline]
    fn columns(&self) -> usize {
        self.columns
    }

    #[inline]
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    #[inline]
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct DisplayUpdate {
    pub dirty: bool,

    dimensions: Option<PhysicalSize<u32>>,
    cursor_dirty: bool,
    font: Option<Font>,
}

impl DisplayUpdate {
    pub fn dimensions(&self) -> Option<PhysicalSize<u32>> {
        self.dimensions
    }

    pub fn font(&self) -> Option<&Font> {
        self.font.as_ref()
    }

    pub fn cursor_dirty(&self) -> bool {
        self.cursor_dirty
    }

    pub fn set_dimensions(&mut self, dimensions: PhysicalSize<u32>) {
        self.dimensions = Some(dimensions);
        self.dirty = true;
    }

    pub fn set_font(&mut self, font: Font) {
        self.font = Some(font);
        self.dirty = true;
    }

    pub fn set_cursor_dirty(&mut self) {
        self.cursor_dirty = true;
        self.dirty = true;
    }
}

/// The display wraps a window, font rasterizer, and GPU renderer.
pub struct PaneRender<'a, T> {
    pub terminal: MutexGuard<'a, Term<T>>,
    pub search_state: &'a mut SearchState,
    pub size: SizeInfo,
    pub focused: bool,
}

pub struct Display {
    pub window: Window,
    pub tab_bar: Option<crate::tabs::TabBar>,
    pub tab_dialog: Option<crate::tab_ui::TabDialog>,
    pub context_menu: Option<crate::context_menu::ContextMenu>,
    pub language_dialog: Option<crate::language_settings::LanguageDialog>,
    pub productivity_dialog: Option<crate::productivity::ProductivityDialog>,
    pub close_confirmation: Option<crate::close_confirmation::CloseConfirmation>,

    pub size_info: SizeInfo,
    pub split_active: bool,

    /// Hint highlighted by the mouse.
    pub highlighted_hint: Option<HintMatch>,
    /// Frames since hint highlight was created.
    highlighted_hint_age: usize,

    /// Hint highlighted by the vi mode cursor.
    pub vi_highlighted_hint: Option<HintMatch>,
    /// Frames since hint highlight was created.
    vi_highlighted_hint_age: usize,

    pub raw_window_handle: RawWindowHandle,

    /// UI cursor visibility for blinking.
    pub cursor_hidden: bool,

    pub visual_bell: VisualBell,

    /// Mapped RGB values for each terminal color.
    pub colors: List,

    /// State of the keyboard hints.
    pub hint_state: HintState,

    /// Unprocessed display updates.
    pub pending_update: DisplayUpdate,

    /// The renderer update that takes place only once before the actual rendering.
    pub pending_renderer_update: Option<RendererUpdate>,

    /// The ime on the given display.
    pub ime: Ime,

    /// The state of the timer for frame scheduling.
    pub frame_timer: FrameTimer,

    /// Damage tracker for the given display.
    pub damage_tracker: DamageTracker,

    /// Font size used by the window.
    pub font_size: FontSize,

    // Mouse point position when highlighting hints.
    hint_mouse_point: Option<Point>,

    renderer: ManuallyDrop<Renderer>,
    renderer_preference: Option<RendererPreference>,

    surface: ManuallyDrop<Surface<WindowSurface>>,

    context: ManuallyDrop<PossiblyCurrentContext>,

    glyph_cache: GlyphCache,
    meter: Meter,
}

impl Display {
    pub fn new(
        window: Window,
        gl_context: NotCurrentContext,
        config: &UiConfig,
        _tabbed: bool,
    ) -> Result<Display, Error> {
        let raw_window_handle = window.raw_window_handle();

        let scale_factor = window.scale_factor as f32;
        let rasterizer = Rasterizer::new()?;

        let font_size = config.font.size().scale(scale_factor);
        debug!("Loading \"{}\" font", &config.font.normal().family);
        let font = config.font.clone().with_size(font_size);
        let mut glyph_cache = GlyphCache::new(rasterizer, &font)?;

        let metrics = glyph_cache.font_metrics();
        let (cell_width, cell_height) = compute_cell_size(config, &metrics);

        // Resize the window to account for the user configured size.
        if let Some(dimensions) = config.window.dimensions() {
            let size = window_size(config, dimensions, cell_width, cell_height, scale_factor);
            window.request_inner_size(size);
        }

        // Create the GL surface to draw into.
        let surface = platform::create_gl_surface(
            &gl_context,
            window.inner_size(),
            window.raw_window_handle(),
        )?;

        // Make the context current.
        let context = gl_context.make_current(&surface)?;

        // Create renderer.
        let mut renderer = Renderer::new(&context, config.debug.renderer)?;

        // Load font common glyphs to accelerate rendering.
        debug!("Filling glyph cache with common glyphs");
        renderer.with_loader(|mut api| {
            glyph_cache.reset_glyph_cache(&mut api);
        });

        let padding = config.window.padding(window.scale_factor as f32);
        let viewport_size = window.inner_size();

        // Create new size with at least one column and row.
        let mut size_info = SizeInfo::new(
            viewport_size.width as f32,
            viewport_size.height as f32,
            cell_width,
            cell_height,
            padding.0,
            padding.1,
            config.window.dynamic_padding && config.window.dimensions().is_none(),
        );

        if cfg!(not(target_os = "macos")) {
            size_info.reserve_top_lines(crate::tabs::TabBar::ROWS);
        }
        info!("Cell size: {cell_width} x {cell_height}");
        info!("Padding: {} x {}", size_info.padding_x(), size_info.padding_y());
        info!("Width: {}, Height: {}", size_info.width(), size_info.height());

        // Update OpenGL projection.
        renderer.resize(&size_info);

        // Clear screen.
        let background_color = config.colors.primary.background;
        renderer.clear(background_color, config.window_opacity());

        // Disable shadows for transparent windows on macOS.
        #[cfg(target_os = "macos")]
        window.set_has_shadow(config.window_opacity() >= 1.0);

        let is_wayland = matches!(raw_window_handle, RawWindowHandle::Wayland(_));

        // On Wayland we can safely ignore this call, since the window isn't visible until you
        // actually draw something into it and commit those changes.
        if !is_wayland {
            surface.swap_buffers(&context).expect("failed to swap buffers.");
            renderer.finish();
        }

        // Set resize increments for the newly created window.
        if config.window.resize_increments {
            window.set_resize_increments(PhysicalSize::new(cell_width, cell_height));
        }

        window.set_visible(true);

        // Always focus new windows, even if no Alacritty window is currently focused.
        #[cfg(target_os = "macos")]
        window.focus_window();

        #[allow(clippy::single_match)]
        #[cfg(not(windows))]
        if !_tabbed {
            match config.window.startup_mode {
                #[cfg(target_os = "macos")]
                StartupMode::SimpleFullscreen => window.set_simple_fullscreen(true),
                StartupMode::Maximized if !is_wayland => window.set_maximized(true),
                _ => (),
            }
        }

        let hint_state = HintState::new(config.hints.alphabet());

        let mut damage_tracker = DamageTracker::new(size_info.screen_lines(), size_info.columns());
        damage_tracker.debug = config.debug.highlight_damage;

        // Disable vsync.
        if let Err(err) = surface.set_swap_interval(&context, SwapInterval::DontWait) {
            info!("Failed to disable vsync: {err}");
        }

        Ok(Self {
            context: ManuallyDrop::new(context),
            visual_bell: VisualBell::from(&config.bell),
            renderer: ManuallyDrop::new(renderer),
            renderer_preference: config.debug.renderer,
            surface: ManuallyDrop::new(surface),
            colors: List::from(&config.colors),
            tab_bar: cfg!(not(target_os = "macos")).then(Default::default),
            tab_dialog: None,
            context_menu: None,
            language_dialog: None,
            productivity_dialog: None,
            close_confirmation: None,
            frame_timer: FrameTimer::new(),
            raw_window_handle,
            damage_tracker,
            glyph_cache,
            hint_state,
            size_info,
            split_active: false,
            font_size,
            window,
            pending_renderer_update: Default::default(),
            vi_highlighted_hint_age: Default::default(),
            highlighted_hint_age: Default::default(),
            vi_highlighted_hint: Default::default(),
            highlighted_hint: Default::default(),
            hint_mouse_point: Default::default(),
            pending_update: Default::default(),
            cursor_hidden: Default::default(),
            meter: Default::default(),
            ime: Default::default(),
        })
    }

    #[inline]
    pub fn gl_context(&self) -> &PossiblyCurrentContext {
        &self.context
    }

    pub fn make_not_current(&mut self) {
        if self.context.is_current() {
            self.context.make_not_current_in_place().expect("failed to disable context");
        }
    }

    pub fn make_current(&mut self) {
        let is_current = self.context.is_current();

        // Attempt to make the context current if it's not.
        let context_loss = if is_current {
            self.renderer.was_context_reset()
        } else {
            match self.context.make_current(&self.surface) {
                Err(err) if err.error_kind() == ErrorKind::ContextLost => {
                    info!("Context lost for window {:?}", self.window.id());
                    true
                },
                _ => false,
            }
        };

        if !context_loss {
            return;
        }

        let gl_display = self.context.display();
        let gl_config = self.context.config();
        let raw_window_handle = Some(self.window.raw_window_handle());
        let context = platform::create_gl_context(&gl_display, &gl_config, raw_window_handle)
            .expect("failed to recreate context.");

        // Drop the old context and renderer.
        unsafe {
            ManuallyDrop::drop(&mut self.renderer);
            ManuallyDrop::drop(&mut self.context);
        }

        // Activate new context.
        let context = context.treat_as_possibly_current();
        self.context = ManuallyDrop::new(context);
        self.context.make_current(&self.surface).expect("failed to reativate context after reset.");

        // Recreate renderer.
        let renderer = Renderer::new(&self.context, self.renderer_preference)
            .expect("failed to recreate renderer after reset");
        self.renderer = ManuallyDrop::new(renderer);

        // Resize the renderer.
        self.renderer.resize(&self.size_info);

        self.reset_glyph_cache();
        self.damage_tracker.frame().mark_fully_damaged();

        debug!("Recovered window {:?} from gpu reset", self.window.id());
    }

    fn swap_buffers(&self) {
        #[allow(clippy::single_match)]
        let res = match (self.surface.deref(), &self.context.deref()) {
            #[cfg(not(any(target_os = "macos", windows)))]
            (Surface::Egl(surface), PossiblyCurrentContext::Egl(context))
                if matches!(self.raw_window_handle, RawWindowHandle::Wayland(_))
                    && !self.damage_tracker.debug =>
            {
                let damage = self.damage_tracker.shape_frame_damage(self.size_info.into());
                surface.swap_buffers_with_damage(context, &damage)
            },
            (surface, context) => surface.swap_buffers(context),
        };
        if let Err(err) = res {
            debug!("error calling swap_buffers: {err}");
        }
    }

    /// Update font size and cell dimensions.
    ///
    /// This will return a tuple of the cell width and height.
    fn update_font_size(
        glyph_cache: &mut GlyphCache,
        config: &UiConfig,
        font: &Font,
    ) -> (f32, f32) {
        let _ = glyph_cache.update_font_size(font);

        // Compute new cell sizes.
        compute_cell_size(config, &glyph_cache.font_metrics())
    }

    /// Reset glyph cache.
    fn reset_glyph_cache(&mut self) {
        let cache = &mut self.glyph_cache;
        self.renderer.with_loader(|mut api| {
            cache.reset_glyph_cache(&mut api);
        });
    }

    // XXX: this function must not call to any `OpenGL` related tasks. Renderer updates are
    // performed in [`Self::process_renderer_update`] right before drawing.
    //
    /// Process update events.
    pub fn handle_update<T>(
        &mut self,
        terminal: &mut Term<T>,
        pty_resize_handle: &mut dyn OnResize,
        message_buffer: &MessageBuffer,
        search_state: &mut SearchState,
        config: &UiConfig,
    ) where
        T: EventListener,
    {
        let pending_update = mem::take(&mut self.pending_update);

        let (mut cell_width, mut cell_height) =
            (self.size_info.cell_width(), self.size_info.cell_height());

        if pending_update.font().is_some() || pending_update.cursor_dirty() {
            let renderer_update = self.pending_renderer_update.get_or_insert(Default::default());
            renderer_update.clear_font_cache = true
        }

        // Update font size and cell dimensions.
        if let Some(font) = pending_update.font() {
            let cell_dimensions = Self::update_font_size(&mut self.glyph_cache, config, font);
            cell_width = cell_dimensions.0;
            cell_height = cell_dimensions.1;

            info!("Cell size: {cell_width} x {cell_height}");

            // Mark entire terminal as damaged since glyph size could change without cell size
            // changes.
            self.damage_tracker.frame().mark_fully_damaged();
        }

        let (mut width, mut height) = (self.size_info.width(), self.size_info.height());
        if let Some(dimensions) = pending_update.dimensions() {
            width = dimensions.width as f32;
            height = dimensions.height as f32;
        }

        let padding = config.window.padding(self.window.scale_factor as f32);

        let mut new_size = SizeInfo::new(
            width,
            height,
            cell_width,
            cell_height,
            padding.0,
            padding.1,
            config.window.dynamic_padding,
        );

        // Update number of column/lines in the viewport.
        if self.tab_bar.is_some() {
            new_size.reserve_top_lines(crate::tabs::TabBar::ROWS);
        }
        let search_active = search_state.history_index.is_some();
        let message_bar_lines = message_buffer
            .message()
            .map_or(0, |m| m.text_with_locale(&new_size, config.general.language.resolve()).len());
        let search_lines = usize::from(search_active && !self.split_active);
        new_size.reserve_lines(message_bar_lines + search_lines);

        // Update resize increments.
        if config.window.resize_increments {
            self.window.set_resize_increments(PhysicalSize::new(cell_width, cell_height));
        }

        // Resize when terminal when its dimensions have changed.
        if self.size_info.screen_lines() != new_size.screen_lines
            || self.size_info.columns() != new_size.columns()
        {
            if !self.split_active {
                pty_resize_handle.on_resize(new_size.into());
                terminal.resize(new_size);
            }

            // Resize damage tracking.
            self.damage_tracker.resize(new_size.screen_lines(), new_size.columns());
        }

        // Check if dimensions have changed.
        if new_size != self.size_info {
            // Queue renderer update.
            let renderer_update = self.pending_renderer_update.get_or_insert(Default::default());
            renderer_update.resize = true;

            // Clear focused search match.
            search_state.clear_focused_match();
        }
        self.size_info = new_size;
    }

    // NOTE: Renderer updates are split off, since platforms like Wayland require resize and other
    // OpenGL operations to be performed right before rendering. Otherwise they could lock the
    // back buffer and render with the previous state. This also solves flickering during resizes.
    //
    /// Update the state of the renderer.
    pub fn process_renderer_update(&mut self) {
        let renderer_update = match self.pending_renderer_update.take() {
            Some(renderer_update) => renderer_update,
            _ => return,
        };

        // Resize renderer.
        if renderer_update.resize {
            let width = NonZeroU32::new(self.size_info.width() as u32).unwrap();
            let height = NonZeroU32::new(self.size_info.height() as u32).unwrap();
            self.surface.resize(&self.context, width, height);
        }

        // Ensure we're modifying the correct OpenGL context.
        self.make_current();

        if renderer_update.clear_font_cache {
            self.reset_glyph_cache();
        }

        self.renderer.resize(&self.size_info);

        info!("Padding: {} x {}", self.size_info.padding_x(), self.size_info.padding_y());
        info!("Width: {}, Height: {}", self.size_info.width(), self.size_info.height());
    }

    /// Draw the screen.
    ///
    /// A reference to Term whose state is being drawn must be provided.
    ///
    /// This call may block if vsync is enabled.
    pub fn draw<T: EventListener>(
        &mut self,
        terminal: MutexGuard<'_, Term<T>>,
        scheduler: &mut Scheduler,
        message_buffer: &MessageBuffer,
        config: &UiConfig,
        search_state: &mut SearchState,
    ) {
        self.draw_terminal(terminal, config, search_state, true);
        self.draw_messages(message_buffer, config, usize::from(search_state.regex().is_some()));
        self.finish_frame(scheduler, config);
    }

    pub fn draw_panes<T: EventListener>(
        &mut self,
        mut panes: Vec<PaneRender<'_, T>>,
        scheduler: &mut Scheduler,
        message_buffer: &MessageBuffer,
        config: &UiConfig,
        mut dividers: Vec<RenderRect>,
    ) {
        let window_size = self.size_info;
        self.make_current();
        self.renderer.set_scissor(None);
        self.renderer.clear(config.colors.primary.background, config.window_opacity());
        self.damage_tracker.frame().mark_fully_damaged();
        self.damage_tracker.next_frame().mark_fully_damaged();
        panes.sort_by_key(|pane| pane.focused);
        let mut focused_size = None;
        for pane in panes {
            self.size_info = pane.size;
            self.renderer.resize(&pane.size);
            self.renderer.set_scissor(Some(&pane.size));
            let inactive_state = (!pane.focused).then(|| {
                let hints =
                    mem::replace(&mut self.hint_state, HintState::new(config.hints.alphabet()));
                let highlighted = self.highlighted_hint.take();
                let vi_highlighted = self.vi_highlighted_hint.take();
                let ime = mem::take(&mut self.ime);
                let cursor_hidden = mem::replace(&mut self.cursor_hidden, true);
                (hints, highlighted, vi_highlighted, ime, cursor_hidden)
            });
            self.draw_terminal(pane.terminal, config, pane.search_state, pane.focused);
            if let Some((hints, highlighted, vi_highlighted, ime, cursor_hidden)) = inactive_state {
                self.hint_state = hints;
                self.highlighted_hint = highlighted;
                self.vi_highlighted_hint = vi_highlighted;
                self.ime = ime;
                self.cursor_hidden = cursor_hidden;
            }
            if pane.focused {
                focused_size = Some(pane.size);
            }
        }
        self.size_info = window_size;
        self.renderer.set_scissor(None);
        self.renderer.resize(&window_size);
        if let Some(size) = focused_size {
            let x = size.padding_x();
            let y = size.padding_y();
            let width = (size.width() - x - size.right_padding_x()).max(0.);
            let height = (size.height() - y - size.bottom_padding_y()).max(0.);
            let color = config.colors.normal.blue;
            dividers.extend([
                RenderRect::new(x, y, width, 1., color, 0.8),
                RenderRect::new(x, y + height - 1., width, 1., color, 0.8),
                RenderRect::new(x, y, 1., height, color, 0.8),
                RenderRect::new(x + width - 1., y, 1., height, color, 0.8),
            ]);
        }
        let metrics = self.glyph_cache.font_metrics();
        self.renderer.draw_rects(&window_size, &metrics, dividers);
        self.draw_messages(message_buffer, config, 0);
        self.finish_frame(scheduler, config);
    }

    fn draw_terminal<T: EventListener>(
        &mut self,
        mut terminal: MutexGuard<'_, Term<T>>,
        config: &UiConfig,
        search_state: &mut SearchState,
        focused: bool,
    ) {
        // Collect renderable content before the terminal is dropped.
        let mut content = RenderableContent::new(config, self, &terminal, search_state);
        let mut grid_cells = Vec::new();
        for cell in &mut content {
            grid_cells.push(cell);
        }
        let selection_range = content.selection_range();
        let foreground_color = content.color(NamedColor::Foreground as usize);
        let background_color = content.color(NamedColor::Background as usize);
        let display_offset = content.display_offset();
        let cursor = content.cursor();

        let cursor_point = terminal.grid().cursor.point;
        let total_lines = terminal.grid().total_lines();
        let metrics = self.glyph_cache.font_metrics();
        let size_info = self.size_info;

        let vi_mode = focused && terminal.mode().contains(TermMode::VI);
        let vi_cursor_point = if vi_mode { Some(terminal.vi_mode_cursor.point) } else { None };

        // Add damage from the terminal.
        match terminal.damage() {
            TermDamage::Full => self.damage_tracker.frame().mark_fully_damaged(),
            TermDamage::Partial(damaged_lines) => {
                for damage in damaged_lines {
                    self.damage_tracker.frame().damage_line(damage);
                }
            },
        }
        terminal.reset_damage();

        // Drop terminal as early as possible to free lock.
        drop(terminal);

        // Invalidate highlighted hints if grid has changed.
        if focused {
            self.validate_hint_highlights(display_offset);
        }

        // Add damage from alacritty's UI elements overlapping terminal.

        let requires_full_damage = self.visual_bell.intensity() != 0.
            || self.hint_state.active()
            || search_state.regex().is_some();
        if requires_full_damage {
            self.damage_tracker.frame().mark_fully_damaged();
            self.damage_tracker.next_frame().mark_fully_damaged();
        }

        let vi_cursor_viewport_point =
            vi_cursor_point.and_then(|cursor| term::point_to_viewport(display_offset, cursor));
        self.damage_tracker.damage_vi_cursor(vi_cursor_viewport_point);
        self.damage_tracker.damage_selection(selection_range, display_offset);

        // Make sure this window's OpenGL context is active.
        self.make_current();

        self.renderer.clear(background_color, config.window_opacity());
        let mut lines = RenderLines::new();

        // Optimize loop hint comparator.
        let has_highlighted_hint =
            self.highlighted_hint.is_some() || self.vi_highlighted_hint.is_some();

        // Draw grid.
        {
            let _sampler = self.meter.sampler();

            // Ensure macOS hasn't reset our viewport.
            #[cfg(target_os = "macos")]
            self.renderer.set_viewport(&size_info);

            let glyph_cache = &mut self.glyph_cache;
            let highlighted_hint = &self.highlighted_hint;
            let vi_highlighted_hint = &self.vi_highlighted_hint;
            let damage_tracker = &mut self.damage_tracker;

            let cells = grid_cells.into_iter().map(|mut cell| {
                // Underline hints hovered by mouse or vi mode cursor.
                if has_highlighted_hint {
                    let point = term::viewport_to_point(display_offset, cell.point);
                    let hyperlink = cell.extra.as_ref().and_then(|extra| extra.hyperlink.as_ref());

                    let should_highlight = |hint: &Option<HintMatch>| {
                        hint.as_ref().is_some_and(|hint| hint.should_highlight(point, hyperlink))
                    };
                    if should_highlight(highlighted_hint) || should_highlight(vi_highlighted_hint) {
                        damage_tracker.frame().damage_point(cell.point);
                        cell.flags.insert(Flags::UNDERLINE);
                    }
                }

                // Update underline/strikeout.
                lines.update(&cell);

                cell
            });
            self.renderer.draw_cells(&size_info, glyph_cache, cells);
        }

        let mut rects = lines.rects(&metrics, &size_info);

        if let Some(vi_cursor_point) = vi_cursor_point {
            // Indicate vi mode by showing the cursor's position in the top right corner.
            let line = (-vi_cursor_point.line.0 + size_info.bottommost_line().0) as usize;
            let obstructed_column = Some(vi_cursor_point)
                .filter(|point| point.line == -(display_offset as i32))
                .map(|point| point.column);
            self.draw_line_indicator(config, total_lines, obstructed_column, line);
        } else if search_state.regex().is_some() {
            // Show current display offset in vi-less search to indicate match position.
            self.draw_line_indicator(config, total_lines, None, display_offset);
        };

        // Draw cursor.
        rects.extend(cursor.rects(&size_info, config.cursor.thickness()));

        // Push visual bell after url/underline/strikeout rects.
        let visual_bell_intensity = self.visual_bell.intensity();
        if visual_bell_intensity != 0. {
            let visual_bell_rect = RenderRect::new(
                0.,
                0.,
                size_info.width(),
                size_info.height(),
                config.bell.color,
                visual_bell_intensity as f32,
            );
            rects.push(visual_bell_rect);
        }

        // Handle IME positioning and search bar rendering.
        let ime_position = match search_state.regex() {
            Some(regex) => {
                let locale = config.general.language.resolve();
                let search_label = locale.text(match search_state.direction() {
                    Direction::Right => crate::localization::Message::SearchForward,
                    Direction::Left => crate::localization::Message::SearchBackward,
                });

                let search_text = Self::format_search(regex, search_label, size_info.columns());

                // Render the search bar.
                self.draw_search(config, &search_text);

                // Draw search bar cursor.
                let line = size_info.screen_lines();
                let column = Column(search_text.chars().count() - 1);

                // Add cursor to search bar if IME is not active.
                if focused && self.ime.preedit().is_none() {
                    let fg = config.colors.footer_bar_foreground();
                    let shape = CursorShape::Underline;
                    let cursor_width = NonZeroU32::new(1).unwrap();
                    let cursor =
                        RenderableCursor::new(Point::new(line, column), shape, fg, cursor_width);
                    rects.extend(cursor.rects(&size_info, config.cursor.thickness()));
                }

                Some(Point::new(line, column))
            },
            None => {
                let num_lines = self.size_info.screen_lines();
                match vi_cursor_viewport_point {
                    None => term::point_to_viewport(display_offset, cursor_point)
                        .filter(|point| point.line < num_lines),
                    point => point,
                }
            },
        };

        // Handle IME.
        if focused && self.ime.is_enabled() {
            if let Some(point) = ime_position {
                let (fg, bg) = if search_state.regex().is_some() {
                    (config.colors.footer_bar_foreground(), config.colors.footer_bar_background())
                } else {
                    (foreground_color, background_color)
                };

                self.draw_ime_preview(point, fg, bg, &mut rects, config);
            }
        }

        self.renderer.draw_rects(&size_info, &metrics, rects);

        if has_highlighted_hint {
            let cursor_point = vi_cursor_point.or(Some(cursor_point));
            self.draw_hyperlink_preview(config, cursor_point, display_offset);
        }
    }

    fn draw_messages(
        &mut self,
        message_buffer: &MessageBuffer,
        config: &UiConfig,
        search_offset: usize,
    ) {
        let size_info = self.size_info;
        let metrics = self.glyph_cache.font_metrics();
        let mut rects = Vec::new();
        if let Some(message) = message_buffer.message() {
            let text = message.text_with_locale(&size_info, config.general.language.resolve());

            // Create a new rectangle for the background.
            let start_line = size_info.screen_lines() + search_offset;
            let y = size_info.cell_height().mul_add(start_line as f32, size_info.padding_y());

            let bg = match message.ty() {
                MessageType::Error => config.colors.normal.red,
                MessageType::Warning => config.colors.normal.yellow,
            };

            let x = 0;
            let width = size_info.width() as i32;
            let height = (size_info.height() - y) as i32;
            let message_bar_rect =
                RenderRect::new(x as f32, y, width as f32, height as f32, bg, 1.);

            // Push message_bar in the end, so it'll be above all other content.
            rects.push(message_bar_rect);

            // Always damage message bar, since it could have messages of the same size in it.
            self.damage_tracker.frame().add_viewport_rect(&size_info, x, y as i32, width, height);

            // Draw rectangles.
            self.renderer.draw_rects(&size_info, &metrics, rects);

            // Relay messages to the user.
            let glyph_cache = &mut self.glyph_cache;
            let fg = config.colors.primary.background;
            for (i, message_text) in text.iter().enumerate() {
                let point = Point::new(start_line + i, Column(0));
                self.renderer.draw_string(
                    point,
                    fg,
                    bg,
                    message_text.chars(),
                    &size_info,
                    glyph_cache,
                );
            }
        }
    }

    fn finish_frame(&mut self, scheduler: &mut Scheduler, config: &UiConfig) {
        let metrics = self.glyph_cache.font_metrics();
        self.draw_tab_bar(config);
        self.draw_render_timer(config);

        self.draw_tab_dialog(config);
        self.draw_language_dialog();
        self.draw_productivity_dialog();
        self.draw_close_confirmation();
        self.draw_context_menu();

        // Notify winit that we're about to present.
        self.window.pre_present_notify();

        // Highlight damage for debugging.
        if self.damage_tracker.debug {
            let damage = self.damage_tracker.shape_frame_damage(self.size_info.into());
            let mut rects = Vec::with_capacity(damage.len());
            self.highlight_damage(&mut rects);
            self.renderer.draw_rects(&self.size_info, &metrics, rects);
        }

        // Clearing debug highlights from the previous frame requires full redraw.
        self.swap_buffers();

        if matches!(self.raw_window_handle, RawWindowHandle::Xcb(_) | RawWindowHandle::Xlib(_)) {
            // On X11 `swap_buffers` does not block for vsync. However the next OpenGl command
            // will block to synchronize (this is `glClear` in Alacritty), which causes a
            // permanent one frame delay.
            self.renderer.finish();
        }

        // XXX: Request the new frame after swapping buffers, so the
        // time to finish OpenGL operations is accounted for in the timeout.
        if !matches!(self.raw_window_handle, RawWindowHandle::Wayland(_)) {
            self.request_frame(scheduler);
        }

        self.damage_tracker.swap_damage();
    }

    /// Update to a new configuration.
    pub fn update_config(&mut self, config: &UiConfig) {
        self.damage_tracker.debug = config.debug.highlight_damage;
        self.visual_bell.update_config(&config.bell);
        self.colors = List::from(&config.colors);
    }

    /// Update the mouse/vi mode cursor hint highlighting.
    ///
    /// This will return whether the highlighted hints changed.
    pub fn update_highlighted_hints<T>(
        &mut self,
        term: &Term<T>,
        config: &UiConfig,
        mouse: &Mouse,
        modifiers: ModifiersState,
    ) -> bool {
        // Update vi mode cursor hint.
        let vi_highlighted_hint = if term.mode().contains(TermMode::VI) {
            let mods = ModifiersState::all();
            let point = term.vi_mode_cursor.point;
            hint::highlighted_at(term, config, point, mods)
        } else {
            None
        };
        let mut dirty = vi_highlighted_hint != self.vi_highlighted_hint;
        self.vi_highlighted_hint = vi_highlighted_hint;
        self.vi_highlighted_hint_age = 0;

        // Force full redraw if the vi mode highlight was cleared.
        if dirty {
            self.damage_tracker.frame().mark_fully_damaged();
        }

        // Abort if mouse highlighting conditions are not met.
        if !self.window.mouse_visible()
            || !mouse.inside_text_area
            || !term.selection.as_ref().is_none_or(Selection::is_empty)
        {
            if self.highlighted_hint.take().is_some() {
                self.damage_tracker.frame().mark_fully_damaged();
                dirty = true;
            }
            return dirty;
        }

        // Find highlighted hint at mouse position.
        let point = mouse.point(&self.size_info, term.grid().display_offset());
        let highlighted_hint = hint::highlighted_at(term, config, point, modifiers);

        // Update cursor shape.
        if highlighted_hint.is_some() {
            // If mouse changed the line, we should update the hyperlink preview, since the
            // highlighted hint could be disrupted by the old preview.
            dirty = self.hint_mouse_point.is_some_and(|p| p.line != point.line);
            self.hint_mouse_point = Some(point);
            self.window.set_mouse_cursor(CursorIcon::Pointer);
        } else if self.highlighted_hint.is_some() {
            self.hint_mouse_point = None;
            if term.mode().intersects(TermMode::MOUSE_MODE) && !term.mode().contains(TermMode::VI) {
                self.window.set_mouse_cursor(CursorIcon::Default);
            } else {
                self.window.set_mouse_cursor(CursorIcon::Text);
            }
        }

        let mouse_highlight_dirty = self.highlighted_hint != highlighted_hint;
        dirty |= mouse_highlight_dirty;
        self.highlighted_hint = highlighted_hint;
        self.highlighted_hint_age = 0;

        // Force full redraw if the mouse cursor highlight was changed.
        if mouse_highlight_dirty {
            self.damage_tracker.frame().mark_fully_damaged();
        }

        dirty
    }

    #[inline(never)]
    fn draw_ime_preview(
        &mut self,
        point: Point<usize>,
        fg: Rgb,
        bg: Rgb,
        rects: &mut Vec<RenderRect>,
        config: &UiConfig,
    ) {
        let preedit = match self.ime.preedit() {
            Some(preedit) => preedit,
            None => {
                // In case we don't have preedit, just set the popup point.
                self.window.update_ime_position(point, &self.size_info);
                return;
            },
        };

        let num_cols = self.size_info.columns();

        // Get the visible preedit.
        let visible_text: String = match (preedit.cursor_byte_offset, preedit.cursor_end_offset) {
            (Some(byte_offset), Some(end_offset)) if end_offset.0 > num_cols => StrShortener::new(
                &preedit.text[byte_offset.0..],
                num_cols,
                ShortenDirection::Right,
                Some(SHORTENER),
            ),
            _ => {
                StrShortener::new(&preedit.text, num_cols, ShortenDirection::Left, Some(SHORTENER))
            },
        }
        .collect();

        let visible_len = visible_text.chars().count();

        let end = cmp::min(point.column.0 + visible_len, num_cols);
        let start = end.saturating_sub(visible_len);

        let start = Point::new(point.line, Column(start));
        let end = Point::new(point.line, Column(end - 1));

        let glyph_cache = &mut self.glyph_cache;
        let metrics = glyph_cache.font_metrics();

        self.renderer.draw_string(
            start,
            fg,
            bg,
            visible_text.chars(),
            &self.size_info,
            glyph_cache,
        );

        // Damage preedit inside the terminal viewport.
        if point.line < self.size_info.screen_lines() {
            let damage = LineDamageBounds::new(start.line, 0, num_cols);
            self.damage_tracker.frame().damage_line(damage);
            self.damage_tracker.next_frame().damage_line(damage);
        }

        // Add underline for preedit text.
        let underline = RenderLine { start, end, color: fg };
        rects.extend(underline.rects(Flags::UNDERLINE, &metrics, &self.size_info));

        let ime_popup_point = match preedit.cursor_end_offset {
            Some(cursor_end_offset) => {
                // Use hollow block when multiple characters are changed at once.
                let (shape, width) = if let Some(width) =
                    NonZeroU32::new((cursor_end_offset.0 - cursor_end_offset.1) as u32)
                {
                    (CursorShape::HollowBlock, width)
                } else {
                    (CursorShape::Beam, NonZeroU32::new(1).unwrap())
                };

                let cursor_column = Column(
                    (end.column.0 as isize - cursor_end_offset.0 as isize + 1).max(0) as usize,
                );
                let cursor_point = Point::new(point.line, cursor_column);
                let cursor = RenderableCursor::new(cursor_point, shape, fg, width);
                rects.extend(cursor.rects(&self.size_info, config.cursor.thickness()));
                cursor_point
            },
            _ => end,
        };

        self.window.update_ime_position(ime_popup_point, &self.size_info);
    }

    /// Format search regex to account for the cursor and fullwidth characters.
    fn format_search(search_regex: &str, search_label: &str, max_width: usize) -> String {
        let label_len = unicode_width::UnicodeWidthStr::width(search_label);

        // Skip `search_regex` formatting if only label is visible.
        if label_len >= max_width {
            return StrShortener::new(search_label, max_width, ShortenDirection::Right, None)
                .collect();
        }

        // The search string consists of `search_label` + `search_regex` + `cursor`.
        let mut bar_text = String::from(search_label);
        let query_width = max_width.saturating_sub(label_len + 1);
        if query_width > 0 {
            bar_text.extend(StrShortener::new(
                search_regex,
                query_width,
                ShortenDirection::Left,
                Some(SHORTENER),
            ));
        }

        // Add place for cursor.
        bar_text.push(' ');

        bar_text
    }

    /// Draw preview for the currently highlighted `Hyperlink`.
    #[inline(never)]
    fn draw_hyperlink_preview(
        &mut self,
        config: &UiConfig,
        cursor_point: Option<Point>,
        display_offset: usize,
    ) {
        let num_cols = self.size_info.columns();
        let uris: Vec<_> = self
            .highlighted_hint
            .iter()
            .chain(&self.vi_highlighted_hint)
            .filter_map(|hint| hint.hyperlink().map(|hyperlink| hyperlink.uri()))
            .map(|uri| StrShortener::new(uri, num_cols, ShortenDirection::Right, Some(SHORTENER)))
            .collect();

        if uris.is_empty() {
            return;
        }

        // The maximum amount of protected lines including the ones we'll show preview on.
        let max_protected_lines = uris.len() * 2;

        // Lines we shouldn't show preview on, because it'll obscure the highlighted hint.
        let mut protected_lines = Vec::with_capacity(max_protected_lines);
        if self.size_info.screen_lines() > max_protected_lines {
            // Prefer to show preview even when it'll likely obscure the highlighted hint, when
            // there's no place left for it.
            protected_lines.push(self.hint_mouse_point.map(|point| point.line));
            protected_lines.push(cursor_point.map(|point| point.line));
        }

        // Find the line in viewport we can draw preview on without obscuring protected lines.
        let viewport_bottom = self.size_info.bottommost_line() - Line(display_offset as i32);
        let viewport_top = viewport_bottom - (self.size_info.screen_lines() - 1);
        let uri_lines = (viewport_top.0..=viewport_bottom.0)
            .rev()
            .map(|line| Some(Line(line)))
            .filter_map(|line| {
                if protected_lines.contains(&line) {
                    None
                } else {
                    protected_lines.push(line);
                    line
                }
            })
            .take(uris.len())
            .flat_map(|line| term::point_to_viewport(display_offset, Point::new(line, Column(0))));

        let fg = config.colors.footer_bar_foreground();
        let bg = config.colors.footer_bar_background();
        for (uri, point) in uris.into_iter().zip(uri_lines) {
            // Damage the uri preview.
            let damage = LineDamageBounds::new(point.line, point.column.0, num_cols);
            self.damage_tracker.frame().damage_line(damage);

            // Damage the uri preview for the next frame as well.
            self.damage_tracker.next_frame().damage_line(damage);

            self.renderer.draw_string(point, fg, bg, uri, &self.size_info, &mut self.glyph_cache);
        }
    }

    /// Draw current search regex.
    #[inline(never)]
    fn draw_search(&mut self, config: &UiConfig, text: &str) {
        // Assure text length is at least num_cols.
        let num_cols = self.size_info.columns();
        let text = format!("{text:<num_cols$}");

        let point = Point::new(self.size_info.screen_lines(), Column(0));

        let fg = config.colors.footer_bar_foreground();
        let bg = config.colors.footer_bar_background();

        self.renderer.draw_string(
            point,
            fg,
            bg,
            text.chars(),
            &self.size_info,
            &mut self.glyph_cache,
        );
    }

    /// Draw render timer.
    fn draw_ui_panel(&mut self, layout: crate::tab_ui::UiRect, modal: bool) {
        use crate::ui::{Palette, rounded};

        let size = self.size_info;
        let metrics = self.glyph_cache.font_metrics();
        let palette = Palette::new(&self.colors);
        let x = size.padding_x() + layout.left as f32 * size.cell_width();
        let y = size.padding_y() + layout.top as f32 * size.cell_height();
        let width = layout.width as f32 * size.cell_width();
        let height = layout.height as f32 * size.cell_height();
        let mut rects = Vec::new();
        if modal {
            rects.push(RenderRect::new(
                0.,
                0.,
                size.width(),
                size.height(),
                Rgb::new(0, 0, 0),
                0.48,
            ));
        }
        for (offset, alpha) in [(7., 0.12), (4., 0.18)] {
            rects.extend(rounded(
                RenderRect::new(x + offset, y + offset, width, height, Rgb::new(0, 0, 0), alpha),
                9.,
            ));
        }
        rects.extend(rounded(RenderRect::new(x, y, width, height, palette.border, 1.), 8.));
        rects.extend(rounded(
            RenderRect::new(
                x + 1.,
                y + 1.,
                (width - 2.).max(0.),
                (height - 2.).max(0.),
                palette.panel,
                1.,
            ),
            7.,
        ));
        self.renderer.draw_rects(&size, &metrics, rects);
        self.damage_tracker.frame().mark_fully_damaged();
        self.damage_tracker.next_frame().mark_fully_damaged();
    }

    fn draw_ui_elements(&mut self, elements: Vec<crate::tab_ui::UiElement>) {
        use crate::tab_ui::UiKind;
        use crate::ui::{Palette, rounded};

        let size = self.size_info;
        let metrics = self.glyph_cache.font_metrics();
        let p = Palette::new(&self.colors);
        for element in elements {
            let mut rect = element.rect;
            if rect.width == 0
                || rect.height == 0
                || rect.left >= size.columns()
                || rect.top >= size.screen_lines()
            {
                continue;
            }
            rect.width = rect.width.min(size.columns() - rect.left);
            rect.height = rect.height.min(size.screen_lines() - rect.top);
            let x = size.padding_x() + rect.left as f32 * size.cell_width();
            let y = size.padding_y() + rect.top as f32 * size.cell_height();
            let width = rect.width as f32 * size.cell_width();
            let height = rect.height as f32 * size.cell_height();
            let active = (element.focused || element.hovered) && !element.disabled;
            let selected =
                matches!(element.kind, UiKind::SelectedNavigation | UiKind::SelectedItem);
            let button = matches!(
                element.kind,
                UiKind::PrimaryButton | UiKind::SecondaryButton | UiKind::DangerButton
            );
            let interactive = button
                || matches!(
                    element.kind,
                    UiKind::Input
                        | UiKind::Checkbox
                        | UiKind::Radio
                        | UiKind::Navigation
                        | UiKind::SelectedNavigation
                        | UiKind::SelectedItem
                        | UiKind::MenuItem
                        | UiKind::DangerMenu
                        | UiKind::Scrollbar
                        | UiKind::Link
                );
            let (mut fg, bg) = match element.kind {
                UiKind::Subtitle | UiKind::Label | UiKind::Status => (p.muted, p.panel),
                UiKind::Input => (p.text, p.input),
                UiKind::Command => (p.text, p.surface),
                UiKind::PrimaryButton if !element.disabled => (p.on_accent, p.accent),
                UiKind::SecondaryButton => (p.text, if active { p.hover } else { p.surface }),
                UiKind::DangerButton => (p.danger, if active { p.danger_hover } else { p.panel }),
                UiKind::DangerMenu => (p.danger, if active { p.hover } else { p.panel }),
                UiKind::SelectedNavigation | UiKind::SelectedItem => (p.text, p.selected),
                UiKind::Checkbox
                | UiKind::Radio
                | UiKind::Navigation
                | UiKind::MenuItem
                | UiKind::Scrollbar => (p.text, if active { p.hover } else { p.panel }),
                UiKind::Shortcut => (p.muted, if active { p.hover } else { p.panel }),
                UiKind::Link => (p.accent, if active { p.hover } else { p.panel }),
                _ => (p.text, p.panel),
            };
            if element.disabled {
                fg = p.disabled;
            }
            if element.kind == UiKind::Divider {
                let divider = if rect.width == 1 && rect.height > 1 {
                    RenderRect::new(x + width / 2., y, 1., height, p.border, 0.65)
                } else {
                    RenderRect::new(x, y + height / 2., width, 1., p.border, 0.65)
                };
                self.renderer.draw_rects(&size, &metrics, vec![divider]);
                continue;
            }
            if element.kind == UiKind::Scrollbar && element.text == "┃" {
                self.renderer.draw_rects(
                    &size,
                    &metrics,
                    rounded(
                        RenderRect::new(
                            x + width / 2. - 2.,
                            y + 2.,
                            4.,
                            (height - 4.).max(1.),
                            p.border,
                            1.,
                        ),
                        2.,
                    ),
                );
                continue;
            }
            let mut rects = Vec::new();
            if interactive || element.kind == UiKind::Command {
                let border = if element.focused && !element.disabled { p.accent } else { p.border };
                let outlined = element.kind == UiKind::Input
                    || (button && element.kind != UiKind::DangerButton)
                    || (element.focused && !element.disabled);
                if outlined {
                    rects.extend(rounded(
                        RenderRect::new(x, y + 1., width, (height - 2.).max(0.), border, 1.),
                        5.,
                    ));
                    rects.extend(rounded(
                        RenderRect::new(
                            x + 1.,
                            y + 2.,
                            (width - 2.).max(0.),
                            (height - 4.).max(0.),
                            bg,
                            1.,
                        ),
                        4.,
                    ));
                } else {
                    rects.extend(rounded(
                        RenderRect::new(x, y + 1., width, (height - 2.).max(0.), bg, 1.),
                        5.,
                    ));
                }
                if selected {
                    rects.extend(rounded(
                        RenderRect::new(x + 1., y + 4., 3., (height - 8.).max(1.), p.accent, 1.),
                        1.,
                    ));
                }
            }
            self.renderer.draw_rects(&size, &metrics, rects);
            let mut string: String = element.text.chars().filter(|ch| !ch.is_control()).collect();
            let scroll_arrow =
                element.kind == UiKind::MenuItem && matches!(string.trim(), "↑" | "↓");
            if scroll_arrow {
                string = string.trim().to_owned();
            }
            let checkbox = element.kind == UiKind::Checkbox
                && (string.starts_with("[ ] ")
                    || string.starts_with("[✓] ")
                    || string.starts_with("☑ ")
                    || string.starts_with("☐ "));
            let radio = element.kind == UiKind::Radio
                && (string.starts_with("● ") || string.starts_with("○ "));
            if checkbox || radio {
                let checked =
                    string.starts_with("[✓]") || string.starts_with("☑") || string.starts_with("●");
                string = if string.starts_with('[') {
                    string.split_once(']').unwrap().1.trim_start().to_owned()
                } else {
                    string.chars().skip(2).collect()
                };
                let side = (size.cell_width() * 1.4).min(height - 6.).max(4.);
                let box_x = x + size.cell_width() * 0.5;
                let box_y = y + (height - side) / 2.;
                let border = if element.disabled {
                    p.disabled
                } else if checked || active {
                    p.accent
                } else {
                    p.muted
                };
                let radius = if radio { side / 2. } else { 3. };
                let mut glyph =
                    rounded(RenderRect::new(box_x, box_y, side, side, border, 1.), radius);
                glyph.extend(rounded(
                    RenderRect::new(
                        box_x + 1.5,
                        box_y + 1.5,
                        side - 3.,
                        side - 3.,
                        if checked && checkbox { p.accent } else { bg },
                        1.,
                    ),
                    (radius - 1.).max(0.),
                ));
                if checked && radio {
                    glyph.extend(rounded(
                        RenderRect::new(
                            box_x + 4.,
                            box_y + 4.,
                            (side - 8.).max(2.),
                            (side - 8.).max(2.),
                            border,
                            1.,
                        ),
                        side / 2.,
                    ));
                } else if checked {
                    let scale = side / 12.;
                    for (dx, dy) in [(3, 6), (4, 7), (5, 8), (6, 7), (7, 6), (8, 5), (9, 4)] {
                        glyph.push(RenderRect::new(
                            box_x + dx as f32 * scale,
                            box_y + dy as f32 * scale,
                            2. * scale,
                            2. * scale,
                            p.on_accent,
                            1.,
                        ));
                    }
                }
                self.renderer.draw_rects(&size, &metrics, glyph);
            }
            let padded = (interactive || element.kind == UiKind::Command) && rect.width > 2;
            let inset = if checkbox || radio {
                3
            } else if padded && rect.width > 2 {
                1
            } else {
                0
            };
            let available = rect.width.saturating_sub(inset + usize::from(padded));
            if available == 0 {
                continue;
            }
            let string_width =
                unicode_width::UnicodeWidthStr::width(string.as_str()).min(available);
            let offset = if button || scroll_arrow {
                rect.width.saturating_sub(string_width) / 2
            } else {
                inset
            };
            let mut text_size = size;
            if rect.height % 2 == 0 {
                text_size.padding_y += size.cell_height() / 2.;
                text_size.bottom_padding_y -= size.cell_height() / 2.;
                self.renderer.set_viewport(&text_size);
            }
            let bold = matches!(
                element.kind,
                UiKind::Title | UiKind::SelectedNavigation | UiKind::PrimaryButton
            );
            self.renderer.draw_string_styled(
                Point::new(
                    rect.top + rect.height.saturating_sub(1) / 2,
                    Column(rect.left + offset),
                ),
                (fg, bg, bold),
                StrShortener::new(&string, available, ShortenDirection::Right, Some('…')),
                &text_size,
                &mut self.glyph_cache,
            );
            self.renderer.set_viewport(&size);
            if element.focused && !element.disabled && interactive {
                let top = y + 1.;
                let bottom = y + height - 2.;
                self.renderer.draw_rects(&size, &metrics, vec![
                    RenderRect::new(x + 5., top, (width - 10.).max(0.), 1., p.accent, 1.),
                    RenderRect::new(x + 5., bottom, (width - 10.).max(0.), 1., p.accent, 1.),
                    RenderRect::new(x, top + 4., 1., (height - 10.).max(0.), p.accent, 1.),
                    RenderRect::new(
                        x + width - 1.,
                        top + 4.,
                        1.,
                        (height - 10.).max(0.),
                        p.accent,
                        1.,
                    ),
                ]);
            }
        }
    }

    fn draw_language_dialog(&mut self) {
        let Some(dialog) = &self.language_dialog else { return };
        let size = self.size_info;
        let layout = dialog.layout(size.columns(), size.screen_lines());
        let elements = dialog.elements(size.columns(), size.screen_lines());
        self.draw_ui_panel(layout, true);
        self.draw_ui_elements(elements);
    }

    fn draw_close_confirmation(&mut self) {
        let Some(dialog) = &self.close_confirmation else { return };
        let size = self.size_info;
        let layout = dialog.layout(size.columns(), size.screen_lines());
        let elements = dialog.elements(size.columns(), size.screen_lines());
        self.draw_ui_panel(layout, true);
        self.draw_ui_elements(elements);
    }

    fn draw_productivity_dialog(&mut self) {
        let Some(dialog) = &self.productivity_dialog else { return };
        let size = self.size_info;
        let layout = dialog.layout(size.columns(), size.screen_lines());
        let elements = dialog.elements(size.columns(), size.screen_lines());
        self.draw_ui_panel(layout, true);
        self.draw_ui_elements(elements);
    }

    fn draw_context_menu(&mut self) {
        use crate::context_menu::MenuAction;
        use crate::tab_ui::{UiElement, UiKind, UiRect};

        let size = self.size_info;
        let Some(menu) = &mut self.context_menu else { return };
        menu.ensure_visible(size.columns(), size.screen_lines());
        let layout = menu.layout(size.columns(), size.screen_lines());
        let mut elements = menu.decorations(size.columns(), size.screen_lines());
        for item in menu.items(size.columns(), size.screen_lines()) {
            let selected = menu.selected == Some(item.action);
            let shortcut_width = unicode_width::UnicodeWidthStr::width(item.shortcut.as_str());
            let show_shortcut = shortcut_width > 0 && item.rect.width >= shortcut_width + 14;
            let available =
                item.rect.width.saturating_sub(if show_shortcut { shortcut_width + 4 } else { 2 });
            let text = StrShortener::new(&item.text, available, ShortenDirection::Right, Some('…'))
                .collect();
            elements.push(UiElement {
                rect: item.rect,
                kind: if matches!(item.action, MenuAction::Close | MenuAction::ClosePane) {
                    UiKind::DangerMenu
                } else {
                    UiKind::MenuItem
                },
                text,
                focused: false,
                hovered: selected,
                disabled: !item.enabled,
            });
            if show_shortcut {
                elements.push(UiElement {
                    rect: UiRect {
                        left: item.rect.left + item.rect.width - shortcut_width - 1,
                        width: shortcut_width,
                        ..item.rect
                    },
                    kind: UiKind::Shortcut,
                    text: item.shortcut,
                    focused: false,
                    hovered: selected,
                    disabled: !item.enabled,
                });
            }
        }
        self.draw_ui_panel(layout, false);
        self.draw_ui_elements(elements);
    }

    fn draw_tab_dialog(&mut self, _config: &UiConfig) {
        let Some(dialog) = &self.tab_dialog else { return };
        let size = self.size_info;
        let layout = dialog.layout(size.columns(), size.screen_lines());
        let elements = dialog.elements(size.columns(), size.screen_lines());
        self.draw_ui_panel(layout, true);
        self.draw_ui_elements(elements);
    }

    fn draw_tab_bar(&mut self, config: &UiConfig) {
        use crate::tabs::{TabActivity, TabBar, TabButton};
        use crate::ui::{Palette, rounded};

        let Some(bar) = &self.tab_bar else { return };
        let mut size = self.size_info;
        let height = size.cell_height() * TabBar::ROWS as f32;
        size.padding_y -= height;
        self.renderer.resize(&size);
        let layout = bar.layout(size.columns());
        let p = Palette::new(&self.colors);
        let metrics = self.glyph_cache.font_metrics();
        let top = size.padding_y();
        let mut rects = vec![
            RenderRect::new(0., 0., size.width(), self.size_info.padding_y(), p.panel, 1.),
            RenderRect::new(0., top + height - 1., size.width(), 1., p.border, 0.55),
        ];
        for slot in 0..layout.visible {
            let index = layout.first + slot;
            let active = index == bar.active;
            let hovered = matches!(bar.hovered, Some(TabButton::Select(i) | TabButton::Close(i)) if i == index);
            let x = size.padding_x() + (slot * layout.width) as f32 * size.cell_width();
            let width = layout.width as f32 * size.cell_width();
            let background = if active {
                p.selected
            } else if hovered {
                p.hover
            } else {
                p.panel
            };
            if active || hovered {
                rects.extend(rounded(
                    RenderRect::new(x + 2., top, (width - 4.).max(0.), height, background, 1.),
                    4.,
                ));
            } else if slot > 0 {
                rects.push(RenderRect::new(x, top + height * 0.3, 1., height * 0.4, p.border, 0.6));
            }
        }
        self.renderer.draw_rects(&size, &metrics, rects);
        let text_size = size;
        for slot in 0..layout.visible {
            let index = layout.first + slot;
            let active = index == bar.active;
            let hovered = matches!(bar.hovered, Some(TabButton::Select(i) | TabButton::Close(i)) if i == index);
            let bg = if active {
                p.selected
            } else if hovered {
                p.hover
            } else {
                p.panel
            };
            let column = slot * layout.width;
            let marker = bar.marker(index);
            let badge = if marker.is_empty() { (index + 1).to_string() } else { marker.into() };
            let show_badge = layout.width >= 12 || !marker.is_empty();
            let prefix = if show_badge { badge.len().min(3) + 2 } else { 1 };
            let prefix = if !marker.is_empty() && show_badge { 3 } else { prefix };
            self.renderer.set_viewport(&text_size);
            if show_badge {
                let color = match bar.activities.get(index).copied().unwrap_or_default() {
                    TabActivity::Running => p.accent,
                    TabActivity::Success => config.colors.normal.green,
                    TabActivity::Failed => p.danger,
                    TabActivity::Idle => p.muted,
                };
                self.renderer.draw_string_styled(
                    Point::new(0, Column(column + 1)),
                    (color, bg, false),
                    badge.chars().take(3),
                    &text_size,
                    &mut self.glyph_cache,
                );
            }
            let title_width = layout.width.saturating_sub(prefix + 4);
            let title = bar.label(index, title_width);
            self.renderer.draw_string_styled(
                Point::new(0, Column(column + prefix)),
                (p.text, bg, false),
                StrShortener::new(&title, title_width, ShortenDirection::Right, None),
                &text_size,
                &mut self.glyph_cache,
            );
            self.renderer.set_viewport(&size);
            if active {
                let x = size.padding_x() + column as f32 * size.cell_width();
                self.renderer.draw_rects(&size, &metrics, vec![RenderRect::new(
                    x + 8.,
                    top + height - 1.,
                    (layout.width as f32 * size.cell_width() - 16.).max(1.),
                    1.,
                    p.accent,
                    1.,
                )]);
            }
            if bar.close_visible(index) && layout.width >= 5 {
                let close_x =
                    size.padding_x() + (column + layout.width - 3) as f32 * size.cell_width();
                let hovered = bar.hovered == Some(TabButton::Close(index));
                let center_x = close_x + size.cell_width();
                let center_y = top + height / 2.;
                let side = (height - 6.).min(size.cell_width() * 2. - 4.).max(4.);
                let color = if hovered { p.danger } else { p.muted };
                let mut icon = Vec::new();
                if hovered {
                    icon.extend(rounded(
                        RenderRect::new(
                            center_x - side / 2.,
                            center_y - side / 2.,
                            side,
                            side,
                            p.danger_hover,
                            1.,
                        ),
                        5.,
                    ));
                }
                let scale = (side / 12.).max(0.5);
                for step in -3..=3 {
                    let offset = step as f32 * scale;
                    icon.push(RenderRect::new(
                        center_x + offset - scale / 2.,
                        center_y + offset - scale / 2.,
                        scale * 1.5,
                        scale * 1.5,
                        color,
                        1.,
                    ));
                    icon.push(RenderRect::new(
                        center_x + offset - scale / 2.,
                        center_y - offset - scale / 2.,
                        scale * 1.5,
                        scale * 1.5,
                        color,
                        1.,
                    ));
                }
                self.renderer.draw_rects(&size, &metrics, icon);
            }
        }
        for (offset, action) in [(0, TabButton::Create), (3, TabButton::Menu)] {
            let column = layout.button_start + offset;
            if column >= size.columns() {
                continue;
            }
            let width = (size.columns() - column).min(3) as f32 * size.cell_width();
            let x = size.padding_x() + column as f32 * size.cell_width();
            let hovered = bar.hovered == Some(action);
            let center_x = x + width / 2.;
            let center_y = top + height / 2.;
            let mut icon = Vec::new();
            if hovered {
                icon.extend(rounded(
                    RenderRect::new(x + 2., top, (width - 4.).max(0.), height, p.hover, 1.),
                    4.,
                ));
            }
            let color = if hovered { p.text } else { p.muted };
            let radius = (size.cell_width() * 0.45).min(width / 3.).max(1.);
            if action == TabButton::Create {
                icon.extend([
                    RenderRect::new(center_x - radius, center_y - 1., radius * 2., 2., color, 1.),
                    RenderRect::new(center_x - 1., center_y - radius, 2., radius * 2., color, 1.),
                ]);
            } else {
                for offset in [-1., 0., 1.] {
                    icon.extend(rounded(
                        RenderRect::new(
                            center_x - 1.5,
                            center_y + offset * radius - 1.5,
                            3.,
                            3.,
                            color,
                            1.,
                        ),
                        1.5,
                    ));
                }
            }
            self.renderer.draw_rects(&size, &metrics, icon);
        }
        self.renderer.resize(&self.size_info);
        self.damage_tracker.frame().add_viewport_rect(
            &self.size_info,
            0,
            0,
            size.width() as i32,
            self.size_info.padding_y() as i32,
        );
    }

    #[inline(never)]
    fn draw_render_timer(&mut self, config: &UiConfig) {
        if !config.debug.render_timer {
            return;
        }

        let timing = format!("{:.3} usec", self.meter.average());
        let point = Point::new(self.size_info.screen_lines().saturating_sub(2), Column(0));
        let fg = config.colors.primary.background;
        let bg = config.colors.normal.red;

        // Damage render timer for current and next frame.
        let damage = LineDamageBounds::new(point.line, point.column.0, timing.len());
        self.damage_tracker.frame().damage_line(damage);
        self.damage_tracker.next_frame().damage_line(damage);

        let glyph_cache = &mut self.glyph_cache;
        self.renderer.draw_string(point, fg, bg, timing.chars(), &self.size_info, glyph_cache);
    }

    /// Draw an indicator for the position of a line in history.
    #[inline(never)]
    fn draw_line_indicator(
        &mut self,
        config: &UiConfig,
        total_lines: usize,
        obstructed_column: Option<Column>,
        line: usize,
    ) {
        let columns = self.size_info.columns();
        let text = format!("[{}/{}]", line, total_lines - 1);
        let column = Column(self.size_info.columns().saturating_sub(text.len()));
        let point = Point::new(0, column);

        // Damage the line indicator for current and next frame.
        let damage = LineDamageBounds::new(point.line, point.column.0, columns - 1);
        self.damage_tracker.frame().damage_line(damage);
        self.damage_tracker.next_frame().damage_line(damage);

        let colors = &config.colors;
        let fg = colors.line_indicator.foreground.unwrap_or(colors.primary.background);
        let bg = colors.line_indicator.background.unwrap_or(colors.primary.foreground);

        // Do not render anything if it would obscure the vi mode cursor.
        if obstructed_column.is_none_or(|obstructed_column| obstructed_column < column) {
            let glyph_cache = &mut self.glyph_cache;
            self.renderer.draw_string(point, fg, bg, text.chars(), &self.size_info, glyph_cache);
        }
    }

    /// Highlight damaged rects.
    ///
    /// This function is for debug purposes only.
    fn highlight_damage(&self, render_rects: &mut Vec<RenderRect>) {
        for damage_rect in &self.damage_tracker.shape_frame_damage(self.size_info.into()) {
            let x = damage_rect.x as f32;
            let height = damage_rect.height as f32;
            let width = damage_rect.width as f32;
            let y = damage_y_to_viewport_y(&self.size_info, damage_rect) as f32;
            let render_rect = RenderRect::new(x, y, width, height, DAMAGE_RECT_COLOR, 0.5);

            render_rects.push(render_rect);
        }
    }

    /// Check whether a hint highlight needs to be cleared.
    fn validate_hint_highlights(&mut self, display_offset: usize) {
        let frame = self.damage_tracker.frame();
        let hints = [
            (&mut self.highlighted_hint, &mut self.highlighted_hint_age, true),
            (&mut self.vi_highlighted_hint, &mut self.vi_highlighted_hint_age, false),
        ];

        let num_lines = self.size_info.screen_lines();
        for (hint, hint_age, reset_mouse) in hints {
            let (start, end) = match hint {
                Some(hint) => (*hint.bounds().start(), *hint.bounds().end()),
                None => continue,
            };

            // Ignore hints that were created this frame.
            *hint_age += 1;
            if *hint_age == 1 {
                continue;
            }

            // Convert hint bounds to viewport coordinates.
            let start = term::point_to_viewport(display_offset, start)
                .filter(|point| point.line < num_lines)
                .unwrap_or_default();
            let end = term::point_to_viewport(display_offset, end)
                .filter(|point| point.line < num_lines)
                .unwrap_or_else(|| Point::new(num_lines - 1, self.size_info.last_column()));

            // Clear invalidated hints.
            if frame.intersects(start, end) {
                if reset_mouse {
                    self.window.set_mouse_cursor(CursorIcon::Default);
                }
                frame.mark_fully_damaged();
                *hint = None;
            }
        }
    }

    /// Request a new frame for a window on Wayland.
    fn request_frame(&mut self, scheduler: &mut Scheduler) {
        // Mark that we've used a frame.
        self.window.has_frame = false;

        // Get the display vblank interval.
        let monitor_vblank_interval = 1_000_000.
            / self
                .window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz())
                .unwrap_or(60_000) as f64;

        // Now convert it to micro seconds.
        let monitor_vblank_interval =
            Duration::from_micros((1000. * monitor_vblank_interval) as u64);

        let swap_timeout = self.frame_timer.compute_timeout(monitor_vblank_interval);

        let window_id = self.window.id();
        let timer_id = TimerId::new(Topic::Frame, window_id);
        let event = Event::new(EventType::Frame, window_id);

        scheduler.schedule(event, swap_timeout, false, timer_id);
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        // Switch OpenGL context before dropping, otherwise objects (like programs) from other
        // contexts might be deleted when dropping renderer.
        self.make_current();
        unsafe {
            ManuallyDrop::drop(&mut self.renderer);
            ManuallyDrop::drop(&mut self.context);
            ManuallyDrop::drop(&mut self.surface);
        }
    }
}

/// Input method state.
#[derive(Debug, Default)]
pub struct Ime {
    /// Whether the IME is enabled.
    enabled: bool,

    /// Current IME preedit.
    preedit: Option<Preedit>,
}

impl Ime {
    #[inline]
    pub fn set_enabled(&mut self, is_enabled: bool) {
        if is_enabled {
            self.enabled = is_enabled
        } else {
            // Clear state when disabling IME.
            *self = Default::default();
        }
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[inline]
    pub fn set_preedit(&mut self, preedit: Option<Preedit>) {
        self.preedit = preedit;
    }

    #[inline]
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    /// The preedit text.
    text: String,

    /// Byte offset for cursor start into the preedit text.
    ///
    /// `None` means that the cursor is invisible.
    cursor_byte_offset: Option<(usize, usize)>,

    /// The cursor offset from the end of the start of the preedit in char width.
    cursor_end_offset: Option<(usize, usize)>,
}

impl Preedit {
    pub fn new(text: String, cursor_byte_offset: Option<(usize, usize)>) -> Self {
        let cursor_end_offset = if let Some(byte_offset) = cursor_byte_offset {
            // Convert byte offset into char offset.
            let start_to_end_offset =
                text[byte_offset.0..].chars().fold(0, |acc, ch| acc + ch.width().unwrap_or(1));
            let end_to_end_offset =
                text[byte_offset.1..].chars().fold(0, |acc, ch| acc + ch.width().unwrap_or(1));

            Some((start_to_end_offset, end_to_end_offset))
        } else {
            None
        };

        Self { text, cursor_byte_offset, cursor_end_offset }
    }
}

/// Pending renderer updates.
///
/// All renderer updates are cached to be applied just before rendering, to avoid platform-specific
/// rendering issues.
#[derive(Debug, Default, Copy, Clone)]
pub struct RendererUpdate {
    /// Should resize the window.
    resize: bool,

    /// Clear font caches.
    clear_font_cache: bool,
}

/// The frame timer state.
pub struct FrameTimer {
    /// Base timestamp used to compute sync points.
    base: Instant,

    /// The last timestamp we synced to.
    last_synced_timestamp: Instant,

    /// The refresh rate we've used to compute sync timestamps.
    refresh_interval: Duration,
}

impl FrameTimer {
    pub fn new() -> Self {
        let now = Instant::now();
        Self { base: now, last_synced_timestamp: now, refresh_interval: Duration::ZERO }
    }

    /// Compute the delay that we should use to achieve the target frame
    /// rate.
    pub fn compute_timeout(&mut self, refresh_interval: Duration) -> Duration {
        let now = Instant::now();

        // Handle refresh rate change.
        if self.refresh_interval != refresh_interval {
            self.base = now;
            self.last_synced_timestamp = now;
            self.refresh_interval = refresh_interval;
            return refresh_interval;
        }

        let next_frame = self.last_synced_timestamp + self.refresh_interval;

        if next_frame < now {
            // Redraw immediately if we haven't drawn in over `refresh_interval` microseconds.
            let elapsed_micros = (now - self.base).as_micros() as u64;
            let refresh_micros = self.refresh_interval.as_micros() as u64;
            self.last_synced_timestamp =
                now - Duration::from_micros(elapsed_micros % refresh_micros);
            Duration::ZERO
        } else {
            // Redraw on the next `refresh_interval` clock tick.
            self.last_synced_timestamp = next_frame;
            next_frame - now
        }
    }
}

/// Calculate the cell dimensions based on font metrics.
///
/// This will return a tuple of the cell width and height.
#[inline]
fn compute_cell_size(config: &UiConfig, metrics: &crossfont::Metrics) -> (f32, f32) {
    let offset_x = f64::from(config.font.offset.x);
    let offset_y = f64::from(config.font.offset.y);
    (
        (metrics.average_advance + offset_x).floor().max(1.) as f32,
        (metrics.line_height + offset_y).floor().max(1.) as f32,
    )
}

/// Calculate the size of the window given padding, terminal dimensions and cell size.
fn window_size(
    config: &UiConfig,
    dimensions: Dimensions,
    cell_width: f32,
    cell_height: f32,
    scale_factor: f32,
) -> PhysicalSize<u32> {
    let padding = config.window.padding(scale_factor);

    let grid_width = cell_width * dimensions.columns.max(MIN_COLUMNS) as f32;
    let grid_height = cell_height * dimensions.lines.max(MIN_SCREEN_LINES) as f32;

    let width = (padding.0).mul_add(2., grid_width).floor();
    let height = (padding.1).mul_add(2., grid_height).floor();

    PhysicalSize::new(width as u32, height as u32)
}

#[cfg(test)]
mod language_tests {
    use super::*;
    use crate::localization::{Locale, Message};

    #[test]
    fn localized_search_labels_fit_even_when_the_label_fills_the_bar() {
        for locale in [Locale::En, Locale::PtBr] {
            for key in [Message::SearchForward, Message::SearchBackward] {
                let label = locale.text(key);
                for columns in 1..=24 {
                    let text = Display::format_search("user-query", label, columns);
                    assert!(text.chars().count() <= columns);
                }
                assert!(Display::format_search("user-query", label, 80).contains("user-query"));
            }
        }
    }
}

#[cfg(test)]
mod pane_size_tests {
    use super::*;

    #[test]
    fn pane_coordinates_preserve_window_projection_and_asymmetric_padding() {
        let mut window = SizeInfo::new(1000., 800., 10., 20., 10., 10., false);
        window.reserve_top_lines(crate::tabs::TabBar::ROWS);
        let pane = window.pane(40, 5, 20, 10);
        assert_eq!((pane.width(), pane.height()), (1000., 800.));
        assert_eq!((pane.padding_x(), pane.padding_y()), (410., 130.));
        assert_eq!((pane.right_padding_x(), pane.bottom_padding_y()), (390., 470.));
        assert_eq!((pane.columns(), pane.screen_lines()), (20, 10));
        assert!(pane.contains_point(411, 131));
        assert!(!pane.contains_point(610, 331));
        assert!(!pane.contains_point(611, 330));
        let physical: SizeInfo<u32> = pane.into();
        assert_eq!((physical.padding_x(), physical.right_padding_x()), (410, 390));
        let pty: WindowSize = pane.into();
        assert_eq!((pty.num_cols, pty.num_lines), (20, 10));
    }

    #[test]
    fn pane_search_reserves_its_own_row_without_changing_clip_region() {
        let window = SizeInfo::new(1000., 800., 10., 20., 10., 10., false);
        let mut pane = window.pane(3, 4, 20, 10);
        let bottom = pane.bottom_padding_y();
        pane.reserve_lines(1);
        assert_eq!(pane.screen_lines(), 9);
        assert_eq!(pane.bottom_padding_y(), bottom);
        assert_eq!(window.screen_lines(), 39);
        let minimum = window.pane(0, 0, 0, 0);
        assert_eq!((minimum.columns(), minimum.screen_lines()), (MIN_COLUMNS, MIN_SCREEN_LINES));
    }
}
