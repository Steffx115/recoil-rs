//! Lightweight stack-based virtual machine for COB animation scripts.
//!
//! The VM executes animation bytecodes produced by the BOS compiler. Each
//! "thread" runs one script function concurrently. Threads can sleep, wait
//! for piece moves/turns to complete, and signal each other.

use crate::loader::CobScript;
use pierce_model::PieceTransform;

// ---------------------------------------------------------------------------
// COB opcodes
// ---------------------------------------------------------------------------

mod opcodes {
    pub const MOVE_PIECE_NOW: u32 = 0x10001;
    pub const TURN_PIECE_NOW: u32 = 0x10002;
    pub const SPIN_PIECE: u32 = 0x10003;
    pub const STOP_SPIN: u32 = 0x10004;
    pub const SHOW_PIECE: u32 = 0x10005;
    pub const HIDE_PIECE: u32 = 0x10006;
    pub const MOVE_PIECE_WITH_SPEED: u32 = 0x1000D;
    pub const TURN_PIECE_WITH_SPEED: u32 = 0x1000E;
    pub const WAIT_FOR_TURN: u32 = 0x1000F;
    pub const WAIT_FOR_MOVE: u32 = 0x10010;
    pub const SLEEP: u32 = 0x10011;
    pub const CREATE_LOCAL_VAR: u32 = 0x10012;
    pub const GET: u32 = 0x10013;
    pub const SET: u32 = 0x10014;
    pub const SIGNAL: u32 = 0x10017;
    pub const SET_SIGNAL_MASK: u32 = 0x10018;
    pub const EXPLODE: u32 = 0x10019;
    pub const EMIT_SFX: u32 = 0x1001A;
    pub const PUSH_CONSTANT: u32 = 0x10021;
    pub const PUSH_LOCAL_VAR: u32 = 0x10022;
    pub const PUSH_STATIC_VAR: u32 = 0x10023;
    pub const POP_LOCAL_VAR: u32 = 0x10024;
    pub const POP_STATIC_VAR: u32 = 0x10025;
    pub const ADD: u32 = 0x10026;
    pub const SUB: u32 = 0x10027;
    pub const MUL: u32 = 0x10028;
    pub const DIV: u32 = 0x10029;
    pub const LESS_THAN: u32 = 0x1002A;
    pub const LESS_OR_EQUAL: u32 = 0x1002B;
    pub const GREATER_THAN: u32 = 0x1002C;
    pub const GREATER_OR_EQUAL: u32 = 0x1002D;
    pub const EQUAL: u32 = 0x1002E;
    pub const NOT_EQUAL: u32 = 0x1002F;
    pub const AND: u32 = 0x10030;
    pub const OR: u32 = 0x10031;
    pub const XOR: u32 = 0x10032;
    pub const NOT: u32 = 0x10033;
    pub const RAND: u32 = 0x10041;
    pub const GET_UNIT_VALUE: u32 = 0x10042;
    #[allow(dead_code)]
    pub const GET_BUILD_PERCENT_LEFT: u32 = 0x10043;
    pub const JUMP: u32 = 0x10064;
    pub const JUMP_NOT_EQUAL: u32 = 0x10065;
    pub const RETURN: u32 = 0x10066;
    pub const CALL_SCRIPT: u32 = 0x10067;
}

// ---------------------------------------------------------------------------
// COB unit constants
// ---------------------------------------------------------------------------

/// COB angular units: 65536 = 360 degrees.
const COB_ANGULAR_UNIT: f32 = 65536.0 / (2.0 * std::f32::consts::PI);

/// COB linear units: 65536 = ~163840 S3O units? In Spring: 1 COB linear unit
/// = 1/65536 of an "elmo" (half a heightmap square). For our purposes we use
/// 65536 = 1.0 world unit as a reasonable approximation.
const COB_LINEAR_UNIT: f32 = 65536.0;

// ---------------------------------------------------------------------------
// Wait conditions
// ---------------------------------------------------------------------------

/// What a thread is waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitCondition {
    /// Waiting for a piece's turn on a specific axis to reach its target.
    Turn { piece: usize, axis: usize },
    /// Waiting for a piece's move on a specific axis to reach its target.
    Move { piece: usize, axis: usize },
}

// ---------------------------------------------------------------------------
// Per-piece animation state
// ---------------------------------------------------------------------------

/// Runtime animation state for a single piece.
#[derive(Debug, Clone)]
pub struct PieceAnimState {
    /// Current position offset (linear units converted to world units).
    pub current_pos: [f32; 3],
    /// Target position for interpolation.
    pub target_pos: [f32; 3],
    /// Speed of movement per tick on each axis.
    pub move_speed: [f32; 3],

    /// Current rotation in radians.
    pub current_rot: [f32; 3],
    /// Target rotation in radians.
    pub target_rot: [f32; 3],
    /// Turn speed per tick on each axis (radians).
    pub turn_speed: [f32; 3],
    /// Spin speed per tick on each axis (radians) — continuous rotation.
    pub spin_speed: [f32; 3],
    /// Spin acceleration per tick on each axis.
    pub spin_accel: [f32; 3],

    /// Whether this piece is visible.
    pub visible: bool,
}

impl Default for PieceAnimState {
    fn default() -> Self {
        Self {
            current_pos: [0.0; 3],
            target_pos: [0.0; 3],
            move_speed: [0.0; 3],
            current_rot: [0.0; 3],
            target_rot: [0.0; 3],
            turn_speed: [0.0; 3],
            spin_speed: [0.0; 3],
            spin_accel: [0.0; 3],
            visible: true,
        }
    }
}

impl PieceAnimState {
    /// Check if a move on the given axis has reached its target.
    fn move_done(&self, axis: usize) -> bool {
        (self.current_pos[axis] - self.target_pos[axis]).abs() < 1e-6
    }

    /// Check if a turn on the given axis has reached its target.
    fn turn_done(&self, axis: usize) -> bool {
        (self.current_rot[axis] - self.target_rot[axis]).abs() < 1e-6
    }
}

// ---------------------------------------------------------------------------
// COB thread
// ---------------------------------------------------------------------------

/// A single execution thread within the COB VM.
#[derive(Debug, Clone)]
pub struct CobThread {
    /// Index of the script function being executed.
    pub script_index: usize,
    /// Instruction pointer (index into the script's code array).
    pub ip: usize,
    /// Data stack.
    pub stack: Vec<i32>,
    /// Local variables.
    pub local_vars: Vec<i32>,
    /// Frames remaining in a SLEEP instruction.
    pub sleep_frames: u32,
    /// Signal mask for this thread.
    pub signal_mask: u32,
    /// Current wait condition (if any).
    pub wait_condition: Option<WaitCondition>,
    /// Call stack for CALL_SCRIPT: (script_index, return_ip, saved_locals).
    pub call_stack: Vec<(usize, usize, Vec<i32>)>,
    /// Whether this thread has finished executing.
    pub finished: bool,
}

// ---------------------------------------------------------------------------
// COB VM
// ---------------------------------------------------------------------------

/// The COB virtual machine. Owns threads and piece animation state.
pub struct CobVm {
    /// Active script threads.
    pub threads: Vec<CobThread>,
    /// Global (static) variables shared across all threads.
    pub static_vars: Vec<i32>,
    /// Per-piece animation state.
    pub pieces: Vec<PieceAnimState>,
    /// Deterministic "random" seed for RAND opcode.
    rand_seed: u32,
}

impl CobVm {
    /// Create a new VM for the given script.
    pub fn new(script: &CobScript) -> Self {
        Self {
            threads: Vec::new(),
            static_vars: vec![0; script.num_static_vars],
            pieces: (0..script.pieces.len())
                .map(|_| PieceAnimState::default())
                .collect(),
            rand_seed: 12345,
        }
    }

    /// Start a new thread for the named script function.
    ///
    /// Returns `true` if the script was found and a thread was started.
    pub fn call_script(&mut self, script: &CobScript, name: &str) -> bool {
        if let Some(idx) = script.scripts.iter().position(|s| s.name == name) {
            self.threads.push(CobThread {
                script_index: idx,
                ip: 0,
                stack: Vec::new(),
                local_vars: Vec::new(),
                sleep_frames: 0,
                signal_mask: 0,
                wait_condition: None,
                call_stack: Vec::new(),
                finished: false,
            });
            true
        } else {
            false
        }
    }

    /// Advance all threads by one frame.
    ///
    /// This interpolates piece positions/rotations toward their targets
    /// and executes bytecode until each thread blocks or finishes.
    pub fn tick(&mut self, script: &CobScript) {
        // 1. Interpolate piece animations.
        self.interpolate_pieces();

        // 2. Execute threads.
        let num_threads = self.threads.len();
        for i in 0..num_threads {
            if self.threads[i].finished {
                continue;
            }
            self.execute_thread(i, script);
        }

        // 3. Remove finished threads.
        self.threads.retain(|t| !t.finished);
    }

    /// Get the current piece transforms for rendering.
    pub fn get_piece_transforms(&self) -> Vec<PieceTransform> {
        self.pieces
            .iter()
            .map(|p| PieceTransform {
                translate: p.current_pos,
                rotate: p.current_rot,
            })
            .collect()
    }

    /// Deterministic pseudo-random number generator.
    fn next_rand(&mut self, max: i32) -> i32 {
        // Simple LCG.
        self.rand_seed = self.rand_seed.wrapping_mul(1103515245).wrapping_add(12345);
        if max <= 0 {
            0
        } else {
            ((self.rand_seed >> 16) as i32).abs() % max
        }
    }

    /// Interpolate all piece positions and rotations toward their targets.
    fn interpolate_pieces(&mut self) {
        for piece in &mut self.pieces {
            for axis in 0..3 {
                // Move interpolation.
                if piece.move_speed[axis] > 0.0 {
                    let diff = piece.target_pos[axis] - piece.current_pos[axis];
                    if diff.abs() <= piece.move_speed[axis] {
                        piece.current_pos[axis] = piece.target_pos[axis];
                        piece.move_speed[axis] = 0.0;
                    } else {
                        piece.current_pos[axis] += diff.signum() * piece.move_speed[axis];
                    }
                }

                // Turn interpolation.
                if piece.turn_speed[axis] > 0.0 {
                    let diff = piece.target_rot[axis] - piece.current_rot[axis];
                    if diff.abs() <= piece.turn_speed[axis] {
                        piece.current_rot[axis] = piece.target_rot[axis];
                        piece.turn_speed[axis] = 0.0;
                    } else {
                        piece.current_rot[axis] += diff.signum() * piece.turn_speed[axis];
                    }
                }

                // Spin (continuous rotation).
                if piece.spin_speed[axis].abs() > 1e-9 {
                    piece.current_rot[axis] += piece.spin_speed[axis];
                    // Apply acceleration toward target spin speed.
                    // (spin_accel is not used to reach a target; spin just keeps going)
                }
            }
        }
    }

    /// Execute one thread until it blocks or finishes.
    fn execute_thread(&mut self, thread_idx: usize, script: &CobScript) {
        // Handle sleep.
        if self.threads[thread_idx].sleep_frames > 0 {
            self.threads[thread_idx].sleep_frames -= 1;
            return;
        }

        // Handle wait conditions.
        if let Some(ref cond) = self.threads[thread_idx].wait_condition.clone() {
            let done = match cond {
                WaitCondition::Turn { piece, axis } => {
                    self.pieces.get(*piece).is_none_or(|ps| ps.turn_done(*axis))
                }
                WaitCondition::Move { piece, axis } => {
                    self.pieces.get(*piece).is_none_or(|ps| ps.move_done(*axis))
                }
            };
            if !done {
                return;
            }
            self.threads[thread_idx].wait_condition = None;
        }

        // Execute instructions (with a safety limit to prevent infinite loops).
        let max_instructions = 10_000;
        for _ in 0..max_instructions {
            let t = &self.threads[thread_idx];
            let code = &script.scripts[t.script_index].code;
            if t.ip >= code.len() {
                self.threads[thread_idx].finished = true;
                return;
            }

            let opcode = code[t.ip];
            match opcode {
                opcodes::PUSH_CONSTANT => {
                    let val = self.read_operand(thread_idx, script);
                    self.threads[thread_idx].stack.push(val as i32);
                }
                opcodes::PUSH_LOCAL_VAR => {
                    let idx = self.read_operand(thread_idx, script) as usize;
                    let val = self.threads[thread_idx]
                        .local_vars
                        .get(idx)
                        .copied()
                        .unwrap_or(0);
                    self.threads[thread_idx].stack.push(val);
                }
                opcodes::PUSH_STATIC_VAR => {
                    let idx = self.read_operand(thread_idx, script) as usize;
                    let val = self.static_vars.get(idx).copied().unwrap_or(0);
                    self.threads[thread_idx].stack.push(val);
                }
                opcodes::POP_LOCAL_VAR => {
                    let idx = self.read_operand(thread_idx, script) as usize;
                    let val = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if idx >= self.threads[thread_idx].local_vars.len() {
                        self.threads[thread_idx].local_vars.resize(idx + 1, 0);
                    }
                    self.threads[thread_idx].local_vars[idx] = val;
                }
                opcodes::POP_STATIC_VAR => {
                    let idx = self.read_operand(thread_idx, script) as usize;
                    let val = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if idx >= self.static_vars.len() {
                        self.static_vars.resize(idx + 1, 0);
                    }
                    self.static_vars[idx] = val;
                }
                opcodes::CREATE_LOCAL_VAR => {
                    self.threads[thread_idx].local_vars.push(0);
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::ADD => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a.wrapping_add(b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::SUB => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a.wrapping_sub(b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::MUL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a.wrapping_mul(b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::DIV => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let result = if b == 0 { 0 } else { a.wrapping_div(b) };
                    self.threads[thread_idx].stack.push(result);
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::LESS_THAN => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a < b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::LESS_OR_EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a <= b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::GREATER_THAN => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a > b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::GREATER_OR_EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a >= b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a == b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::NOT_EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a != b));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::AND => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx]
                        .stack
                        .push(i32::from(a != 0 && b != 0));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::OR => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx]
                        .stack
                        .push(i32::from(a != 0 || b != 0));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::XOR => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a ^ b);
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::NOT => {
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a == 0));
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::JUMP => {
                    let target = self.read_operand(thread_idx, script) as usize;
                    self.threads[thread_idx].ip = target;
                }
                opcodes::JUMP_NOT_EQUAL => {
                    let target = self.read_operand(thread_idx, script) as usize;
                    let val = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if val != 0 {
                        self.threads[thread_idx].ip = target;
                    }
                }
                opcodes::RETURN => {
                    if let Some((ret_script, ret_ip, saved_locals)) =
                        self.threads[thread_idx].call_stack.pop()
                    {
                        self.threads[thread_idx].script_index = ret_script;
                        self.threads[thread_idx].ip = ret_ip;
                        self.threads[thread_idx].local_vars = saved_locals;
                    } else {
                        self.threads[thread_idx].finished = true;
                        return;
                    }
                }
                opcodes::CALL_SCRIPT => {
                    let func_idx = self.read_operand(thread_idx, script) as usize;
                    let num_args = self.read_operand(thread_idx, script) as usize;

                    // Pop arguments from stack.
                    let mut args = Vec::with_capacity(num_args);
                    for _ in 0..num_args {
                        args.push(self.threads[thread_idx].stack.pop().unwrap_or(0));
                    }
                    args.reverse();

                    // Save current state on call stack.
                    let current_script = self.threads[thread_idx].script_index;
                    let current_ip = self.threads[thread_idx].ip;
                    let saved_locals = std::mem::take(&mut self.threads[thread_idx].local_vars);

                    self.threads[thread_idx].call_stack.push((
                        current_script,
                        current_ip,
                        saved_locals,
                    ));

                    // Jump to called function.
                    self.threads[thread_idx].script_index = func_idx;
                    self.threads[thread_idx].ip = 0;
                    self.threads[thread_idx].local_vars = args;
                }
                opcodes::SLEEP => {
                    let ms = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    // Convert milliseconds to frames (assume 30 fps).
                    let frames = (ms as u32).div_ceil(33);
                    self.threads[thread_idx].sleep_frames = frames;
                    self.threads[thread_idx].ip += 1;
                    return;
                }
                opcodes::MOVE_PIECE_NOW => {
                    let pos = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        let world_pos = pos as f32 / COB_LINEAR_UNIT;
                        if axis < 3 {
                            p.current_pos[axis] = world_pos;
                            p.target_pos[axis] = world_pos;
                            p.move_speed[axis] = 0.0;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::TURN_PIECE_NOW => {
                    let angle = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        let radians = angle as f32 / COB_ANGULAR_UNIT;
                        if axis < 3 {
                            p.current_rot[axis] = radians;
                            p.target_rot[axis] = radians;
                            p.turn_speed[axis] = 0.0;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::MOVE_PIECE_WITH_SPEED => {
                    // Spring: pop destination first (top), speed second
                    let target = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let speed = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.target_pos[axis] = target as f32 / COB_LINEAR_UNIT;
                            // Speed is in linear units per second; convert to per-frame.
                            p.move_speed[axis] = (speed as f32 / COB_LINEAR_UNIT).abs() / 30.0;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::TURN_PIECE_WITH_SPEED => {
                    // Spring: pop destination first, speed second
                    let speed = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let target = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.target_rot[axis] = target as f32 / COB_ANGULAR_UNIT;
                            // Speed in angular units per second -> radians per frame.
                            p.turn_speed[axis] = (speed as f32 / COB_ANGULAR_UNIT).abs() / 30.0;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::WAIT_FOR_TURN => {
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    self.threads[thread_idx].wait_condition =
                        Some(WaitCondition::Turn { piece, axis });
                    self.threads[thread_idx].ip += 1;
                    return;
                }
                opcodes::WAIT_FOR_MOVE => {
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    self.threads[thread_idx].wait_condition =
                        Some(WaitCondition::Move { piece, axis });
                    self.threads[thread_idx].ip += 1;
                    return;
                }
                opcodes::SPIN_PIECE => {
                    let accel = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let speed = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.spin_speed[axis] = speed as f32 / COB_ANGULAR_UNIT / 30.0;
                            p.spin_accel[axis] = accel as f32 / COB_ANGULAR_UNIT / 30.0;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::STOP_SPIN => {
                    let _decel = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let axis = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.spin_speed[axis] = 0.0;
                            p.spin_accel[axis] = 0.0;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::SHOW_PIECE => {
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        p.visible = true;
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::HIDE_PIECE => {
                    let piece = self.threads[thread_idx].stack.pop().unwrap_or(0) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        p.visible = false;
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::SIGNAL => {
                    let mask = self.threads[thread_idx].stack.pop().unwrap_or(0) as u32;
                    // Kill all other threads whose signal_mask overlaps.
                    for (j, t) in self.threads.iter_mut().enumerate() {
                        if j != thread_idx && (t.signal_mask & mask) != 0 {
                            t.finished = true;
                        }
                    }
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::SET_SIGNAL_MASK => {
                    let mask = self.threads[thread_idx].stack.pop().unwrap_or(0) as u32;
                    self.threads[thread_idx].signal_mask = mask;
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::RAND => {
                    let hi = self.threads[thread_idx].stack.pop().unwrap_or(1);
                    let lo = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let range = (hi - lo).max(1);
                    let val = lo + self.next_rand(range);
                    self.threads[thread_idx].stack.push(val);
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::GET | opcodes::GET_UNIT_VALUE => {
                    // GET: pop the type identifier, push a value.
                    // For now, return 0 for all unit state queries.
                    let _type_id = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(0);
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::SET => {
                    // SET: pop value and type, apply unit state change.
                    // Stub: just consume the values.
                    let _value = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let _type_id = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].ip += 1;
                }
                opcodes::EXPLODE | opcodes::EMIT_SFX => {
                    // Consume arguments, no-op for now.
                    let _arg2 = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let _arg1 = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].ip += 1;
                }
                _ => {
                    // Unknown opcode — skip it.
                    tracing::warn!("Unknown COB opcode: 0x{:X}", opcode);
                    self.threads[thread_idx].ip += 1;
                }
            }
        }

        // Safety limit reached — thread is stuck.
        tracing::warn!("COB thread exceeded instruction limit, forcing finish");
        self.threads[thread_idx].finished = true;
    }

    /// Read the next word from the current thread's code as an operand,
    /// advancing the IP past both the opcode and the operand.
    fn read_operand(&mut self, thread_idx: usize, script: &CobScript) -> u32 {
        let t = &mut self.threads[thread_idx];
        t.ip += 1; // skip opcode
        let code = &script.scripts[t.script_index].code;
        let val = code.get(t.ip).copied().unwrap_or(0);
        t.ip += 1; // skip operand
        val
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        // push piece=0, push axis=1 (y), push position=65536 (1.0 world unit), MOVE_PIECE_NOW
        let code = vec![
            opcodes::PUSH_CONSTANT,
            0,
            opcodes::PUSH_CONSTANT,
            1,
            opcodes::PUSH_CONSTANT,
            65536,
            opcodes::MOVE_PIECE_NOW,
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
        // Turn piece 0, axis 0 (heading), angle = 16384 (90 degrees)
        // 16384 / (65536 / (2*PI)) = 16384 / 10430.38 ~= PI/2
        let code = vec![
            opcodes::PUSH_CONSTANT,
            0,
            opcodes::PUSH_CONSTANT,
            0,
            opcodes::PUSH_CONSTANT,
            16384,
            opcodes::TURN_PIECE_NOW,
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
            opcodes::PUSH_CONSTANT,
            0, // piece 0
            opcodes::HIDE_PIECE,
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
        // push 0, jump_not_equal to after-store, push 42, pop static[0], return
        // Since 0 is falsy, JNE should NOT jump; we should store 42.
        let code = vec![
            opcodes::PUSH_CONSTANT,
            0, // push 0 (false)
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
        let code = vec![
            opcodes::PUSH_CONSTANT,
            0, // piece
            opcodes::PUSH_CONSTANT,
            1, // axis Y
            opcodes::PUSH_CONSTANT,
            65536, // target = 1.0
            opcodes::PUSH_CONSTANT,
            65536, // speed = 1.0/s
            opcodes::MOVE_PIECE_WITH_SPEED,
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
}
