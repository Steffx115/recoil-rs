use super::*;
use crate::loader::{CobFunction, CobScript};

/// Helper: build a CobScript with given pieces and scripts.
fn make_script(
    pieces: &[&str],
    scripts: Vec<(&str, Vec<u32>)>,
    num_static_vars: usize,
) -> CobScript {
    CobScript {
        pieces: pieces.iter().map(|s| s.to_string()).collect(),
        scripts: scripts
            .into_iter()
            .map(|(name, code)| CobFunction {
                name: name.to_string(),
                code,
            })
            .collect(),
        num_static_vars,
    }
}

#[test]
fn vm_creation() {
    let script = make_script(&["base", "turret"], vec![], 3);
    let vm = CobVm::new(&script);
    assert_eq!(vm.pieces.len(), 2);
    assert_eq!(vm.static_vars.len(), 3);
    assert!(vm.threads.is_empty());
}

#[test]
fn call_nonexistent_script_returns_false() {
    let script = make_script(&["base"], vec![("Create", vec![opcodes::RETURN])], 0);
    let mut vm = CobVm::new(&script);
    assert!(!vm.call_script(&script, "NonExistent"));
    assert!(vm.threads.is_empty());
}

#[test]
fn call_existing_script_starts_thread() {
    let script = make_script(&["base"], vec![("Create", vec![opcodes::RETURN])], 0);
    let mut vm = CobVm::new(&script);
    assert!(vm.call_script(&script, "Create"));
    assert_eq!(vm.threads.len(), 1);
}

#[test]
fn return_finishes_thread() {
    let script = make_script(&["base"], vec![("Create", vec![opcodes::RETURN])], 0);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Create");
    vm.tick(&script);
    assert!(vm.threads.is_empty());
}

#[test]
fn push_constant_and_arithmetic() {
    // Script: push 10, push 3, add, pop to static[0], return
    let code = vec![
        opcodes::PUSH_CONSTANT,
        10,
        opcodes::PUSH_CONSTANT,
        3,
        opcodes::ADD,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 13);
}

#[test]
fn subtraction() {
    let code = vec![
        opcodes::PUSH_CONSTANT,
        10,
        opcodes::PUSH_CONSTANT,
        3,
        opcodes::SUB,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 7);
}

#[test]
fn division_by_zero() {
    let code = vec![
        opcodes::PUSH_CONSTANT,
        10,
        opcodes::PUSH_CONSTANT,
        0,
        opcodes::DIV,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 0);
}

#[test]
fn sleep_blocks_for_frames() {
    // Push 100 (ms), sleep, push 42 to static[0], return
    let code = vec![
        opcodes::PUSH_CONSTANT,
        100, // 100ms
        opcodes::SLEEP,
        opcodes::PUSH_CONSTANT,
        42,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");

    // First tick: should sleep.
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 0); // not reached yet
    assert_eq!(vm.threads.len(), 1);

    // Tick through the sleep duration (~4 frames for 100ms at 30fps).
    for _ in 0..5 {
        vm.tick(&script);
    }
    // Should have woken up and set static[0] to 42.
    assert_eq!(vm.static_vars[0], 42);
}

#[test]
fn move_piece_now() {
    // Push position=65536 (1.0), then MOVE_PIECE_NOW with inline piece=0, axis=1 (y)
    let code = vec![
        opcodes::PUSH_CONSTANT,
        65536,
        opcodes::MOVE_PIECE_NOW,
        0, // piece (inline)
        1, // axis Y (inline)
        opcodes::RETURN,
    ];
    let script = make_script(&["body"], vec![("Create", code)], 0);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Create");
    vm.tick(&script);

    let transforms = vm.get_piece_transforms();
    assert_eq!(transforms.len(), 1);
    assert!((transforms[0].translate[1] - 1.0).abs() < 1e-4);
}

#[test]
fn turn_piece_now() {
    // Push angle=16384 (90 degrees), then TURN_PIECE_NOW with inline piece=0, axis=0
    let code = vec![
        opcodes::PUSH_CONSTANT,
        16384,
        opcodes::TURN_PIECE_NOW,
        0, // piece (inline)
        0, // axis heading (inline)
        opcodes::RETURN,
    ];
    let script = make_script(&["body"], vec![("Create", code)], 0);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Create");
    vm.tick(&script);

    let transforms = vm.get_piece_transforms();
    let expected_rad = 16384.0 / COB_ANGULAR_UNIT;
    assert!((transforms[0].rotate[0] - expected_rad).abs() < 1e-4);
}

#[test]
fn show_hide_piece() {
    let code = vec![
        opcodes::HIDE_PIECE,
        0, // piece 0 (inline)
        opcodes::RETURN,
    ];
    let script = make_script(&["body"], vec![("Create", code)], 0);
    let mut vm = CobVm::new(&script);
    assert!(vm.pieces[0].visible);
    vm.call_script(&script, "Create");
    vm.tick(&script);
    assert!(!vm.pieces[0].visible);
}

#[test]
fn signal_kills_matching_threads() {
    // Script A: set signal mask 1, sleep forever
    let code_a = vec![
        opcodes::PUSH_CONSTANT,
        1,
        opcodes::SET_SIGNAL_MASK,
        opcodes::PUSH_CONSTANT,
        10000, // long sleep
        opcodes::SLEEP,
        opcodes::RETURN,
    ];
    // Script B: signal mask 1 (kills A), then return
    let code_b = vec![opcodes::PUSH_CONSTANT, 1, opcodes::SIGNAL, opcodes::RETURN];
    let script = make_script(&["body"], vec![("ScriptA", code_a), ("ScriptB", code_b)], 0);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "ScriptA");
    vm.tick(&script); // A sleeps
    assert_eq!(vm.threads.len(), 1);

    vm.call_script(&script, "ScriptB");
    vm.tick(&script); // B signals, killing A; B returns
    assert!(vm.threads.is_empty());
}

#[test]
fn comparison_operators() {
    // Test LESS_THAN: push 3, push 5, less_than => 1
    let code = vec![
        opcodes::PUSH_CONSTANT,
        3,
        opcodes::PUSH_CONSTANT,
        5,
        opcodes::LESS_THAN,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 1);
}

#[test]
fn jump_and_conditional_jump() {
    // push 1 (true), JNE should NOT jump (only jumps if zero), so store 42.
    let code = vec![
        opcodes::PUSH_CONSTANT,
        1, // push 1 (true)
        opcodes::JUMP_NOT_EQUAL,
        8, // jump target: past the store (to RETURN)
        opcodes::PUSH_CONSTANT,
        42,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 42);
}

#[test]
fn local_variables() {
    let code = vec![
        opcodes::CREATE_LOCAL_VAR,
        opcodes::PUSH_CONSTANT,
        99,
        opcodes::POP_LOCAL_VAR,
        0,
        opcodes::PUSH_LOCAL_VAR,
        0,
        opcodes::POP_STATIC_VAR,
        0,
        opcodes::RETURN,
    ];
    let script = make_script(&[], vec![("Test", code)], 1);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Test");
    vm.tick(&script);
    assert_eq!(vm.static_vars[0], 99);
}

#[test]
fn move_piece_with_speed_interpolates() {
    // Move piece 0 on Y axis to position 65536 (1.0) at speed 65536 (1.0/s)
    // Spring pop order: destination first (top), speed second (bottom)
    let code = vec![
        opcodes::PUSH_CONSTANT,
        65536, // speed = 1.0/s (pushed first = bottom)
        opcodes::PUSH_CONSTANT,
        65536, // target = 1.0 (pushed second = top)
        opcodes::MOVE_PIECE_WITH_SPEED,
        0, // piece (inline)
        1, // axis Y (inline)
        opcodes::RETURN,
    ];
    let script = make_script(&["body"], vec![("Create", code)], 0);
    let mut vm = CobVm::new(&script);
    vm.call_script(&script, "Create");

    // Tick 1: interpolate (nothing to move yet), then execute (sets target + returns).
    vm.tick(&script);
    // Target is now set, but interpolation already ran this tick.

    // Tick 2: interpolation moves piece toward target.
    vm.tick(&script);

    // Piece should have moved but not reached target yet.
    // Speed = 1.0 / 30fps = ~0.033 per frame.
    assert!(
        vm.pieces[0].current_pos[1] > 0.0,
        "piece should have moved, got {}",
        vm.pieces[0].current_pos[1]
    );
    assert!(vm.pieces[0].current_pos[1] < 1.0);

    // Tick many more times to reach target.
    for _ in 0..60 {
        vm.tick(&script);
    }
    assert!((vm.pieces[0].current_pos[1] - 1.0).abs() < 0.1);
}

#[test]
fn get_piece_transforms_returns_correct_count() {
    let script = make_script(&["a", "b", "c"], vec![], 0);
    let vm = CobVm::new(&script);
    let transforms = vm.get_piece_transforms();
    assert_eq!(transforms.len(), 3);
}

#[test]
fn default_piece_state_is_identity() {
    let script = make_script(&["base"], vec![], 0);
    let vm = CobVm::new(&script);
    let transforms = vm.get_piece_transforms();
    assert_eq!(transforms[0].translate, [0.0, 0.0, 0.0]);
    assert_eq!(transforms[0].rotate, [0.0, 0.0, 0.0]);
}

#[test]
fn real_armcom_walk() {
    // Try both relative paths (from repo root and from target/debug)
    let candidates = [
        "../Beyond-All-Reason-Sandbox/scripts/Units/armcom.cob",
        "../../../Beyond-All-Reason-Sandbox/scripts/Units/armcom.cob",
        "../../Beyond-All-Reason-Sandbox/scripts/Units/armcom.cob",
    ];
    let path = candidates.iter().map(std::path::Path::new).find(|p| p.exists());
    let Some(path) = path else {
        eprintln!("Skipping: armcom.cob not found");
        return;
    };
    let data = std::fs::read(path).unwrap();
    let script = crate::loader::parse_cob(&data).unwrap();
    let mut vm = CobVm::new(&script);
    
    // Run Create for 30 ticks
    vm.call_script(&script, "Create");
    for _ in 0..30 { vm.tick(&script); }
    
    // Run StartMoving then tick 100 frames
    vm.call_script(&script, "StartMoving");
    for _ in 0..100 {
        vm.tick(&script);
    }
    
    eprintln!("Threads after Walk: {}", vm.threads.len());
    eprintln!("isMoving (static[0]): {}", vm.static_vars.first().copied().unwrap_or(-1));
    
    let transforms = vm.get_piece_transforms();
    let mut non_zero = 0;
    for (i, t) in transforms.iter().enumerate() {
        if t.translate != [0.0; 3] || t.rotate != [0.0; 3] {
            let name = script.pieces.get(i).map(|s| s.as_str()).unwrap_or("?");
            eprintln!("  piece[{}] {}: rot={:.3?}", i, name, t.rotate);
            non_zero += 1;
        }
    }
    eprintln!("Total non-zero: {}", non_zero);
    // Walk should animate at least legs (lthigh, rthigh, etc.)
    assert!(non_zero > 5, "Walk should animate many pieces, got {}", non_zero);
}


#[test]
fn real_armcom_idle_transforms() {
    let candidates = [
        "../Beyond-All-Reason-Sandbox/scripts/Units/armcom.cob",
        "../../Beyond-All-Reason-Sandbox/scripts/Units/armcom.cob",
        "../../../Beyond-All-Reason-Sandbox/scripts/Units/armcom.cob",
    ];
    let Some(path) = candidates.iter().map(std::path::Path::new).find(|p| p.exists()) else {
        eprintln!("Skipping: armcom.cob not found");
        return;
    };
    let data = std::fs::read(path).unwrap();
    let script = crate::loader::parse_cob(&data).unwrap();
    let mut vm = CobVm::new(&script);
    
    // Run Create for 60 ticks (enough for StopWalking to complete)
    vm.call_script(&script, "Create");
    for _ in 0..60 { vm.tick(&script); }
    
    let transforms = vm.get_piece_transforms();
    for (i, t) in transforms.iter().enumerate() {
        if t.translate != [0.0; 3] || t.rotate != [0.0; 3] {
            let name = script.pieces.get(i).map(|s| s.as_str()).unwrap_or("?");
            eprintln!("  cob[{:2}] {:15} rot=[{:8.4}, {:8.4}, {:8.4}] pos=[{:8.4}, {:8.4}, {:8.4}]",
                i, name, t.rotate[0], t.rotate[1], t.rotate[2],
                t.translate[0], t.translate[1], t.translate[2]);
        }
    }
    let non_zero = transforms.iter().filter(|t| t.translate != [0.0;3] || t.rotate != [0.0;3]).count();
    eprintln!("Total non-zero: {}/{}", non_zero, transforms.len());
    eprintln!("Threads remaining: {}", vm.threads.len());
}
