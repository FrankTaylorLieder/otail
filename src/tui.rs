#![allow(unused_imports, unused_variables)]
use crate::{
    colour_spec::{Colour, ColouringRule, ColouringSpec, Colours},
    filter_spec::{FilterSpec, FilterType},
};
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
    style::{Color, Modifier, Style, Styled, Stylize},
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

    pub colouring: ColouringSpec,

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

            let base_style = if i == current {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            // TODO: We are looking at the rendered line content... does this matter for colouring?
            let mut content_style = base_style.clone();
            if let Some((fg, bg)) = state.colouring.maybe_colour(&l) {
                if let Some(fg) = fg {
                    content_style = content_style.fg(colour_to_color(fg));
                }
                if let Some(bg) = bg {
                    content_style = content_style.bg(colour_to_color(bg));
                }
            }

            // Break the line into margin and content. Only colour the content.

            let margin = format!(
                "{i:>margin_width$}{c}",
                i = i,
                c = if i == current { ">" } else { " " }
            );

            let content = format!(
                "{l:.content_width$}",
                content_width = content_width,
                l = l.get(self.start_point..).unwrap_or(""),
            );

            // TODO: Render the line_no, not the match_no for FilterLine. Will need to encapsulate
            // String and have a render columns method or similar.
            lines.push(Line::from(vec![
                Span::styled(margin, base_style),
                Span::styled(content, content_style),
            ]));

            state.cell_renders += 1;
        }
        Text::from(lines).render(inner, buf);
    }
}

fn colour_to_color(colour: Colour) -> Color {
    match colour {
        Colour::Black => Color::Black,
        Colour::Red => Color::Red,
        Colour::Green => Color::Green,
        Colour::Blue => Color::Blue,
        Colour::Yellow => Color::Yellow,
        Colour::Magenta => Color::Magenta,
        Colour::Cyan => Color::Cyan,
        Colour::Gray => Color::Gray,
        Colour::White => Color::White,
    }
}

#[derive(Debug, Clone)]
struct FilterEditState {
    enabled: bool,
    input: Input,
    filter_type: FilterType,
}

#[derive(Debug, Clone)]
struct ColouringEditState {
    spec: ColouringSpec,
    selected_rule_index: usize,
    focus_area: ColouringFocusArea,
    filter_edit_state: FilterEditState,
    selected_fg_color: Option<Colour>,
    selected_bg_color: Option<Colour>,
    pending_deletion: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
enum ColouringFocusArea {
    RulesList,
    PatternEditor,
    ColourPicker,
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

    // Current colouring to apply to all output
    colouring: ColouringSpec,

    // Are are we showing the colouring edit modal?
    colouring_edit: Option<ColouringEditState>,
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

        let colouring = ColouringSpec::new().set_rules(vec![
            ColouringRule {
                enabled: true,
                filter_spec: FilterSpec::new(FilterType::SimpleCaseInsensitive, "hello")
                    .expect("Failed to build sample filter spec"),
                fg_colour: Some(Colour::Red),
                bg_colour: None,
            },
            ColouringRule {
                enabled: true,
                filter_spec: FilterSpec::new(FilterType::SimpleCaseInsensitive, "123")
                    .expect("Failed to unwrap 123"),
                fg_colour: Some(Colour::Black),
                bg_colour: Some(Colour::Green),
            },
        ]);

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
                colouring: colouring.clone(),
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
                colouring: colouring.clone(),
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

            colouring,
            colouring_edit: None,
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
                match (&mut self.filter_edit, &mut self.colouring_edit) {
                    // Showing the main window.
                    (None, None) => match (key.code, key.modifiers) {
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

                        (KeyCode::Char('s'), _) => self.sync_filter_to_content().await?,
                        (KeyCode::Char('S'), _) => self.toggle_sync_lock().await?,

                        (KeyCode::Char('/'), _) => self.start_edit_filter(),
                        (KeyCode::Char('C'), _) => self.start_edit_colouring(),
                        _ => {}
                    },
                    // Showing the filter edit dialog.
                    (Some(filter_edit), None) => match (key.code, key.modifiers) {
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
                    // Showing the colouring edit dialog.
                    (_, Some(colouring_edit)) => match (key.code, key.modifiers) {
                        (KeyCode::Esc, _) => self.colouring_edit = None,
                        (KeyCode::BackTab, _) => {
                            // Cycle backwards through focus areas (Shift+Tab)
                            self.cycle_colouring_focus_backwards();
                        }
                        (KeyCode::Tab, _) => {
                            // Cycle forwards through focus areas
                            self.cycle_colouring_focus();
                        }
                        (KeyCode::Up, KeyModifiers::SHIFT) | (KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                            self.handle_colouring_move_rule_up();
                        }
                        (KeyCode::Down, KeyModifiers::SHIFT) | (KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                            self.handle_colouring_move_rule_down();
                        }
                        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                            self.handle_colouring_up_key();
                        }
                        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                            self.handle_colouring_down_key();
                        }
                        (KeyCode::Insert, _) | (KeyCode::Char('+'), _) => {
                            self.handle_colouring_add_rule();
                        }
                        (KeyCode::Delete, _) | (KeyCode::Char('-'), _) => {
                            self.handle_colouring_delete_rule();
                        }
                        (KeyCode::Char('y'), _) if colouring_edit.pending_deletion.is_some() => {
                            self.handle_colouring_confirm_deletion();
                        }
                        _ if colouring_edit.pending_deletion.is_some() => {
                            // Any other key cancels deletion
                            self.handle_colouring_cancel_deletion();
                        }
                        (KeyCode::Enter, _) => {
                            // Apply changes and close dialog
                            self.apply_colouring_changes();
                            self.colouring_edit = None;
                        }
                        // Handle pattern editing keys when focus is on pattern editor
                        _ if colouring_edit.focus_area == ColouringFocusArea::PatternEditor => {
                            match (key.code, key.modifiers) {
                                (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                                    let colouring_edit = self.colouring_edit.as_mut().unwrap();
                                    colouring_edit.filter_edit_state.enabled =
                                        !colouring_edit.filter_edit_state.enabled;
                                }
                                (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                                    let colouring_edit = self.colouring_edit.as_mut().unwrap();
                                    colouring_edit.filter_edit_state.filter_type =
                                        FilterType::SimpleCaseInsensitive;
                                }
                                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                    let colouring_edit = self.colouring_edit.as_mut().unwrap();
                                    colouring_edit.filter_edit_state.filter_type =
                                        FilterType::SimpleCaseSensitive;
                                }
                                (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                                    let colouring_edit = self.colouring_edit.as_mut().unwrap();
                                    colouring_edit.filter_edit_state.filter_type =
                                        FilterType::Regex;
                                }
                                _ => {
                                    let colouring_edit = self.colouring_edit.as_mut().unwrap();
                                    colouring_edit
                                        .filter_edit_state
                                        .input
                                        .handle_event(&Event::Key(*key));
                                    // Update the currently selected rule with the new pattern
                                    self.update_selected_rule_from_editor();
                                }
                            }
                        }
                        // Handle color selection keys (works regardless of focus area)
                        (KeyCode::Char('1'..='9' | '0'), _) | 
                        (KeyCode::Char('!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '(' | ')'), _) => {
                            self.handle_colouring_color_key(&key.code, &key.modifiers);
                        }
                        // Handle other keys when focus is on color picker
                        _ if colouring_edit.focus_area == ColouringFocusArea::ColourPicker => {
                            // Any other keys in color picker area are ignored
                        }
                        _ => {
                            // For rules list, other keys are ignored
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

    fn start_edit_colouring(&mut self) {
        let first_rule = self.colouring.rules().get(0);
        let initial_filter_state = if let Some(rule) = first_rule {
            FilterEditState {
                enabled: rule.enabled,
                input: rule.filter_spec.filter_pattern.clone().into(),
                filter_type: rule.filter_spec.filter_type.clone(),
            }
        } else {
            FilterEditState {
                enabled: true,
                input: "".into(),
                filter_type: FilterType::SimpleCaseInsensitive,
            }
        };

        self.colouring_edit = Some(ColouringEditState {
            spec: self.colouring.clone(),
            selected_rule_index: 0,
            focus_area: ColouringFocusArea::RulesList,
            filter_edit_state: initial_filter_state,
            selected_fg_color: first_rule.map(|r| r.fg_colour.clone()).flatten(),
            selected_bg_color: first_rule.map(|r| r.bg_colour.clone()).flatten(),
            pending_deletion: None,
        })
    }

    fn cycle_colouring_focus(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            colouring_edit.focus_area = match colouring_edit.focus_area {
                ColouringFocusArea::RulesList => ColouringFocusArea::PatternEditor,
                ColouringFocusArea::PatternEditor => ColouringFocusArea::ColourPicker,
                ColouringFocusArea::ColourPicker => ColouringFocusArea::RulesList,
            };
        }
    }

    fn cycle_colouring_focus_backwards(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            colouring_edit.focus_area = match colouring_edit.focus_area {
                ColouringFocusArea::RulesList => ColouringFocusArea::ColourPicker,
                ColouringFocusArea::PatternEditor => ColouringFocusArea::RulesList,
                ColouringFocusArea::ColourPicker => ColouringFocusArea::PatternEditor,
            };
        }
    }

    fn handle_colouring_up_key(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            match colouring_edit.focus_area {
                ColouringFocusArea::RulesList => {
                    if colouring_edit.selected_rule_index > 0 {
                        colouring_edit.selected_rule_index -= 1;
                        self.load_selected_rule_into_editor();
                    }
                }
                ColouringFocusArea::ColourPicker => {
                    // Handle color selection cycling
                    // This is a simplified version - in a full implementation,
                    // you'd want to track which color is being selected
                }
                _ => {}
            }
        }
    }

    fn handle_colouring_down_key(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            match colouring_edit.focus_area {
                ColouringFocusArea::RulesList => {
                    let max_index = colouring_edit.spec.rules().len().saturating_sub(1);
                    if colouring_edit.selected_rule_index < max_index {
                        colouring_edit.selected_rule_index += 1;
                        self.load_selected_rule_into_editor();
                    }
                }
                ColouringFocusArea::ColourPicker => {
                    // Handle color selection cycling
                    // This is a simplified version - in a full implementation,
                    // you'd want to track which color is being selected
                }
                _ => {}
            }
        }
    }

    fn handle_colouring_color_key(&mut self, key_code: &KeyCode, _modifiers: &KeyModifiers) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            match key_code {
                // Background color selection (shifted symbols)
                KeyCode::Char('!') => colouring_edit.selected_bg_color = None, // Shift+1
                KeyCode::Char('@') => colouring_edit.selected_bg_color = Some(Colour::Black), // Shift+2
                KeyCode::Char('#') => colouring_edit.selected_bg_color = Some(Colour::Red), // Shift+3
                KeyCode::Char('$') => colouring_edit.selected_bg_color = Some(Colour::Green), // Shift+4
                KeyCode::Char('%') => colouring_edit.selected_bg_color = Some(Colour::Blue), // Shift+5
                KeyCode::Char('^') => colouring_edit.selected_bg_color = Some(Colour::Yellow), // Shift+6
                KeyCode::Char('&') => colouring_edit.selected_bg_color = Some(Colour::Magenta), // Shift+7
                KeyCode::Char('*') => colouring_edit.selected_bg_color = Some(Colour::Cyan), // Shift+8
                KeyCode::Char('(') => colouring_edit.selected_bg_color = Some(Colour::White), // Shift+9
                KeyCode::Char(')') => colouring_edit.selected_bg_color = Some(Colour::Gray), // Shift+0
                // Foreground color selection (number keys)
                KeyCode::Char('1') => colouring_edit.selected_fg_color = None,
                KeyCode::Char('2') => colouring_edit.selected_fg_color = Some(Colour::Black),
                KeyCode::Char('3') => colouring_edit.selected_fg_color = Some(Colour::Red),
                KeyCode::Char('4') => colouring_edit.selected_fg_color = Some(Colour::Green),
                KeyCode::Char('5') => colouring_edit.selected_fg_color = Some(Colour::Blue),
                KeyCode::Char('6') => colouring_edit.selected_fg_color = Some(Colour::Yellow),
                KeyCode::Char('7') => colouring_edit.selected_fg_color = Some(Colour::Magenta),
                KeyCode::Char('8') => colouring_edit.selected_fg_color = Some(Colour::Cyan),
                KeyCode::Char('9') => colouring_edit.selected_fg_color = Some(Colour::White),
                KeyCode::Char('0') => colouring_edit.selected_fg_color = Some(Colour::Gray),
                _ => {}
            }

            // Update the current rule with the new color selection immediately
            self.update_selected_rule_from_editor();
        }
    }

    fn load_selected_rule_into_editor(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            if let Some(rule) = colouring_edit
                .spec
                .rules()
                .get(colouring_edit.selected_rule_index)
            {
                colouring_edit.filter_edit_state = FilterEditState {
                    enabled: rule.enabled,
                    input: rule.filter_spec.filter_pattern.clone().into(),
                    filter_type: rule.filter_spec.filter_type.clone(),
                };
                colouring_edit.selected_fg_color = rule.fg_colour.clone();
                colouring_edit.selected_bg_color = rule.bg_colour.clone();
            }
        }
    }

    fn update_selected_rule_from_editor(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            if let Ok(filter_spec) = FilterSpec::new(
                colouring_edit.filter_edit_state.filter_type.clone(),
                colouring_edit.filter_edit_state.input.value(),
            ) {
                let updated_rule = ColouringRule {
                    enabled: colouring_edit.filter_edit_state.enabled,
                    filter_spec,
                    fg_colour: colouring_edit.selected_fg_color.clone(),
                    bg_colour: colouring_edit.selected_bg_color.clone(),
                };

                colouring_edit
                    .spec
                    .update_rule(colouring_edit.selected_rule_index, updated_rule);
            }
        }
    }

    fn apply_colouring_changes(&mut self) {
        // First update the current rule with any pending editor changes
        self.update_selected_rule_from_editor();

        // Apply the modified spec to the main colouring
        if let Some(colouring_edit) = &self.colouring_edit {
            self.colouring = colouring_edit.spec.clone();

            // Also update the colouring in both UI panes
            self.content_state.colouring = colouring_edit.spec.clone();
            self.filter_state.colouring = colouring_edit.spec.clone();
        }
    }

    fn handle_colouring_add_rule(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            let new_rule = ColouringRule::default();
            let insert_index = colouring_edit.selected_rule_index + 1;

            colouring_edit
                .spec
                .add_rule(new_rule.clone(), Some(insert_index));
            colouring_edit.selected_rule_index = insert_index;

            // Load the new rule into the editor
            colouring_edit.filter_edit_state = FilterEditState {
                enabled: new_rule.enabled,
                input: new_rule.filter_spec.filter_pattern.clone().into(),
                filter_type: new_rule.filter_spec.filter_type.clone(),
            };
            colouring_edit.selected_fg_color = new_rule.fg_colour.clone();
            colouring_edit.selected_bg_color = new_rule.bg_colour.clone();
        }
    }

    fn handle_colouring_delete_rule(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            if !colouring_edit.spec.rules().is_empty() {
                colouring_edit.pending_deletion = Some(colouring_edit.selected_rule_index);
            }
        }
    }

    fn handle_colouring_confirm_deletion(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            if let Some(deletion_index) = colouring_edit.pending_deletion.take() {
                if colouring_edit.spec.remove_rule(deletion_index).is_some() {
                    // Adjust selection after deletion
                    let max_index = colouring_edit.spec.rules().len().saturating_sub(1);
                    if colouring_edit.selected_rule_index > max_index {
                        colouring_edit.selected_rule_index = max_index;
                    }

                    // Load the current rule (or clear if no rules left)
                    if colouring_edit.spec.rules().is_empty() {
                        // Reset to default state when no rules
                        let default_rule = ColouringRule::default();
                        colouring_edit.filter_edit_state = FilterEditState {
                            enabled: default_rule.enabled,
                            input: default_rule.filter_spec.filter_pattern.clone().into(),
                            filter_type: default_rule.filter_spec.filter_type.clone(),
                        };
                        colouring_edit.selected_fg_color = None;
                        colouring_edit.selected_bg_color = None;
                    } else {
                        self.load_selected_rule_into_editor();
                    }
                }
            }
        }
    }

    fn handle_colouring_cancel_deletion(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            colouring_edit.pending_deletion = None;
        }
    }

    fn handle_colouring_move_rule_up(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            if colouring_edit
                .spec
                .move_rule_up(colouring_edit.selected_rule_index)
            {
                colouring_edit.selected_rule_index -= 1;
            }
        }
    }

    fn handle_colouring_move_rule_down(&mut self) {
        if let Some(colouring_edit) = &mut self.colouring_edit {
            if colouring_edit
                .spec
                .move_rule_down(colouring_edit.selected_rule_index)
            {
                colouring_edit.selected_rule_index += 1;
            }
        }
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
            Tui::draw_filter_dlg(filter_edit, area, frame);
        }

        // Render the colours dlg if needed.
        if let Some(colouring_edit) = &self.colouring_edit {
            Tui::draw_colouring_dlg(colouring_edit, area, frame);
        }
    }

    fn draw_filter_dlg(filter_edit: &FilterEditState, area: Rect, frame: &mut Frame) {
        let area = Tui::popup_area(area, 60, 20);
        frame.render_widget(Clear, area);

        let surrounding_block =
            Block::bordered().title("Filter (Enter to apply, Esc to close, C-_ to toggle)");
        let inner_area = surrounding_block.inner(area);

        Tui::draw_filter_edit(filter_edit, inner_area, frame);
        frame.render_widget(surrounding_block, area);
    }

    fn draw_colouring_dlg(colouring_edit: &ColouringEditState, area: Rect, frame: &mut Frame) {
        let area = Tui::popup_area(area, 80, 70);
        frame.render_widget(Clear, area);

        let surrounding_block = Block::bordered().title("Colouring");
        let inner_area = surrounding_block.inner(area);

        let colouring_dlg_layout = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]);
        let [rules_area, edit_area] = colouring_dlg_layout.areas(inner_area);

        // Draw rules list (top section)
        Tui::draw_colouring_rules_list(colouring_edit, rules_area, frame);

        // Draw edit section (bottom section)
        Tui::draw_colouring_edit_section(colouring_edit, edit_area, frame);

        frame.render_widget(surrounding_block, area);
    }

    fn draw_colouring_rules_list(
        colouring_edit: &ColouringEditState,
        area: Rect,
        frame: &mut Frame,
    ) {
        let is_focused = colouring_edit.focus_area == ColouringFocusArea::RulesList;
        let border_style = if is_focused {
            symbols::border::THICK
        } else {
            symbols::border::PLAIN
        };

        let rules_title = if colouring_edit.pending_deletion.is_some() {
            "⚠️ Press 'y' to DELETE rule, any other key to CANCEL"
        } else {
            "Rules (Tab/Shift+Tab=focus, j/k/↑↓=nav, +/-=add/del, Shift+j/k/↑↓=move, Enter=apply, Esc=close)"
        };

        let rules_block = Block::new()
            .borders(Borders::ALL)
            .border_set(border_style)
            .title(rules_title);
        let inner_area = rules_block.inner(area);

        // Create list items for each rule
        let rules = colouring_edit.spec.rules();
        let items: Vec<Line> = rules
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                let enabled_str = if rule.enabled { "✓" } else { "✗" };
                let fg_str = rule
                    .fg_colour
                    .as_ref()
                    .map(|c| format!("{:?}", c))
                    .unwrap_or_else(|| "None".to_string());
                let bg_str = rule
                    .bg_colour
                    .as_ref()
                    .map(|c| format!("{:?}", c))
                    .unwrap_or_else(|| "None".to_string());

                let text = format!(
                    "{} {} → fg:{}/bg:{}",
                    enabled_str,
                    rule.filter_spec.render(),
                    fg_str,
                    bg_str
                );

                if i == colouring_edit.selected_rule_index {
                    Line::from(format!("> {}", text))
                        .style(Style::default().add_modifier(Modifier::BOLD))
                } else {
                    Line::from(format!("  {}", text))
                }
            })
            .collect();

        // If no rules, show placeholder
        let items = if items.is_empty() {
            vec![Line::from("  No rules defined")]
        } else {
            items
        };

        let list = Paragraph::new(items);
        frame.render_widget(rules_block, area);
        frame.render_widget(list, inner_area);
    }

    fn draw_colouring_edit_section(
        colouring_edit: &ColouringEditState,
        area: Rect,
        frame: &mut Frame,
    ) {
        // Split the edit area vertically: pattern editor on top, color picker on bottom
        let edit_layout = Layout::vertical([Constraint::Fill(1), Constraint::Min(6)]);
        let [pattern_area, color_area] = edit_layout.areas(area);

        // Draw pattern editor (reusing existing draw_filter_edit)
        let is_pattern_focused = colouring_edit.focus_area == ColouringFocusArea::PatternEditor;
        let pattern_border_style = if is_pattern_focused {
            symbols::border::THICK
        } else {
            symbols::border::PLAIN
        };

        let pattern_block = Block::new()
            .borders(Borders::ALL)
            .border_set(pattern_border_style)
            .title("Pattern");
        let pattern_inner_area = pattern_block.inner(pattern_area);

        Tui::draw_filter_edit(&colouring_edit.filter_edit_state, pattern_inner_area, frame);
        frame.render_widget(pattern_block, pattern_area);

        // Draw color picker
        Tui::draw_colour_picker(colouring_edit, color_area, frame);
    }

    fn draw_colour_picker(colouring_edit: &ColouringEditState, area: Rect, frame: &mut Frame) {
        let is_focused = colouring_edit.focus_area == ColouringFocusArea::ColourPicker;
        let border_style = if is_focused {
            symbols::border::THICK
        } else {
            symbols::border::PLAIN
        };

        let color_block = Block::new()
            .borders(Borders::ALL)
            .border_set(border_style)
            .title("Colours");
        let inner_area = color_block.inner(area);

        // Split into two columns: foreground and background
        let color_layout = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]);
        let [fg_area, bg_area] = color_layout.areas(inner_area);

        // Draw foreground color options
        let fg_colors = vec![
            Line::from(vec![Tui::draw_radiobutton(
                "[1] (None)",
                colouring_edit.selected_fg_color.is_none(),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[2] Black",
                colouring_edit.selected_fg_color == Some(Colour::Black),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[3] Red",
                colouring_edit.selected_fg_color == Some(Colour::Red),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[4] Green",
                colouring_edit.selected_fg_color == Some(Colour::Green),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[5] Blue",
                colouring_edit.selected_fg_color == Some(Colour::Blue),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[6] Yellow",
                colouring_edit.selected_fg_color == Some(Colour::Yellow),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[7] Magenta",
                colouring_edit.selected_fg_color == Some(Colour::Magenta),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[8] Cyan",
                colouring_edit.selected_fg_color == Some(Colour::Cyan),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[9] White",
                colouring_edit.selected_fg_color == Some(Colour::White),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[0] Gray",
                colouring_edit.selected_fg_color == Some(Colour::Gray),
            )]),
        ];

        // Draw background color options
        let bg_colors = vec![
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+1] (None)",
                colouring_edit.selected_bg_color.is_none(),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+2] Black",
                colouring_edit.selected_bg_color == Some(Colour::Black),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+3] Red",
                colouring_edit.selected_bg_color == Some(Colour::Red),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+4] Green",
                colouring_edit.selected_bg_color == Some(Colour::Green),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+5] Blue",
                colouring_edit.selected_bg_color == Some(Colour::Blue),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+6] Yellow",
                colouring_edit.selected_bg_color == Some(Colour::Yellow),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+7] Magenta",
                colouring_edit.selected_bg_color == Some(Colour::Magenta),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+8] Cyan",
                colouring_edit.selected_bg_color == Some(Colour::Cyan),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+9] White",
                colouring_edit.selected_bg_color == Some(Colour::White),
            )]),
            Line::from(vec![Tui::draw_radiobutton(
                "[Shift+0] Gray",
                colouring_edit.selected_bg_color == Some(Colour::Gray),
            )]),
        ];

        let fg_paragraph = Paragraph::new(fg_colors).block(Block::bordered().title("Foreground"));
        let bg_paragraph = Paragraph::new(bg_colors).block(Block::bordered().title("Background"));

        frame.render_widget(color_block, area);
        frame.render_widget(fg_paragraph, fg_area);
        frame.render_widget(bg_paragraph, bg_area);
    }

    fn draw_filter_edit(filter_edit: &FilterEditState, inner_area: Rect, frame: &mut Frame) {
        let vertical = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(10),
            Constraint::Length(1),
        ]);
        let [enabled_area, spec_area, filter_type_area] = vertical.areas(inner_area);

        let enabled = Line::from(vec![
            Span::raw("   "),
            Tui::draw_checkbox("[T]oggle enabled", filter_edit.enabled),
        ]);
        frame.render_widget(enabled, enabled_area);

        let filter_type = Line::from(vec![
            Span::raw("   "),
            Tui::draw_radiobutton(
                "In[s]ensitive",
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
