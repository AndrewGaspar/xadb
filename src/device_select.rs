use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crossterm::event::{self, KeyCode};
use quick_error::quick_error;
use tokio::pin;
use tokio_stream::StreamExt;
use tui::{
    backend::Backend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame, Terminal,
};

type CrosstermEvent = crossterm::event::Event;

use crate::{
    cache::Cache,
    devices::{query_devices_continuously, AdbDevice, AdbDeviceProperties},
};

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Cache(err: crate::cache::Error) {
            from()
        }
        Device(err: crate::devices::Error) {
            from()
        }
        Io(err: crate::io::Error) {
            from()
        }
    }
}

pub struct StatefulList<T> {
    state: ListState,
    items: Vec<T>,
}

impl<T> StatefulList<T> {
    fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => Some(if i >= self.items.len() - 1 { 0 } else { i + 1 }),
            None => {
                if self.items.is_empty() {
                    None
                } else {
                    Some(0)
                }
            }
        };
        self.state.select(i);
    }

    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => Some(if i == 0 { self.items.len() - 1 } else { i - 1 }),
            None => {
                if self.items.is_empty() {
                    None
                } else {
                    Some(0)
                }
            }
        };
        self.state.select(i);
    }

    fn unselect(&mut self) {
        self.state.select(None);
    }

    fn delete_selected(&mut self) {
        if let Some(index) = self.state.selected() {
            self.items.remove(index);

            // if no items left, then deselect
            if self.items.is_empty() {
                self.state.select(None);
            } else {
                // move to next
                self.next();
            }
        }
    }

    fn selected(&self) -> Option<&T> {
        self.items.get(self.state.selected()?)
    }
}

#[derive(Debug)]
struct DeviceItem {
    serial: String,
    live: Option<AdbDeviceProperties>,
    cache: Option<AdbDeviceProperties>,
}

/// This struct holds the current state of the app. In particular, it has the `items` field which is a wrapper
/// around `ListState`. Keeping track of the items state let us render the associated widget with its state
/// and have access to features such as natural scrolling.
///
/// Check the event handling at the bottom to see how to change the state on incoming events.
/// Check the drawing logic for items on how to specify the highlighting style for selected items.
pub struct DeviceSelectApp {
    items: StatefulList<DeviceItem>,
    cache: Cache,
}

impl DeviceSelectApp {
    pub async fn load_initial_state() -> Result<DeviceSelectApp, Error> {
        let cache = Cache::load_from_disk();

        let live_devices = crate::devices::online_devices().collect();

        let (cache, live_devices): (_, Result<Vec<_>, _>) = tokio::join!(cache, live_devices);
        let mut cache = cache?;
        let live_devices = live_devices?;

        let mut live_device_map = HashMap::new();

        let mut devices = Vec::new();
        for (i, device) in live_devices.into_iter().enumerate() {
            cache.save_device(&device.serial, &device.properties);
            live_device_map.insert(device.serial.clone(), i);
            devices.push(DeviceItem {
                serial: device.serial,
                live: Some(device.properties),
                cache: None,
            });
        }

        cache.persist().await?;

        for (serial, properties) in &cache.devices {
            match live_device_map.get(serial) {
                Some(index) => devices[*index].cache = Some(properties.clone()),
                None => devices.push(DeviceItem {
                    serial: serial.clone(),
                    live: None,
                    cache: Some(properties.clone()),
                }),
            }
        }

        Ok(DeviceSelectApp {
            items: StatefulList::with_items(devices),
            cache,
        })
    }

    async fn update_devices(&mut self, devices: Vec<AdbDevice>) -> Result<(), Error> {
        let mut new_devices: HashMap<String, AdbDevice> =
            devices.into_iter().map(|d| (d.serial.clone(), d)).collect();

        // check which devices have new state
        for current in &mut self.items.items {
            if let Some(new_device) = new_devices.remove(&current.serial) {
                current.live = Some(new_device.properties.clone());

                let cache = current.cache.as_mut().unwrap();
                cache.connection_state = new_device.properties.connection_state;
                cache.devpath = new_device.properties.devpath;
                if let Some(live) = new_device.properties.live {
                    cache.live = Some(live);
                }
                self.cache.save_device(&current.serial, &cache);
            } else {
                current.live = None;
            }
        }

        // add new devices
        for (serial, device) in new_devices {
            self.cache.save_device(&serial, &device.properties);
            self.items.items.push(DeviceItem {
                serial,
                live: Some(device.properties.clone()),
                cache: Some(device.properties),
            });
        }

        self.cache.persist().await?;

        Ok(())
    }

    pub async fn run<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        tick_rate: Duration,
    ) -> Result<Option<String>, Error> {
        let mut last_tick = Instant::now();
        let query_devices = query_devices_continuously(Duration::from_secs(1));
        pin!(query_devices);

        loop {
            terminal.draw(|f| self.ui(f))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            enum Event {
                Devices(Result<Vec<AdbDevice>, crate::devices::Error>),
                CrosstermEvent(Option<CrosstermEvent>),
            }

            let next = tokio::select! {
                devices = query_devices.next() => {
                    Event::Devices(devices.unwrap())
                },
                is_event = tokio::task::spawn_blocking(move || crossterm::event::poll(timeout)) => {
                    let is_event = is_event.unwrap();
                    if is_event? {
                        Event::CrosstermEvent(Some(event::read()?))
                    } else {
                        Event::CrosstermEvent(None)
                    }
                },
            };

            match next {
                Event::Devices(Ok(devices)) => {
                    self.update_devices(devices).await?;
                }
                Event::CrosstermEvent(event) => {
                    match event {
                        Some(CrosstermEvent::Key(key)) => match key.code {
                            KeyCode::Char('q') => return Ok(None),
                            KeyCode::Left | KeyCode::Char('h') => self.items.unselect(),
                            KeyCode::Down | KeyCode::Char('j') => self.items.next(),
                            KeyCode::Up | KeyCode::Char('k') => self.items.previous(),
                            KeyCode::Delete => {
                                if let Some(item) = self.items.selected() {
                                    self.cache.remove_device(&item.serial);
                                    self.cache.persist().await?;
                                    self.items.delete_selected();
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(item) = self.items.selected() {
                                    return Ok(Some(item.serial.clone()));
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }

                    if last_tick.elapsed() >= tick_rate {
                        last_tick = Instant::now();
                    }
                }
                _ => {}
            }
        }
    }

    fn ui<B: Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .constraints([Constraint::Percentage(100)])
            .split(f.size());

        // Iterate through all elements in the `items` app and append some debug text to it.
        let items: Vec<ListItem> = self
            .items
            .items
            .iter()
            .map(|i| {
                let product = match &i.live {
                    Some(AdbDeviceProperties {
                        live: Some(live), ..
                    }) => live.product.clone(),
                    _ => match &i.cache {
                        Some(AdbDeviceProperties {
                            live: Some(live), ..
                        }) => format!("{} (stale)", live.product),
                        _ => i.serial.clone(),
                    },
                };

                // build top line
                let mut top_line: Vec<Span> = vec![i.serial.as_str().into()];
                if let Some(live) = &i.live {
                    let color = match live.connection_state.as_str() {
                        "device" => Color::Green,
                        "fastboot" => Color::Yellow,
                        _ => Color::Cyan,
                    };

                    top_line.push(Span::styled(
                        format!(" (online, {})", live.connection_state),
                        Style::default().fg(color),
                    ));
                } else {
                    top_line.push(Span::styled(" (offline)", Style::default().fg(Color::Red)));
                }

                let lines = vec![
                    Spans::from(top_line),
                    Spans::from(Span::styled(
                        format!("product: {product}"),
                        Style::default().add_modifier(Modifier::ITALIC),
                    )),
                ];

                ListItem::new(lines).style(Style::default().fg(Color::White))
            })
            .collect();

        // Create a List from all list items and highlight the currently selected one
        let items = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("devices"))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );

        // We can now render the item list
        f.render_stateful_widget(items, chunks[0], &mut self.items.state);
    }
}
