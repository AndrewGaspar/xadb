use std::{collections::HashSet, io::Stderr, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent};
use futures::Stream;
use quick_error::quick_error;
use tokio::pin;
use tokio_stream::StreamExt;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Cell, Row, Table},
    Frame, Terminal,
};

use crate::commands::adb::{LogBuffer, LogMessage, TextLogBuffer};

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

pub struct LogcatApp {
    logs: Vec<LogMessage>,
}

impl LogcatApp {
    pub fn new() -> Self {
        Self {
            logs: Default::default(),
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
                    None => std::process::exit(1),
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

        // self.logs = logs.take(10).collect().await;

        loop {
            terminal.draw(|f| self.ui(f)).unwrap();

            enum Event {
                Log(LogMessage),
                KeyEvent(KeyEvent),
            }

            let next = tokio::select! {
                log = logs.next() => {
                    Event::Log(log.unwrap())
                },
                key = poll_events.next() => {
                    Event::KeyEvent(key.unwrap())
                }
            };

            match next {
                Event::Log(log) => {
                    self.logs.push(log);
                }
                Event::KeyEvent(key) => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    _ => {}
                },
            }
        }
    }

    fn ui<B: Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .constraints([Constraint::Percentage(100)])
            .split(f.size());

        let header = Row::new(["tag", "message"]);

        let rows = self
            .logs
            .iter()
            .rev()
            .scan(chunks[0].height, |height, message| {
                let LogBuffer::TextLog(buffer) = message else { panic!() };

                let lines = buffer.message.lines().rev().take(height).rev().count();
                Some(
                    Row::new([
                        Cell::from(buffer.tag.clone()),
                        Cell::from(buffer.message.clone()),
                    ]), // .height(lines.try_into().unwrap()),
                )
            })
            .collect::<Vec<_>>();

        let table = Table::new(rows.into_iter().rev())
            .style(Style::default().fg(Color::White))
            .block(Block::default().title("Table"))
            .header(header.style(Style::default().bg(Color::Gray)))
            .widths(&[Constraint::Length(20), Constraint::Percentage(100)]);

        f.render_widget(table, chunks[0]);
    }
}
