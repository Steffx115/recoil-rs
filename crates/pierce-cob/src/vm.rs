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

/// Spring COB opcodes — raw 32-bit values as they appear in compiled COB files.
///
/// The opcode IS the full word (no bit-shifting needed). Some opcodes
/// encode a sub-type in the lower bits (e.g., PUSH_CONSTANT=0x10021001,
/// PUSH_LOCAL=0x10021002, PUSH_STATIC=0x10021004).
#[allow(dead_code)]
mod opcodes {
    // --- Model piece manipulation ---
    pub const MOVE_PIECE_WITH_SPEED: u32 = 0x10001000; // inline: piece, axis; stack: target, speed
    pub const TURN_PIECE_WITH_SPEED: u32 = 0x10002000; // inline: piece, axis; stack: target, speed
    pub const SPIN_PIECE: u32 = 0x10003000;             // inline: piece, axis; stack: speed, accel
    pub const STOP_SPIN: u32 = 0x10004000;              // inline: piece, axis; stack: decel
    pub const SHOW_PIECE: u32 = 0x10005000;             // inline: piece
    pub const HIDE_PIECE: u32 = 0x10006000;             // inline: piece
    pub const CACHE: u32 = 0x10007000;                  // inline: piece (no-op)
    pub const DONT_CACHE: u32 = 0x10008000;             // inline: piece (no-op)
    pub const DONT_SHADOW: u32 = 0x10009000;            // inline: piece (no-op)
    pub const DONT_SHADE: u32 = 0x1000A000;             // inline: piece (no-op)
    pub const MOVE_PIECE_NOW: u32 = 0x1000B000;         // inline: piece, axis; stack: position
    pub const TURN_PIECE_NOW: u32 = 0x1000C000;         // inline: piece, axis; stack: angle
    pub const WAIT_FOR_TURN: u32 = 0x10011000;          // inline: piece, axis
    pub const WAIT_FOR_MOVE: u32 = 0x10012000;          // inline: piece, axis
    pub const SLEEP: u32 = 0x10013000;                  // stack: milliseconds
    pub const EMIT_SFX: u32 = 0x1000F000;               // inline: piece; stack: sfx_type
    pub const EXPLODE: u32 = 0x10010000;                // inline: piece; stack: explosion type

    // --- Stack & variables ---
    pub const PUSH_CONSTANT: u32 = 0x10021001;          // inline: value
    pub const PUSH_LOCAL_VAR: u32 = 0x10021002;         // inline: var_index
    pub const PUSH_STATIC_VAR: u32 = 0x10021004;        // inline: var_index
    pub const POP_LOCAL_VAR: u32 = 0x10023002;          // inline: var_index; stack: value
    pub const POP_STATIC_VAR: u32 = 0x10023004;         // inline: var_index; stack: value
    pub const CREATE_LOCAL_VAR: u32 = 0x10022000;       // no operands

    // --- Arithmetic & logic (all stack-only) ---
    pub const ADD: u32 = 0x10031000;
    pub const SUB: u32 = 0x10032000;
    pub const MUL: u32 = 0x10033000;
    pub const DIV: u32 = 0x10034000;
    pub const BITWISE_AND: u32 = 0x10035000;
    pub const BITWISE_OR: u32 = 0x10036000;
    pub const RAND: u32 = 0x10041000;
    pub const GET_UNIT_VALUE: u32 = 0x10042000;         // stack: type_id; pushes value
    pub const GET: u32 = 0x10043000;                    // stack: type_id, piece_num; pushes value

    // --- Comparison (all stack-only, push 0 or 1) ---
    pub const LESS_THAN: u32 = 0x10051000;
    pub const LESS_OR_EQUAL: u32 = 0x10052000;
    pub const GREATER_THAN: u32 = 0x10053000;
    pub const GREATER_OR_EQUAL: u32 = 0x10054000;
    pub const EQUAL: u32 = 0x10055000;
    pub const NOT_EQUAL: u32 = 0x10056000;
    pub const AND: u32 = 0x10057000;
    pub const OR: u32 = 0x10058000;
    pub const XOR: u32 = 0x10059000;
    pub const NOT: u32 = 0x1005A000;

    // --- Flow control ---
    pub const START_SCRIPT: u32 = 0x10061000;           // inline: script_idx, num_args
    pub const CALL_SCRIPT: u32 = 0x10062001;            // inline: script_idx, num_args
    pub const CALL_SCRIPT_ALT: u32 = 0x10062000;       // alternative encoding
    pub const JUMP: u32 = 0x10064000;                   // inline: target_word
    pub const RETURN: u32 = 0x10065000;                 // no operands
    pub const JUMP_NOT_EQUAL: u32 = 0x10066000;         // inline: target_word; stack: value

    // --- Signals ---
    pub const SIGNAL: u32 = 0x10067000;                 // stack: mask
    pub const SET_SIGNAL_MASK: u32 = 0x10068000;        // stack: mask

    // --- Unit state ---
    pub const SET: u32 = 0x10082000;                    // stack: value, type_id
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
            // Advance past the opcode word. read_operand will advance further.
            self.threads[thread_idx].ip += 1;
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
                }
                opcodes::ADD => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a.wrapping_add(b));
                }
                opcodes::SUB => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a.wrapping_sub(b));
                }
                opcodes::MUL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a.wrapping_mul(b));
                }
                opcodes::DIV => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let result = if b == 0 { 0 } else { a.wrapping_div(b) };
                    self.threads[thread_idx].stack.push(result);
                }
                opcodes::LESS_THAN => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a < b));
                }
                opcodes::LESS_OR_EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a <= b));
                }
                opcodes::GREATER_THAN => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a > b));
                }
                opcodes::GREATER_OR_EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a >= b));
                }
                opcodes::EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a == b));
                }
                opcodes::NOT_EQUAL => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a != b));
                }
                opcodes::AND => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx]
                        .stack
                        .push(i32::from(a != 0 && b != 0));
                }
                opcodes::OR => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx]
                        .stack
                        .push(i32::from(a != 0 || b != 0));
                }
                opcodes::XOR => {
                    let b = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(a ^ b);
                }
                opcodes::NOT => {
                    let a = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(i32::from(a == 0));
                }
                opcodes::JUMP => {
                    let target = self.read_operand(thread_idx, script) as usize;
                    self.threads[thread_idx].ip = target;
                }
                opcodes::JUMP_NOT_EQUAL => {
                    // Spring convention: jump if top-of-stack IS zero.
                    // ("not equal" refers to the condition not being met.)
                    let target = self.read_operand(thread_idx, script) as usize;
                    let val = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if val == 0 {
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
                opcodes::CALL_SCRIPT | opcodes::CALL_SCRIPT_ALT => {
                    let func_idx = self.read_operand(thread_idx, script) as usize;
                    let num_args = self.read_operand(thread_idx, script) as usize;

                    if func_idx >= script.scripts.len() {
                        // Invalid script index — skip.
                        continue;
                    }

                    // Pop arguments from stack.
                    let mut args = Vec::with_capacity(num_args.min(64));
                    for _ in 0..num_args.min(64) {
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
                opcodes::START_SCRIPT => {
                    let func_idx = self.read_operand(thread_idx, script) as usize;
                    let num_args = self.read_operand(thread_idx, script) as usize;

                    // Pop arguments from stack.
                    let mut args = Vec::with_capacity(num_args);
                    for _ in 0..num_args {
                        args.push(self.threads[thread_idx].stack.pop().unwrap_or(0));
                    }
                    args.reverse();

                    // Launch a new thread (unlike CALL_SCRIPT which is synchronous).
                    if func_idx < script.scripts.len() {
                        self.threads.push(CobThread {
                            script_index: func_idx,
                            ip: 0,
                            stack: Vec::new(),
                            local_vars: args,
                            sleep_frames: 0,
                            signal_mask: 0,
                            wait_condition: None,
                            call_stack: Vec::new(),
                            finished: false,
                        });
                    }
                }
                opcodes::SLEEP => {
                    let ms = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    // Convert milliseconds to frames (assume 30 fps).
                    let frames = (ms as u32).div_ceil(33);
                    self.threads[thread_idx].sleep_frames = frames;
                    return;
                }
                opcodes::MOVE_PIECE_NOW => {
                    // Inline: piece, axis. Stack: position.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    let pos = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if let Some(p) = self.pieces.get_mut(piece) {
                        let world_pos = pos as f32 / COB_LINEAR_UNIT;
                        if axis < 3 {
                            p.current_pos[axis] = world_pos;
                            p.target_pos[axis] = world_pos;
                            p.move_speed[axis] = 0.0;
                        }
                    }
                }
                opcodes::TURN_PIECE_NOW => {
                    // Inline: piece, axis. Stack: angle.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    let angle = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if let Some(p) = self.pieces.get_mut(piece) {
                        let radians = angle as f32 / COB_ANGULAR_UNIT;
                        if axis < 3 {
                            p.current_rot[axis] = radians;
                            p.target_rot[axis] = radians;
                            p.turn_speed[axis] = 0.0;
                        }
                    }
                }
                opcodes::MOVE_PIECE_WITH_SPEED => {
                    // Spring: r4=Pop (destination), r3=Pop (speed), then Move(piece, axis, r3, r4)
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    let target = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let speed = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.target_pos[axis] = target as f32 / COB_LINEAR_UNIT;
                            p.move_speed[axis] = (speed as f32 / COB_LINEAR_UNIT).abs() / 30.0;
                        }
                    }
                }
                opcodes::TURN_PIECE_WITH_SPEED => {
                    // Spring: r2=Pop (destination), r1=Pop (speed), then Turn(piece, axis, r1, r2)
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    let target = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let speed = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.target_rot[axis] = target as f32 / COB_ANGULAR_UNIT;
                            // Speed in angular units per second -> radians per frame.
                            p.turn_speed[axis] = (speed as f32 / COB_ANGULAR_UNIT).abs() / 30.0;
                        }
                    }
                }
                opcodes::WAIT_FOR_TURN => {
                    // Inline: piece, axis.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    self.threads[thread_idx].wait_condition =
                        Some(WaitCondition::Turn { piece, axis });
                    return;
                }
                opcodes::WAIT_FOR_MOVE => {
                    // Inline: piece, axis.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    self.threads[thread_idx].wait_condition =
                        Some(WaitCondition::Move { piece, axis });
                    return;
                }
                opcodes::SPIN_PIECE => {
                    // Inline: piece, axis. Stack: speed, accel.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    let speed = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let accel = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.spin_speed[axis] = speed as f32 / COB_ANGULAR_UNIT / 30.0;
                            p.spin_accel[axis] = accel as f32 / COB_ANGULAR_UNIT / 30.0;
                        }
                    }
                }
                opcodes::STOP_SPIN => {
                    // Inline: piece, axis. Stack: decel.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let axis = self.read_operand(thread_idx, script) as usize;
                    let _decel = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    if let Some(p) = self.pieces.get_mut(piece) {
                        if axis < 3 {
                            p.spin_speed[axis] = 0.0;
                            p.spin_accel[axis] = 0.0;
                        }
                    }
                }
                opcodes::SHOW_PIECE => {
                    // Inline: piece.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        p.visible = true;
                    }
                }
                opcodes::HIDE_PIECE => {
                    // Inline: piece.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    if let Some(p) = self.pieces.get_mut(piece) {
                        p.visible = false;
                    }
                }
                opcodes::SIGNAL => {
                    let mask = self.threads[thread_idx].stack.pop().unwrap_or(0) as u32;
                    for (j, t) in self.threads.iter_mut().enumerate() {
                        if j != thread_idx && (t.signal_mask & mask) != 0 {
                            t.finished = true;
                        }
                    }
                }
                opcodes::SET_SIGNAL_MASK => {
                    let mask = self.threads[thread_idx].stack.pop().unwrap_or(0) as u32;
                    self.threads[thread_idx].signal_mask = mask;
                }
                opcodes::RAND => {
                    let hi = self.threads[thread_idx].stack.pop().unwrap_or(1);
                    let lo = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let range = (hi - lo).max(1);
                    let val = lo + self.next_rand(range);
                    self.threads[thread_idx].stack.push(val);
                }
                opcodes::GET | opcodes::GET_UNIT_VALUE => {
                    let _type_id = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    self.threads[thread_idx].stack.push(0);
                }
                opcodes::SET => {
                    let _value = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let _type_id = self.threads[thread_idx].stack.pop().unwrap_or(0);
                }
                opcodes::EXPLODE => {
                    // Inline: piece. Stack: explosion type.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let _exp_type = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let _ = piece; // no-op for now
                }
                opcodes::EMIT_SFX => {
                    // Inline: piece. Stack: sfx type.
                    let piece = self.read_operand(thread_idx, script) as usize;
                    let _sfx_type = self.threads[thread_idx].stack.pop().unwrap_or(0);
                    let _ = piece; // no-op for now
                }
                opcodes::CACHE | opcodes::DONT_CACHE | opcodes::DONT_SHADOW | opcodes::DONT_SHADE => {
                    // No-op with 1 inline operand (piece).
                    let _piece = self.read_operand(thread_idx, script);
                }
                _ => {
                    // Unknown opcode — skip it.
                    tracing::warn!("Unknown COB opcode: 0x{:08X}", opcode);
                }
            }
        }

        // Safety limit reached — thread is stuck.
        tracing::warn!("COB thread exceeded instruction limit, forcing finish");
        self.threads[thread_idx].finished = true;
    }

    /// Read the next word from the current thread's code as an operand,
    /// advancing the IP by 1. The caller must have already advanced past
    /// the opcode word (ip should point to the first operand).
    fn read_operand(&mut self, thread_idx: usize, script: &CobScript) -> u32 {
        let t = &mut self.threads[thread_idx];
        let code = &script.scripts[t.script_index].code;
        let val = code.get(t.ip).copied().unwrap_or(0);
        t.ip += 1;
        val
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/vm_tests.rs"]
mod tests;
