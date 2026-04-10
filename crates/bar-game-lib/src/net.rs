//! Network integration for headless and rendered game clients.
//!
//! Provides the game tick loop driven by server `FrameAdvance` messages,
//! command application from network frames, and stats tracking.

use bevy_ecs::world::World;

use pierce_net::client::{AdaptLevel, ClientFrameBuffer};
use pierce_net::protocol::{CommandFrame, NetMessage};
use pierce_sim::sim_runner::sim_tick;
use pierce_sim::{Allegiance, Dead, SimId};

use crate::building;
use crate::GameState;

/// Stats returned after each network-driven tick batch.
pub struct TickStats {
    /// Number of sim ticks processed this call.
    pub ticks_run: u64,
    /// Total frames processed since the session started.
    pub total_frames: u64,
    /// Number of units still alive.
    pub alive_count: usize,
    /// How many frames are still buffered (waiting to be processed).
    pub frames_behind: usize,
    /// Current adaptation level.
    pub adapt_level: AdaptLevel,
}

/// Network-driven game session. Wraps a `GameState` and a
/// `ClientFrameBuffer`, providing a single `process_tick` entry point.
pub struct NetGame {
    pub game: GameState,
    pub frame_buf: ClientFrameBuffer,
    pub max_catch_up: usize,
}

impl NetGame {
    /// Create a new networked game session from an existing `GameState`.
    pub fn new(game: GameState) -> Self {
        Self {
            game,
            frame_buf: ClientFrameBuffer::new(),
            max_catch_up: 5,
        }
    }

    /// Push a received `NetMessage` into the frame buffer.
    pub fn receive(&mut self, msg: &NetMessage) {
        self.frame_buf.push(msg);
    }

    /// Push a `FrameAdvance` directly.
    pub fn receive_frame(&mut self, frame: u64, commands: Vec<CommandFrame>) {
        self.frame_buf.push_frame(frame, commands);
    }

    /// Process as many buffered frames as appropriate this render frame.
    ///
    /// Runs 1 tick normally, or up to `max_catch_up` ticks when the
    /// client is critically behind. Returns stats about what happened.
    pub fn process_ticks(&mut self) -> TickStats {
        let ticks_to_run = self.frame_buf.ticks_this_frame(self.max_catch_up);
        let mut ticks_run = 0;

        for _ in 0..ticks_to_run {
            if let Some((_frame, commands)) = self.frame_buf.next_frame() {
                apply_commands(&mut self.game.world, &commands);
                run_game_tick(&mut self.game);
                ticks_run += 1;
            }
        }

        let alive_count = self
            .game
            .world
            .query_filtered::<&Allegiance, bevy_ecs::query::Without<Dead>>()
            .iter(&self.game.world)
            .count();

        TickStats {
            ticks_run,
            total_frames: self.game.frame_count,
            alive_count,
            frames_behind: self.frame_buf.buffered_frames(),
            adapt_level: self.frame_buf.adapt_level(),
        }
    }

    /// Compute a deterministic world checksum for sync validation.
    pub fn checksum(&mut self) -> u64 {
        pierce_sim::sim_runner::world_checksum(&mut self.game.world)
    }
}

/// Run one full game tick (construction + sim + equip + finalize).
/// This is the shared tick loop used by both headless and rendered clients.
pub fn run_game_tick(game: &mut GameState) {
    pierce_sim::construction::construction_system(&mut game.world);
    sim_tick(&mut game.world);
    building::equip_factory_spawned_units(&mut game.world, &game.weapon_def_ids);
    building::finalize_completed_buildings(&mut game.world);
    game.frame_count += 1;
}

/// Apply commands from FrameAdvance to the ECS command queues.
pub fn apply_commands(world: &mut World, frames: &[CommandFrame]) {
    for frame in frames {
        for cmd in &frame.commands {
            let entity = {
                let mut query = world.query::<(bevy_ecs::entity::Entity, &SimId)>();
                query
                    .iter(world)
                    .find(|(_, sid)| sid.id == cmd.target_sim_id)
                    .map(|(e, _)| e)
            };

            if let Some(entity) = entity {
                if let Some(mut queue) =
                    world.get_mut::<pierce_sim::commands::CommandQueue>(entity)
                {
                    queue.push(cmd.command.clone());
                }
            }
        }
    }
}
