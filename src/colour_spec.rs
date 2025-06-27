use strum::{Display, EnumString, VariantArray};

use crate::filter_spec::FilterSpec;

#[derive(Display, Debug, EnumString, VariantArray, PartialEq, Eq, Clone)]
pub enum Colour {
    Black,
    Red,
    Green,
    Blue,
    Yellow,
    Magenta,
    Cyan,
    Gray,
    White,
}

#[derive(Debug, Clone)]
pub struct ColouringRule {
    pub enabled: bool,
    pub filter_spec: FilterSpec,
    pub fg_colour: Option<Colour>,
    pub bg_colour: Option<Colour>,
}

#[derive(Debug, Clone)]
pub struct ColouringSpec {
    rules: Vec<ColouringRule>,
}

pub type Colours = (Option<Colour>, Option<Colour>);

impl ColouringSpec {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn set_rules(mut self, rules: Vec<ColouringRule>) -> Self {
        self.rules = rules;

        self
    }

    pub fn maybe_colour(&self, line: &str) -> Option<Colours> {
        for r in &self.rules {
            if r.filter_spec.matches(line) {
                return Some((r.fg_colour.clone(), r.bg_colour.clone()));
            }
        }

        None
    }
}
