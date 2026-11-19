//! Microbenchmark used by the README. It is not a statistical harness:
//! it reports the median of a few timed runs on one machine.

use cortex9_x64_jit::bytecode::{verify, Insn, ARGS};
use cortex9_x64_jit::interp;
use cortex9_x64_jit::jit::JitFn;
use std::hint::black_box;
use std::time::Instant;

fn sum_loop() -> Vec<Insn> {
    use Insn::*;
    vec![
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
    ]
}

/// Bytecode for `arg0 * (arg1 + 1) - 12`.
fn mul_add_sub() -> Vec<Insn> {
    use Insn::*;
    vec![Arg(0), Arg(1), Push(1), Add, Mul, Push(12), Sub, Ret]
}

fn median_ns(samples: &[f64]) -> f64 {
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time_ns(runs: usize, iters: u64, mut body: impl FnMut()) -> (f64, Vec<f64>) {
    for _ in 0..iters {
        body();
    }
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        for _ in 0..iters {
            body();
        }
        samples.push(t.elapsed().as_secs_f64() * 1e9 / iters as f64);
    }
    (median_ns(&samples), samples)
}

fn fmt_samples(samples: &[f64]) -> String {
    samples
        .iter()
        .map(|s| format!("{s:.2}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() {
    let runs = 7;
    let args_sum: [i64; ARGS] = [100, 0, 0, 0];
    let args_expr: [i64; ARGS] = [6, 6, 0, 0];
    let fuel_sum = 1_000u64;

    let v_sum = verify(&sum_loop()).expect("sum loop verifies");
    let jit_sum = JitFn::compile(&v_sum).expect("sum loop compiles");
    assert_eq!(jit_sum.call(&args_sum, fuel_sum), Ok(5050));
    assert_eq!(interp::run(&v_sum, &args_sum, fuel_sum), Ok(5050));

    let v_expr = verify(&mul_add_sub()).expect("expr verifies");
    let jit_expr = JitFn::compile(&v_expr).expect("expr compiles");
    assert_eq!(jit_expr.call(&args_expr, 0), Ok(30));
    assert_eq!(interp::run(&v_expr, &args_expr, 0), Ok(30));

    println!("sum_loop code_len = {} bytes", jit_sum.code_len());
    println!("mul_add_sub code_len = {} bytes", jit_expr.code_len());

    let compile_iters = 2_000u64;
    let (compile_med, compile_s) = time_ns(runs, compile_iters, || {
        let j = JitFn::compile(&v_sum).expect("compile");
        black_box(j.code_len());
    });
    println!(
        "compile(sum_loop): median {compile_med:.2} ns/compile ({runs} runs of {compile_iters}): {}",
        fmt_samples(&compile_s)
    );

    let call_iters_sum = 200_000u64;
    let (interp_sum, interp_sum_s) = time_ns(runs, call_iters_sum, || {
        black_box(interp::run(&v_sum, black_box(&args_sum), fuel_sum).unwrap());
    });
    let (jit_sum_ns, jit_sum_s) = time_ns(runs, call_iters_sum, || {
        black_box(jit_sum.call(black_box(&args_sum), fuel_sum).unwrap());
    });
    println!(
        "interp(sum 1..=100): median {interp_sum:.2} ns/call ({runs} runs of {call_iters_sum}): {}",
        fmt_samples(&interp_sum_s)
    );
    println!(
        "jit(sum 1..=100): median {jit_sum_ns:.2} ns/call ({runs} runs of {call_iters_sum}): {}",
        fmt_samples(&jit_sum_s)
    );

    let call_iters_expr = 5_000_000u64;
    let (interp_expr, interp_expr_s) = time_ns(runs, call_iters_expr, || {
        black_box(interp::run(&v_expr, black_box(&args_expr), 0).unwrap());
    });
    let (jit_expr_ns, jit_expr_s) = time_ns(runs, call_iters_expr, || {
        black_box(jit_expr.call(black_box(&args_expr), 0).unwrap());
    });
    println!(
        "interp(arg0*(arg1+1)-12): median {interp_expr:.2} ns/call ({runs} runs of {call_iters_expr}): {}",
        fmt_samples(&interp_expr_s)
    );
    println!(
        "jit(arg0*(arg1+1)-12): median {jit_expr_ns:.2} ns/call ({runs} runs of {call_iters_expr}): {}",
        fmt_samples(&jit_expr_s)
    );
}
