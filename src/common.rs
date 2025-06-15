pub const CHANNEL_BUFFER: usize = 1000;

pub const FPS: u64 = 20;
pub const MS_PER_FRAME: u64 = 2_000 / FPS;

pub const FILTER_SPOOLING_BATCH_SIZE: usize = 10;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_content_trait_for_string() {
        let line = "test string".to_string();
        assert_eq!(line.len(), 11);
        assert_eq!(line.render(), "test string");
    }

    #[test]
    fn test_line_content_trait_with_tab() {
        let line = "test\tstring".to_string();
        assert_eq!(line.len(), 11);
        assert_eq!(line.render(), "test string"); // Tab replaced with space
    }

    #[test]
    fn test_clamped_sub_normal() {
        assert_eq!(clamped_sub(10, 3), 7);
        assert_eq!(clamped_sub(100, 50), 50);
    }

    #[test]
    fn test_clamped_sub_underflow() {
        assert_eq!(clamped_sub(5, 10), 0);
        assert_eq!(clamped_sub(0, 1), 0);
        assert_eq!(clamped_sub(3, 3), 0);
    }

    #[test]
    fn test_clamped_add_normal() {
        assert_eq!(clamped_add(10, 5, 0, 100), 15);
        assert_eq!(clamped_add(50, -10, 0, 100), 40);
    }

    #[test]
    fn test_clamped_add_overflow() {
        assert_eq!(clamped_add(90, 20, 0, 100), 100);
        assert_eq!(clamped_add(10, 200, 0, 100), 100);
    }

    #[test]
    fn test_clamped_add_underflow() {
        assert_eq!(clamped_add(10, -20, 0, 100), 0);
        assert_eq!(clamped_add(5, -10, 0, 100), 0);
    }

    #[test]
    fn test_clamped_add_negative_values() {
        assert_eq!(clamped_add(0, -5, 0, 100), 0);
        assert_eq!(clamped_add(50, -100, 0, 100), 0);
    }

    #[test]
    fn test_count_digits() {
        assert_eq!(count_digits(0), 1);
        assert_eq!(count_digits(1), 1);
        assert_eq!(count_digits(9), 1);
        assert_eq!(count_digits(10), 2);
        assert_eq!(count_digits(99), 2);
        assert_eq!(count_digits(100), 3);
        assert_eq!(count_digits(999), 3);
        assert_eq!(count_digits(1000), 4);
        assert_eq!(count_digits(12345), 5);
    }

    #[test]
    fn test_replace_for_view_no_tabs() {
        let line = "regular string".to_string();
        assert_eq!(replace_for_view(&line), "regular string");
    }

    #[test]
    fn test_replace_for_view_with_tabs() {
        let line = "line\twith\ttabs".to_string();
        assert_eq!(replace_for_view(&line), "line with tabs");
    }

    #[test]
    fn test_replace_for_view_multiple_tabs() {
        let line = "line\t\twith\t\t\tmultiple\ttabs".to_string();
        assert_eq!(replace_for_view(&line), "line  with   multiple tabs");
    }

    #[test]
    fn test_replace_for_view_tab_at_ends() {
        let line = "\ttab at start and end\t".to_string();
        assert_eq!(replace_for_view(&line), " tab at start and end ");
    }

    #[test]
    fn test_replace_for_view_only_tabs() {
        let line = "\t\t\t".to_string();
        assert_eq!(replace_for_view(&line), "   ");
    }

    #[test]
    fn test_constants() {
        assert_eq!(CHANNEL_BUFFER, 1000);
        assert_eq!(FPS, 20);
        assert_eq!(MS_PER_FRAME, 100); // 2000 / 20
        assert_eq!(FILTER_SPOOLING_BATCH_SIZE, 10);
    }
}
