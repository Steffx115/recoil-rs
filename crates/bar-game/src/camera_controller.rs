//! Camera controller: pan via WASD, zoom via scroll wheel.

use std::collections::VecDeque;
use std::time::Instant;

use pierce_render::camera::Camera;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const PAN_SPEED: f32 = 5.0;
pub const ZOOM_SPEED: f32 = 10.0;
pub const MIN_HEIGHT: f32 = 50.0;
pub const MAX_HEIGHT: f32 = 800.0;

// ---------------------------------------------------------------------------
// CameraController
// ---------------------------------------------------------------------------

pub struct CameraController {
    pub center: [f32; 2],
    pub height: f32,
    forward: bool,
    left: bool,
    backward: bool,
    right: bool,
}

impl CameraController {
    pub fn new(cx: f32, cz: f32, height: f32) -> Self {
        Self {
            center: [cx, cz],
            height,
            forward: false,
            left: false,
            backward: false,
            right: false,
        }
    }

    pub fn process_key(&mut self, key: winit::keyboard::KeyCode, pressed: bool) {
        use winit::keyboard::KeyCode;
        match key {
            KeyCode::KeyW => self.forward = pressed,
            KeyCode::KeyA => self.left = pressed,
            KeyCode::KeyS => self.backward = pressed,
            KeyCode::KeyD => self.right = pressed,
            _ => {}
        }
    }

    pub fn process_scroll(&mut self, delta: f32) {
        self.height = (self.height - delta * ZOOM_SPEED).clamp(MIN_HEIGHT, MAX_HEIGHT);
    }

    pub fn update(&mut self) {
        let speed = PAN_SPEED * (self.height / 400.0);
        if self.forward {
            self.center[1] -= speed;
        }
        if self.backward {
            self.center[1] += speed;
        }
        if self.left {
            self.center[0] -= speed;
        }
        if self.right {
            self.center[0] += speed;
        }
    }

    pub fn camera(&self, aspect: f32) -> Camera {
        Camera {
            eye: [
                self.center[0],
                self.height,
                self.center[1] + self.height * 0.75,
            ],
            target: [self.center[0], 0.0, self.center[1]],
            up: [0.0, 1.0, 0.0],
            fov_y: std::f32::consts::FRAC_PI_4,
            aspect,
            near: 1.0,
            far: 2000.0,
        }
    }
}

// ---------------------------------------------------------------------------
// FPS counter
// ---------------------------------------------------------------------------

pub struct FpsCounter {
    frame_times: VecDeque<Instant>,
}

impl FpsCounter {
    pub fn new() -> Self {
        Self {
            frame_times: VecDeque::with_capacity(120),
        }
    }

    pub fn tick(&mut self) -> f32 {
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > 100 {
            self.frame_times.pop_front();
        }
        if self.frame_times.len() < 2 {
            return 0.0;
        }
        let elapsed = now
            .duration_since(*self.frame_times.front().unwrap())
            .as_secs_f32();
        if elapsed > 0.0 {
            (self.frame_times.len() - 1) as f32 / elapsed
        } else {
            0.0
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/camera_controller_tests.rs"]
mod tests;
