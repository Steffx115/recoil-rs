//! Camera controller: pan via WASD, zoom via scroll wheel.

use std::collections::VecDeque;
use std::time::Instant;

use recoil_render::camera::Camera;

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
mod tests {
    use super::*;

    #[test]
    fn pan_forward_moves_center() {
        let mut ctrl = CameraController::new(100.0, 100.0, 400.0);
        let start_z = ctrl.center[1];
        ctrl.process_key(winit::keyboard::KeyCode::KeyW, true);
        ctrl.update();
        assert!(ctrl.center[1] < start_z, "forward pan should decrease Z");
    }

    #[test]
    fn pan_backward_moves_center() {
        let mut ctrl = CameraController::new(100.0, 100.0, 400.0);
        let start_z = ctrl.center[1];
        ctrl.process_key(winit::keyboard::KeyCode::KeyS, true);
        ctrl.update();
        assert!(ctrl.center[1] > start_z, "backward pan should increase Z");
    }

    #[test]
    fn pan_left_moves_center() {
        let mut ctrl = CameraController::new(100.0, 100.0, 400.0);
        let start_x = ctrl.center[0];
        ctrl.process_key(winit::keyboard::KeyCode::KeyA, true);
        ctrl.update();
        assert!(ctrl.center[0] < start_x, "left pan should decrease X");
    }

    #[test]
    fn pan_right_moves_center() {
        let mut ctrl = CameraController::new(100.0, 100.0, 400.0);
        let start_x = ctrl.center[0];
        ctrl.process_key(winit::keyboard::KeyCode::KeyD, true);
        ctrl.update();
        assert!(ctrl.center[0] > start_x, "right pan should increase X");
    }

    #[test]
    fn zoom_in_clamps_at_min() {
        let mut ctrl = CameraController::new(0.0, 0.0, MIN_HEIGHT + 1.0);
        // Scroll up a huge amount
        ctrl.process_scroll(10000.0);
        assert!(
            ctrl.height >= MIN_HEIGHT,
            "height should not go below MIN_HEIGHT"
        );
    }

    #[test]
    fn zoom_out_clamps_at_max() {
        let mut ctrl = CameraController::new(0.0, 0.0, MAX_HEIGHT - 1.0);
        // Scroll down a huge amount
        ctrl.process_scroll(-10000.0);
        assert!(
            ctrl.height <= MAX_HEIGHT,
            "height should not go above MAX_HEIGHT"
        );
    }

    #[test]
    fn zoom_scroll_positive_decreases_height() {
        let mut ctrl = CameraController::new(0.0, 0.0, 400.0);
        let h_before = ctrl.height;
        ctrl.process_scroll(1.0);
        assert!(ctrl.height < h_before, "positive scroll should zoom in");
    }

    #[test]
    fn no_movement_when_keys_released() {
        let mut ctrl = CameraController::new(50.0, 50.0, 400.0);
        ctrl.process_key(winit::keyboard::KeyCode::KeyW, true);
        ctrl.process_key(winit::keyboard::KeyCode::KeyW, false);
        let snapshot = ctrl.center;
        ctrl.update();
        assert_eq!(ctrl.center, snapshot, "no movement after key release");
    }

    #[test]
    fn camera_produces_valid_output() {
        let ctrl = CameraController::new(100.0, 200.0, 300.0);
        let cam = ctrl.camera(16.0 / 9.0);
        assert_eq!(cam.target, [100.0, 0.0, 200.0]);
        assert_eq!(cam.eye[0], 100.0);
        assert!(cam.eye[1] > 0.0, "eye should be above ground");
    }

    #[test]
    fn pan_speed_scales_with_height() {
        let mut low = CameraController::new(0.0, 0.0, 100.0);
        let mut high = CameraController::new(0.0, 0.0, 800.0);
        low.process_key(winit::keyboard::KeyCode::KeyD, true);
        high.process_key(winit::keyboard::KeyCode::KeyD, true);
        low.update();
        high.update();
        assert!(
            high.center[0] > low.center[0],
            "higher camera should pan faster"
        );
    }

    #[test]
    fn fps_counter_starts_at_zero() {
        let mut fps = FpsCounter::new();
        assert_eq!(fps.tick(), 0.0);
    }
}
