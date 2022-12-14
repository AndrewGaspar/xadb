pub mod fps_overlay;
pub mod log;
pub mod status;

#[derive(Copy, Clone)]
pub enum Control {
    Up,
    Down,
    Top,
    Bottom,
}
