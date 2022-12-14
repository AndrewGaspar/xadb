use std::{
    collections::VecDeque,
    io::Stderr,
    pin::Pin,
    time::{Duration, Instant},
};

use async_stream::try_stream;
use crossterm::event::{Event, KeyCode, KeyEvent};
use futures::Stream;
use quick_error::quick_error;
use tokio::pin;
use tokio_stream::StreamExt;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
    Frame, Terminal,
};

use crate::{
    battery::battery,
    commands::adb::{LogBuffer, LogLevel, LogMessage},
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
    frames: VecDeque<Instant>,
    battery: Option<Result<i32, crate::battery::Error>>,
}

impl LogcatApp {
    pub fn new() -> Self {
        Self {
            logs: Default::default(),
            frames: Default::default(),
            battery: Default::default(),
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

        let mut battery_level_stream: Pin<
            Box<dyn Stream<Item = Result<i32, crate::battery::Error>>>,
        > = Box::pin(try_stream! {
            let mut interval = tokio::time::interval(Duration::from_secs(10));

            loop {
                let battery = battery().await?;
                yield battery;
                interval.tick().await;
            }
        });

        // self.logs = logs.take(10).collect().await;

        let target_fps = 60;
        let mut interval = tokio::time::interval(Duration::from_micros(
            (1000000.0 / target_fps as f64) as u64,
        ));
        loop {
            enum Event {
                Log(LogMessage),
                KeyEvent(KeyEvent),
                Battery(Result<i32, crate::battery::Error>),
                Tick,
            }

            let next = tokio::select! {
                log = logs.next() => {
                    Event::Log(log.unwrap())
                },
                key = poll_events.next() => {
                    Event::KeyEvent(key.unwrap())
                },
                battery = battery_level_stream.next() => {
                    Event::Battery(battery.unwrap())
                },
                _ = interval.tick() => {
                    Event::Tick
                },
            };

            match next {
                Event::Log(log) => {
                    self.logs.push(log);
                }
                Event::KeyEvent(key) => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    _ => {}
                },
                Event::Battery(battery) => {
                    self.battery = Some(battery);
                }
                Event::Tick => {
                    self.frames.push_back(Instant::now());
                    if self.frames.len() > 1024 {
                        self.frames.pop_front();
                    }
                    terminal.draw(|f| self.ui(f)).unwrap();
                }
            }
        }
    }

    fn ui<B: Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(1)])
            .split(f.size());

        let fps = if self.frames.len() >= 16 {
            Some(
                (self.frames.len() as f32
                    / (*self.frames.back().unwrap() - *self.frames.front().unwrap()).as_secs_f32())
                    as u32,
            )
        } else {
            None
        };

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

        let battery = match self.battery {
            Some(Ok(battery)) => battery.to_string(),
            Some(Err(_)) => "err".to_string(),
            None => "-".to_string(),
        };

        let fps = match fps {
            Some(fps) => fps.to_string(),
            None => "-".to_string(),
        };

        let status = Paragraph::new(format!("battery: {battery} fps: {fps}"))
            .style(Style::default().bg(Color::Magenta).fg(Color::White))
            .alignment(Alignment::Right)
            .wrap(Wrap { trim: false });

        f.render_widget(table, chunks[0]);
        f.render_widget(status, chunks[1]);
    }
}
