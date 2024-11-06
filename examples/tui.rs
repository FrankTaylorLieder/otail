#![allow(unused)]
use anyhow::Result;
use std::io::{self, stdout};

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
        TableState,
    },
    DefaultTerminal, Frame, Terminal,
};

struct App {
    state: TableState,
    items: Vec<String>,
    scroll_state: ScrollbarState,

    cell_renders: u32,
}

impl App {
    fn new() -> Self {
        let mut content: Vec<String> = Vec::new();
        let len = 10000;
        for i in (0..len) {
            content.push(format!(
                "Line {} - {}",
                i,
                std::iter::repeat("X").take(i).collect::<String>()
            ));
        }

        Self {
            state: TableState::default().with_selected(0),
            items: content,
            scroll_state: ScrollbarState::new(len),

            cell_renders: 0,
        }
    }

    fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        let mut should_quit = false;
        while !should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            should_quit = self.handle_events()?;
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
                        _ => {}
                    }
                }
            }
        }

        Ok(false)
    }

    fn place(&mut self, i: usize) {
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i);
    }

    fn scroll(&mut self, delta: i32) {
        let i = match self.state.selected() {
            Some(i) => clamped_add(i as u32, delta, (self.items.len() - 1) as u32) as usize,
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
        // let file_table = Table::new(rows, widths)
        //     .block(Block::bordered().title("File"))
        //     .highlight_symbol(">");
        // frame.render_widget(file_table, file_area);

        self.render_file_table(frame, file_area);
        self.render_scrollbar(frame, file_area);
        frame.render_widget(Block::bordered().title("Controls"), controls_area);
        frame.render_widget(Block::bordered().title("Filtered"), filter_area);
    }

    fn compute_file_stats(&mut self) -> String {
        format!(
            "{} Lines ({} cell renders)",
            self.items.len(),
            self.cell_renders
        )
    }

    fn render_file_table(&mut self, frame: &mut Frame, area: Rect) {
        // let header_style = Style::default().reversed();
        // let header = ["Index", "Content"]
        //     .into_iter()
        //     .map(Cell::from)
        //     .collect::<Row>()
        //     .style(header_style)
        //     .height(1);

        let selected_style = Style::default().bold();

        let rows = self.items.iter().enumerate().map(|(i, data)| {
            // TODO: Only render cells that are going to be displayed.
            self.cell_renders += 1;
            // TODO: Render proper line numbers.
            let item = ["XXX", data];
            item.into_iter()
                .map(|c| Cell::from(Text::raw(c)))
                .collect::<Row>()
                .height(1)
        });

        let table = Table::new(rows, [Constraint::Length(5), Constraint::Fill(1)])
            // .header(header)
            .block(Block::bordered().title("Content"))
            .highlight_symbol(Text::from(">"))
            .highlight_style(selected_style);

        frame.render_stateful_widget(table, area, &mut self.state);
    }

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
            &mut self.scroll_state,
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
