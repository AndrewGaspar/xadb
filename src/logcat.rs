use std::{io::Stderr, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent};
use futures::Stream;
use quick_error::quick_error;
use tokio::pin;
use tokio_stream::StreamExt;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders},
    Frame, Terminal,
};

use crate::{
    widgets::{
        fps_overlay::{FpsOverlay, FpsOverlayState},
        log::LogState,
    },
    widgets::{
        log::Log,
        status::{StatusBar, StatusBarState},
    },
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

pub struct LogcatApp {
    zoom: bool,
    debug: bool,
    log: Option<LogState>,
    status_bar: StatusBarState,
    fps_overlay: FpsOverlayState,
}

impl LogcatApp {
    pub fn new() -> Self {
        Self {
            zoom: false,
            debug: false,
            log: Default::default(),
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

        self.log = Some(LogState::new(serial.as_str()));

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
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut update = false;

        loop {
            enum Event {
                KeyEvent(KeyEvent),
                WidgetUpdate,
                Tick,
            }

            let next = tokio::select! {
                key = poll_events.next() => {
                    Event::KeyEvent(key.unwrap())
                },
                _ = self.log.as_mut().unwrap().poll() => {
                    Event::WidgetUpdate
                }
                _ = self.status_bar.poll() => {
                    Event::WidgetUpdate
                },
                _ = interval.tick(), if update => {
                    Event::Tick
                },
            };

            match next {
                Event::KeyEvent(key) => match key.code {
                    KeyCode::Char('z') => {
                        self.zoom = !self.zoom;
                        update = true;
                    }
                    KeyCode::Char('?') => {
                        self.debug = !self.debug;
                        update = true;
                    }
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
        self.fps_overlay.record_new_frame();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(1)])
            .split(f.size());

        let mut log = Log::new();
        if !self.zoom {
            log = log.block(
                Block::default()
                    .title("Log")
                    .title_alignment(tui::layout::Alignment::Left)
                    .borders(Borders::all()),
            );
        }
        f.render_stateful_widget(log, chunks[0], self.log.as_mut().unwrap());

        let status_bar = StatusBar::new();
        f.render_stateful_widget(status_bar, chunks[1], &mut self.status_bar);

        if self.debug {
            // render overlay last so it can pop over everything else
            let fps_overlay = FpsOverlay::new();
            f.render_stateful_widget(fps_overlay, f.size(), &mut self.fps_overlay);
        }
    }
}
