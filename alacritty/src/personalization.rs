use std::io::Cursor;
use std::sync::OnceLock;

use alacritty_config_derive::SerdeReplace;
use serde::{Deserialize, Serialize};

use crate::display::color::Rgb;
use crate::localization::{Locale, Message};
use crate::renderer::rects::RenderRect;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, SerdeReplace, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IconPreset {
    #[default]
    Termust,
    Cat,
    Dog,
    Fox,
    Robot,
    Yorkie,
}

impl IconPreset {
    pub const ALL: [Self; 6] =
        [Self::Termust, Self::Cat, Self::Dog, Self::Fox, Self::Robot, Self::Yorkie];

    pub fn id(self) -> &'static str {
        match self {
            Self::Termust => "termust",
            Self::Cat => "cat",
            Self::Dog => "dog",
            Self::Fox => "fox",
            Self::Robot => "robot",
            Self::Yorkie => "yorkie",
        }
    }

    pub fn label(self, locale: Locale) -> &'static str {
        locale.text(match self {
            Self::Termust => Message::IconOriginal,
            Self::Cat => Message::IconCat,
            Self::Dog => Message::IconDog,
            Self::Fox => Message::IconFox,
            Self::Robot => Message::IconRobot,
            Self::Yorkie => Message::IconYorkie,
        })
    }

    pub fn png(self) -> &'static [u8] {
        match self {
            Self::Termust => include_bytes!("../../extra/logo/termust.png"),
            Self::Cat => include_bytes!("../../extra/icons/cat.png"),
            Self::Dog => include_bytes!("../../extra/icons/dog.png"),
            Self::Fox => include_bytes!("../../extra/icons/fox.png"),
            Self::Robot => include_bytes!("../../extra/icons/robot.png"),
            Self::Yorkie => include_bytes!("../../extra/icons/yorkie.png"),
        }
    }

    fn image(self) -> &'static IconImage {
        static IMAGES: [OnceLock<IconImage>; IconPreset::ALL.len()] =
            [const { OnceLock::new() }; IconPreset::ALL.len()];
        IMAGES[self as usize].get_or_init(|| IconImage::decode(self.png()))
    }

    pub fn rects(self, x: f32, y: f32, side: f32) -> Vec<RenderRect> {
        let image = self.image();
        let samples = 64usize;
        let pixel = side / samples as f32;
        let mut rects = Vec::new();
        for row in 0..samples {
            let mut column = 0;
            while column < samples {
                let color = image.sample(column, row, samples);
                let start = column;
                column += 1;
                while column < samples && image.sample(column, row, samples) == color {
                    column += 1;
                }
                if color[3] == 0 {
                    continue;
                }
                rects.push(RenderRect::new(
                    x + start as f32 * pixel,
                    y + row as f32 * pixel,
                    (column - start) as f32 * pixel,
                    pixel,
                    Rgb::new(color[0], color[1], color[2]),
                    f32::from(color[3]) / 255.,
                ));
            }
        }
        rects
    }

    #[cfg(not(target_os = "macos"))]
    pub fn window_icon(self) -> winit::window::Icon {
        let image = self.image();
        winit::window::Icon::from_rgba(image.rgba.clone(), image.width, image.height)
            .expect("valid embedded terminal icon")
    }
}

struct IconImage {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

impl IconImage {
    fn decode(bytes: &[u8]) -> Self {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().expect("valid embedded icon PNG");
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).expect("complete embedded icon PNG");
        let rgba = match info.color_type {
            png::ColorType::Rgba => buffer[..info.buffer_size()].to_vec(),
            png::ColorType::Rgb => buffer[..info.buffer_size()]
                .chunks_exact(3)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
                .collect(),
            _ => panic!("embedded icon must use RGB or RGBA"),
        };
        Self { rgba, width: info.width, height: info.height }
    }

    fn sample(&self, x: usize, y: usize, samples: usize) -> [u8; 4] {
        let x = x * self.width as usize / samples;
        let y = y * self.height as usize / samples;
        let index = (y * self.width as usize + x) * 4;
        self.rgba[index..index + 4].try_into().unwrap()
    }
}

pub fn set_window_icon(window: &winit::window::Window, preset: IconPreset) {
    #[cfg(not(target_os = "macos"))]
    {
        let icon = preset.window_icon();
        window.set_window_icon(Some(icon.clone()));
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowExtWindows;
            window.set_taskbar_icon(Some(icon));
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;

        let _ = window;
        let Some(main_thread) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(main_thread);
        let image = if preset == IconPreset::Termust {
            None
        } else {
            let data = NSData::with_bytes(preset.png());
            NSImage::initWithData(NSImage::alloc(), &data)
        };
        unsafe { app.setApplicationIconImage(image.as_deref()) };
    }
}

#[cfg(target_os = "linux")]
mod launcher {
    use std::fs;
    use std::io::{self, Write};
    use std::path::Path;

    use super::IconPreset;

    pub fn sync(preset: IconPreset) -> io::Result<()> {
        let directories = xdg::BaseDirectories::new();
        let root = directories
            .get_data_home()
            .ok_or_else(|| io::Error::other("no user data directory"))?;
        let destination = root.join("applications/Termust.desktop");
        if destination.is_symlink() {
            return Err(io::Error::other("personalized launcher must not be a symbolic link"));
        }
        let source = if destination.exists() {
            fs::read_to_string(&destination)?
        } else if preset == IconPreset::Termust {
            return Ok(());
        } else if let Some(path) = directories.find_data_file("applications/Termust.desktop") {
            fs::read_to_string(path)?
        } else {
            let executable = std::env::current_exe()?;
            launcher_template(&executable)?
        };
        let icon = root.join(format!("termust/icons/v1/{}.png", preset.id()));
        let updated = desktop_entry(&source, preset, &icon)?;
        if preset != IconPreset::Termust {
            write(&icon, preset.png())?;
        }
        if updated != source {
            write(&destination, updated.as_bytes())?;
        }
        Ok(())
    }

    fn launcher_template(executable: &Path) -> io::Result<String> {
        let executable = executable
            .to_str()
            .filter(|path| !path.chars().any(char::is_control))
            .ok_or_else(|| io::Error::other("invalid executable path"))?;
        let escaped = executable
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
            .replace('%', "%%")
            .replace('\\', "\\\\");
        let command = format!("Exec=\"{escaped}\"");
        let try_command = format!("TryExec={}", executable.replace('\\', "\\\\"));
        let lines: Vec<_> = include_str!("../../extra/linux/Termust.desktop")
            .lines()
            .map(|line| match line {
                "Exec=termust" => command.as_str(),
                "TryExec=termust" => try_command.as_str(),
                _ => line,
            })
            .collect();
        Ok(lines.join("\n") + "\n")
    }

    fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
        if fs::read(path).is_ok_and(|current| current == contents) {
            return Ok(());
        }
        let parent = path.parent().ok_or_else(|| io::Error::other("no icon directory"))?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(contents)?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub(super) fn desktop_entry(
        source: &str,
        preset: IconPreset,
        path: &Path,
    ) -> io::Result<String> {
        let path = path
            .to_str()
            .filter(|path| !path.chars().any(char::is_control))
            .ok_or_else(|| io::Error::other("invalid icon path"))?;
        let mut section = false;
        let mut original = None;
        let mut current = None;
        let mut found = false;
        for line in source.lines() {
            if line.starts_with('[') {
                section = line == "[Desktop Entry]";
                found |= section;
            } else if section {
                if let Some(icon) = line.strip_prefix("Icon=") {
                    current = Some(icon);
                } else if let Some(icon) = line.strip_prefix("X-Termust-OriginalIcon=") {
                    original = Some(icon);
                }
            }
        }
        if !found {
            return Err(io::Error::other("launcher has no Desktop Entry section"));
        }
        if preset == IconPreset::Termust && original.is_none() {
            return Ok(source.into());
        }
        let original = original.or(current).unwrap_or("Termust");
        let icon = if preset == IconPreset::Termust {
            original.to_owned()
        } else {
            path.replace('\\', "\\\\")
        };
        let mut lines = Vec::new();
        section = false;
        for line in source.lines() {
            if line.starts_with('[') {
                section = line == "[Desktop Entry]";
                lines.push(line.to_owned());
                if section {
                    lines.push(format!("Icon={icon}"));
                    if preset != IconPreset::Termust {
                        lines.push(format!("X-Termust-OriginalIcon={original}"));
                        lines.push(format!("X-Termust-IconPreset={}", preset.id()));
                    }
                }
            } else if !section
                || !(line.starts_with("Icon=")
                    || line.starts_with("X-Termust-OriginalIcon=")
                    || line.starts_with("X-Termust-IconPreset="))
            {
                lines.push(line.to_owned());
            }
        }
        Ok(lines.join("\n") + "\n")
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn icon_changes_preserve_launcher_identity_and_actions_and_restore_original() {
            let original = "[Desktop Entry]\nName=My Termust\nIcon=custom-original\nExec=/opt/termust --hold\nStartupWMClass=Termust\n\n[Desktop Action New]\nIcon=another\nExec=/opt/termust\n";
            let icon = PathBuf::from("/home/user data/cat.png");
            let cat = desktop_entry(original, IconPreset::Cat, &icon).unwrap();
            assert!(cat.contains("Icon=/home/user data/cat.png\n"));
            assert!(cat.contains("Exec=/opt/termust --hold\nStartupWMClass=Termust"));
            assert!(cat.contains("[Desktop Action New]\nIcon=another"));
            assert_eq!(cat, desktop_entry(&cat, IconPreset::Cat, &icon).unwrap());
            let dog = desktop_entry(&cat, IconPreset::Dog, Path::new("/dog.png")).unwrap();
            let restored = desktop_entry(&dog, IconPreset::Termust, &icon).unwrap();
            assert!(restored.contains("Icon=custom-original\n"));
            assert!(!restored.contains("X-Termust-"));
            assert_eq!(original, desktop_entry(original, IconPreset::Termust, &icon).unwrap());
        }

        #[test]
        fn icon_changes_reject_invalid_launchers_and_paths() {
            assert!(
                desktop_entry("[Other]\nIcon=x", IconPreset::Cat, Path::new("/cat.png")).is_err()
            );
            assert!(
                desktop_entry("[Desktop Entry]", IconPreset::Cat, Path::new("/bad\npath")).is_err()
            );
        }

        #[test]
        fn icon_launcher_escapes_commands_without_quoting_try_exec_or_changing_identity() {
            let launcher = launcher_template(Path::new("/home/user space/$test/termust")).unwrap();
            assert!(launcher.contains("TryExec=/home/user space/$test/termust\n"));
            assert_eq!(
                launcher
                    .lines()
                    .filter(|line| *line == "Exec=\"/home/user space/\\\\$test/termust\"")
                    .count(),
                2
            );
            assert!(launcher.contains("StartupWMClass=Termust\n"));
            assert!(launcher_template(Path::new("/bad\npath")).is_err());
        }
    }
}

pub fn sync_launcher(preset: IconPreset) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    return launcher::sync(preset);
    #[cfg(windows)]
    return windows_launcher::sync(preset);
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = preset;
        Ok(())
    }
}

#[cfg(windows)]
mod windows_launcher {
    use std::fs;
    use std::io;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use super::IconPreset;

    pub fn sync(preset: IconPreset) -> io::Result<()> {
        let data = dirs::data_local_dir()
            .ok_or_else(|| io::Error::other("no local application directory"))?
            .join("Termust");
        let backup = data.join("launcher-original-icon.txt");
        if preset == IconPreset::Termust && !backup.exists() {
            return Ok(());
        }
        let shortcut = dirs::data_dir()
            .ok_or_else(|| io::Error::other("no shortcut directory"))?
            .join("Microsoft/Windows/Start Menu/Programs/Termust/Termust.lnk");
        let icon = data.join(format!("icons/v1/{}.ico", preset.id()));
        let bytes: &[u8] = match preset {
            IconPreset::Termust => include_bytes!("../../extra/icons/termust.ico"),
            IconPreset::Cat => include_bytes!("../../extra/icons/cat.ico"),
            IconPreset::Dog => include_bytes!("../../extra/icons/dog.ico"),
            IconPreset::Fox => include_bytes!("../../extra/icons/fox.ico"),
            IconPreset::Robot => include_bytes!("../../extra/icons/robot.ico"),
            IconPreset::Yorkie => include_bytes!("../../extra/icons/yorkie.ico"),
        };
        fs::create_dir_all(icon.parent().unwrap())?;
        fs::create_dir_all(shortcut.parent().unwrap())?;
        if !fs::read(&icon).is_ok_and(|current| current == bytes) {
            fs::write(&icon, bytes)?;
        }
        let script = r#"
$ErrorActionPreference = 'Stop'
$shell = New-Object -ComObject WScript.Shell
$exists = Test-Path -LiteralPath $env:TERMUST_ICON_SHORTCUT
$link = $shell.CreateShortcut($env:TERMUST_ICON_SHORTCUT)
if ($exists -and $link.TargetPath -and -not [String]::Equals([IO.Path]::GetFullPath($link.TargetPath), $env:TERMUST_ICON_EXE, [StringComparison]::OrdinalIgnoreCase)) { throw 'The existing Termust shortcut belongs to another installation.' }
if (-not $exists) { $link.TargetPath = $env:TERMUST_ICON_EXE; $link.Description = 'Termust' }
if ($env:TERMUST_ICON_PRESET -eq 'termust') {
    $link.IconLocation = [IO.File]::ReadAllText($env:TERMUST_ICON_BACKUP)
    $link.Save()
    Remove-Item -LiteralPath $env:TERMUST_ICON_BACKUP
} else {
    if (-not (Test-Path -LiteralPath $env:TERMUST_ICON_BACKUP)) { [IO.File]::WriteAllText($env:TERMUST_ICON_BACKUP, $link.IconLocation) }
    $link.IconLocation = $env:TERMUST_ICON_FILE + ',0'
    $link.Save()
}
"#;
        let executable = std::env::current_exe()?;
        let executable = executable.to_string_lossy();
        let executable = executable.strip_prefix(r"\\?\").unwrap_or(&executable);
        let output = Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
            .env("TERMUST_ICON_SHORTCUT", shortcut)
            .env("TERMUST_ICON_EXE", executable)
            .env("TERMUST_ICON_PRESET", preset.id())
            .env("TERMUST_ICON_BACKUP", backup)
            .env("TERMUST_ICON_FILE", icon)
            .creation_flags(0x08000000)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other("could not update the per-user Termust shortcut"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_presets_decode_with_alpha_and_distinct_artwork() {
        for (index, preset) in IconPreset::ALL.into_iter().enumerate() {
            let image = preset.image();
            assert!(image.width >= 64 && image.width == image.height);
            assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
            assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0));
            assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
            assert!(!preset.rects(0., 0., 64.).is_empty());
            for previous in &IconPreset::ALL[..index] {
                assert_ne!(preset.png(), previous.png());
            }
        }
    }
}
