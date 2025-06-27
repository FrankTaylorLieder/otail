use anyhow::Result;
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterType {
    SimpleCaseSensitive,
    SimpleCaseInsensitive,
    Regex,
}

#[derive(Debug, Clone)]
pub struct FilterSpec {
    pub filter_type: FilterType,
    pub filter_pattern: String,
    regex: Option<Regex>,
}

impl FilterSpec {
    pub fn new(filter_type: FilterType, filter_pattern: &str) -> Result<Self> {
        Ok(FilterSpec {
            filter_type: filter_type.clone(),
            filter_pattern: filter_pattern.to_owned(),
            regex: if filter_type == FilterType::Regex {
                Some(Regex::new(filter_pattern)?)
            } else {
                None
            },
        })
    }
    pub fn render(&self) -> String {
        format!(
            "\"{}\" ({})",
            self.filter_pattern,
            match self.filter_type {
                FilterType::SimpleCaseSensitive => "Sensitive",
                FilterType::SimpleCaseInsensitive => "Insensitive",
                FilterType::Regex => "Regex",
            }
        )
    }

    pub fn matches(&self, line: &str) -> bool {
        match self.filter_type {
            FilterType::SimpleCaseSensitive => line.contains(&self.filter_pattern),
            FilterType::SimpleCaseInsensitive => line
                .to_lowercase()
                .contains(&self.filter_pattern.to_lowercase()),
            FilterType::Regex => {
                if let Some(ref regex) = self.regex {
                    regex.find(line).is_some()
                } else {
                    // TODO should we report this missing regex?
                    false
                }
            }
        }
    }
}

impl PartialEq for FilterSpec {
    fn eq(&self, other: &Self) -> bool {
        self.filter_type == other.filter_type && self.filter_pattern == other.filter_pattern
    }
}
