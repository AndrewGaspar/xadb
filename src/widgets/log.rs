use std::{ops::Range, pin::Pin};

use futures::Stream;
use tokio_stream::StreamExt;
use tui::{
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Cell, Row, StatefulWidget, Table, TableState, Widget},
};

use crate::{
    commands::adb::{LogBuffer, LogLevel, LogMessage, LogcatDecodeError},
    widgets::Control,
};

fn level_to_bg_color(level: LogLevel) -> Option<Color> {
    match level {
        LogLevel::Fatal => Some(Color::Red),
        LogLevel::Error => Some(Color::LightRed),
        LogLevel::Warning => Some(Color::LightYellow),
        _ => None,
    }
}

fn level_to_fg_color(level: LogLevel) -> Option<Color> {
    match level {
        LogLevel::Fatal | LogLevel::Error | LogLevel::Warning => Some(Color::Black),
        _ => None,
    }
}

fn style_from_level(level: LogLevel) -> Style {
    let mut style = Style::default();
    if let Some(bg) = level_to_bg_color(level) {
        style = style.bg(bg);
    }
    if let Some(fg) = level_to_fg_color(level) {
        style = style.fg(fg);
    }
    style
}

pub struct Log<'a> {
    block: Option<Block<'a>>,
}

impl<'a> Log<'a> {
    pub fn new() -> Self {
        Self {
            block: Default::default(),
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

#[derive(Copy, Clone)]
enum Anchor {
    Autoscroll,
    Bottom(usize),
    Top(usize),
}

pub struct LogState {
    log_stream: Pin<Box<dyn Stream<Item = Result<LogMessage, LogcatDecodeError>>>>,
    logs: Vec<LogMessage>,
    selected: Option<usize>,
    anchor: Anchor,
}

impl LogState {
    pub fn new(serial: &str) -> Self {
        let log_stream = Box::pin(crate::commands::adb::logcat(serial));
        Self {
            log_stream,
            logs: Default::default(),
            selected: None,
            anchor: Anchor::Autoscroll,
        }
    }

    pub async fn poll(&mut self) {
        if let Some(message) = self.log_stream.next().await {
            match message {
                Ok(message) => {
                    self.logs.push(message);
                    return;
                }
                _ => {}
            }
        }
    }

    pub fn control(&mut self, control: Control) {
        match control {
            Control::Up => {
                if let Some(selected) = self.selected {
                    self.selected = Some(selected.saturating_sub(1));
                } else if self.logs.len() > 0 {
                    self.selected = Some(self.logs.len() - 1);
                }
            }
            Control::Down => {
                if let Some(selected) = self.selected {
                    self.selected = Some((selected + 1).min(self.logs.len() - 1));
                }
            }
            Control::Bottom => {
                self.selected = None;
                self.anchor = Anchor::Autoscroll;
            }
            Control::Top => {
                if self.logs.len() > 0 {
                    self.selected = Some(0);
                }
            }
        }
    }

    fn rows_to_display(&self, height: usize) -> Range<usize> {
        if self.logs.len() <= height {
            return 0..self.logs.len();
        }

        match self.anchor {
            Anchor::Autoscroll => self.logs.len() - height..self.logs.len(),
            Anchor::Top(index) => index..index + height,
            Anchor::Bottom(index) => index - height + 1..index + 1,
        }
    }
}

impl<'a> StatefulWidget for Log<'a> {
    type State = LogState;

    fn render(
        self,
        area: tui::layout::Rect,
        buf: &mut tui::buffer::Buffer,
        state: &mut Self::State,
    ) {
        let header = Row::new(["Tag", "Date", "Message"]);

        let mut num_rows = area.height - 1;
        if self.block.is_some() {
            num_rows -= 2;
        }

        let rows_to_display = state.rows_to_display(num_rows as usize);

        // update anchoring
        if let Some(selected) = state.selected {
            if selected < rows_to_display.start {
                state.anchor = Anchor::Top(selected);
            } else if selected >= rows_to_display.end {
                state.anchor = Anchor::Bottom(selected);
            }
        }

        // update rows to display after fixing anchoring
        let rows_to_display = state.rows_to_display(num_rows as usize);

        let rows = state.logs[rows_to_display.clone()]
            .iter()
            .enumerate()
            .map(|(i, m)| (i + rows_to_display.start, m))
            .map(|(i, message)| {
                let LogBuffer::TextLog(ref buffer) = message.buffer else { panic!() };

                let mut base_style = style_from_level(buffer.level);
                if Some(i) == state.selected {
                    base_style = base_style.patch(
                        Style::default()
                            .bg(Color::Gray)
                            .fg(Color::Black)
                            .add_modifier(Modifier::BOLD),
                    );
                }

                Row::new([
                    Cell::from(buffer.tag.as_str()),
                    Cell::from(message.timestamp.to_string()),
                    Cell::from(buffer.message.as_str()),
                ])
                .style(base_style)
            })
            .take(num_rows as usize)
            .collect::<Vec<_>>();

        let mut table = Table::new(rows)
            .header(header.style(Style::default().bg(Color::Gray).fg(Color::Black)))
            .widths(&[
                Constraint::Length(20),
                Constraint::Length(20),
                Constraint::Percentage(100),
            ]);

        if let Some(block) = self.block {
            table = table.block(block);
        }

        Widget::render(table, area, buf)
    }
}
