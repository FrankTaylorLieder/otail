#![allow(unused_imports, unused_variables)]
use crate::common::{FilterSpec, FilterType};
use anyhow::{bail, Result};
use clap::builder::Styles;
use crossterm::event::{EventStream, KeyModifiers};
use fmtsize::{Conventional, FmtSize};
use futures::{FutureExt, StreamExt};
use futures_timer::Delay;
use log::{debug, error, info, trace, warn};
use num_format::{Locale, ToFormattedString};
use regex::Regex;
use std::{
    fmt::Display,
    io::{self, stdout},
    isize,
    marker::PhantomData,
    thread::{self, Thread},
    time::Duration,
};
use tokio::{select, sync::mpsc, time::interval};
use tui_input::{backend::crossterm::EventHandler, Input};

use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    crossterm::{
        event::{self, Event, KeyCode},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    layout::{Alignment, Constraint, Flex, Layout, Margin, Position, Rect},
    style::{Modifier, Style, Styled, Stylize},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        block::BlockExt, Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, TableState, Widget,
    },
    DefaultTerminal, Frame, Terminal,
};

use crate::{
    common::{self, clamped_add, LineContent, CHANNEL_BUFFER, MS_PER_FRAME},
    ffile::{FFReq, FFReqSender, FFResp, FFRespReceiver, FilterLine},
    ifile::{FileReqSender, FileRespReceiver, IFResp},
    view::View,
};

const MARGIN_EXTRAS: usize = 1; // Allow space between line number ond content
const SCROLLBAR_EXTRAS: usize = 1; // Allow space for scrollbar
const TOTAL_EXTRAS: usize = MARGIN_EXTRAS + SCROLLBAR_EXTRAS;

const RADIO_SELECTED: &str = "●";
const RADIO_UNSELECTED: &str = "○";

const CHECK_SELECTED: &str = "☑";
const CHECK_UNSELECTED: &str = "☐";

#[derive(Debug)]
struct LazyState<T, L> {
    pub view: View<T, L>,

    pub height_hint: usize,
    pub width_hint: usize,

    pub content_num_lines: usize,

    cell_renders: u32,
}

#[derive(Debug)]
struct LazyList<'a, T, L> {
    block: Option<Block<'a>>,
    start_point: usize,
    _phantom_resp: PhantomData<T>,
    _phantom_line: PhantomData<L>,
}

impl<'a, T, L> LazyList<'a, T, L> {
    pub fn new(start_point: usize) -> Self {
        Self {
            block: None,
            start_point,

            _phantom_resp: PhantomData,
            _phantom_line: PhantomData,
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl<'a, T: std::marker::Send + 'static, L: Clone + Default + LineContent> StatefulWidget
    for LazyList<'a, T, L>
{
    type State = LazyState<T, L>;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        self.block.render(area, buf);
        let inner = self.block.inner_if_some(area);

        let height = inner.height;
        let width = inner.width;

        state.height_hint = height as usize;
        state.width_hint = width as usize;

        let num_lines = state.view.get_stats().view_lines;

        let current = state.view.current();

        let margin_width = common::count_digits(state.content_num_lines) + MARGIN_EXTRAS;
        let all_subtractions = margin_width + SCROLLBAR_EXTRAS;
        let content_width = common::clamped_sub(width as usize, all_subtractions);

        let mut lines = Vec::with_capacity(state.height_hint);
        for i in state.view.range() {
            if i >= num_lines {
                break;
            }
            let maybe_l = state.view.get_line(i);

            let l = match maybe_l {
                Some(l) => l.render(),
                None => "...".to_owned(),
            };

            let style = if i == current {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let content = format!(
                "{i:>margin_width$}{c}{l:.content_width$}",
                i = i,
                c = if i == current { ">" } else { " " },
                content_width = content_width,
                l = l.get(self.start_point..).unwrap_or(""),
            );

            // TODO: Render the line_no, not the match_no for FilterLine. Will need to encapsulate
            // String and have a render columns method or similar.
            lines.push(Line::from(Span::styled(content, style)));

            state.cell_renders += 1;
        }
        Text::from(lines).render(inner, buf);
    }
}

#[derive(Debug, Clone)]
struct FilterEditState {
    enabled: bool,
    input: Input,
    filter_type: FilterType,
}

pub struct Tui {
    path: String,

    content_ifresp_recv: FileRespReceiver<IFResp<String>>,
    filter_ffresp_recv: FFRespReceiver,

    ff_sender: FFReqSender,

    content_state: LazyState<IFResp<String>, String>,
    content_scroll_state: ScrollbarState,
    content_tail: bool,

    filter_state: LazyState<FFResp, FilterLine>,
    filter_scroll_state: ScrollbarState,
    filter_tail: bool,

    // The current filter
    filter_spec: FilterSpec,
    filter_enabled: bool,

    // true for content, false for filter
    current_window: bool,
    // Fill ratio for content pane... 1..9
    content_fill: usize,
    // Margin for line numbers and carret
    line_no_width: usize,

    // Are we showing the filter edit modal?
    filter_edit: Option<FilterEditState>,

    // Make content follow filter selection.
    sync_filter_to_content: bool,
}

impl Tui {
    pub fn new(
        path: String,
        ifreq_sender: FileReqSender<IFResp<String>>,
        ffreq_sender: FileReqSender<FFResp>,
        ff_sender: FFReqSender,
    ) -> Self {
        let (content_ifresp_sender, content_ifresp_recv) = mpsc::channel(CHANNEL_BUFFER);
        let (filter_ifresp_sender, filter_ifresp_recv) = mpsc::channel(CHANNEL_BUFFER);

        let content_view = View::new(
            "content".to_owned(),
            ifreq_sender.clone(),
            content_ifresp_sender,
        );
        let filter_view = View::new(
            "filter".to_owned(),
            ffreq_sender.clone(),
            filter_ifresp_sender,
        );

        let s = Self {
            path,

            content_ifresp_recv,
            filter_ffresp_recv: filter_ifresp_recv,

            ff_sender,

            content_state: LazyState {
                view: content_view,
                height_hint: 0,
                width_hint: 0,
                content_num_lines: 0,
                cell_renders: 0,
            },
            content_scroll_state: ScrollbarState::new(0),
            content_tail: false,

            filter_scroll_state: ScrollbarState::new(0),
            filter_state: LazyState {
                view: filter_view,
                height_hint: 0,
                width_hint: 0,
                content_num_lines: 0,
                cell_renders: 0,
            },
            filter_tail: false,
            filter_spec: FilterSpec::new(FilterType::SimpleCaseInsensitive, "")
                .expect("Unexpected error building empty filter"),
            filter_enabled: false,

            current_window: true,
            content_fill: 7,
            line_no_width: 0,

            filter_edit: None,
            sync_filter_to_content: false,
        };

        s
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        let mut should_quit = false;

        self.content_state.view.init().await?;
        self.filter_state.view.init().await?;

        // Initialise the filter spec.
        self.set_filter_spec(self.filter_spec.clone()).await?;

        let mut reader = EventStream::new();
        let mut interval = tokio::time::interval(Duration::from_millis(MS_PER_FRAME));

        // Indicate if enough time has passed to render, or if something timely should render.
        let mut can_render = true;

        // Indicate if something needs to be rendered.
        let mut dirty = true;

        while !should_quit {
            if can_render && dirty {
                // Let the states know the current file length to ensure margin layout
                let content_stats = self.content_state.view.get_stats();
                self.content_state.content_num_lines = content_stats.file_lines;
                self.filter_state.content_num_lines = self.content_state.content_num_lines;

                // Sync the content/viewport size for scrollbars
                self.content_scroll_state = self
                    .content_scroll_state
                    .content_length(content_stats.file_lines)
                    .viewport_content_length(self.content_state.view.get_viewport_height());
                self.filter_scroll_state = self
                    .filter_scroll_state
                    .content_length(self.filter_state.view.get_stats().view_lines)
                    .viewport_content_length(self.filter_state.view.get_viewport_height());

                trace!("Draw!");
                terminal.draw(|frame| self.draw(frame))?;
                can_render = false;
                dirty = false;

                // After render, sync the window sizes back to the view.
                self.content_state
                    .view
                    .set_height(self.content_state.height_hint)
                    .await?;

                self.filter_state
                    .view
                    .set_height(self.filter_state.height_hint)
                    .await?;
            }

            let timeout = interval.tick();
            let crossterm_event = reader.next().fuse();
            select! {
                _ = timeout => {
                    can_render = true;
                },
                maybe_event = crossterm_event => {
                    trace!("Event: {:?}", maybe_event);
                    dirty = true;
                    can_render = true;
                    match maybe_event {
                        Some(Ok(e)) => {
                            should_quit = self.handle_event(&e).await?;
                        },
                        Some(Err(err)) => {
                            error!("Terminal error: {:?}", err);
                            bail!("Terminal error: {:?}", err);
                        },
                        None => {}
                    }
                },
                content_resp = self.content_ifresp_recv.recv() => {
                    trace!("TUI: Received content response from IFile channel: {:?}", content_resp);
                    dirty = true;
                    match content_resp {
                        None => {
                            debug!("Content IFResp closed... finishing");
                            break;
                        }
                        Some(cr) => {
                            match cr {
                                IFResp::ViewUpdate { update } => {
                                    trace!("TUI: Processing content view update: {:?}", update);
                                    self.content_state.view.handle_update(update).await;
                                }
                                IFResp::Truncated => {
                                    trace!("TUI: Content file truncated, resetting views");
                                    debug!("{}: File truncated", self.path);

                                    self.content_state.view.reset().await?;
                                    self.filter_state.view.reset().await?;
                                }
                                IFResp::FileError { reason } => {
                                    trace!("TUI: Content file error received: {}", reason);
                                    error!("{}: File error: {reason}", self.path);

                                    // TODO: Put this in a dlg...
                                }
                            }
                        }
                    }

                    self.line_no_width = common::count_digits(self.content_state.view.get_stats().file_lines) + MARGIN_EXTRAS;
                },
                filter_resp = self.filter_ffresp_recv.recv() => {
                    trace!("TUI: Received filter response from FFile channel: {:?}", filter_resp);
                    dirty = true;
                    match filter_resp {
                        None => {
                            debug!("Filter IFResp closed... finishing");
                            break;
                        }
                        Some(fr) => {
                            match fr {
                                FFResp::ViewUpdate { update } => {
                                    trace!("TUI: Processing filter view update: {:?}", update);
                                    self.filter_state.view.handle_update(update).await;
                                }
                                FFResp::Clear => {
                                    trace!("TUI: Filter cleared, resetting filter view");
                                    self.filter_state.view.reset().await?;
                                }
                            }

                            trace!("TUI: Auto-syncing after filter response if needed");
                            self.auto_sync_if_needed().await?;
                        }
                    }
                }
            }
        }

        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        Ok(())
    }

    async fn handle_event(&mut self, event: &Event) -> Result<bool> {
        let mut filter_spec_to_apply = None;
        if let Event::Key(key) = event {
            if key.kind == event::KeyEventKind::Press {
                match &mut self.filter_edit {
                    // Showing the main window.
                    None => match (key.code, key.modifiers) {
                        (KeyCode::Char('q'), _) => return Ok(true),

                        (KeyCode::Char('j') | KeyCode::Down, _) => self.scroll(1).await?,
                        (KeyCode::Char('k') | KeyCode::Up, _) => self.scroll(-1).await?,
                        (KeyCode::Char('d'), _) => self.scroll(20).await?,
                        (KeyCode::Char('u'), _) => self.scroll(-20).await?,
                        (KeyCode::Char(' ') | KeyCode::PageDown, _) => self.scroll_page(1).await?,
                        (KeyCode::Backspace | KeyCode::PageUp, _) => self.scroll_page(-1).await?,
                        (KeyCode::Char('g'), _) => self.top().await?,
                        (KeyCode::Char('G'), _) => self.bottom().await?,
                        (KeyCode::Char('z'), _) => self.center().await?,

                        (KeyCode::Char('H'), KeyModifiers::SHIFT) => self.pan(-20).await?,
                        (KeyCode::Char('L'), KeyModifiers::SHIFT) => self.pan(20).await?,
                        (KeyCode::Char('h'), _) => self.pan(-1).await?,
                        (KeyCode::Char('l'), _) => self.pan(1).await?,
                        (KeyCode::Char('0'), _) => self.pan_start().await?,
                        (KeyCode::Char('$'), _) => self.pan_end().await?,

                        (KeyCode::Char('=') | KeyCode::Char('+'), _) => self.resize(1).await,
                        (KeyCode::Char('-') | KeyCode::Char('_'), _) => self.resize(-1).await,

                        (KeyCode::Char('t'), _) => self.toggle_tail().await?,

                        (KeyCode::Tab, _) => self.current_window = !self.current_window,

                        (KeyCode::Char('/'), _) => self.start_edit_filter(),

                        (KeyCode::Char('s'), _) => self.sync_filter_to_content().await?,
                        (KeyCode::Char('S'), _) => self.toggle_sync_lock().await?,

                        _ => {}
                    },
                    // Showing the filter edit dialog.
                    Some(filter_edit) => match (key.code, key.modifiers) {
                        (KeyCode::Esc, _) => self.filter_edit = None,
                        (KeyCode::Enter, _) => {
                            trace!(
                                "TUI: Filter edit confirmed - enabled: {}, filter: '{}'",
                                filter_edit.enabled,
                                filter_edit.input.value()
                            );
                            self.filter_enabled = filter_edit.enabled;
                            let input = filter_edit.input.value();
                            filter_spec_to_apply =
                                Some(FilterSpec::new(filter_edit.filter_type.clone(), input)?);
                        }
                        (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                            filter_edit.enabled = !filter_edit.enabled;
                        }
                        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                            // Note: C-i is sent as a TAB keycode, so we cannot use it for this
                            // option.
                            filter_edit.filter_type = FilterType::SimpleCaseInsensitive;
                        }
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            filter_edit.filter_type = FilterType::SimpleCaseSensitive;
                        }
                        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                            filter_edit.filter_type = FilterType::Regex;
                        }
                        _ => {
                            filter_edit.input.handle_event(&Event::Key(*key));
                        }
                    },
                }
            }
        }

        if let Some(filter_spec) = filter_spec_to_apply {
            trace!(
                "TUI: Applying new filter spec from user input: {:?}",
                filter_spec
            );
            self.set_filter_spec(filter_spec.clone()).await?;
            self.filter_spec = filter_spec;
            self.filter_edit = None;
            trace!("TUI: Filter edit dialog closed after applying filter");
        }

        Ok(false)
    }

    async fn toggle_sync_lock(&mut self) -> Result<()> {
        trace!(
            "Toggling sync lock: current: {}",
            self.sync_filter_to_content
        );

        self.sync_filter_to_content = !self.sync_filter_to_content;
        self.auto_sync_if_needed().await?;

        Ok(())
    }

    async fn auto_sync_if_needed(&mut self) -> Result<()> {
        if self.sync_filter_to_content {
            trace!("TUI: Auto-sync enabled, syncing filter to content");
            self.sync_filter_to_content().await?;
        } else {
            trace!("TUI: Auto-sync disabled, skipping sync");
        }

        Ok(())
    }

    async fn sync_filter_to_content(&mut self) -> Result<()> {
        trace!("Sync filter to content");

        if !self.filter_enabled {
            trace!("No current filter, done.");
            return Ok(());
        };

        let match_no = self.filter_state.view.current();
        let filter_line = self.filter_state.view.get_line(match_no);

        let Some(filter_line) = filter_line else {
            trace!("Match line not yet populated, cannot sync yet.");
            return Ok(());
        };

        let line_no = filter_line.line_no;

        self.content_state.view.set_current(line_no).await?;
        self.content_scroll_state = self.content_scroll_state.position(line_no);

        self.content_state.view.center_current_line().await?;

        // Cancel tailing on content if just synced.
        self.content_tail = false;
        self.content_state.view.set_tail(false).await?;

        Ok(())
    }

    async fn set_filter_spec(&mut self, filter_spec: FilterSpec) -> Result<()> {
        trace!(
            "TUI: Setting filter spec: {:?}, enabled: {}",
            filter_spec,
            self.filter_enabled
        );
        self.filter_spec = filter_spec;

        let filter_to_send = if self.filter_enabled {
            Some(self.filter_spec.clone())
        } else {
            None
        };

        trace!(
            "TUI: Sending SetFilter request to FFile channel: filter_spec={:?}",
            filter_to_send
        );
        self.ff_sender
            .send(FFReq::SetFilter {
                filter_spec: filter_to_send,
            })
            .await?;
        trace!("TUI: SetFilter request sent successfully");

        Ok(())
    }

    async fn place(&mut self, i: usize) -> Result<()> {
        if self.current_window {
            self.content_state.view.set_current(i).await?;
            self.content_scroll_state = self.content_scroll_state.position(i);
        } else {
            self.filter_state.view.set_current(i).await?;
            self.filter_scroll_state = self.filter_scroll_state.position(i);
            self.auto_sync_if_needed().await?;
        }

        self.set_tail(false).await?;

        Ok(())
    }

    async fn scroll(&mut self, delta: isize) -> Result<()> {
        let i = if self.current_window {
            clamped_add(
                self.content_state.view.current(),
                delta,
                0,
                self.content_state.view.get_stats().file_lines - 1,
            )
        } else {
            clamped_add(
                self.filter_state.view.current(),
                delta,
                0,
                self.filter_state.view.get_stats().view_lines - 1,
            )
        };

        self.place(i).await
    }

    async fn scroll_page(&mut self, direction: isize) -> Result<()> {
        let amount = if self.current_window {
            self.content_state.height_hint
        } else {
            self.filter_state.height_hint
        };
        self.scroll(amount as isize * direction).await
    }

    async fn top(&mut self) -> Result<()> {
        self.place(0).await
    }

    async fn bottom(&mut self) -> Result<()> {
        let view_lines = if self.current_window {
            self.content_state.view.get_stats().view_lines
        } else {
            self.filter_state.view.get_stats().view_lines
        };
        self.place(view_lines - 1).await
    }

    async fn center(&mut self) -> Result<()> {
        if self.current_window {
            self.content_state.view.center_current_line().await?;
        } else {
            self.filter_state.view.center_current_line().await?;
        }

        Ok(())
    }

    async fn resize(&mut self, delta: isize) {
        let mut delta = delta;

        if !self.current_window {
            delta = -delta;
        }
        self.content_fill = clamped_add(self.content_fill, delta, 1, 9);
    }

    async fn pan(&mut self, delta: isize) -> Result<()> {
        if self.current_window {
            self.content_state.view.pan(
                delta,
                self.content_state.width_hint - self.line_no_width - TOTAL_EXTRAS,
            );
        } else {
            self.filter_state.view.pan(
                delta,
                self.filter_state.width_hint - self.line_no_width - TOTAL_EXTRAS,
            );
        };

        Ok(())
    }

    async fn pan_start(&mut self) -> Result<()> {
        if self.current_window {
            self.content_state.view.pan_start();
        } else {
            self.filter_state.view.pan_start();
        }

        Ok(())
    }

    async fn pan_end(&mut self) -> Result<()> {
        if self.current_window {
            self.content_state
                .view
                .pan_end(self.content_state.width_hint - self.line_no_width - TOTAL_EXTRAS);
        } else {
            self.filter_state
                .view
                .pan_end(self.filter_state.width_hint - self.line_no_width - TOTAL_EXTRAS);
        }

        Ok(())
    }

    async fn toggle_tail(&mut self) -> Result<()> {
        if self.current_window {
            self.set_tail(!self.content_tail).await
        } else {
            self.set_tail(!self.filter_tail).await
        }
    }

    async fn set_tail(&mut self, tail: bool) -> Result<()> {
        if self.current_window {
            self.content_tail = tail;
            self.content_state.view.set_tail(tail).await
        } else {
            self.filter_tail = tail;
            self.filter_state.view.set_tail(tail).await
        }
    }

    fn start_edit_filter(&mut self) {
        self.filter_edit = Some(FilterEditState {
            enabled: true,
            input: self.filter_spec.filter_pattern.clone().into(),
            filter_type: self.filter_spec.filter_type.clone(),
        });
    }

    fn draw_checkbox(label: &str, current: bool) -> Span<'_> {
        Span::from(format!(
            "{} {}",
            if current {
                CHECK_SELECTED
            } else {
                CHECK_UNSELECTED
            },
            label
        ))
    }

    fn draw_radiobutton(label: &str, current: bool) -> Span<'_> {
        Span::from(format!(
            "{} {}",
            if current {
                RADIO_SELECTED
            } else {
                RADIO_UNSELECTED
            },
            label
        ))
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let [title_area, main_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
        let [file_area, controls_area, filter_area] = Layout::vertical([
            Constraint::Fill(self.content_fill as u16),
            Constraint::Length(1),
            Constraint::Fill(10 - self.content_fill as u16),
        ])
        .areas(main_area);

        let filename = Span::from(format!("File: {}", &self.path)).italic();
        let tail_status = Tui::draw_checkbox("Tail", self.content_tail);
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

        let content = LazyList::new(self.content_state.view.get_start_point()).block(
            Block::bordered()
                .border_set(self.selected_border(self.current_window))
                .title("Content"),
        );
        frame.render_stateful_widget(content, file_area, &mut self.content_state);
        frame.render_stateful_widget(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            file_area.inner(Margin {
                vertical: 1,
                horizontal: 1,
            }),
            &mut self.content_scroll_state,
        );

        let filter_control_filter = Span::from(format!("Filter: {}", self.render_filter_spec()));
        let filter_controls = Line::from(vec![
            Tui::draw_checkbox("Sync", self.sync_filter_to_content),
            Span::from("  "),
            Tui::draw_checkbox("Tail", self.filter_tail),
        ]);
        let filter_control_stats =
            Line::from(self.compute_filter_stats(self.content_state.content_num_lines))
                .reversed()
                .alignment(Alignment::Right);
        let filter_control_layout = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(20),
            Constraint::Length(30),
        ]);
        let [filter_control_filter_area, filter_control_tail_area, filter_control_tail_matches] =
            filter_control_layout.areas(controls_area);
        frame.render_widget(filter_control_filter, filter_control_filter_area);
        frame.render_widget(filter_controls, filter_control_tail_area);
        frame.render_widget(filter_control_stats, filter_control_tail_matches);

        let filter_content = LazyList::new(self.filter_state.view.get_start_point()).block(
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

        // Render the filter spec dialog if needed.
        if let Some(filter_edit) = &self.filter_edit {
            let area = Tui::popup_area(area, 60, 20);
            frame.render_widget(Clear, area);

            let surrounding_block = Block::bordered().title("Filter");
            let inner_area = surrounding_block.inner(area);

            let vertical = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Fill(10),
                Constraint::Length(1),
            ]);
            let [instructions_area, enabled_area, spec_area, filter_type_area] =
                vertical.areas(inner_area);

            let instructions =
                Paragraph::new("(Enter to apply, Esc to close, C-x to toggle)").centered();
            frame.render_widget(instructions, instructions_area);

            let enabled = Line::from(vec![
                Span::raw("   "),
                Tui::draw_checkbox("[T]oggle enabled", filter_edit.enabled),
            ]);
            frame.render_widget(enabled, enabled_area);

            let filter_type = Line::from(vec![
                Span::raw("   "),
                Tui::draw_radiobutton(
                    "In[S]ensitive",
                    filter_edit.filter_type == FilterType::SimpleCaseInsensitive,
                ),
                Span::raw("  "),
                Tui::draw_radiobutton(
                    "[C]ase sensitive",
                    filter_edit.filter_type == FilterType::SimpleCaseSensitive,
                ),
                Span::raw("  "),
                Tui::draw_radiobutton("[R]egex", filter_edit.filter_type == FilterType::Regex),
            ]);
            frame.render_widget(filter_type, filter_type_area);

            let input_widget = Paragraph::new(filter_edit.input.value())
                .block(Block::default().borders(Borders::ALL).title("Expression"));
            frame.render_widget(input_widget, spec_area);

            let cursor_position = filter_edit.input.cursor() as u16;
            frame.set_cursor_position(Position::new(
                spec_area.x + cursor_position + 1,
                spec_area.y + 1,
            ));

            frame.render_widget(surrounding_block, area);
        }
    }

    fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
        let vertical = Layout::vertical([Constraint::Max(5), Constraint::Percentage(percent_y)])
            .flex(Flex::Start);
        let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
        let [_, area] = vertical.areas(area);
        let [area] = horizontal.areas(area);
        area
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

        format!(
            "{} L / {}",
            stats.file_lines.to_formatted_string(&Locale::en),
            (stats.file_bytes as u64).fmt_size(Conventional)
        )
    }

    fn compute_filter_stats(&mut self, num_lines: usize) -> String {
        let stats = self.filter_state.view.get_stats();

        let perc = if stats.file_lines > 0 {
            &(((stats.file_lines as f32 / num_lines as f32) * 100_f32) as usize)
                .to_formatted_string(&Locale::en)
        } else {
            "-"
        };

        format!(
            "{} M / {}%",
            stats.view_lines.to_formatted_string(&Locale::en),
            perc
        )
    }

    fn render_filter_spec(&self) -> String {
        if self.filter_enabled {
            format!("{}", self.filter_spec.render())
        } else {
            "(None)".to_owned()
        }
    }
}
