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
