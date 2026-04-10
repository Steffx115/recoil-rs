//! Queue-based matchmaking. Players enqueue and get grouped into games
//! when enough players are waiting.

use super::ConnId;

/// Simple FIFO matchmaking queue.
pub struct Matchmaker {
    queue: Vec<ConnId>,
    players_per_game: u8,
}

impl Matchmaker {
    pub fn new(players_per_game: u8) -> Self {
        Self {
            queue: Vec::new(),
            players_per_game,
        }
    }

    /// Add a player to the queue. Returns the matched group if the queue
    /// now has enough players to start a game.
    pub fn enqueue(&mut self, conn_id: ConnId) -> Option<Vec<ConnId>> {
        self.queue.push(conn_id);

        if self.queue.len() >= self.players_per_game as usize {
            let players: Vec<ConnId> = self
                .queue
                .drain(..self.players_per_game as usize)
                .collect();
            Some(players)
        } else {
            None
        }
    }

    /// Remove a player from the queue (e.g. on disconnect before match).
    pub fn dequeue(&mut self, conn_id: ConnId) {
        self.queue.retain(|&id| id != conn_id);
    }

    /// Number of players currently waiting.
    pub fn queue_size(&self) -> usize {
        self.queue.len()
    }
}
