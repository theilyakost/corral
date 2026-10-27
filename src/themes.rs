use crate::config::Appearance;

pub struct Theme {
    pub name: &'static str,
    header: &'static str,
    body: &'static str,
    title: &'static str,
    border: &'static str,
    label: &'static str,
    header_opacity: u8,
    body_opacity: u8,
    rounded: bool,
}

const fn theme(
    name: &'static str, header: &'static str, body: &'static str, title: &'static str, border: &'static str,
    label: &'static str, header_opacity: u8, body_opacity: u8, rounded: bool,
) -> Theme {
    Theme { name, header, body, title, border, label, header_opacity, body_opacity, rounded }
}

pub const THEMES: &[Theme] = &[
    theme("Графит", "#1A1C20", "#101214", "#F0F0F0", "#4A525E", "#FFFFFF", 215, 110, true),
    theme("Mica", "#2B2B2B", "#202020", "#FFFFFF", "#3D3D3D", "#FFFFFF", 235, 140, true),
    theme("Полночь", "#141B2D", "#0B1120", "#E3EAFF", "#2B3A5E", "#FFFFFF", 225, 120, true),
    theme("Nord", "#3B4252", "#2E3440", "#ECEFF4", "#4C566A", "#ECEFF4", 225, 125, true),
    theme("Dracula", "#44475A", "#282A36", "#F8F8F2", "#6272A4", "#F8F8F2", 225, 125, true),
    theme("Catppuccin", "#313244", "#1E1E2E", "#CDD6F4", "#585B70", "#CDD6F4", 225, 125, true),
    theme("Tokyo Night", "#24283B", "#1A1B26", "#C0CAF5", "#414868", "#C0CAF5", 225, 125, true),
    theme("Rosé Pine", "#26233A", "#191724", "#E0DEF4", "#524F67", "#E0DEF4", 225, 125, true),
    theme("One Dark", "#2C313A", "#21252B", "#ABB2BF", "#3E4451", "#DCDFE4", 225, 125, false),
    theme("Gruvbox", "#3C3836", "#282828", "#EBDBB2", "#665C54", "#EBDBB2", 225, 125, false),
    theme("Solarized", "#073642", "#002B36", "#EEE8D5", "#2A5A66", "#EEE8D5", 225, 125, false),
    theme("Изумруд", "#0F3D33", "#0A2A23", "#D6F5EC", "#1F6B58", "#FFFFFF", 220, 120, true),
    theme("Океан", "#0E3A5C", "#082640", "#DDF1FF", "#1B5E8C", "#FFFFFF", 220, 120, true),
    theme("Закат", "#4A2040", "#2A1426", "#FFE3D8", "#7A3460", "#FFFFFF", 220, 120, true),
    theme("Неон", "#120A24", "#0A0614", "#00F0FF", "#FF2BD6", "#FFFFFF", 235, 130, false),
    theme("Минимал", "#000000", "#000000", "#FFFFFF", "#5A5A5A", "#FFFFFF", 110, 35, true),
    theme("Стекло", "#E8ECF2", "#F4F6FA", "#1A1C20", "#C3CAD6", "#14161A", 210, 90, true),
    theme("Бумага", "#FFFFFF", "#FAFAF7", "#202124", "#DADCE0", "#202124", 235, 150, true),
];

impl Theme {
    pub fn apply(&self, base: &Appearance) -> Appearance {
        Appearance {
            header_color: self.header.into(),
            body_color: self.body.into(),
            title_color: self.title.into(),
            border_color: self.border.into(),
            label_color: self.label.into(),
            header_opacity: self.header_opacity,
            body_opacity: self.body_opacity,
            rounded: self.rounded,
            ..base.clone()
        }
    }

    fn matches(&self, appearance: &Appearance) -> bool {
        self.apply(appearance) == *appearance
    }
}

pub fn current(appearance: &Appearance) -> Option<usize> {
    THEMES.iter().position(|t| t.matches(appearance))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_color;

    #[test]
    fn default_look_is_the_first_theme() {
        assert_eq!(current(&Appearance::default()), Some(0));
    }

    #[test]
    fn themes_are_valid_and_distinct() {
        for (i, theme) in THEMES.iter().enumerate() {
            for color in [theme.header, theme.body, theme.title, theme.border, theme.label] {
                assert!(parse_color(color).is_some(), "{}: {color}", theme.name);
            }
            let applied = theme.apply(&Appearance::default());
            assert_eq!(current(&applied), Some(i), "{}", theme.name);
        }
    }

    #[test]
    fn applying_keeps_behaviour_settings() {
        let base = Appearance { animate: false, hover_open: false, ..Appearance::default() };
        let applied = THEMES[3].apply(&base);
        assert!(!applied.animate && !applied.hover_open);
        let mut custom = applied.clone();
        custom.body_opacity = 1;
        assert_eq!(current(&custom), None);
    }
}
