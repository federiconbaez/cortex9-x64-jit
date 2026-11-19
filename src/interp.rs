//! Reference interpreter. The JIT must agree with it on every program.

use crate::bytecode::{binop, Insn, Trap, Verified, ARGS};

/// Run a verified program.
pub fn run(p: &Verified, args: &[i64; ARGS], mut fuel: u64) -> Result<i64, Trap> {
    let mut stack: Vec<i64> = Vec::with_capacity(p.max_depth);
    let mut pc = 0usize;
    loop {
        let insn = p.code[pc];
        pc += 1;
        match insn {
            Insn::Push(v) => stack.push(v),
            Insn::Arg(a) => stack.push(args[a as usize]),
            Insn::Neg => {
                let a = stack.pop().unwrap();
                stack.push(a.wrapping_neg());
            }
            Insn::Not => {
                let a = stack.pop().unwrap();
                stack.push(!a);
            }
            Insn::Dup => {
                let a = *stack.last().unwrap();
                stack.push(a);
            }
            Insn::Swap => {
                let n = stack.len();
                stack.swap(n - 1, n - 2);
            }
            Insn::Drop => {
                stack.pop();
            }
            Insn::Over => {
                let a = stack[stack.len() - 2];
                stack.push(a);
            }
            Insn::Jmp(t) => {
                if fuel == 0 {
                    return Err(Trap::OutOfFuel);
                }
                fuel -= 1;
                pc = t as usize;
            }
            Insn::Jz(t) => {
                if fuel == 0 {
                    return Err(Trap::OutOfFuel);
                }
                fuel -= 1;
                if stack.pop().unwrap() == 0 {
                    pc = t as usize;
                }
            }
            Insn::Ret => return Ok(stack.pop().unwrap()),
            op => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(binop(op, a, b));
            }
        }
    }
}
