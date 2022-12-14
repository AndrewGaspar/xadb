use std::pin::Pin;

use futures::Stream;
use tokio_stream::StreamExt;
use tui::{
    layout::Constraint,
    style::{Color, Style},
    widgets::{Block, Cell, Row, StatefulWidget, Table, Widget},
};

use crate::commands::adb::{LogBuffer, LogLevel, LogMessage, LogcatDecodeError};

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

pub struct LogState {
    log_stream: Pin<Box<dyn Stream<Item = Result<LogMessage, LogcatDecodeError>>>>,
    logs: Vec<LogMessage>,
}

impl LogState {
    pub fn new(serial: &str) -> Self {
        let log_stream = Box::pin(crate::commands::adb::logcat(serial));
        Self {
            log_stream,
            logs: Default::default(),
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

        let rows = state
            .logs
            .iter()
            .rev()
            .map(|message| {
                let LogBuffer::TextLog(ref buffer) = message.buffer else { panic!() };

                Row::new([
                    Cell::from(buffer.tag.as_str()),
                    Cell::from(message.timestamp.to_string()),
                    Cell::from(buffer.message.as_str()),
                ])
                .style(style_from_level(buffer.level))
            })
            .take(area.height as usize)
            .collect::<Vec<_>>();

        let mut table = Table::new(rows.into_iter().rev())
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
