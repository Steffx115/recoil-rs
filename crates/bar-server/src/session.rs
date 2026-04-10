//! Game session: owns a ServerTick, paces the game at a fixed tick rate,
//! deduplicates commands, and broadcasts the authoritative command stream
//! to all clients. Clients run the sim locally.

use std::collections::BTreeMap;

use tokio::sync::mpsc;
use tracing::{info, warn};

use pierce_net::protocol::NetMessage;
use pierce_net::replay::{ReplayHeader, ReplayRecorder};
use pierce_net::server_tick::ServerTick;
use pierce_net::{encode_framed, PlayerCommand};

use super::ConnId;

/// Events sent to the session task from the main server loop.
pub enum SessionEvent {
    Commands {
        conn_id: ConnId,
        commands: Vec<PlayerCommand>,
    },
    Disconnected {
        conn_id: ConnId,
    },
}

/// Run a game session to completion.
///
/// Each tick (at `tick_rate` Hz):
/// 1. Drain incoming commands from all players.
/// 2. Feed to `ServerTick` for deduplication.
/// 3. Broadcast `FrameAdvance` to all clients (they run sim_tick locally).
/// 4. Record commands for replay.
pub async fn run_session(
    game_id: u64,
    players: Vec<ConnId>,
    writers: Vec<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    mut events: mpsc::UnboundedReceiver<SessionEvent>,
    tick_rate: u64,
) {
    let num_players = players.len() as u8;
    info!(game_id, num_players, tick_rate, "Session started");

    // Map conn_id → player_id (index in the players vec).
    let conn_to_pid: BTreeMap<ConnId, u8> = players
        .iter()
        .enumerate()
        .map(|(i, &cid)| (cid, i as u8))
        .collect();

    let mut server_tick = ServerTick::new(num_players);

    // Replay recorder.
    let mut recorder = ReplayRecorder::new(ReplayHeader {
        version: 1,
        map_hash: 0,
        num_players,
        game_settings: Vec::new(),
    });

    let tick_interval = std::time::Duration::from_micros(1_000_000 / tick_rate);
    let mut interval = tokio::time::interval(tick_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut active_players = players.len();

    loop {
        // Wait for the next tick.
        interval.tick().await;

        // Drain all pending events (non-blocking).
        while let Ok(event) = events.try_recv() {
            match event {
                SessionEvent::Commands { conn_id, commands } => {
                    if let Some(&pid) = conn_to_pid.get(&conn_id) {
                        server_tick.receive_commands(pid, &commands);
                    }
                }
                SessionEvent::Disconnected { conn_id } => {
                    if conn_to_pid.contains_key(&conn_id) {
                        active_players = active_players.saturating_sub(1);
                        info!(game_id, conn_id, active_players, "Player left session");
                    }
                }
            }
        }

        // Advance the server tick — always succeeds, produces deduplicated commands.
        let command_frames = server_tick.advance();

        // Record for replay.
        recorder.record_frame(command_frames.clone());

        // Broadcast FrameAdvance to all clients.
        let msg = encode_framed(&NetMessage::FrameAdvance {
            frame: server_tick.current_frame - 1,
            commands: command_frames,
        });
        for writer in &writers {
            if let Some(tx) = writer {
                let _ = tx.send(msg.clone());
            }
        }

        // End session if all players left.
        if active_players == 0 {
            info!(game_id, frame = server_tick.current_frame, "All players left, ending session");
            break;
        }
    }

    // Save replay.
    let replay = recorder.finish();
    let replay_path = format!("replays/game_{game_id}.replay");
    if let Err(e) = std::fs::create_dir_all("replays") {
        warn!(%e, "Failed to create replays directory");
    }
    match pierce_net::save_replay(&replay, std::path::Path::new(&replay_path)) {
        Ok(()) => info!(game_id, path = %replay_path, "Replay saved"),
        Err(e) => warn!(game_id, %e, "Failed to save replay"),
    }
}
