use super::*;

#[test]
fn emit_creates_particles() {
    let mut sys = ParticleSystem::new(100);
    assert!(sys.is_empty());

    sys.emit(
        [0.0, 0.0, 0.0],
        10,
        [1.0, 0.5, 0.0, 1.0],
        (1.0, 5.0),
        (0.5, 2.0),
        (0.1, 0.5),
    );
    assert_eq!(sys.len(), 10);
}

#[test]
fn update_removes_expired() {
    let mut sys = ParticleSystem::new(100);
    sys.emit(
        [0.0, 0.0, 0.0],
        5,
        [1.0, 1.0, 1.0, 1.0],
        (1.0, 1.0),
        (0.1, 0.1), // very short life
        (1.0, 1.0),
    );
    assert_eq!(sys.len(), 5);

    // After a large time step, all should be dead.
    sys.update(1.0);
    assert_eq!(sys.len(), 0);
}

#[test]
fn update_moves_particles() {
    let mut sys = ParticleSystem::new(100);
    sys.emit(
        [0.0, 0.0, 0.0],
        1,
        [1.0, 1.0, 1.0, 1.0],
        (10.0, 10.0),
        (5.0, 5.0),
        (1.0, 1.0),
    );

    let before = sys.particles[0].position;
    sys.update(0.1);
    let after = sys.particles[0].position;

    // Position should have changed.
    let moved = (after[0] - before[0]).abs()
        + (after[1] - before[1]).abs()
        + (after[2] - before[2]).abs();
    assert!(moved > 0.0, "particle should have moved");
}

#[test]
fn pool_does_not_exceed_max() {
    let mut sys = ParticleSystem::new(10);
    sys.emit(
        [0.0, 0.0, 0.0],
        20, // try to emit more than max
        [1.0, 1.0, 1.0, 1.0],
        (1.0, 1.0),
        (1.0, 1.0),
        (1.0, 1.0),
    );
    assert_eq!(sys.len(), 10);

    // Emit more — should be capped.
    sys.emit(
        [0.0, 0.0, 0.0],
        5,
        [1.0, 1.0, 1.0, 1.0],
        (1.0, 1.0),
        (1.0, 1.0),
        (1.0, 1.0),
    );
    assert_eq!(sys.len(), 10);
}

#[test]
fn instances_returns_correct_count() {
    let mut sys = ParticleSystem::new(100);
    sys.emit(
        [0.0, 0.0, 0.0],
        7,
        [1.0, 0.5, 0.0, 1.0],
        (1.0, 5.0),
        (1.0, 2.0),
        (0.1, 0.5),
    );
    let insts = sys.instances();
    assert_eq!(insts.len(), 7);
}

#[test]
fn instances_have_valid_data() {
    let mut sys = ParticleSystem::new(100);
    sys.emit(
        [1.0, 2.0, 3.0],
        1,
        [1.0, 0.5, 0.0, 1.0],
        (0.0, 0.0), // zero speed
        (5.0, 5.0),
        (2.0, 2.0),
    );
    let insts = sys.instances();
    assert_eq!(insts.len(), 1);
    assert_eq!(insts[0].position, [1.0, 2.0, 3.0]);
    assert!(insts[0].size > 0.0);
}
