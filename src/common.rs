use anyhow::Result;
use regex::Regex;

pub const CHANNEL_BUFFER: usize = 1000;

pub const FPS: u64 = 20;
pub const MS_PER_FRAME: u64 = 2_000 / FPS;

pub const FILTER_SPOOLING_BATCH_SIZE: usize = 10;

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
            FilterType::SimpleCaseInsensitive => {
                line.to_lowercase().contains(&self.filter_pattern.to_lowercase())
            }
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

pub trait LineContent {
    fn len(&self) -> usize;
    fn render(&self) -> String; // TODO: Return structure for better display
}

impl LineContent for String {
    fn len(&self) -> usize {
        self.len()
    }

    fn render(&self) -> String {
        replace_for_view(self)
    }
}

pub fn clamped_sub(a: usize, b: usize) -> usize {
    if b > a {
        0
    } else {
        a - b
    }
}

pub fn clamped_add(a: usize, b: isize, min: usize, max: usize) -> usize {
    let v = a as i64 + b as i64;
    if v > max as i64 {
        max
    } else if v < min as i64 {
        min
    } else {
        v as usize
    }
}

pub fn count_digits(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    (n as f64).log10().floor() as usize + 1
}

pub fn replace_for_view(line: &String) -> String {
    line.replace("\t", " ")
}
