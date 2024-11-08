#![allow(unused)]
use anyhow::Result;
use std::{
    io::{self, stdout},
    thread::{self, Thread},
    time::Duration,
};

use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    crossterm::{
        event::{self, Event, KeyCode},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        block::BlockExt, Block, BorderType, Borders, Cell, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, TableState, Widget,
    },
    DefaultTerminal, Frame, Terminal,
};

#[derive(Debug)]
struct LazyState {
    items: Vec<String>,
    current: Option<u32>,
}

impl LazyState {
    pub fn select(&mut self, position: Option<u32>) {
        self.current = position;
    }

    pub fn selected(&self) -> Option<u32> {
        self.current
    }
}

#[derive(Debug)]
struct LazyList<'a> {
    block: Option<Block<'a>>,
}

impl<'a> LazyList<'a> {
    pub fn new() -> Self {
        Self { block: None }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl<'a> StatefulWidget for LazyList<'a> {
    type State = LazyState;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        self.block.render(area, buf);
        let inner = self.block.inner_if_some(area);

        let height = inner.height;
        let width = inner.width;

        Text::from(
            state
                .items
                .iter()
                .enumerate()
                .skip(state.current.unwrap_or(0) as usize)
                .take(height as usize)
                .map(|(i, s)| Line::from(format!("{:>5} {l:.w$}", i, w = width as usize, l = s)))
                .collect::<Vec<Line>>(),
        )
        .render(inner, buf);
    }
}

struct App {
    items: Vec<String>,
    content_state: LazyState,
    content_scroll_state: ScrollbarState,
    filter_state: LazyState,
    filter_scroll_state: ScrollbarState,

    cell_renders: u32,

    current_window: bool,
}

impl App {
    fn new() -> Self {
        let mut content: Vec<String> = Vec::new();
        let len = 100;
        for i in (0..len) {
            content.push(format!(
                "Line {} - {}",
                i,
                std::iter::repeat("X").take(i).collect::<String>()
            ));
        }

        Self {
            items: content.clone(),

            content_state: LazyState {
                items: content.clone(),
                current: None,
            },
            content_scroll_state: ScrollbarState::new(len),

            filter_scroll_state: ScrollbarState::new(len),
            filter_state: LazyState {
                items: content.clone(),
                current: None,
            },

            cell_renders: 0,

            current_window: false,
        }
    }

    fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        let mut should_quit = false;
        while !should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            should_quit = self.handle_events()?;

            // thread::sleep(Duration::from_millis(10));
        }

        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        Ok(())
    }

    fn handle_events(&mut self) -> io::Result<bool> {
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
                        KeyCode::Char('j') | KeyCode::Down => self.scroll(1),
                        KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
                        // TODO: Scroll by visible page size
                        KeyCode::Char('d') => self.scroll(20),
                        KeyCode::Char('u') => self.scroll(-20),
                        KeyCode::Char('g') => self.top(),
                        KeyCode::Char('G') => self.bottom(),

                        KeyCode::Tab => self.current_window = !self.current_window,

                        _ => {}
                    }
                }
            }
        }

        Ok(false)
    }

    fn get_window_bits(&mut self) -> (&mut LazyState, &mut ScrollbarState) {
        if self.current_window {
            (&mut self.filter_state, &mut self.filter_scroll_state)
        } else {
            (&mut self.content_state, &mut self.content_scroll_state)
        }
    }

    fn place(&mut self, i: usize) {
        let (state, scroll_state) = self.get_window_bits();
        state.select(Some(i as u32));
        scroll_state.position(i);
    }

    fn scroll(&mut self, delta: i32) {
        let (state, scroll_state) = self.get_window_bits();
        let i = match state.selected() {
            Some(i) => clamped_add(i as u32, delta, (state.items.len() - 1) as u32) as usize,
            None => 0,
        };

        self.place(i);
    }

    fn top(&mut self) {
        self.place(0);
    }

    fn bottom(&mut self) {
        self.place(self.items.len() - 1);
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [title_area, main_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());
        let [file_area, controls_area, filter_area] = Layout::vertical([
            Constraint::Percentage(70),
            Constraint::Length(3),
            Constraint::Fill(1),
        ])
        .areas(main_area);

        let filename = Span::from("File: /foo/bar").italic();
        // let file_stats = Line::from("10 Lines (1024 bytes)")
        let file_stats = Line::from(self.compute_file_stats())
            .reversed()
            .alignment(Alignment::Right);
        let title_layout = Layout::horizontal([Constraint::Fill(4), Constraint::Length(30)]);
        let [filename_area, stats_area] = title_layout.areas(title_area);

        frame.render_widget(filename, filename_area);
        frame.render_widget(file_stats, stats_area);

        let widths = [Constraint::Length(5), Constraint::Fill(1)];

        let mut content = LazyList::new().block(
            Block::bordered()
                .border_set(self.selected_border(!self.current_window))
                .title("Content"),
        );
        frame.render_stateful_widget(content, file_area, &mut self.content_state);
        self.render_scrollbar(frame, file_area);

        frame.render_widget(Block::bordered().title("Controls"), controls_area);

        let mut ll = LazyList::new().block(
            Block::bordered()
                .border_set(self.selected_border(self.current_window))
                .title("Filtered"),
        );
        frame.render_stateful_widget(ll, filter_area, &mut self.filter_state);
        frame.render_stateful_widget(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            filter_area.inner(Margin {
                vertical: 1,
                horizontal: 1,
            }),
            &mut self.filter_scroll_state,
        );
    }

    fn selected_border(&self, selected: bool) -> symbols::border::Set {
        if selected {
            symbols::border::THICK
        } else {
            symbols::border::PLAIN
        }
    }

    fn compute_file_stats(&mut self) -> String {
        format!(
            "{} Lines ({} cell renders)",
            self.items.len(),
            self.cell_renders
        )
    }

    // fn render_file_table(&mut self, frame: &mut Frame, area: Rect) {
    //     // let header_style = Style::default().reversed();
    //     // let header = ["Index", "Content"]
    //     //     .into_iter()
    //     //     .map(Cell::from)
    //     //     .collect::<Row>()
    //     //     .style(header_style)
    //     //     .height(1);
    //
    //     let selected_style = Style::default().bold();
    //
    //     let rows = self.items.iter().enumerate().map(|(i, data)| {
    //         // TODO: Only render cells that are going to be displayed.
    //         self.cell_renders += 1;
    //         // TODO: Render proper line numbers.
    //         let item = ["XXX", data];
    //         item.into_iter()
    //             .map(|c| Cell::from(Text::raw(c)))
    //             .collect::<Row>()
    //             .height(1)
    //     });
    //
    //     let table = Table::new(rows, [Constraint::Length(5), Constraint::Fill(1)])
    //         // .header(header)
    //         .block(Block::bordered().title("Content"))
    //         .highlight_symbol(Text::from(">"))
    //         .highlight_style(selected_style);
    //
    //     frame.render_stateful_widget(table, area, &mut self.state);
    // }

    fn render_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_stateful_widget(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 1,
            }),
            &mut self.content_scroll_state,
        );
    }
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let app = App::new();
    app.run(terminal);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn clamped_add(a: u32, b: i32, max: u32) -> u32 {
    let v = a as i64 + b as i64;
    if v > max as i64 {
        max
    } else if v < 0 {
        0
    } else {
        v as u32
    }
}
