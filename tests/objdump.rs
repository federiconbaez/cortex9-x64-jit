//! Disassemble the encoder's output with GNU objdump (if installed) and
//! compare against the intended instructions.
#![cfg(target_os = "linux")]

use cortex9_x64_jit::asm::{Alu, Asm, Cond, Reg::*};
use std::process::Command;

#[test]
fn objdump_agrees_with_encoder() {
    if Command::new("objdump").arg("--version").output().is_err() {
        eprintln!("objdump not installed; skipping");
        return;
    }
    let mut a = Asm::new();
    a.load(Rax, Rbp, -8);
    a.load(Rcx, Rdi, 24);
    a.store(Rbp, -16, Rcx);
    a.mov_imm(Rax, 0x1122334455667788);
    a.alu(Alu::Xor, Rax, Rcx);
    a.alu(Alu::Or, Rax, Rcx);
    a.alu(Alu::And, Rax, Rcx);
    a.imul(Rax, Rcx);
    a.cqo();
    a.idiv(Rcx);
    a.mov(Rax, Rdx);
    a.sar_cl(Rax);
    a.set_rax(Cond::E);
    a.cmp_mem0(Rsi);
    a.dec_mem(Rsi);
    a.store_imm32(Rsi, 8, 1);
    a.prologue(48);
    a.epilogue();
    let code = a.finish();
    let dir = std::env::temp_dir().join(format!("c9jit-objdump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("code.bin");
    std::fs::write(&f, &code).unwrap();
    let out = Command::new("objdump")
        .args(["-D", "-b", "binary", "-m", "i386:x86-64", "-M", "intel"])
        .arg(&f)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let got: Vec<String> = text
        .lines()
        .filter_map(|l| l.split('\t').nth(2))
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let want = [
        "mov rax,QWORD PTR [rbp-0x8]",
        "mov rcx,QWORD PTR [rdi+0x18]",
        "mov QWORD PTR [rbp-0x10],rcx",
        "movabs rax,0x1122334455667788",
        "xor rax,rcx",
        "or rax,rcx",
        "and rax,rcx",
        "imul rax,rcx",
        "cqo",
        "idiv rcx",
        "mov rax,rdx",
        "sar rax,cl",
        "sete al",
        "movzx eax,al",
        "cmp QWORD PTR [rsi],0x0",
        "dec QWORD PTR [rsi]",
        "mov QWORD PTR [rsi+0x8],0x1",
        "push rbp",
        "mov rbp,rsp",
        "sub rsp,0x30",
        "mov rsp,rbp",
        "pop rbp",
        "ret",
    ];
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(got, want, "\n{text}");
}
