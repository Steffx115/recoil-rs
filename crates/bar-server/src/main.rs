//! Dedicated game server with queue-based matchmaking.
//!
//! Players connect via TCP, enter a matchmaking queue, and get paired
//! into game sessions. Each session runs the simulation at a fixed 30 Hz
//! tick rate on the server.
//!
//! Usage: `bar-server [--port PORT] [--players-per-game N] [--tick-rate HZ]`

mod matchmaker;
mod session;

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use pierce_net::protocol::NetMessage;
use pierce_net::{decode, encode_framed};

use crate::matchmaker::Matchmaker;

/// Unique connection ID assigned to each player.
type ConnId = u64;

/// Messages from connection handlers to the main server loop.
enum ServerEvent {
    /// Player connected and wants to play.
    PlayerReady { conn_id: ConnId },
    /// Player sent commands.
    PlayerCommands {
        conn_id: ConnId,
        commands: Vec<pierce_net::PlayerCommand>,
    },
    /// Player disconnected.
    PlayerDisconnected { conn_id: ConnId },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port = std::env::args()
        .position(|a| a == "--port")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(7878);

    let players_per_game = std::env::args()
        .position(|a| a == "--players-per-game")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(2);

    let tick_rate = std::env::args()
        .position(|a| a == "--tick-rate")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);

    info!(port, players_per_game, tick_rate, "Starting server");

    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Listening on 0.0.0.0:{port}");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ServerEvent>();

    // Connection writers, keyed by ConnId.
    let writers: Arc<tokio::sync::Mutex<BTreeMap<ConnId, mpsc::UnboundedSender<Vec<u8>>>>> =
        Arc::new(tokio::sync::Mutex::new(BTreeMap::new()));

    let mut next_conn_id: ConnId = 1;
    let mut matchmaker = Matchmaker::new(players_per_game);
    let mut next_game_id: u64 = 1;

    // Map conn_id → game task handle + command sender.
    let mut active_games: BTreeMap<u64, tokio::task::JoinHandle<()>> = BTreeMap::new();
    let mut conn_to_game: BTreeMap<ConnId, u64> = BTreeMap::new();
    // Per-game command sender.
    let mut game_cmd_txs: BTreeMap<u64, mpsc::UnboundedSender<session::SessionEvent>> =
        BTreeMap::new();

    // Spawn acceptor task.
    let event_tx_accept = event_tx.clone();
    let writers_accept = writers.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let conn_id = next_conn_id;
                    next_conn_id += 1;
                    info!(conn_id, %addr, "Player connected");

                    let (read_half, write_half) = stream.into_split();
                    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();

                    writers_accept
                        .lock()
                        .await
                        .insert(conn_id, write_tx);

                    // Writer task.
                    let mut write_half = write_half;
                    tokio::spawn(async move {
                        while let Some(data) = write_rx.recv().await {
                            if write_half.write_all(&data).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Reader task.
                    let event_tx = event_tx_accept.clone();
                    tokio::spawn(async move {
                        let mut read_half = read_half;
                        let mut buf = vec![0u8; 64 * 1024];

                        // Signal ready immediately on connect.
                        let _ = event_tx.send(ServerEvent::PlayerReady { conn_id });

                        loop {
                            // Read length-prefixed frames.
                            let mut len_buf = [0u8; 4];
                            if read_half.read_exact(&mut len_buf).await.is_err() {
                                let _ = event_tx.send(ServerEvent::PlayerDisconnected { conn_id });
                                break;
                            }
                            let len = u32::from_le_bytes(len_buf) as usize;
                            if len > buf.len() {
                                buf.resize(len, 0);
                            }
                            if read_half.read_exact(&mut buf[..len]).await.is_err() {
                                let _ = event_tx.send(ServerEvent::PlayerDisconnected { conn_id });
                                break;
                            }

                            match decode(&buf[..len]) {
                                Ok(NetMessage::Commands { commands, .. }) => {
                                    let _ = event_tx.send(ServerEvent::PlayerCommands {
                                        conn_id,
                                        commands,
                                    });
                                }
                                Ok(_) => {
                                    // Ignore other message types for now.
                                }
                                Err(e) => {
                                    warn!(conn_id, %e, "Bad message from client");
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    error!(%e, "Accept failed");
                }
            }
        }
    });

    // Main server loop: process events.
    loop {
        let event = match event_rx.recv().await {
            Some(e) => e,
            None => break,
        };

        match event {
            ServerEvent::PlayerReady { conn_id } => {
                if let Some(players) = matchmaker.enqueue(conn_id) {
                    // Match found — start a game session.
                    let game_id = next_game_id;
                    next_game_id += 1;

                    info!(game_id, ?players, "Match found, starting game");

                    // Send Hello to each player with their player_id.
                    let writers_lock = writers.lock().await;
                    for (pid, &cid) in players.iter().enumerate() {
                        if let Some(tx) = writers_lock.get(&cid) {
                            let hello = encode_framed(&NetMessage::Hello {
                                player_id: pid as u8,
                                game_id,
                            });
                            let _ = tx.send(hello);
                        }
                        conn_to_game.insert(cid, game_id);
                    }
                    drop(writers_lock);

                    // Build per-player writer senders for the session.
                    let mut session_writers = Vec::new();
                    let writers_lock = writers.lock().await;
                    for &cid in &players {
                        session_writers
                            .push(writers_lock.get(&cid).cloned());
                    }
                    drop(writers_lock);

                    let (sess_tx, sess_rx) = mpsc::unbounded_channel();
                    game_cmd_txs.insert(game_id, sess_tx);

                    let handle = tokio::spawn(session::run_session(
                        game_id,
                        players.clone(),
                        session_writers,
                        sess_rx,
                        tick_rate,
                    ));
                    active_games.insert(game_id, handle);
                }
            }

            ServerEvent::PlayerCommands { conn_id, commands } => {
                if let Some(&game_id) = conn_to_game.get(&conn_id) {
                    if let Some(tx) = game_cmd_txs.get(&game_id) {
                        let _ = tx.send(session::SessionEvent::Commands {
                            conn_id,
                            commands,
                        });
                    }
                }
            }

            ServerEvent::PlayerDisconnected { conn_id } => {
                info!(conn_id, "Player disconnected");
                matchmaker.dequeue(conn_id);
                writers.lock().await.remove(&conn_id);

                if let Some(game_id) = conn_to_game.remove(&conn_id) {
                    if let Some(tx) = game_cmd_txs.get(&game_id) {
                        let _ = tx.send(session::SessionEvent::Disconnected { conn_id });
                    }
                }
            }
        }
    }

    Ok(())
}
