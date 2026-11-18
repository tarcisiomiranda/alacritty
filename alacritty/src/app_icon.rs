use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use libloading::Library;

use crate::desktop_apps::AppKind;
use crate::display::color::Rgb;
use crate::renderer::rects::RenderRect;

#[repr(C)]
struct ImageError {
    domain: c_uint,
    code: c_int,
    message: *mut c_char,
}

struct Span {
    x: usize,
    y: usize,
    width: usize,
    rgba: [u8; 4],
}

pub struct AppIcon {
    width: usize,
    height: usize,
    spans: Vec<Span>,
}

impl AppIcon {
    pub fn load(kind: AppKind, id: &str) -> Arc<Self> {
        let names: &[&str] = match id {
            "code" => &["vscode", "code", "com.visualstudio.code"],
            "zed" => &["dev.zed.Zed", "zed"],
            "codium" => &["vscodium", "codium", "com.vscodium.codium"],
            "code-insiders" => &["code-insiders", "vscode-insiders"],
            "kate" => &["org.kde.kate", "kate"],
            "sublime" => &["sublime-text", "sublime_text"],
            "nautilus" => &["org.gnome.Nautilus", "nautilus"],
            "dolphin" => &["org.kde.dolphin", "dolphin"],
            "nemo" => &["nemo"],
            "thunar" => &["org.xfce.thunar", "Thunar", "thunar"],
            "caja" => &["caja"],
            "pcmanfm" => &["pcmanfm", "pcmanfm-qt"],
            _ if kind == AppKind::FileManager => &["system-file-manager", "folder"],
            _ => &["accessories-text-editor", "text-editor"],
        };
        let mut roots = Vec::new();
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            roots.push(home.join(".icons"));
            roots.push(home.join(".local/share/icons"));
            roots.push(home.join(".local/share/flatpak/exports/share/icons"));
        }
        if let Some(data) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            roots.insert(0, data.join("icons"));
        }
        let data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        roots.extend(
            std::env::split_paths(&data_dirs)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("icons")),
        );
        roots.push("/var/lib/flatpak/exports/share/icons".into());
        let icon = find_icon(names, &roots, &[
            "/usr/local/share/pixmaps".into(),
            "/usr/share/pixmaps".into(),
        ])
        .and_then(|path| {
            let icon = Self::from_file(&path);
            log::debug!(
                "Application icon {}: {}",
                path.display(),
                if icon.is_some() { "loaded" } else { "unavailable" }
            );
            icon
        })
        .unwrap_or_else(|| Self::fallback(kind));
        Arc::new(icon)
    }

    fn from_file(path: &Path) -> Option<Self> {
        let library = decoder()?;
        let filename = CString::new(path.as_os_str().as_bytes()).ok()?;
        if !path.metadata().ok()?.is_file() || path.metadata().ok()?.len() > 8 * 1024 * 1024 {
            return None;
        }
        unsafe {
            let load = library
                .get::<unsafe extern "C" fn(
                    *const c_char,
                    c_int,
                    c_int,
                    c_int,
                    *mut *mut ImageError,
                ) -> *mut c_void>(b"gdk_pixbuf_new_from_file_at_scale\0")
                .ok()?;
            let width = library
                .get::<unsafe extern "C" fn(*const c_void) -> c_int>(b"gdk_pixbuf_get_width\0")
                .ok()?;
            let height = library
                .get::<unsafe extern "C" fn(*const c_void) -> c_int>(b"gdk_pixbuf_get_height\0")
                .ok()?;
            let stride = library
                .get::<unsafe extern "C" fn(*const c_void) -> c_int>(b"gdk_pixbuf_get_rowstride\0")
                .ok()?;
            let channels = library
                .get::<unsafe extern "C" fn(*const c_void) -> c_int>(b"gdk_pixbuf_get_n_channels\0")
                .ok()?;
            let bits = library
                .get::<unsafe extern "C" fn(*const c_void) -> c_int>(
                    b"gdk_pixbuf_get_bits_per_sample\0",
                )
                .ok()?;
            let pixels = library
                .get::<unsafe extern "C" fn(*const c_void, *mut c_uint) -> *mut u8>(
                    b"gdk_pixbuf_get_pixels_with_length\0",
                )
                .ok()?;
            let unref =
                library.get::<unsafe extern "C" fn(*mut c_void)>(b"g_object_unref\0").ok()?;
            let free_error =
                library.get::<unsafe extern "C" fn(*mut ImageError)>(b"g_error_free\0").ok()?;
            let mut error = std::ptr::null_mut();
            let image = load(filename.as_ptr(), 32, 32, 1, &mut error);
            if image.is_null() {
                if !error.is_null() {
                    if !(*error).message.is_null() {
                        log::debug!(
                            "Application icon loader: {}",
                            CStr::from_ptr((*error).message).to_string_lossy()
                        );
                    }
                    free_error(error);
                }
                return None;
            }
            let (width, height, stride, channels) =
                (width(image), height(image), stride(image), channels(image));
            let mut length = 0;
            let data = pixels(image, &mut length);
            let result = if (1..=32).contains(&width)
                && (1..=32).contains(&height)
                && (3..=4).contains(&channels)
                && stride >= width * channels
                && bits(image) == 8
                && !data.is_null()
                && (stride as u64 * (height as u64 - 1) + (width * channels) as u64)
                    <= u64::from(length)
            {
                let data = std::slice::from_raw_parts(data, length as usize);
                let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                for row in 0..height as usize {
                    for column in 0..width as usize {
                        let offset = row * stride as usize + column * channels as usize;
                        rgba.extend_from_slice(&data[offset..offset + 3]);
                        rgba.push(if channels == 4 { data[offset + 3] } else { 255 });
                    }
                }
                Self::from_rgba(width as usize, height as usize, &rgba)
            } else {
                None
            };
            unref(image);
            result
        }
    }

    fn from_rgba(width: usize, height: usize, pixels: &[u8]) -> Option<Self> {
        if width == 0
            || height == 0
            || width > 64
            || height > 64
            || pixels.len() != width * height * 4
        {
            return None;
        }
        let mut spans = Vec::new();
        for y in 0..height {
            let mut x = 0;
            while x < width {
                let start = x;
                let offset = (y * width + x) * 4;
                let rgba =
                    [pixels[offset], pixels[offset + 1], pixels[offset + 2], pixels[offset + 3]];
                x += 1;
                while x < width && pixels[(y * width + x) * 4..(y * width + x + 1) * 4] == rgba {
                    x += 1;
                }
                if rgba[3] != 0 {
                    spans.push(Span { x: start, y, width: x - start, rgba });
                }
            }
        }
        Some(Self { width, height, spans })
    }

    fn fallback(kind: AppKind) -> Self {
        let mut pixels = vec![0; 16 * 16 * 4];
        for y in 2usize..14 {
            for x in 1usize..15 {
                let visible = match kind {
                    AppKind::FileManager => y >= 5 || (y >= 3 && x <= 7),
                    AppKind::Editor => {
                        (x.abs_diff(4usize) + y.abs_diff(8usize) <= 3)
                            || (x.abs_diff(11usize) + y.abs_diff(8usize) <= 3)
                    },
                };
                if visible {
                    let rgba = if y < 6 { [104, 180, 235, 255] } else { [55, 136, 215, 255] };
                    let offset = (y * 16 + x) * 4;
                    pixels[offset..offset + 4].copy_from_slice(&rgba);
                }
            }
        }
        Self::from_rgba(16, 16, &pixels).unwrap()
    }

    pub fn rects(&self, x: f32, y: f32, side: f32) -> Vec<RenderRect> {
        if !side.is_finite() || side <= 0. {
            return Vec::new();
        }
        let scale = side / self.width.max(self.height) as f32;
        let left = x + (side - self.width as f32 * scale) / 2.;
        let top = y + (side - self.height as f32 * scale) / 2.;
        self.spans
            .iter()
            .map(|span| {
                RenderRect::new(
                    left + span.x as f32 * scale,
                    top + span.y as f32 * scale,
                    span.width as f32 * scale,
                    scale,
                    Rgb::new(span.rgba[0], span.rgba[1], span.rgba[2]),
                    f32::from(span.rgba[3]) / 255.,
                )
            })
            .collect()
    }
}

fn decoder() -> Option<&'static Library> {
    static DECODER: OnceLock<Option<Library>> = OnceLock::new();
    DECODER.get_or_init(|| unsafe { Library::new("libgdk_pixbuf-2.0.so.0").ok() }).as_ref()
}

fn find_icon(names: &[&str], roots: &[PathBuf], pixmaps: &[PathBuf]) -> Option<PathBuf> {
    for name in names {
        for root in roots {
            for size in [
                "32x32", "48x48", "24x24", "scalable", "64x64", "128x128", "256x256", "512x512",
                "16x16",
            ] {
                for context in ["apps", "places"] {
                    for extension in ["png", "svg", "xpm"] {
                        let path = root
                            .join("hicolor")
                            .join(size)
                            .join(context)
                            .join(format!("{name}.{extension}"));
                        if path.is_file() {
                            return Some(path);
                        }
                    }
                }
            }
        }
        for root in roots.iter().chain(pixmaps) {
            for extension in ["png", "svg", "xpm"] {
                let path = root.join(format!("{name}.{extension}"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_icon_spans_preserve_color_alpha_and_rectangular_bounds() {
        let pixels = [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 255, 128];
        let icon = AppIcon::from_rgba(2, 2, &pixels).unwrap();
        assert_eq!(icon.spans.len(), 2);
        let rects = icon.rects(10., 20., 18.);
        assert_eq!(rects[0].width, 18.);
        assert_eq!(rects[1].alpha, 128. / 255.);
        assert!(rects.iter().all(|rect| rect.x >= 10.
            && rect.y >= 20.
            && rect.x + rect.width <= 28.
            && rect.y + rect.height <= 38.));
        assert!(icon.rects(0., 0., 0.).is_empty());
        assert!(AppIcon::from_rgba(2, 2, &pixels[..4]).is_none());
        assert!(AppIcon::from_rgba(0, 0, &[]).is_none());
    }

    #[test]
    fn app_icon_lookup_uses_user_roots_and_pixmap_fallbacks() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let system = root.path().join("system");
        std::fs::create_dir_all(user.join("hicolor/scalable/apps")).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        let svg = user.join("hicolor/scalable/apps/org.gnome.Nautilus.svg");
        std::fs::write(&svg, "svg").unwrap();
        let png = system.join("vscode.png");
        std::fs::write(&png, "png").unwrap();
        assert_eq!(
            find_icon(
                &["org.gnome.Nautilus"],
                std::slice::from_ref(&user),
                std::slice::from_ref(&system)
            ),
            Some(svg)
        );
        assert_eq!(find_icon(&["vscode"], &[user], &[system]), Some(png.clone()));
        assert!(AppIcon::from_file(&root.path().join("missing")).is_none());
        assert!(AppIcon::from_file(&png).is_none());
    }

    #[test]
    fn app_icon_native_fallbacks_remain_visible_without_desktop_libraries() {
        for kind in [AppKind::Editor, AppKind::FileManager] {
            let icon = AppIcon::fallback(kind);
            assert!(!icon.rects(0., 0., 16.).is_empty());
        }
    }
}
