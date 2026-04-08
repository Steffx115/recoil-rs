use crate::projectile_renderer::ProjectileInstance;

// ---------------------------------------------------------------------------
// Particle
// ---------------------------------------------------------------------------

/// A single CPU-driven particle with position, velocity, color, lifetime, and
/// size. Particles are ticked on the CPU and converted to
/// [`ProjectileInstance`]s for rendering.
#[derive(Clone, Debug)]
pub struct Particle {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub color: [f32; 4],
    pub life: f32,
    pub max_life: f32,
    pub size: f32,
}

// ---------------------------------------------------------------------------
// ParticleSystem
// ---------------------------------------------------------------------------

/// CPU-driven particle pool with a fixed maximum capacity.
pub struct ParticleSystem {
    particles: Vec<Particle>,
    max_particles: usize,
}

impl ParticleSystem {
    /// Create a new particle system that can hold at most `max_particles` live
    /// particles. The internal storage is pre-allocated.
    pub fn new(max_particles: usize) -> Self {
        Self {
            particles: Vec::with_capacity(max_particles),
            max_particles,
        }
    }

    /// Spawn `count` particles at `position` with deterministic spread.
    ///
    /// Velocity directions are evenly distributed using a simple deterministic
    /// pattern (golden-angle spiral on a sphere) so no external RNG is needed.
    /// Each particle gets a speed between `speed_range.0` and `speed_range.1`,
    /// a lifetime between `life_range.0` and `life_range.1`, and a size between
    /// `size_range.0` and `size_range.1`, all linearly interpolated by the
    /// particle index within the batch.
    pub fn emit(
        &mut self,
        position: [f32; 3],
        count: usize,
        color: [f32; 4],
        speed_range: (f32, f32),
        life_range: (f32, f32),
        size_range: (f32, f32),
    ) {
        let budget = self.max_particles.saturating_sub(self.particles.len());
        let actual = count.min(budget);

        // Golden angle in radians (~2.399)
        let golden_angle: f32 = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());

        for i in 0..actual {
            let t = if actual <= 1 {
                0.5
            } else {
                i as f32 / (actual - 1) as f32
            };

            // Deterministic direction via golden-angle spiral on a sphere.
            let y = 1.0 - 2.0 * t; // ranges from 1 to -1
            let radius = (1.0 - y * y).max(0.0).sqrt();
            let theta = golden_angle * i as f32;
            let x = radius * theta.cos();
            let z = radius * theta.sin();

            let speed = speed_range.0 + (speed_range.1 - speed_range.0) * t;
            let life = life_range.0 + (life_range.1 - life_range.0) * t;
            let size = size_range.0 + (size_range.1 - size_range.0) * t;

            self.particles.push(Particle {
                position,
                velocity: [x * speed, y * speed, z * speed],
                color,
                life,
                max_life: life,
                size,
            });
        }
    }

    /// Tick all particles: integrate position, decrement life, remove dead.
    pub fn update(&mut self, dt: f32) {
        self.particles.retain_mut(|p| {
            p.life -= dt;
            if p.life <= 0.0 {
                return false;
            }
            p.position[0] += p.velocity[0] * dt;
            p.position[1] += p.velocity[1] * dt;
            p.position[2] += p.velocity[2] * dt;
            true
        });
    }

    /// Convert live particles to renderable instances, reusing the
    /// [`ProjectileInstance`] format. Alpha from the particle color is used to
    /// modulate brightness as the particle ages.
    pub fn instances(&self) -> Vec<ProjectileInstance> {
        self.particles
            .iter()
            .map(|p| {
                let age_frac = if p.max_life > 0.0 {
                    p.life / p.max_life
                } else {
                    0.0
                };
                // Fade color by remaining life fraction and alpha channel.
                let fade = age_frac * p.color[3];
                let vel_len = (p.velocity[0] * p.velocity[0]
                    + p.velocity[1] * p.velocity[1]
                    + p.velocity[2] * p.velocity[2])
                    .sqrt();
                let dir = if vel_len > 1e-6 {
                    [
                        p.velocity[0] / vel_len,
                        p.velocity[1] / vel_len,
                        p.velocity[2] / vel_len,
                    ]
                } else {
                    [0.0, 1.0, 0.0]
                };
                ProjectileInstance {
                    position: p.position,
                    size: p.size * age_frac, // shrink as it dies
                    velocity_dir: dir,
                    _pad: 0.0,
                    color: [p.color[0] * fade, p.color[1] * fade, p.color[2] * fade],
                    _pad2: 0.0,
                }
            })
            .collect()
    }

    /// Number of currently live particles.
    pub fn len(&self) -> usize {
        self.particles.len()
    }

    /// Whether the system has no live particles.
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }
}

#[cfg(test)]
mod tests {
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
}
