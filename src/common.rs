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
        self.clone()
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
