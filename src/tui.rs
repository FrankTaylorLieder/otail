#![allow(unused)]
use anyhow::{bail, Result};
use crossterm::event::EventStream;
use futures::{FutureExt, StreamExt};
use futures_timer::Delay;
use log::{debug, error, info, trace};
use std::{
    io::{self, stdout},
    thread::{self, Thread},
    time::Duration,
};
use tokio::{select, sync::mpsc, time::interval};

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

use crate::{
    ifile::{IFReqSender, IFRespReceiver, IFRespSender},
    view::{UpdateAction, View, ViewPort},
};

#[derive(Debug)]
struct ListRange {
    first: u32,
    current: u32,
    last: u32,
}

#[derive(Debug)]
struct LazyState {
    view: View,
    current: Option<u32>,

    cell_renders: u32,
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

        let current = state.current.unwrap_or(0);

        let mut lines = Vec::new();
        for i in current..(current + height as u32) {
            let s = state.view.get_line(i);

            let Some(s) = s else {
                trace!("Line {i} not yet available...");
                break;
            };

            lines.push(Line::from(format!(
                "{:>5} {l:.w$}",
                i,
                w = width as usize,
                l = s
            )));

            state.cell_renders += 1;
        }
        Text::from(lines).render(inner, buf);
    }
}

pub struct Tui {
    path: String,

    content_ifresp_recv: IFRespReceiver,
    filter_ifresp_recv: IFRespReceiver,

    content_state: LazyState,
    content_scroll_state: ScrollbarState,
    content_tail: bool,

    filter_state: LazyState,
    filter_scroll_state: ScrollbarState,
    filter_tail: bool,

    // true for content, false for filter
    current_window: bool,
    // Fill ratio for content pane... 1..9
    content_fill: u32,
}

impl Tui {
    pub fn new(path: String, view_ifreq_sender: IFReqSender) -> Self {
        let (content_ifresp_sender, content_ifresp_recv) = mpsc::channel(10000);
        let (filter_ifresp_sender, filter_ifresp_recv) = mpsc::channel(10000);

        let content_view = View::new(
            "content".to_owned(),
            path.clone(),
            view_ifreq_sender.clone(),
            content_ifresp_sender,
        );
        let filter_view = View::new(
            "filter".to_owned(),
            path.clone(),
            view_ifreq_sender.clone(),
            filter_ifresp_sender,
        );

        let s = Self {
            path,

            content_ifresp_recv,
            filter_ifresp_recv,

            content_state: LazyState {
                view: content_view,
                current: None,
                cell_renders: 0,
            },
            content_scroll_state: ScrollbarState::new(0),
            content_tail: false,

            filter_scroll_state: ScrollbarState::new(0),
            filter_state: LazyState {
                view: filter_view,
                current: None,
                cell_renders: 0,
            },
            filter_tail: false,

            current_window: true,
            content_fill: 7,
        };

        s.debug_recv("tui in new");
        s
    }

    pub fn debug_recv(&self, location: &str) {
        trace!(
            "XXX TUI ifresp recv: {} {}",
            location,
            self.content_ifresp_recv.is_closed()
        );
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        let mut should_quit = false;

        self.debug_recv("run");

        self.content_state.view.init().await?;
        self.filter_state.view.init().await?;

        self.content_state
            .view
            .set_viewport(ViewPort {
                first_line: 0,
                num_lines: 10,
            })
            .await;
        self.filter_state
            .view
            .set_viewport(ViewPort {
                first_line: 0,
                num_lines: 10,
            })
            .await;
        let mut reader = EventStream::new();
        let mut interval = tokio::time::interval(Duration::from_millis(10_000));
        while !should_quit {
            terminal.draw(|frame| self.draw(frame))?;

            let mut timeout = interval.tick();
            let crossterm_event = reader.next().fuse();
            select! {
                _ = timeout => {
                    trace!("Run loop timeout... tick");
                },
                maybe_event = crossterm_event => {
                    trace!("Event: {:?}", maybe_event);
                    match maybe_event {
                        Some(Ok(e)) => {
                            should_quit = self.handle_event(&e).await?;
                        },
                        Some(Err(err)) => {
                            println!("Error: {:?}", err);
                            bail!("Event error: {:?}", err);
                        },
                        None => {}
                    }
                },
                content_resp = self.content_ifresp_recv.recv() => {
                    trace!("Content resp: {:?}", content_resp);
                    match content_resp {
                        None => {
                            debug!("Content IFResp closed... finishing");
                            break;
                        }
                        Some(cr) => {
                            let reply = self.content_state.view.handle_update(cr).await;
                            if let Some(update_action) = reply {
                                self.handle_update_action(update_action);
                            }
                        }
                    }
                },
                filter_resp = self.filter_ifresp_recv.recv() => {
                    trace!("Filter resp: {:?}", filter_resp);
                    match filter_resp {
                        None => {
                            debug!("Filter IFResp closed... finishing");
                            break;
                        }
                        Some(fr) => {
                            let reply = self.filter_state.view.handle_update(fr).await;
                            if let Some(update_action) = reply {
                                self.handle_update_action(update_action);
                            }
                        }
                    }
                }
            }
        }

        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        Ok(())
    }

    fn handle_update_action(&mut self, update_action: UpdateAction) -> Result<()> {
        trace!("Update action: {:?}", update_action);
        match update_action {
            UpdateAction::Truncated => {
                self.content_state.view.reset();
                self.filter_state.view.reset();
            }
            UpdateAction::Error { msg } => {
                error!("Error: {}", msg);
                // TODO: Put this in a dlg...
            }
        }
        Ok(())
    }

    async fn handle_event(&mut self, event: &Event) -> io::Result<bool> {
        if let Event::Key(key) = event {
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

                    KeyCode::Char('=') | KeyCode::Char('+') => self.resize(1),
                    KeyCode::Char('-') | KeyCode::Char('_') => self.resize(-1),

                    KeyCode::Char('t') => self.toggle_tail(),

                    KeyCode::Tab => self.current_window = !self.current_window,

                    _ => {}
                }
            }
        }

        Ok(false)
    }

    fn get_window_bits(&mut self) -> (&mut LazyState, &mut ScrollbarState) {
        if self.current_window {
            (&mut self.content_state, &mut self.content_scroll_state)
        } else {
            (&mut self.filter_state, &mut self.filter_scroll_state)
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
            Some(i) => clamped_add(
                i as u32,
                delta,
                0,
                (state.view.get_stats().file_lines - 1) as u32,
            ) as usize,
            None => 0,
        };

        self.place(i);
    }

    fn top(&mut self) {
        self.place(0);
    }

    fn bottom(&mut self) {
        self.place((self.content_state.view.get_stats().file_lines - 1) as usize)
    }

    fn resize(&mut self, delta: i32) {
        let mut delta = delta;
        if !self.current_window {
            delta = -delta;
        }
        self.content_fill = clamped_add(self.content_fill, delta, 1, 9);
    }

    fn toggle_tail(&mut self) {
        if self.current_window {
            self.content_tail = !self.content_tail;
        } else {
            self.filter_tail = !self.filter_tail;
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [title_area, main_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());
        let [file_area, controls_area, filter_area] = Layout::vertical([
            Constraint::Fill(self.content_fill as u16),
            Constraint::Length(3),
            Constraint::Fill(10 - self.content_fill as u16),
        ])
        .areas(main_area);

        let filename = Span::from(format!("File: {}", &self.path)).italic();
        let tail_status = Line::from(format!(
            "{} Tail",
            if self.content_tail { "☑" } else { "☐" }
        ));
        let file_stats = Line::from(self.compute_file_stats())
            .reversed()
            .alignment(Alignment::Right);
        let title_layout = Layout::horizontal([
            Constraint::Fill(4),
            Constraint::Length(10),
            Constraint::Length(30),
        ]);
        let [filename_area, tail_area, stats_area] = title_layout.areas(title_area);

        frame.render_widget(filename, filename_area);
        frame.render_widget(tail_status, tail_area);
        frame.render_widget(file_stats, stats_area);

        let widths = [Constraint::Length(5), Constraint::Fill(1)];

        let mut content = LazyList::new().block(
            Block::bordered()
                .border_set(self.selected_border(self.current_window))
                .title("Content"),
        );
        frame.render_stateful_widget(content, file_area, &mut self.content_state);
        self.render_scrollbar(frame, file_area);

        // frame.render_widget(Block::bordered().title("Controls"), controls_area);
        let filter_controls = Span::from(format!(
            " {} Tail",
            if self.filter_tail { "☑" } else { "☐" }
        ));
        frame.render_widget(
            Paragraph::new(filter_controls).block(Block::bordered().title("Controls")),
            controls_area,
        );

        let mut filter_content = LazyList::new().block(
            Block::bordered()
                .border_set(self.selected_border(!self.current_window))
                .title("Filtered"),
        );
        frame.render_stateful_widget(filter_content, filter_area, &mut self.filter_state);
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
        let stats = self.content_state.view.get_stats();

        format!("{}L / {}B", stats.file_lines, stats.file_bytes)
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
            &mut self.content_scroll_state,
        );
    }
}

fn clamped_add(a: u32, b: i32, min: u32, max: u32) -> u32 {
    let v = a as i64 + b as i64;
    if v > max as i64 {
        max
    } else if v < min as i64 {
        min
    } else {
        v as u32
    }
}
