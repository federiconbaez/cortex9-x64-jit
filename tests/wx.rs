//! W^X: the code mapping is read+execute, never writable, after compilation.
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use cortex9_x64_jit::bytecode::{verify, Insn};
use cortex9_x64_jit::jit::JitFn;

fn perms_of(addr: usize) -> String {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    for line in maps.lines() {
        let mut it = line.split_whitespace();
        let range = it.next().unwrap();
        let perms = it.next().unwrap();
        let (lo, hi) = range.split_once('-').unwrap();
        let (lo, hi) = (
            usize::from_str_radix(lo, 16).unwrap(),
            usize::from_str_radix(hi, 16).unwrap(),
        );
        if (lo..hi).contains(&addr) {
            return perms.to_string();
        }
    }
    panic!("address not mapped");
}

#[test]
fn code_pages_are_rx_not_w() {
    let v = verify(&[Insn::Push(42), Insn::Ret]).unwrap();
    let j = JitFn::compile(&v).unwrap();
    assert_eq!(j.call(&[0; 4], 0), Ok(42));
    let p = perms_of(j.addr());
    assert!(p.starts_with("r-x"), "{p}");
}

#[test]
fn no_mapping_is_writable_and_executable() {
    let v = verify(&[Insn::Arg(0), Insn::Ret]).unwrap();
    let _keep: Vec<JitFn> = (0..16).map(|_| JitFn::compile(&v).unwrap()).collect();
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    for line in maps.lines() {
        let perms = line.split_whitespace().nth(1).unwrap();
        assert!(
            !(perms.contains('w') && perms.contains('x')),
            "W+X mapping: {line}"
        );
    }
}
