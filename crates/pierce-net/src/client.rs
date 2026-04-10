//! Client-side frame buffer and adaptive lag tracking.
//!
//! Buffers incoming [`FrameAdvance`] messages from the server and feeds
//! them to the local simulation one at a time. Tracks how far behind the
//! client is and exposes an [`AdaptLevel`] that the game can use to
//! dynamically reduce visual quality or simulation detail.

use std::collections::VecDeque;

use crate::protocol::{CommandFrame, NetMessage};

/// How stressed the client is — the game maps this to quality settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdaptLevel {
    /// Keeping up with the server. No adaptation needed.
    Normal,
    /// Slightly behind (e.g. 3-10 frames). Reduce non-essential visuals.
    Reduced,
    /// Significantly behind (e.g. 10-30 frames). Aggressive quality cuts.
    Minimal,
    /// Critically behind (e.g. 30+ frames). Skip rendering, fast-forward
    /// multiple sim ticks per render frame to catch up.
    CatchUp,
}

/// Thresholds (in buffered frames) for each [`AdaptLevel`].
#[derive(Debug, Clone)]
pub struct AdaptThresholds {
    pub reduced: usize,
    pub minimal: usize,
    pub catch_up: usize,
}

impl Default for AdaptThresholds {
    fn default() -> Self {
        Self {
            reduced: 3,
            minimal: 10,
            catch_up: 30,
        }
    }
}

/// Client-side frame buffer.
///
/// Incoming `FrameAdvance` messages are pushed in; the game pulls them
/// out one (or more) at a time via [`next_frame`].
pub struct ClientFrameBuffer {
    /// Queued frames waiting to be processed.
    queue: VecDeque<(u64, Vec<CommandFrame>)>,
    /// Last frame number processed by the client.
    last_processed: Option<u64>,
    /// Adaptation thresholds.
    thresholds: AdaptThresholds,
}

impl ClientFrameBuffer {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            last_processed: None,
            thresholds: AdaptThresholds::default(),
        }
    }

    pub fn with_thresholds(thresholds: AdaptThresholds) -> Self {
        Self {
            queue: VecDeque::new(),
            last_processed: None,
            thresholds,
        }
    }

    /// Push a received `FrameAdvance` message into the buffer.
    /// Call this whenever a message arrives from the server.
    pub fn push(&mut self, msg: &NetMessage) {
        if let NetMessage::FrameAdvance { frame, commands } = msg {
            self.queue.push_back((*frame, commands.clone()));
        }
    }

    /// Push a frame directly (frame number + commands).
    pub fn push_frame(&mut self, frame: u64, commands: Vec<CommandFrame>) {
        self.queue.push_back((frame, commands));
    }

    /// Pull the next frame to process. Returns `None` if the buffer is
    /// empty (client is waiting for the server).
    pub fn next_frame(&mut self) -> Option<(u64, Vec<CommandFrame>)> {
        let item = self.queue.pop_front()?;
        self.last_processed = Some(item.0);
        Some(item)
    }

    /// Number of frames buffered and waiting to be processed.
    pub fn buffered_frames(&self) -> usize {
        self.queue.len()
    }

    /// The last frame number successfully processed, if any.
    pub fn last_processed_frame(&self) -> Option<u64> {
        self.last_processed
    }

    /// Current adaptation level based on buffer depth.
    pub fn adapt_level(&self) -> AdaptLevel {
        let depth = self.queue.len();
        if depth >= self.thresholds.catch_up {
            AdaptLevel::CatchUp
        } else if depth >= self.thresholds.minimal {
            AdaptLevel::Minimal
        } else if depth >= self.thresholds.reduced {
            AdaptLevel::Reduced
        } else {
            AdaptLevel::Normal
        }
    }

    /// Suggested number of sim ticks to run this render frame.
    ///
    /// At `Normal`/`Reduced`/`Minimal`, returns 1 (process one tick per
    /// render frame). At `CatchUp`, returns up to `max_catch_up` to
    /// fast-forward through the backlog without rendering every frame.
    pub fn ticks_this_frame(&self, max_catch_up: usize) -> usize {
        match self.adapt_level() {
            AdaptLevel::Normal | AdaptLevel::Reduced | AdaptLevel::Minimal => 1,
            AdaptLevel::CatchUp => self.queue.len().min(max_catch_up),
        }
    }
}

impl Default for ClientFrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame_advance(frame: u64) -> NetMessage {
        NetMessage::FrameAdvance {
            frame,
            commands: vec![CommandFrame {
                frame,
                player_id: 0,
                commands: vec![],
            }],
        }
    }

    #[test]
    fn empty_buffer_returns_none() {
        let mut buf = ClientFrameBuffer::new();
        assert!(buf.next_frame().is_none());
        assert_eq!(buf.buffered_frames(), 0);
        assert_eq!(buf.adapt_level(), AdaptLevel::Normal);
    }

    #[test]
    fn push_and_pull() {
        let mut buf = ClientFrameBuffer::new();
        buf.push(&make_frame_advance(0));
        buf.push(&make_frame_advance(1));

        assert_eq!(buf.buffered_frames(), 2);

        let (frame, _) = buf.next_frame().unwrap();
        assert_eq!(frame, 0);
        assert_eq!(buf.last_processed_frame(), Some(0));

        let (frame, _) = buf.next_frame().unwrap();
        assert_eq!(frame, 1);
        assert!(buf.next_frame().is_none());
    }

    #[test]
    fn adapt_level_thresholds() {
        let mut buf = ClientFrameBuffer::new();

        // Normal: 0 frames.
        assert_eq!(buf.adapt_level(), AdaptLevel::Normal);

        // Still normal: 2 frames (< 3).
        for i in 0..2 {
            buf.push(&make_frame_advance(i));
        }
        assert_eq!(buf.adapt_level(), AdaptLevel::Normal);

        // Reduced: 3 frames.
        buf.push(&make_frame_advance(2));
        assert_eq!(buf.adapt_level(), AdaptLevel::Reduced);

        // Minimal: 10 frames.
        for i in 3..10 {
            buf.push(&make_frame_advance(i));
        }
        assert_eq!(buf.adapt_level(), AdaptLevel::Minimal);

        // CatchUp: 30 frames.
        for i in 10..30 {
            buf.push(&make_frame_advance(i));
        }
        assert_eq!(buf.adapt_level(), AdaptLevel::CatchUp);
    }

    #[test]
    fn ticks_this_frame_normal() {
        let mut buf = ClientFrameBuffer::new();
        buf.push(&make_frame_advance(0));
        assert_eq!(buf.ticks_this_frame(10), 1);
    }

    #[test]
    fn ticks_this_frame_catch_up() {
        let mut buf = ClientFrameBuffer::new();
        for i in 0..50 {
            buf.push(&make_frame_advance(i));
        }
        assert_eq!(buf.adapt_level(), AdaptLevel::CatchUp);
        assert_eq!(buf.ticks_this_frame(5), 5); // capped at max_catch_up
        assert_eq!(buf.ticks_this_frame(100), 50); // capped at queue size
    }

    #[test]
    fn ignores_non_frame_advance_messages() {
        let mut buf = ClientFrameBuffer::new();
        buf.push(&NetMessage::Hello {
            player_id: 0,
            game_id: 1,
        });
        assert_eq!(buf.buffered_frames(), 0);
    }

    #[test]
    fn processing_reduces_adapt_level() {
        let mut buf = ClientFrameBuffer::new();
        for i in 0..30 {
            buf.push(&make_frame_advance(i));
        }
        assert_eq!(buf.adapt_level(), AdaptLevel::CatchUp);

        // Process until below CatchUp threshold.
        for _ in 0..21 {
            buf.next_frame();
        }
        assert_eq!(buf.buffered_frames(), 9);
        assert_eq!(buf.adapt_level(), AdaptLevel::Reduced);
    }
}
