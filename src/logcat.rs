use std::{io::Stderr, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent};
use futures::Stream;
use quick_error::quick_error;
use tokio::pin;
use tokio_stream::StreamExt;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Cell, Row, Table},
    Frame, Terminal,
};

use crate::{
    commands::adb::{LogBuffer, LogLevel, LogMessage},
    widgets::fps_overlay::{FpsOverlay, FpsOverlayState},
    widgets::status::{StatusBar, StatusBarState},
};

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: crate::io::Error) {
            from()
        }
        Decode(err: crate::commands::adb::LogcatDecodeError) {
            from()
        }
        DeviceSelect(err: crate::device_select::Error) {
            from()
        }
    }
}

fn crossterm_event_stream() -> impl Stream<Item = crossterm::Result<Event>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    tokio::task::spawn_blocking(move || loop {
        if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
            tx.send(crossterm::event::read()).unwrap();
        }

        if tx.is_closed() {
            break;
        }
    });

    return tokio_stream::wrappers::UnboundedReceiverStream::from(rx);
}

fn level_to_bg_color(level: LogLevel) -> Option<Color> {
    match level {
        LogLevel::Fatal => Some(Color::Red),
        LogLevel::Error => Some(Color::LightRed),
        LogLevel::Warning => Some(Color::Yellow),
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

pub struct LogcatApp {
    logs: Vec<LogMessage>,
    status_bar: StatusBarState,
    fps_overlay: FpsOverlayState,
}

impl LogcatApp {
    pub fn new() -> Self {
        Self {
            logs: Default::default(),
            status_bar: StatusBarState::new(),
            fps_overlay: FpsOverlayState::new(128),
        }
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stderr>>,
    ) -> Result<(), Error> {
        let serial = match std::env::var("ANDROID_SERIAL") {
            Ok(serial) => serial,
            _ => {
                let mut device_list =
                    crate::device_select::DeviceSelectApp::load_initial_state().await?;

                match device_list
                    .run(terminal, std::time::Duration::from_millis(250))
                    .await?
                {
                    Some(serial) => serial,
                    None => return Ok(()),
                }
            }
        };

        let logs = crate::commands::adb::logcat(serial.as_str()).filter_map(Result::ok);
        // let logs = tokio_stream::pending::<Option<LogMessage>>();
        pin!(logs);

        let poll_events = crossterm_event_stream().filter_map(|event| {
            if let Ok(Event::Key(key)) = event {
                Some(key)
            } else {
                None
            }
        });
        pin!(poll_events);

        let target_fps = 60;
        let mut interval = tokio::time::interval(Duration::from_micros(
            (1000000.0 / target_fps as f64) as u64,
        ));

        let mut update = false;

        loop {
            enum Event {
                Log(LogMessage),
                KeyEvent(KeyEvent),
                WidgetUpdate,
                Tick,
            }

            let next = tokio::select! {
                log = logs.next() => {
                    Event::Log(log.unwrap())
                },
                key = poll_events.next() => {
                    Event::KeyEvent(key.unwrap())
                },
                _ = self.status_bar.poll() => {
                    Event::WidgetUpdate
                },
                _ = interval.tick() => {
                    Event::Tick
                },
            };

            match next {
                Event::Log(log) => {
                    update = true;
                    self.logs.push(log);
                }
                Event::KeyEvent(key) => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    _ => {}
                },
                Event::WidgetUpdate => {
                    update = true;
                }
                Event::Tick => {
                    if update {
                        terminal.draw(|f| self.ui(f)).unwrap();
                        update = false;
                    }
                }
            }
        }
    }

    fn ui<B: Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(1)])
            .split(f.size());

        let header = Row::new(["Tag", "Date", "Message"]);

        let rows = self
            .logs
            .iter()
            .rev()
            .map(|message| {
                let LogBuffer::TextLog(ref buffer) = message.buffer else { panic!() };

                Row::new([
                    Cell::from(buffer.tag.clone()),
                    Cell::from(message.timestamp.to_string()),
                    Cell::from(buffer.message.clone()),
                ])
                .style(style_from_level(buffer.level))
            })
            .take(chunks[0].height as usize)
            .collect::<Vec<_>>();

        let table = Table::new(rows.into_iter().rev())
            .header(header.style(Style::default().bg(Color::Gray).fg(Color::Black)))
            .widths(&[
                Constraint::Length(20),
                Constraint::Length(20),
                Constraint::Percentage(100),
            ]);

        f.render_widget(table, chunks[0]);

        let status_bar = StatusBar::new();
        f.render_stateful_widget(status_bar, chunks[1], &mut self.status_bar);

        let fps_overlay = FpsOverlay::new();
        f.render_stateful_widget(fps_overlay, f.size(), &mut self.fps_overlay);
    }
}
