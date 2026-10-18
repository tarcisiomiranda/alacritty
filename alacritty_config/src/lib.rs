use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::PathBuf;

use log::LevelFilter;
use serde::Deserialize;
use toml::Value;

pub trait SerdeReplace {
    fn replace(&mut self, value: Value) -> Result<(), Box<dyn Error>>;
}

#[macro_export]
macro_rules! impl_replace {
    ($($ty:ty),*$(,)*) => {
        $(
            impl SerdeReplace for $ty {
                fn replace(&mut self, value: Value) -> Result<(), Box<dyn Error>> {
                    replace_simple(self, value)
                }
            }
        )*
    };
}

#[rustfmt::skip]
impl_replace!(
    usize, u8, u16, u32, u64, u128,
    isize, i8, i16, i32, i64, i128,
    f32, f64,
    bool,
    char,
    String,
    PathBuf,
    LevelFilter,
);

fn replace_simple<'de, D>(data: &mut D, value: Value) -> Result<(), Box<dyn Error>>
where
    D: Deserialize<'de>,
{
    *data = D::deserialize(value)?;

    Ok(())
}

impl<'de, T: Deserialize<'de>> SerdeReplace for Vec<T> {
    fn replace(&mut self, value: Value) -> Result<(), Box<dyn Error>> {
        replace_simple(self, value)
    }
}

impl<'de, T: SerdeReplace + Deserialize<'de>> SerdeReplace for Option<T> {
    fn replace(&mut self, value: Value) -> Result<(), Box<dyn Error>> {
        match self {
            Some(inner) => inner.replace(value),
            None => replace_simple(self, value),
        }
    }
}

impl<'de, T: Deserialize<'de>> SerdeReplace for HashMap<String, T> {
    fn replace(&mut self, value: Value) -> Result<(), Box<dyn Error>> {
        // Deserialize replacement as HashMap.
        let hashmap: HashMap<String, T> = Self::deserialize(value)?;

        // Merge the two HashMaps, replacing existing values.
        for (key, value) in hashmap {
            self.insert(key, value);
        }

        Ok(())
    }
}

impl<'de, T: Deserialize<'de>> SerdeReplace for BTreeMap<String, T> {
    fn replace(&mut self, value: Value) -> Result<(), Box<dyn Error>> {
        self.extend(Self::deserialize(value)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate as alacritty_config;
    use alacritty_config_derive::ConfigDeserialize;

    #[test]
    fn replace_ordered_map_merges_without_erasing_unrelated_entries() {
        let mut settings = BTreeMap::from([
            ("SplitVertical".to_string(), "Control+Shift+E".to_string()),
            ("Unrelated".to_string(), "keep".to_string()),
        ]);
        settings
            .replace(toml::from_str("SplitVertical='Control+Alt+E'\nClosePane=''\n").unwrap())
            .unwrap();
        assert_eq!(settings["Unrelated"], "keep");
        assert_eq!(settings["SplitVertical"], "Control+Alt+E");
        assert_eq!(settings["ClosePane"], "");
        let before = settings.clone();
        assert!(settings.replace(toml::from_str("SplitVertical=7").unwrap()).is_err());
        assert_eq!(settings, before);
    }

    #[test]
    fn replace_option() {
        #[derive(ConfigDeserialize, Default, PartialEq, Eq, Debug)]
        struct ReplaceOption {
            a: usize,
            b: usize,
        }

        let mut subject: Option<ReplaceOption> = None;

        let value: Value = toml::from_str("a=1").unwrap();
        SerdeReplace::replace(&mut subject, value).unwrap();

        let value: Value = toml::from_str("b=2").unwrap();
        SerdeReplace::replace(&mut subject, value).unwrap();

        assert_eq!(subject, Some(ReplaceOption { a: 1, b: 2 }));
    }
}
