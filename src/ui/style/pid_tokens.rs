//! Design tokens of the P&ID interface: the colours, radii and font names the
//! Pencil design declares as variables.
//!
//! The values are not written here. `scripts/pid-platform/gen_pid_tokens.py`
//! reads `docs/pid-platform/design/pid-platform.pen` and writes them to
//! `generated.rs`; when the design's variables change, run the script again.
//! The tests below read the same `.pen` on their own and hold the table to
//! it: every variable the design has, in every theme the design has, with the
//! design's value.
//!
//! A value is asked for by theme. While the design draws one theme only, that
//! theme is the dark one and [`PidTheme::Light`] answers with the same values;
//! [`PidTheme::in_design`] tells the two cases apart.
//!
//! This module hands out values and nothing else: no widget style is defined
//! or changed here.

mod generated;

pub use generated::*;

/// A colour as the design states it: red, green, blue and alpha, 0-255 each.
pub type Rgba = [u8; 4];

/// The theme a value is asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PidTheme {
    /// The dark theme: the one the design started with.
    Dark,
    /// The light theme.
    #[default]
    Light,
}

impl PidTheme {
    /// Both themes, dark first.
    pub const ALL: [PidTheme; 2] = [PidTheme::Dark, PidTheme::Light];

    /// The theme matching `theme`'s dark/light polarity.
    pub fn from_iced(theme: &iced::Theme) -> Self {
        if theme.palette().is_dark {
            PidTheme::Dark
        } else {
            PidTheme::Light
        }
    }

    /// Whether the design gives this theme values of its own. A theme it does
    /// not draw answers every question with the dark values.
    pub fn in_design(self) -> bool {
        generated::DESIGN_THEMES.contains(&self)
    }

    /// Where this theme's value sits in a generated `[dark, light]` pair.
    const fn index(self) -> usize {
        match self {
            PidTheme::Dark => 0,
            PidTheme::Light => 1,
        }
    }
}

/// The value of one design variable, whatever its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenValue {
    /// A colour variable.
    Color(Rgba),
    /// A number variable (a radius, a size).
    Number(f32),
    /// A text variable (a font family).
    Text(&'static str),
    /// An on/off variable.
    Flag(bool),
}

/// The names of the design's variables, as the design spells them and in its
/// order.
pub fn names() -> &'static [&'static str] {
    generated::NAMES
}

/// The value of the variable the design calls `name` (`"bg-app"`) in `theme`;
/// `None` when the design has no variable of that name.
pub fn by_name(name: &str, theme: PidTheme) -> Option<TokenValue> {
    generated::lookup(name, theme)
}

impl ColorToken {
    /// The colour as iced draws it.
    pub fn color(self, theme: PidTheme) -> iced::Color {
        let [r, g, b, a] = self.rgba(theme);
        iced::Color::from_rgba8(r, g, b, f32::from(a) / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};
    use std::collections::BTreeMap;

    /// The design the table was generated from.
    const PEN: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/pid-platform/design/pid-platform.pen"
    );

    // What a theme value may be called; the generator knows the same names.
    const DARK_NAMES: [&str; 5] = ["dark", "night", "暗", "暗色", "深色"];
    const LIGHT_NAMES: [&str; 5] = ["light", "day", "亮", "亮色", "浅色"];

    /// One axis value per theme axis: what "this theme" means to a variable.
    type Context = BTreeMap<String, String>;

    /// A value as the design states it, typed by the variable's `type`.
    #[derive(Debug, PartialEq)]
    enum Stated {
        Color(Rgba),
        Number(f64),
        Text(String),
        Flag(bool),
    }

    impl Stated {
        fn is(&self, code: TokenValue) -> bool {
            match (self, code) {
                (Stated::Color(want), TokenValue::Color(got)) => *want == got,
                (Stated::Number(want), TokenValue::Number(got)) => *want as f32 == got,
                (Stated::Text(want), TokenValue::Text(got)) => want == got,
                (Stated::Flag(want), TokenValue::Flag(got)) => *want == got,
                _ => false,
            }
        }
    }

    /// The `variables` and `themes` of a `.pen` document, read here without
    /// the generator: a second reading of the same rules, so that a mistake in
    /// either shows as a disagreement.
    struct Design {
        variables: Map<String, Value>,
        themes: Vec<(PidTheme, Context)>,
    }

    impl Design {
        fn committed() -> Self {
            let text = std::fs::read_to_string(PEN).unwrap_or_else(|e| panic!("{PEN}: {e}"));
            Self::parse(&text)
        }

        fn parse(text: &str) -> Self {
            let doc: Value = serde_json::from_str(text).expect("the design is JSON");
            let variables = doc["variables"].as_object().cloned().unwrap_or_default();
            // A value stated for a theme the design does not declare applies
            // nowhere; reading on would compare everything but that value.
            for (name, variable) in &variables {
                for entry in variable["value"].as_array().into_iter().flatten() {
                    for (axis, value) in entry["theme"].as_object().into_iter().flatten() {
                        let declared = doc["themes"][axis.as_str()]
                            .as_array()
                            .is_some_and(|values| values.contains(value));
                        assert!(
                            declared,
                            "{name}: a value is stated for {axis}={value}, \
                             a theme the design does not declare"
                        );
                    }
                }
            }
            Design {
                variables,
                themes: theme_contexts(&doc),
            }
        }

        fn theme_list(&self) -> Vec<PidTheme> {
            self.themes.iter().map(|(theme, _)| *theme).collect()
        }

        /// The value the design states for `name` in a theme it draws.
        fn stated(&self, name: &str, theme: PidTheme) -> Stated {
            let context = self
                .themes
                .iter()
                .find(|(drawn, _)| *drawn == theme)
                .map(|(_, context)| context)
                .unwrap_or_else(|| panic!("the design does not draw {theme:?}"));
            let raw = resolve(&self.variables, name, context, 0);
            let kind = self.variables[name]["type"].as_str().unwrap_or_default();
            match kind {
                "color" => Stated::Color(parse_color(raw.as_str().unwrap_or_default(), name)),
                "number" => Stated::Number(
                    raw.as_f64()
                        .unwrap_or_else(|| panic!("{name}: {raw} is not a number")),
                ),
                "string" => Stated::Text(
                    raw.as_str()
                        .unwrap_or_else(|| panic!("{name}: {raw} is not text"))
                        .to_owned(),
                ),
                "boolean" => Stated::Flag(
                    raw.as_bool()
                        .unwrap_or_else(|| panic!("{name}: {raw} is not a flag")),
                ),
                other => panic!("{name}: unknown variable type {other:?}"),
            }
        }
    }

    /// The themes a document draws and the axis values that select each. No
    /// theme axis at all means one set of values, and that set is the dark
    /// theme. Dark and light are all the code can be asked for, so a design
    /// that declares any other theme value is not read at all: its values
    /// for that theme would go uncompared.
    fn theme_contexts(doc: &Value) -> Vec<(PidTheme, Context)> {
        let axes: Vec<(String, Vec<String>)> = doc["themes"]
            .as_object()
            .map(|axes| {
                axes.iter()
                    .map(|(axis, values)| {
                        let values = values
                            .as_array()
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(|value| value.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (axis.clone(), values)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if axes.is_empty() {
            return vec![(PidTheme::Dark, Context::new())];
        }
        // The first value of an axis is its default.
        let defaults: Context = axes
            .iter()
            .filter_map(|(axis, values)| Some((axis.clone(), values.first()?.clone())))
            .collect();
        let named = |value: &String, names: &[&str]| names.contains(&value.to_lowercase().as_str());
        for (axis, values) in &axes {
            let Some(light) = values.iter().find(|value| named(value, &LIGHT_NAMES)) else {
                continue;
            };
            // An axis like ["default", "light"]: what is not light is dark.
            let dark = values
                .iter()
                .find(|value| named(value, &DARK_NAMES))
                .or_else(|| values.iter().find(|value| *value != light))
                .unwrap_or_else(|| panic!("theme axis {axis:?} names a light theme only"));
            let with = |value: &String| {
                let mut context = defaults.clone();
                context.insert(axis.clone(), value.clone());
                context
            };
            let contexts = vec![(PidTheme::Dark, with(dark)), (PidTheme::Light, with(light))];
            let beyond: Vec<String> = axes
                .iter()
                .flat_map(|(other, values)| values.iter().map(move |value| (other, value)))
                .filter(|(other, value)| {
                    contexts
                        .iter()
                        .all(|(_, context)| context.get(*other) != Some(*value))
                })
                .map(|(other, value)| format!("{other}={value}"))
                .collect();
            assert!(
                beyond.is_empty(),
                "the design declares theme values the code has no theme for \
                 (it has dark and light): {}",
                beyond.join(", ")
            );
            return contexts;
        }
        panic!("the design declares theme axes, none of which names a light theme: {axes:?}");
    }

    /// The raw value of `name` in the theme `context` selects: the last entry
    /// of a per-theme list that applies, with `$other` followed to its value.
    fn resolve(
        variables: &Map<String, Value>,
        name: &str,
        context: &Context,
        depth: usize,
    ) -> Value {
        assert!(
            depth < 32,
            "{name}: variables refer to each other in a circle"
        );
        let variable = variables
            .get(name)
            .unwrap_or_else(|| panic!("{name}: no such variable in the design"));
        let mut raw = variable["value"].clone();
        if let Some(entries) = raw.as_array() {
            let applies = |entry: &&Value| {
                entry["theme"].as_object().is_none_or(|theme| {
                    theme.iter().all(|(axis, value)| {
                        context.get(axis).map(String::as_str) == value.as_str()
                    })
                })
            };
            raw = entries
                .iter()
                .rfind(applies)
                .unwrap_or_else(|| panic!("{name}: no value applies in {context:?}"))["value"]
                .clone();
        }
        match raw.as_str().and_then(|text| text.strip_prefix('$')) {
            Some(other) => resolve(variables, other, context, depth + 1),
            None => raw,
        }
    }

    fn parse_color(text: &str, name: &str) -> Rgba {
        let digits = text.trim().strip_prefix('#').unwrap_or_default();
        let nibble = |at: usize| {
            u8::from_str_radix(&digits[at..at + 1], 16)
                .unwrap_or_else(|_| panic!("{name}: {text:?} is not a colour"))
        };
        assert!(digits.is_ascii(), "{name}: {text:?} is not a colour");
        match digits.len() {
            3 | 4 => {
                let mut rgba = [0, 0, 0, 0xFF];
                for (at, channel) in rgba.iter_mut().enumerate().take(digits.len()) {
                    *channel = nibble(at) * 17;
                }
                rgba
            }
            6 | 8 => {
                let mut rgba = [0, 0, 0, 0xFF];
                for (at, channel) in rgba.iter_mut().enumerate().take(digits.len() / 2) {
                    *channel = nibble(at * 2) * 16 + nibble(at * 2 + 1);
                }
                rgba
            }
            _ => panic!("{name}: {text:?} is not a #RGB / #RGBA / #RRGGBB / #RRGGBBAA colour"),
        }
    }

    /// The rule the table is held to: whatever the design states, the code
    /// states too, and states the same. No variable and no theme is exempt.
    #[test]
    fn every_variable_of_the_design_has_its_value_in_code_in_every_theme() {
        let design = Design::committed();
        assert!(!design.variables.is_empty(), "the design has variables");
        let mut wrong = Vec::new();
        for name in design.variables.keys() {
            for theme in design.theme_list() {
                let stated = design.stated(name, theme);
                match by_name(name, theme) {
                    None => wrong.push(format!(
                        "{name} ({theme:?}): missing in code, design {stated:?}"
                    )),
                    Some(code) if !stated.is(code) => {
                        wrong.push(format!(
                            "{name} ({theme:?}): code {code:?}, design {stated:?}"
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "the token table disagrees with the design; run \
             `python scripts/pid-platform/gen_pid_tokens.py`:\n{}",
            wrong.join("\n")
        );
    }

    /// The other direction, so the table cannot keep a variable the design
    /// dropped or claim a theme the design does not draw.
    #[test]
    fn the_code_states_nothing_the_design_does_not() {
        let design = Design::committed();
        let stated: Vec<&str> = design.variables.keys().map(String::as_str).collect();
        assert_eq!(
            names(),
            stated.as_slice(),
            "the variables, in the design's order"
        );
        let drawn: Vec<PidTheme> = PidTheme::ALL
            .into_iter()
            .filter(|theme| theme.in_design())
            .collect();
        assert_eq!(drawn, design.theme_list(), "the themes the design draws");
    }

    /// A theme the design does not draw still answers, with the dark value,
    /// so a caller never has to ask whether light exists yet.
    #[test]
    fn a_theme_the_design_does_not_draw_answers_with_the_dark_value() {
        assert_eq!(PidTheme::default(), PidTheme::Light);
        assert!(PidTheme::Dark.in_design(), "dark is always drawn");
        for name in names() {
            let dark = by_name(name, PidTheme::Dark).expect("every name has a value");
            let light = by_name(name, PidTheme::Light).expect("every name has a value");
            if !PidTheme::Light.in_design() {
                assert_eq!(light, dark, "{name}");
            }
        }
        assert_eq!(by_name("no-such-variable", PidTheme::Dark), None);
    }

    /// The typed accessors and the by-name lookup are one table, and a
    /// colour reaches iced with the design's alpha.
    #[test]
    fn typed_tokens_and_iced_colours_carry_the_same_values() {
        for theme in PidTheme::ALL {
            for token in ColorToken::ALL {
                assert_eq!(
                    by_name(token.name(), theme),
                    Some(TokenValue::Color(token.rgba(theme)))
                );
                let [r, g, b, a] = token.rgba(theme);
                let color = token.color(theme);
                assert_eq!(
                    color,
                    iced::Color::from_rgba8(r, g, b, f32::from(a) / 255.0)
                );
                assert!((color.a - f32::from(a) / 255.0).abs() < f32::EPSILON);
            }
            for token in NumberToken::ALL {
                assert_eq!(
                    by_name(token.name(), theme),
                    Some(TokenValue::Number(token.value(theme)))
                );
            }
            for token in TextToken::ALL {
                assert_eq!(
                    by_name(token.name(), theme),
                    Some(TokenValue::Text(token.value(theme)))
                );
            }
        }
    }

    /// The reader above is what will judge the light theme when the design
    /// gains one, so it is tried on a document that already has one: a
    /// per-theme list, an entry that applies always, a reference to another
    /// variable, and an axis with no choice on it.
    #[test]
    fn the_design_reader_takes_a_document_with_two_themes() {
        let design = Design::parse(
            r##"{
              "themes": {"density": ["regular"], "mode": ["dark", "light"]},
              "variables": {
                "bg": {"type": "color", "value": [
                  {"value": "#0B0E13", "theme": {"mode": "dark"}},
                  {"value": "#FFFFFF", "theme": {"mode": "light"}}
                ]},
                "accent": {"type": "color", "value": "#4C9EFF"},
                "accent-soft": {"type": "color", "value": [
                  {"value": "#4C9EFF1F"},
                  {"value": "#000", "theme": {"mode": "light"}},
                  {"value": "#4C9EFF33", "theme": {"mode": "light", "density": "regular"}}
                ]},
                "focus": {"type": "color", "value": "$accent"},
                "radius": {"type": "number", "value": 4},
                "font": {"type": "string", "value": "Inter"}
              }
            }"##,
        );
        assert_eq!(design.theme_list(), vec![PidTheme::Dark, PidTheme::Light]);
        let color = |name: &str, theme| design.stated(name, theme);
        assert_eq!(
            color("bg", PidTheme::Dark),
            Stated::Color([0x0B, 0x0E, 0x13, 0xFF])
        );
        assert_eq!(
            color("bg", PidTheme::Light),
            Stated::Color([0xFF, 0xFF, 0xFF, 0xFF])
        );
        // One value: the same in both themes.
        assert_eq!(
            color("accent", PidTheme::Light),
            Stated::Color([0x4C, 0x9E, 0xFF, 0xFF])
        );
        // An entry without a theme applies always; the last entry that
        // applies wins.
        assert_eq!(
            color("accent-soft", PidTheme::Dark),
            Stated::Color([0x4C, 0x9E, 0xFF, 0x1F])
        );
        assert_eq!(
            color("accent-soft", PidTheme::Light),
            Stated::Color([0x4C, 0x9E, 0xFF, 0x33])
        );
        assert_eq!(
            color("focus", PidTheme::Light),
            Stated::Color([0x4C, 0x9E, 0xFF, 0xFF])
        );
        assert_eq!(
            design.stated("radius", PidTheme::Light),
            Stated::Number(4.0)
        );
        assert_eq!(
            design.stated("font", PidTheme::Dark),
            Stated::Text("Inter".to_owned())
        );

        // No theme axis: one theme, the dark one.
        let plain = Design::parse(r##"{"variables": {"bg": {"type": "color", "value": "#abc"}}}"##);
        assert_eq!(plain.theme_list(), vec![PidTheme::Dark]);
        assert_eq!(
            plain.stated("bg", PidTheme::Dark),
            Stated::Color([0xAA, 0xBB, 0xCC, 0xFF])
        );

        // Other names for the two themes, and a default that is simply "not
        // light".
        let named = Design::parse(r##"{"themes": {"主题": ["默认", "亮色"]}, "variables": {}}"##);
        assert_eq!(named.theme_list(), vec![PidTheme::Dark, PidTheme::Light]);
        assert_eq!(named.themes[0].1["主题"], "默认");
        assert_eq!(named.themes[1].1["主题"], "亮色");
    }

    /// The rule has no way round it: a design with a theme the code cannot
    /// be asked for is refused, not compared on its dark and light values
    /// alone.
    #[test]
    #[should_panic(expected = "no theme for (it has dark and light): mode=contrast")]
    fn a_third_theme_in_the_design_is_refused() {
        Design::parse(r##"{"themes": {"mode": ["dark", "light", "contrast"]}, "variables": {}}"##);
    }

    #[test]
    #[should_panic(expected = "no theme for (it has dark and light): density=compact")]
    fn a_second_axis_with_a_choice_on_it_is_refused() {
        Design::parse(
            r##"{"themes": {"mode": ["dark", "light"], "density": ["regular", "compact"]},
                 "variables": {}}"##,
        );
    }

    #[test]
    #[should_panic(expected = "bg: a value is stated for mode=\"Light\"")]
    fn a_value_for_a_theme_the_design_does_not_declare_is_refused() {
        Design::parse(
            r##"{"themes": {"mode": ["dark", "light"]}, "variables": {
                  "bg": {"type": "color", "value": [
                    {"value": "#000000"},
                    {"value": "#FFFFFF", "theme": {"mode": "Light"}}
                  ]}}}"##,
        );
    }
}
