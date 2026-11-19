//! The bytecode, its semantics (shared by the interpreter and the JIT) and
//! the verifier.
//!
//! Values are `i64`. Arithmetic wraps. `Div`/`Rem` by zero yield 0;
//! `i64::MIN / -1` yields `i64::MIN` and `i64::MIN % -1` yields 0. Shift
//! counts are masked to 6 bits. `Shr` is arithmetic. Every executed `Jmp` or
//! `Jz` consumes one unit of fuel *before* it runs; with no fuel left the
//! program stops with [`Trap::OutOfFuel`].

use std::fmt;

/// Number of `i64` arguments a program receives.
pub const ARGS: usize = 4;
/// Maximum operand stack depth accepted by the verifier.
pub const MAX_DEPTH: usize = 256;

/// One instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insn {
    /// Push a constant.
    Push(i64),
    /// Push argument `n` (`n < ARGS`).
    Arg(u8),
    /// `a + b`.
    Add,
    /// `a - b`.
    Sub,
    /// `a * b`.
    Mul,
    /// `a / b` (see module docs for edge cases).
    Div,
    /// `a % b`.
    Rem,
    /// Bitwise and.
    And,
    /// Bitwise or.
    Or,
    /// Bitwise xor.
    Xor,
    /// `a << (b & 63)`.
    Shl,
    /// `a >> (b & 63)`, arithmetic.
    Shr,
    /// `1` if `a == b` else `0`.
    Eq,
    /// `1` if `a < b` (signed) else `0`.
    Lt,
    /// `-a`.
    Neg,
    /// `!a`.
    Not,
    /// Duplicate top.
    Dup,
    /// Swap the two top values.
    Swap,
    /// Discard top.
    Drop,
    /// Copy the second value to the top.
    Over,
    /// Unconditional jump to an instruction index.
    Jmp(u32),
    /// Pop; jump if it was zero.
    Jz(u32),
    /// Return the top value.
    Ret,
}

impl Insn {
    /// (values popped, values pushed).
    pub fn effect(self) -> (usize, usize) {
        use Insn::*;
        match self {
            Push(_) | Arg(_) => (0, 1),
            Add | Sub | Mul | Div | Rem | And | Or | Xor | Shl | Shr | Eq | Lt => (2, 1),
            Neg | Not => (1, 1),
            Dup => (1, 2),
            Swap => (2, 2),
            Drop => (1, 0),
            Over => (2, 3),
            Jmp(_) => (0, 0),
            Jz(_) => (1, 0),
            Ret => (1, 0),
        }
    }
}

/// Binary operation semantics, shared by interpreter and tests.
pub fn binop(op: Insn, a: i64, b: i64) -> i64 {
    use Insn::*;
    match op {
        Add => a.wrapping_add(b),
        Sub => a.wrapping_sub(b),
        Mul => a.wrapping_mul(b),
        Div => {
            if b == 0 {
                0
            } else {
                a.wrapping_div(b)
            }
        }
        Rem => {
            if b == 0 {
                0
            } else {
                a.wrapping_rem(b)
            }
        }
        And => a & b,
        Or => a | b,
        Xor => a ^ b,
        Shl => a.wrapping_shl((b & 63) as u32),
        Shr => a.wrapping_shr((b & 63) as u32),
        Eq => (a == b) as i64,
        Lt => (a < b) as i64,
        _ => unreachable!("not a binary op: {op:?}"),
    }
}

/// Why execution stopped without a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// Fuel exhausted at a jump.
    OutOfFuel,
}

/// Verifier errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// Empty program.
    Empty,
    /// Stack underflow at `pc`.
    Underflow(usize),
    /// Stack deeper than [`MAX_DEPTH`] at `pc`.
    TooDeep(usize),
    /// Two paths reach `pc` with different depths.
    DepthMismatch(usize),
    /// Jump target out of range at `pc`.
    BadTarget(usize),
    /// Argument index out of range at `pc`.
    BadArg(usize),
    /// Execution can run past the last instruction.
    FallsOffEnd,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for VerifyError {}

/// A program that passed the verifier, with the stack depth before every
/// reachable instruction (`None` = unreachable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// Instructions.
    pub code: Vec<Insn>,
    /// Depth on entry to each instruction.
    pub depth: Vec<Option<u16>>,
    /// Maximum depth reached.
    pub max_depth: usize,
}

/// Verify `code`: static stack depths, consistent at join points, in-range
/// jumps and arguments, no fall-through past the end.
pub fn verify(code: &[Insn]) -> Result<Verified, VerifyError> {
    if code.is_empty() {
        return Err(VerifyError::Empty);
    }
    let n = code.len();
    let mut depth: Vec<Option<u16>> = vec![None; n];
    let mut work = vec![(0usize, 0usize)];
    let mut max_depth = 0;
    while let Some((pc, d)) = work.pop() {
        if pc >= n {
            return Err(VerifyError::FallsOffEnd);
        }
        match depth[pc] {
            Some(x) if x as usize == d => continue,
            Some(_) => return Err(VerifyError::DepthMismatch(pc)),
            None => depth[pc] = Some(d as u16),
        }
        let insn = code[pc];
        let (pop, push) = insn.effect();
        if d < pop {
            return Err(VerifyError::Underflow(pc));
        }
        let nd = d - pop + push;
        if nd > MAX_DEPTH {
            return Err(VerifyError::TooDeep(pc));
        }
        max_depth = max_depth.max(nd).max(d);
        match insn {
            Insn::Arg(a) if a as usize >= ARGS => return Err(VerifyError::BadArg(pc)),
            Insn::Ret => {}
            Insn::Jmp(t) => {
                if t as usize >= n {
                    return Err(VerifyError::BadTarget(pc));
                }
                work.push((t as usize, nd));
            }
            Insn::Jz(t) => {
                if t as usize >= n {
                    return Err(VerifyError::BadTarget(pc));
                }
                work.push((t as usize, nd));
                work.push((pc + 1, nd));
            }
            _ => work.push((pc + 1, nd)),
        }
    }
    Ok(Verified {
        code: code.to_vec(),
        depth,
        max_depth,
    })
}

/// Decode arbitrary bytes into instructions (used by the fuzzer). Never
/// fails; the result still has to pass [`verify`].
pub fn decode(bytes: &[u8]) -> Vec<Insn> {
    use Insn::*;
    let mut out = Vec::new();
    let mut i = 0;
    let take = |n: usize, i: &mut usize| -> Option<Vec<u8>> {
        let s = bytes.get(*i..*i + n)?.to_vec();
        *i += n;
        Some(s)
    };
    while i < bytes.len() {
        let op = bytes[i] % 23;
        i += 1;
        let insn = match op {
            0 => match take(8, &mut i) {
                Some(b) => Push(i64::from_le_bytes(b.try_into().unwrap())),
                None => break,
            },
            1 => match take(1, &mut i) {
                Some(b) => Arg(b[0] % ARGS as u8),
                None => break,
            },
            2 => Add,
            3 => Sub,
            4 => Mul,
            5 => Div,
            6 => Rem,
            7 => And,
            8 => Or,
            9 => Xor,
            10 => Shl,
            11 => Shr,
            12 => Eq,
            13 => Lt,
            14 => Neg,
            15 => Not,
            16 => Dup,
            17 => Swap,
            18 => Drop,
            19 => Over,
            20 | 21 => match take(1, &mut i) {
                Some(b) => {
                    let t = b[0] as u32;
                    if op == 20 {
                        Jmp(t)
                    } else {
                        Jz(t)
                    }
                }
                None => break,
            },
            _ => Ret,
        };
        out.push(insn);
    }
    // Retarget jumps into range so more inputs verify.
    let n = out.len().max(1) as u32;
    for insn in out.iter_mut() {
        match insn {
            Jmp(t) | Jz(t) => *t %= n,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Insn::*;

    #[test]
    fn edge_semantics() {
        assert_eq!(binop(Div, 7, 0), 0);
        assert_eq!(binop(Rem, 7, 0), 0);
        assert_eq!(binop(Div, i64::MIN, -1), i64::MIN);
        assert_eq!(binop(Rem, i64::MIN, -1), 0);
        assert_eq!(binop(Div, -7, 2), -3);
        assert_eq!(binop(Rem, -7, 2), -1);
        assert_eq!(binop(Shl, 1, 65), 2);
        assert_eq!(binop(Shr, -8, 1), -4);
        assert_eq!(binop(Lt, -1, 0), 1);
        assert_eq!(binop(Add, i64::MAX, 1), i64::MIN);
    }

    #[test]
    fn verifier_accepts_loops_and_rejects_bad_programs() {
        // sum = 0; i = arg0; while i != 0 { sum += i; i -= 1 } return sum
        let truncated = [
            Push(0),
            Arg(0),
            Dup,
            Jz(11),
            Swap,
            Over,
            Add,
            Swap,
            Push(1),
            Sub,
            Jmp(2),
        ];
        assert_eq!(verify(&truncated), Err(VerifyError::BadTarget(3)));
        let ok = [
            Push(0),
            Arg(0),
            Dup,
            Jz(11),
            Swap,
            Over,
            Add,
            Swap,
            Push(1),
            Sub,
            Jmp(2),
            Drop,
            Ret,
        ];
        let v = verify(&ok).unwrap();
        assert_eq!(v.depth[2], Some(2));
        assert_eq!(v.depth[11], Some(2));
        assert_eq!(v.max_depth, 3);
        assert_eq!(verify(&[Add, Ret]), Err(VerifyError::Underflow(0)));
        assert_eq!(verify(&[Push(1)]), Err(VerifyError::FallsOffEnd));
        assert_eq!(verify(&[Arg(9), Ret]), Err(VerifyError::BadArg(0)));
        // Loop that grows the stack: depth mismatch at the join.
        assert_eq!(
            verify(&[Push(1), Jmp(0)]),
            Err(VerifyError::DepthMismatch(0))
        );
        assert_eq!(verify(&[]), Err(VerifyError::Empty));
        // Unreachable code is allowed and has no depth.
        let v = verify(&[Push(1), Ret, Add]).unwrap();
        assert_eq!(v.depth[2], None);
    }

    #[test]
    fn decode_never_panics_and_keeps_targets_in_range() {
        let mut x = 0x1234_5678u64;
        for len in 0..300 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect();
            let code = decode(&bytes);
            for i in &code {
                if let Jmp(t) | Jz(t) = i {
                    assert!((*t as usize) < code.len());
                }
            }
        }
    }
}
