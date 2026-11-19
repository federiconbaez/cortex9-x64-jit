//! Differential tests: the JIT and the interpreter must agree on every
//! verified program, including traps.
#![cfg(all(target_arch = "x86_64", unix))]

use cortex9_x64_jit::bytecode::{decode, verify, Insn, Trap, ARGS};
use cortex9_x64_jit::interp;
use cortex9_x64_jit::jit::JitFn;
use proptest::prelude::*;
use Insn::*;

fn both(code: &[Insn], args: [i64; ARGS], fuel: u64) -> (Result<i64, Trap>, Result<i64, Trap>) {
    let v = verify(code).expect("verifies");
    let j = JitFn::compile(&v).expect("compiles");
    (interp::run(&v, &args, fuel), j.call(&args, fuel))
}

#[test]
fn sum_loop() {
    // sum = 0; i = arg0; while i != 0 { sum += i; i -= 1 }; return sum
    let code = [
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
    let (i, j) = both(&code, [100, 0, 0, 0], 1_000);
    assert_eq!(i, Ok(5050));
    assert_eq!(j, Ok(5050));
    // Not enough fuel: both trap.
    let (i, j) = both(&code, [100, 0, 0, 0], 50);
    assert_eq!(i, Err(Trap::OutOfFuel));
    assert_eq!(j, Err(Trap::OutOfFuel));
}

#[test]
fn every_binary_op_on_edge_values() {
    let edges = [
        0,
        1,
        -1,
        2,
        -2,
        63,
        64,
        65,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
        0x5555_5555_5555_5555,
    ];
    let ops = [Add, Sub, Mul, Div, Rem, And, Or, Xor, Shl, Shr, Eq, Lt];
    for op in ops {
        let code = [Arg(0), Arg(1), op, Ret];
        let v = verify(&code).unwrap();
        let j = JitFn::compile(&v).unwrap();
        for a in edges {
            for b in edges {
                let args = [a, b, 0, 0];
                assert_eq!(
                    j.call(&args, 0),
                    interp::run(&v, &args, 0),
                    "{op:?} {a} {b}"
                );
            }
        }
    }
    for op in [Neg, Not] {
        let code = [Arg(0), op, Ret];
        let v = verify(&code).unwrap();
        let j = JitFn::compile(&v).unwrap();
        for a in edges {
            assert_eq!(
                j.call(&[a, 0, 0, 0], 0),
                interp::run(&v, &[a, 0, 0, 0], 0),
                "{op:?} {a}"
            );
        }
    }
}

#[test]
fn stack_shuffles_and_deep_stack() {
    let code = [Arg(0), Arg(1), Over, Over, Swap, Sub, Swap, Drop, Mul, Ret];
    let (i, j) = both(&code, [7, 3, 0, 0], 0);
    assert_eq!(i, j);
    // 255 pushes then 254 adds.
    let mut code: Vec<Insn> = (0..255).map(Push).collect();
    code.extend(std::iter::repeat(Add).take(254));
    code.push(Ret);
    let (i, j) = both(&code, [0; 4], 0);
    assert_eq!(i, Ok((0..255).sum()));
    assert_eq!(j, i);
}

#[test]
fn infinite_loop_is_stopped_by_fuel() {
    let (i, j) = both(&[Jmp(0)], [0; 4], 1_000_000);
    assert_eq!(i, Err(Trap::OutOfFuel));
    assert_eq!(j, Err(Trap::OutOfFuel));
}

/// Generates verifiable programs: random straight-line code mixed with
/// counted loops and forward conditional skips, so jumps are exercised.
fn program() -> impl Strategy<Value = (Vec<Insn>, [i64; ARGS])> {
    (any::<u64>(), any::<[i64; ARGS]>()).prop_map(|(seed, args)| (gen(seed), args))
}

struct G {
    x: u64,
    code: Vec<Insn>,
}

impl G {
    fn r(&mut self, n: u64) -> u64 {
        self.x ^= self.x << 13;
        self.x ^= self.x >> 7;
        self.x ^= self.x << 17;
        self.x % n
    }
    fn leaf(&mut self) -> Insn {
        match self.r(5) {
            0 => Arg(self.r(ARGS as u64) as u8),
            1 => Push([0, 1, -1, 63, 64, i64::MIN, i64::MAX][self.r(7) as usize]),
            _ => Push(self.r(2001) as i64 - 1000),
        }
    }
    /// Emit code with net stack effect 0, starting at `depth` (>= 1).
    fn block(&mut self, depth: usize, budget: usize, nest: u32) {
        let mut d = depth;
        let mut n = 0;
        while n < budget {
            n += 1;
            let choice = self.r(12);
            if choice == 0 && nest < 2 {
                // Counted loop: counter = small; while counter { body; counter -= 1 }
                let c = self.r(6) as i64;
                self.code.push(Push(c));
                let top = self.code.len();
                self.code.push(Dup);
                let jz = self.code.len();
                self.code.push(Jz(0));
                self.block(d + 1, 6, nest + 1);
                self.code.push(Push(1));
                self.code.push(Sub);
                self.code.push(Jmp(top as u32));
                let end = self.code.len();
                self.code[jz] = Jz(end as u32);
                self.code.push(Drop);
            } else if choice == 1 && nest < 3 && d >= 1 {
                // if top-of-stack-derived cond == 0 skip block
                self.code.push(Dup);
                let jz = self.code.len();
                self.code.push(Jz(0));
                self.block(d, 4, nest + 1);
                let end = self.code.len();
                self.code[jz] = Jz(end as u32);
            } else if d < 2 || (d < 12 && self.r(3) == 0) {
                let l = self.leaf();
                self.code.push(l);
                d += 1;
            } else {
                let op = [
                    Add, Sub, Mul, Div, Rem, And, Or, Xor, Shl, Shr, Eq, Lt, Neg, Not, Dup, Swap,
                    Drop, Over,
                ][self.r(18) as usize];
                let (pop, push) = op.effect();
                if d > pop && d - pop + push >= 1 {
                    self.code.push(op);
                    d = d - pop + push;
                }
            }
        }
        // Restore net effect 0.
        while d > depth {
            let op = if self.r(2) == 0 { Drop } else { Add };
            self.code.push(op);
            d -= 1;
        }
        while d < depth {
            let l = self.leaf();
            self.code.push(l);
            d += 1;
        }
    }
}

fn gen(seed: u64) -> Vec<Insn> {
    let mut g = G {
        x: seed | 1,
        code: vec![Arg(0)],
    };
    let budget = 1 + (seed % 40) as usize;
    g.block(1, budget, 0);
    g.code.push(Ret);
    g.code
}

proptest! {
    #![proptest_config(ProptestConfig { cases: if cfg!(debug_assertions) { 2_000 } else { 20_000 }, ..ProptestConfig::default() })]

    #[test]
    fn jit_matches_interpreter((code, args) in program(), fuel in 0u64..300) {
        let v = verify(&code).expect("generator emits verifiable code");
        let j = JitFn::compile(&v).unwrap();
        prop_assert_eq!(j.call(&args, fuel), interp::run(&v, &args, fuel));
    }

    #[test]
    fn decoded_bytes_agree_when_they_verify(bytes in proptest::collection::vec(any::<u8>(), 0..200), args in any::<[i64; ARGS]>()) {
        let code = decode(&bytes);
        if let Ok(v) = verify(&code) {
            let j = JitFn::compile(&v).unwrap();
            prop_assert_eq!(j.call(&args, 500), interp::run(&v, &args, 500));
        }
    }
}

#[test]
fn generator_produces_jumps() {
    let with_jumps = (0..500u64)
        .filter(|s| gen(*s).iter().any(|i| matches!(i, Jz(_))))
        .count();
    assert!(with_jumps > 100, "{with_jumps}");
    for s in 0..2000u64 {
        verify(&gen(s)).unwrap_or_else(|e| panic!("seed {s}: {e:?} {:?}", gen(s)));
    }
}
