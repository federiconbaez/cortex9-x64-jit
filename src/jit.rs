//! Compiler from verified bytecode to x86-64, and executable memory with
//! W^X: code is written into a read-write mapping, which is then switched to
//! read-execute with `mprotect` before the first call. It is never writable
//! and executable at the same time.
//!
//! Because the verifier fixes the stack depth before every instruction,
//! operand-stack slot `i` is a fixed frame location `[rbp - 8*(i+1)]`; no
//! push/pop bookkeeping is needed at join points.
//!
//! Generated function (System V): `fn(args: *const i64, state: *mut State) -> i64`.

use crate::asm::{Alu, Asm, Cond, Reg};
use crate::bytecode::{Insn, Trap, Verified, ARGS};
use std::fmt;

/// Fuel counter and exit status shared with generated code.
#[repr(C)]
#[derive(Debug, Default)]
pub struct State {
    /// Remaining fuel.
    pub fuel: u64,
    /// 0 = returned normally, 1 = out of fuel.
    pub status: u64,
}

/// Compilation / mapping errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitError {
    /// `mmap` or `mprotect` failed.
    Map(String),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JitError::Map(s) => write!(f, "memory mapping: {s}"),
        }
    }
}

impl std::error::Error for JitError {}

fn slot(i: usize) -> i32 {
    -8 * (i as i32 + 1)
}

/// Emit machine code for `p`.
pub fn emit(p: &Verified) -> Vec<u8> {
    use Reg::*;
    let mut a = Asm::new();
    let frame = ((p.max_depth * 8 + 15) & !15) as i32;
    a.prologue(frame);
    let labels: Vec<usize> = (0..p.code.len()).map(|_| a.label()).collect();
    let oof = a.label();
    for (pc, insn) in p.code.iter().enumerate() {
        a.bind(labels[pc]);
        let Some(d) = p.depth[pc] else { continue };
        let d = d as usize;
        match *insn {
            Insn::Push(v) => {
                a.mov_imm(Rax, v);
                a.store(Rbp, slot(d), Rax);
            }
            Insn::Arg(n) => {
                debug_assert!((n as usize) < ARGS);
                a.load(Rax, Rdi, 8 * n as i32);
                a.store(Rbp, slot(d), Rax);
            }
            Insn::Add
            | Insn::Sub
            | Insn::And
            | Insn::Or
            | Insn::Xor
            | Insn::Mul
            | Insn::Shl
            | Insn::Shr
            | Insn::Eq
            | Insn::Lt => {
                a.load(Rax, Rbp, slot(d - 2));
                a.load(Rcx, Rbp, slot(d - 1));
                match *insn {
                    Insn::Add => a.alu(Alu::Add, Rax, Rcx),
                    Insn::Sub => a.alu(Alu::Sub, Rax, Rcx),
                    Insn::And => a.alu(Alu::And, Rax, Rcx),
                    Insn::Or => a.alu(Alu::Or, Rax, Rcx),
                    Insn::Xor => a.alu(Alu::Xor, Rax, Rcx),
                    Insn::Mul => a.imul(Rax, Rcx),
                    Insn::Shl => a.shl_cl(Rax),
                    Insn::Shr => a.sar_cl(Rax),
                    Insn::Eq => {
                        a.alu(Alu::Cmp, Rax, Rcx);
                        a.set_rax(Cond::E);
                    }
                    Insn::Lt => {
                        a.alu(Alu::Cmp, Rax, Rcx);
                        a.set_rax(Cond::L);
                    }
                    _ => unreachable!(),
                }
                a.store(Rbp, slot(d - 2), Rax);
            }
            Insn::Div | Insn::Rem => {
                // b == 0 -> 0; b == -1 -> (-a, 0); else idiv.
                let is_div = *insn == Insn::Div;
                let (zero, minus1, done) = (a.label(), a.label(), a.label());
                a.load(Rax, Rbp, slot(d - 2));
                a.load(Rcx, Rbp, slot(d - 1));
                a.test(Rcx);
                a.jcc(Cond::E, zero);
                a.cmp_imm8(Rcx, -1);
                a.jcc(Cond::E, minus1);
                a.cqo();
                a.idiv(Rcx);
                if !is_div {
                    a.mov(Rax, Rdx);
                }
                a.jmp(done);
                a.bind(minus1);
                if is_div {
                    a.neg(Rax);
                } else {
                    a.zero_rax();
                }
                a.jmp(done);
                a.bind(zero);
                a.zero_rax();
                a.bind(done);
                a.store(Rbp, slot(d - 2), Rax);
            }
            Insn::Neg | Insn::Not => {
                a.load(Rax, Rbp, slot(d - 1));
                if *insn == Insn::Neg {
                    a.neg(Rax);
                } else {
                    a.not(Rax);
                }
                a.store(Rbp, slot(d - 1), Rax);
            }
            Insn::Dup => {
                a.load(Rax, Rbp, slot(d - 1));
                a.store(Rbp, slot(d), Rax);
            }
            Insn::Over => {
                a.load(Rax, Rbp, slot(d - 2));
                a.store(Rbp, slot(d), Rax);
            }
            Insn::Swap => {
                a.load(Rax, Rbp, slot(d - 2));
                a.load(Rcx, Rbp, slot(d - 1));
                a.store(Rbp, slot(d - 2), Rcx);
                a.store(Rbp, slot(d - 1), Rax);
            }
            Insn::Drop => {}
            Insn::Jmp(t) => {
                a.cmp_mem0(Rsi);
                a.jcc(Cond::E, oof);
                a.dec_mem(Rsi);
                a.jmp(labels[t as usize]);
            }
            Insn::Jz(t) => {
                a.cmp_mem0(Rsi);
                a.jcc(Cond::E, oof);
                a.dec_mem(Rsi);
                a.load(Rax, Rbp, slot(d - 1));
                a.test(Rax);
                a.jcc(Cond::E, labels[t as usize]);
            }
            Insn::Ret => {
                a.load(Rax, Rbp, slot(d - 1));
                a.epilogue();
            }
        }
    }
    a.bind(oof);
    a.store_imm32(Rsi, 8, 1);
    a.zero_rax();
    a.epilogue();
    a.finish()
}

/// Executable code region (unmapped on drop).
pub struct ExecMem {
    ptr: *mut u8,
    len: usize,
}

impl ExecMem {
    /// Map `code` read-write, copy it, then flip the mapping to read-execute.
    pub fn new(code: &[u8]) -> Result<Self, JitError> {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        let len = code.len().div_ceil(page).max(1) * page;
        // SAFETY: anonymous private mapping; arguments are valid.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(JitError::Map(std::io::Error::last_os_error().to_string()));
        }
        let ptr = ptr as *mut u8;
        // SAFETY: the mapping is `len >= code.len()` bytes and writable.
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), ptr, code.len()) };
        // Fill the slack with int3 so a stray jump traps.
        unsafe { std::ptr::write_bytes(ptr.add(code.len()), 0xCC, len - code.len()) };
        // SAFETY: same mapping; drop write permission before allowing execute.
        if unsafe {
            libc::mprotect(
                ptr as *mut libc::c_void,
                len,
                libc::PROT_READ | libc::PROT_EXEC,
            )
        } != 0
        {
            let e = std::io::Error::last_os_error().to_string();
            unsafe { libc::munmap(ptr as *mut libc::c_void, len) };
            return Err(JitError::Map(e));
        }
        Ok(ExecMem { ptr, len })
    }

    /// Start address.
    pub fn addr(&self) -> usize {
        self.ptr as usize
    }

    /// Mapping length.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Never empty (at least one page).
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Drop for ExecMem {
    fn drop(&mut self) {
        // SAFETY: we own this mapping.
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.len) };
    }
}

type RawFn = unsafe extern "sysv64" fn(*const i64, *mut State) -> i64;

/// A compiled program.
pub struct JitFn {
    mem: ExecMem,
    f: RawFn,
    code_len: usize,
}

impl JitFn {
    /// Compile a verified program.
    pub fn compile(p: &Verified) -> Result<Self, JitError> {
        let code = emit(p);
        let mem = ExecMem::new(&code)?;
        // SAFETY: `mem` holds code produced by `emit` for this ABI.
        let f: RawFn = unsafe { std::mem::transmute::<*mut u8, RawFn>(mem.ptr) };
        Ok(JitFn {
            mem,
            f,
            code_len: code.len(),
        })
    }

    /// Run with arguments and a fuel budget.
    pub fn call(&self, args: &[i64; ARGS], fuel: u64) -> Result<i64, Trap> {
        let mut st = State { fuel, status: 0 };
        // SAFETY: code was produced by `emit` from a verified program: every
        // stack slot it touches lies inside its own frame, it reads only
        // `args[0..ARGS]` and writes only `*state`, and it always returns.
        let v = unsafe { (self.f)(args.as_ptr(), &mut st) };
        if st.status == 1 {
            Err(Trap::OutOfFuel)
        } else {
            Ok(v)
        }
    }

    /// Generated code size in bytes.
    pub fn code_len(&self) -> usize {
        self.code_len
    }

    /// Address of the executable mapping.
    pub fn addr(&self) -> usize {
        self.mem.addr()
    }
}
