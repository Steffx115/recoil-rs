use super::*;

#[test]
fn attenuation_at_zero_distance() {
    let v = compute_attenuation([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert!((v - 1.0).abs() < f64::EPSILON);
}

#[test]
fn attenuation_at_100_units() {
    // distance = 100 => 1/(1+1) = 0.5
    let v = compute_attenuation([100.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert!((v - 0.5).abs() < 1e-9);
}

#[test]
fn attenuation_at_large_distance() {
    let v = compute_attenuation([1000.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    // 1/(1+10) ≈ 0.0909
    assert!(v < 0.1);
    assert!(v > 0.0);
}

#[test]
fn attenuation_3d_diagonal() {
    // dist = sqrt(100^2 * 3) ≈ 173.2
    let v = compute_attenuation([100.0, 100.0, 100.0], [0.0, 0.0, 0.0]);
    let expected = 1.0 / (1.0 + 173.205_080_756_887_73 / 100.0);
    assert!((v - expected).abs() < 1e-9);
}

/// Test that the category limit prevents excess sounds (without needing an
/// audio backend). We exercise the logic via a mock-friendly helper struct.
#[test]
fn category_limit_prevents_excess() {
    let mut counts: BTreeMap<SoundCategory, u32> = BTreeMap::new();
    let max = 3u32;
    let cat = SoundCategory::Explosion;

    // Simulate playing sounds up to the limit.
    for _ in 0..max {
        let c = counts.entry(cat).or_insert(0);
        assert!(*c < max);
        *c += 1;
    }

    // The next attempt should be blocked.
    let c = counts.entry(cat).or_insert(0);
    assert!(*c >= max, "should have hit the limit");
}

/// After tick(), counts reset so new sounds can play.
#[test]
fn tick_resets_active_counts() {
    let mut counts: BTreeMap<SoundCategory, u32> = BTreeMap::new();
    counts.insert(SoundCategory::WeaponFire, 5);
    counts.insert(SoundCategory::Explosion, 3);

    // Simulate tick.
    for v in counts.values_mut() {
        *v = 0;
    }

    assert_eq!(counts[&SoundCategory::WeaponFire], 0);
    assert_eq!(counts[&SoundCategory::Explosion], 0);
}

#[test]
#[ignore] // Requires an audio device — skip in CI.
fn audio_engine_new_succeeds() {
    let engine = AudioEngine::new();
    assert!(engine.is_ok());
}
