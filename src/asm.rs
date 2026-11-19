//! Hand-written x86-64 encoder for the handful of instructions the JIT
//! needs. Only `rax`, `rcx`, `rdx`, `rsp`, `rbp`, `rsi`, `rdi` are used, so
//! the only REX prefix needed is `REX.W` (0x48).

/// General-purpose registers (hardware numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Reg {
    /// rax
    Rax = 0,
    /// rcx
    Rcx = 1,
    /// rdx
    Rdx = 2,
    /// rsp
    Rsp = 4,
    /// rbp
    Rbp = 5,
    /// rsi
    Rsi = 6,
    /// rdi
    Rdi = 7,
}

/// Two-register ALU operations encoded as `op r/m64, r64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alu {
    /// add
    Add = 0x01,
    /// or
    Or = 0x09,
    /// and
    And = 0x21,
    /// sub
    Sub = 0x29,
    /// xor
    Xor = 0x31,
    /// cmp
    Cmp = 0x39,
}

/// Condition codes for `jcc` / `setcc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    /// equal / zero
    E = 0x4,
    /// not equal / not zero
    Ne = 0x5,
    /// signed less
    L = 0xC,
}

/// A forward or backward branch whose 32-bit displacement is patched later.
#[derive(Debug, Clone, Copy)]
pub struct Fixup {
    /// Offset of the rel32 field.
    pub at: usize,
    /// Label id it refers to.
    pub label: usize,
}

const W: u8 = 0x48;

fn modrm(md: u8, reg: u8, rm: u8) -> u8 {
    (md << 6) | ((reg & 7) << 3) | (rm & 7)
}

/// Code buffer with labels.
#[derive(Debug, Default, Clone)]
pub struct Asm {
    /// Encoded bytes.
    pub buf: Vec<u8>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
}

impl Asm {
    /// Empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    fn b(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    fn mem(&mut self, reg: u8, base: Reg, disp: i32) {
        // [base + disp32]; rsp would need a SIB byte and is never used as a base.
        assert!(base != Reg::Rsp, "rsp base needs SIB");
        self.buf.push(modrm(0b10, reg, base as u8));
        self.b(&disp.to_le_bytes());
    }

    /// `mov dst, [base + disp]`
    pub fn load(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.b(&[W, 0x8B]);
        self.mem(dst as u8, base, disp);
    }

    /// `mov [base + disp], src`
    pub fn store(&mut self, base: Reg, disp: i32, src: Reg) {
        self.b(&[W, 0x89]);
        self.mem(src as u8, base, disp);
    }

    /// `mov dst, imm64`
    pub fn mov_imm(&mut self, dst: Reg, imm: i64) {
        self.b(&[W, 0xB8 + dst as u8]);
        self.b(&imm.to_le_bytes());
    }

    /// `mov dst, src`
    pub fn mov(&mut self, dst: Reg, src: Reg) {
        self.b(&[W, 0x89, modrm(0b11, src as u8, dst as u8)]);
    }

    /// `op dst, src` for add/or/and/sub/xor/cmp.
    pub fn alu(&mut self, op: Alu, dst: Reg, src: Reg) {
        self.b(&[W, op as u8, modrm(0b11, src as u8, dst as u8)]);
    }

    /// `imul dst, src`
    pub fn imul(&mut self, dst: Reg, src: Reg) {
        self.b(&[W, 0x0F, 0xAF, modrm(0b11, dst as u8, src as u8)]);
    }

    /// `cqo` (sign-extend rax into rdx:rax)
    pub fn cqo(&mut self) {
        self.b(&[W, 0x99]);
    }

    /// `idiv r` (rdx:rax / r -> rax quotient, rdx remainder)
    pub fn idiv(&mut self, r: Reg) {
        self.b(&[W, 0xF7, modrm(0b11, 7, r as u8)]);
    }

    /// `neg r`
    pub fn neg(&mut self, r: Reg) {
        self.b(&[W, 0xF7, modrm(0b11, 3, r as u8)]);
    }

    /// `not r`
    pub fn not(&mut self, r: Reg) {
        self.b(&[W, 0xF7, modrm(0b11, 2, r as u8)]);
    }

    /// `shl r, cl`
    pub fn shl_cl(&mut self, r: Reg) {
        self.b(&[W, 0xD3, modrm(0b11, 4, r as u8)]);
    }

    /// `sar r, cl`
    pub fn sar_cl(&mut self, r: Reg) {
        self.b(&[W, 0xD3, modrm(0b11, 7, r as u8)]);
    }

    /// `test r, r`
    pub fn test(&mut self, r: Reg) {
        self.b(&[W, 0x85, modrm(0b11, r as u8, r as u8)]);
    }

    /// `cmp r, imm8` (sign-extended)
    pub fn cmp_imm8(&mut self, r: Reg, imm: i8) {
        self.b(&[W, 0x83, modrm(0b11, 7, r as u8), imm as u8]);
    }

    /// `setcc al; movzx eax, al` — rax = condition ? 1 : 0
    pub fn set_rax(&mut self, c: Cond) {
        self.b(&[0x0F, 0x90 + c as u8, 0xC0, 0x0F, 0xB6, 0xC0]);
    }

    /// `xor eax, eax`
    pub fn zero_rax(&mut self) {
        self.b(&[0x31, 0xC0]);
    }

    /// `cmp qword [r], 0`
    pub fn cmp_mem0(&mut self, r: Reg) {
        assert!(r != Reg::Rsp && r != Reg::Rbp);
        self.b(&[W, 0x83, modrm(0b00, 7, r as u8), 0x00]);
    }

    /// `dec qword [r]`
    pub fn dec_mem(&mut self, r: Reg) {
        assert!(r != Reg::Rsp && r != Reg::Rbp);
        self.b(&[W, 0xFF, modrm(0b00, 1, r as u8)]);
    }

    /// `mov qword [r + disp8], imm32`
    pub fn store_imm32(&mut self, r: Reg, disp: i8, imm: i32) {
        assert!(r != Reg::Rsp);
        self.b(&[W, 0xC7, modrm(0b01, 0, r as u8), disp as u8]);
        self.b(&imm.to_le_bytes());
    }

    /// `push rbp; mov rbp, rsp; sub rsp, frame`
    pub fn prologue(&mut self, frame: i32) {
        self.b(&[0x55]);
        self.mov(Reg::Rbp, Reg::Rsp);
        self.b(&[W, 0x81, modrm(0b11, 5, Reg::Rsp as u8)]);
        self.b(&frame.to_le_bytes());
    }

    /// `mov rsp, rbp; pop rbp; ret`
    pub fn epilogue(&mut self) {
        self.mov(Reg::Rsp, Reg::Rbp);
        self.b(&[0x5D, 0xC3]);
    }

    /// New unbound label.
    pub fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }

    /// Bind a label to the current position.
    pub fn bind(&mut self, l: usize) {
        assert!(self.labels[l].is_none(), "label bound twice");
        self.labels[l] = Some(self.buf.len());
    }

    /// `jmp label` (rel32)
    pub fn jmp(&mut self, l: usize) {
        self.b(&[0xE9]);
        self.fixups.push(Fixup {
            at: self.buf.len(),
            label: l,
        });
        self.b(&[0; 4]);
    }

    /// `jcc label` (rel32)
    pub fn jcc(&mut self, c: Cond, l: usize) {
        self.b(&[0x0F, 0x80 + c as u8]);
        self.fixups.push(Fixup {
            at: self.buf.len(),
            label: l,
        });
        self.b(&[0; 4]);
    }

    /// Resolve every branch. Panics on an unbound label.
    pub fn finish(mut self) -> Vec<u8> {
        for f in &self.fixups {
            let target = self.labels[f.label].expect("unbound label") as i64;
            let rel = target - (f.at as i64 + 4);
            let rel = i32::try_from(rel).expect("branch out of rel32 range");
            self.buf[f.at..f.at + 4].copy_from_slice(&rel.to_le_bytes());
        }
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Reg::*;

    fn enc(f: impl FnOnce(&mut Asm)) -> Vec<u8> {
        let mut a = Asm::new();
        f(&mut a);
        a.finish()
    }

    // Expected bytes cross-checked with GNU as + objdump (see README).
    #[test]
    fn encodings_match_reference() {
        assert_eq!(
            enc(|a| a.load(Rax, Rbp, -8)),
            [0x48, 0x8B, 0x85, 0xF8, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            enc(|a| a.load(Rcx, Rdi, 16)),
            [0x48, 0x8B, 0x8F, 0x10, 0, 0, 0]
        );
        assert_eq!(
            enc(|a| a.store(Rbp, -16, Rax)),
            [0x48, 0x89, 0x85, 0xF0, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            enc(|a| a.mov_imm(Rax, -2)),
            [0x48, 0xB8, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(enc(|a| a.mov(Rax, Rdx)), [0x48, 0x89, 0xD0]);
        assert_eq!(enc(|a| a.alu(Alu::Add, Rax, Rcx)), [0x48, 0x01, 0xC8]);
        assert_eq!(enc(|a| a.alu(Alu::Sub, Rax, Rcx)), [0x48, 0x29, 0xC8]);
        assert_eq!(enc(|a| a.alu(Alu::Cmp, Rax, Rcx)), [0x48, 0x39, 0xC8]);
        assert_eq!(enc(|a| a.imul(Rax, Rcx)), [0x48, 0x0F, 0xAF, 0xC1]);
        assert_eq!(enc(|a| a.cqo()), [0x48, 0x99]);
        assert_eq!(enc(|a| a.idiv(Rcx)), [0x48, 0xF7, 0xF9]);
        assert_eq!(enc(|a| a.neg(Rax)), [0x48, 0xF7, 0xD8]);
        assert_eq!(enc(|a| a.not(Rax)), [0x48, 0xF7, 0xD0]);
        assert_eq!(enc(|a| a.shl_cl(Rax)), [0x48, 0xD3, 0xE0]);
        assert_eq!(enc(|a| a.sar_cl(Rax)), [0x48, 0xD3, 0xF8]);
        assert_eq!(enc(|a| a.test(Rcx)), [0x48, 0x85, 0xC9]);
        assert_eq!(enc(|a| a.cmp_imm8(Rcx, -1)), [0x48, 0x83, 0xF9, 0xFF]);
        assert_eq!(
            enc(|a| a.set_rax(Cond::L)),
            [0x0F, 0x9C, 0xC0, 0x0F, 0xB6, 0xC0]
        );
        assert_eq!(enc(|a| a.cmp_mem0(Rsi)), [0x48, 0x83, 0x3E, 0x00]);
        assert_eq!(enc(|a| a.dec_mem(Rsi)), [0x48, 0xFF, 0x0E]);
        assert_eq!(
            enc(|a| a.store_imm32(Rsi, 8, 1)),
            [0x48, 0xC7, 0x46, 0x08, 1, 0, 0, 0]
        );
        assert_eq!(
            enc(|a| a.prologue(32)),
            [0x55, 0x48, 0x89, 0xE5, 0x48, 0x81, 0xEC, 32, 0, 0, 0]
        );
        assert_eq!(enc(|a| a.epilogue()), [0x48, 0x89, 0xEC, 0x5D, 0xC3]);
    }

    #[test]
    fn branches_resolve_both_directions() {
        let code = enc(|a| {
            let top = a.label();
            let end = a.label();
            a.bind(top);
            a.jcc(Cond::E, end); // 6 bytes
            a.jmp(top); // 5 bytes
            a.bind(end);
        });
        // je +5 (skip the jmp), jmp -11 (back to 0)
        assert_eq!(code, [0x0F, 0x84, 5, 0, 0, 0, 0xE9, 0xF5, 0xFF, 0xFF, 0xFF]);
    }
}
