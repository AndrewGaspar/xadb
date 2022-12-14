use std::{collections::VecDeque, time::Instant};

use tui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, StatefulWidget, Widget},
};

pub struct FpsOverlay {}

impl FpsOverlay {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct FpsOverlayState {
    num_frames: usize,
    frames: VecDeque<Instant>,
}

impl FpsOverlayState {
    pub fn new(num_frames: usize) -> Self {
        let mut frames = VecDeque::new();
        frames.reserve(num_frames);
        Self { num_frames, frames }
    }
}

impl StatefulWidget for FpsOverlay {
    type State = FpsOverlayState;

    fn render(
        self,
        area: tui::layout::Rect,
        buf: &mut tui::buffer::Buffer,
        state: &mut Self::State,
    ) {
        state.frames.push_back(Instant::now());
        if state.frames.len() > state.num_frames {
            state.frames.pop_front();
        }

        let fps = if state.frames.len() >= 2 {
            Some(
                (state.frames.len() as f32
                    / (*state.frames.back().unwrap() - *state.frames.front().unwrap())
                        .as_secs_f32()) as u32,
            )
        } else {
            None
        };

        let fps = match fps {
            Some(fps) => fps.to_string(),
            None => "-".to_string(),
        };

        let fps = Paragraph::new(format!("fps: {fps}"))
            .alignment(tui::layout::Alignment::Right)
            .style(Style::default().bg(Color::Red).fg(Color::White));

        let target = Rect::new(area.width.saturating_sub(8), 0, area.width.min(8), 1);

        fps.render(target, buf)
    }
}
