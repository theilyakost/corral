use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, rename = "group")]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub rules: Vec<String>,
}

impl Group {
    pub fn matches(&self, file_name: &str) -> bool {
        self.rules.iter().any(|rule| glob_match(rule, file_name))
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("corral").join("config.toml")
    }

    pub fn load_or_create() -> Result<Config, String> {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}\n\n{e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let config = Self::starter();
                config.save().map_err(|e| format!("{}\n\n{e}", path.display()))?;
                Ok(config)
            }
            Err(e) => Err(format!("{}\n\n{e}", path.display())),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path)
    }

    fn starter() -> Config {
        Config {
            groups: vec![
                Group {
                    name: "Документы".into(),
                    x: 120,
                    y: 120,
                    width: 420,
                    height: 260,
                    rules: vec!["*.txt".into(), "*.pdf".into(), "*.doc*".into(), "*.xls*".into(), "*.md".into()],
                },
                Group {
                    name: "Картинки".into(),
                    x: 120,
                    y: 420,
                    width: 420,
                    height: 260,
                    rules: vec!["*.png".into(), "*.jpg".into(), "*.jpeg".into(), "*.gif".into(), "*.webp".into()],
                },
            ],
        }
    }
}

pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob() {
        assert!(glob_match("*.txt", "Notes.TXT"));
        assert!(glob_match("*.doc*", "report.docx"));
        assert!(glob_match("img_??.png", "img_01.png"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("*.txt", "notes.txt.lnk"));
        assert!(!glob_match("img_??.png", "img_1.png"));
    }

    #[test]
    fn config_roundtrip() {
        let config = Config::starter();
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.groups.len(), config.groups.len());
        assert_eq!(back.groups[0].rules, config.groups[0].rules);
    }
}
