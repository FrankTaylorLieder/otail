use anyhow::Result;

use crate::ifile::ViewCommandsSender;

#[derive(Debug)]
pub struct LineRange {
    first: u32,
    last: u32,
}

#[allow(async_fn_in_trait)]
pub trait View {
    fn set_tail(&mut self, tail: bool);
    fn num_lines(&self) -> u32;
    fn get_line(&mut self, line: u32) -> Option<String>;
    fn set_line_range(&mut self, range: LineRange);
    // async fn run(&mut self, command_sender: ViewCommandsSender) -> Result<()>;
}
