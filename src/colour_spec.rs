use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, VariantArray};

use crate::filter_spec::{FilterSpec, FilterType};

#[derive(
    Display, Debug, EnumString, VariantArray, PartialEq, Eq, Clone, Serialize, Deserialize,
)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColouringRule {
    pub enabled: bool,
    pub filter_spec: FilterSpec,
    pub fg_colour: Option<Colour>,
    pub bg_colour: Option<Colour>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColouringSpec {
    rules: Vec<ColouringRule>,
}

pub type Colours = (Option<Colour>, Option<Colour>);

impl ColouringRule {
    pub fn default() -> Self {
        Self {
            enabled: true,
            filter_spec: FilterSpec::new(crate::filter_spec::FilterType::SimpleCaseInsensitive, "")
                .unwrap_or_else(|_| {
                    FilterSpec::new(
                        crate::filter_spec::FilterType::SimpleCaseInsensitive,
                        "pattern",
                    )
                    .unwrap()
                }),
            fg_colour: None,
            bg_colour: None,
        }
    }
}

impl ColouringSpec {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn default() -> Self {
        ColouringSpec::new().set_rules(vec![ColouringRule {
            enabled: true,
            filter_spec: FilterSpec::new(FilterType::SimpleCaseInsensitive, "error")
                .expect("Failed to build sample filter spec"),
            fg_colour: Some(Colour::Red),
            bg_colour: None,
        }])
    }

    pub fn set_rules(mut self, rules: Vec<ColouringRule>) -> Self {
        self.rules = rules;

        self
    }

    pub fn rules(&self) -> &Vec<ColouringRule> {
        &self.rules
    }

    pub fn add_rule(&mut self, rule: ColouringRule, index: Option<usize>) {
        match index {
            Some(i) if i <= self.rules.len() => self.rules.insert(i, rule),
            _ => self.rules.push(rule),
        }
    }

    pub fn remove_rule(&mut self, index: usize) -> Option<ColouringRule> {
        if index < self.rules.len() {
            Some(self.rules.remove(index))
        } else {
            None
        }
    }

    pub fn move_rule_up(&mut self, index: usize) -> bool {
        if index > 0 && index < self.rules.len() {
            self.rules.swap(index - 1, index);
            true
        } else {
            false
        }
    }

    pub fn move_rule_down(&mut self, index: usize) -> bool {
        if index < self.rules.len().saturating_sub(1) {
            self.rules.swap(index, index + 1);
            true
        } else {
            false
        }
    }

    pub fn update_rule(&mut self, index: usize, rule: ColouringRule) -> bool {
        if index < self.rules.len() {
            self.rules[index] = rule;
            true
        } else {
            false
        }
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
