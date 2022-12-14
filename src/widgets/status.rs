use std::{pin::Pin, time::Duration};

use async_stream::stream;
use futures::{Stream, StreamExt};
use tui::{
    layout::Alignment,
    style::{Color, Style},
    widgets::{Paragraph, StatefulWidget, Widget, Wrap},
};

use crate::battery::battery;

type BatteryError = crate::battery::Error;

enum StatusEvent {
    Battery(Result<i32, BatteryError>),
}

pub struct StatusBar {}

impl StatusBar {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct StatusBarState {
    event_stream: Pin<Box<dyn Stream<Item = StatusEvent>>>,
    battery: Option<Result<i32, BatteryError>>,
}

impl StatusBarState {
    pub fn new() -> Self {
        let event_stream: Pin<Box<dyn Stream<Item = StatusEvent>>> = Box::pin(stream! {
            let mut interval = tokio::time::interval(Duration::from_secs(10));

            loop {
                interval.tick().await;
                yield StatusEvent::Battery(battery().await);
            }
        });

        Self {
            event_stream,
            battery: None,
        }
    }

    pub async fn poll(&mut self) {
        if let Some(event) = self.event_stream.next().await {
            match event {
                StatusEvent::Battery(battery) => {
                    self.battery = Some(battery);
                    return;
                }
            }
        }
    }
}

impl StatefulWidget for StatusBar {
    type State = StatusBarState;

    fn render(
        self,
        area: tui::layout::Rect,
        buf: &mut tui::buffer::Buffer,
        state: &mut Self::State,
    ) {
        let battery = match state.battery {
            Some(Ok(battery)) => battery.to_string(),
            Some(Err(_)) => "err".to_string(),
            None => "-".to_string(),
        };

        let status = Paragraph::new(format!("battery: {battery}"))
            .style(Style::default().bg(Color::Magenta).fg(Color::White))
            .alignment(Alignment::Right)
            .wrap(Wrap { trim: false });

        status.render(area, buf)
    }
}
