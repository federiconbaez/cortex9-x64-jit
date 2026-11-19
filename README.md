# cortex9-x64-jit

Compilador JIT de un bytecode de pila pequeño a código máquina x86-64. El encoder se escribe a mano (sin ensamblador ni bibliotecas de codegen). La memoria ejecutable es W^X: se escribe en un mapeo de lectura y escritura, y después `mprotect` lo pasa a solo lectura y ejecución; nunca es escribible y ejecutable a la vez.

Es una biblioteca. El módulo `jit` solo se compila en Unix sobre `x86_64`. El verificador y el intérprete de referencia no dependen de la arquitectura.

Parte de la serie Cortex-9: [github.com/federiconbaez](https://github.com/federiconbaez#cortex-9).

## Qué hace

- **Bytecode** (`bytecode`): enteros `i64`, cuatro argumentos, pila de operandos y un conjunto fijo de instrucciones: constantes, argumentos, aritmética (`Add`/`Sub`/`Mul`/`Div`/`Rem`), bits y desplazamientos, comparaciones, unarios, barajado de pila (`Dup`/`Swap`/`Drop`/`Over`), saltos (`Jmp`/`Jz`) y `Ret`.
- **Semántica definida en los bordes**: la aritmética envuelve (módulo 2^64). `Div`/`Rem` por cero dan 0. `i64::MIN / -1` da `i64::MIN` y `i64::MIN % -1` da 0 (la misma regla que `wrapping_div` / `wrapping_rem` en Rust, y evitan el #DE de `idiv` en x86). Los desplazamientos se enmascaran a 6 bits; `Shr` es aritmético.
- **Combustible**: cada `Jmp` o `Jz` ejecutado consume una unidad de combustible *antes* de saltar. Si no queda, la ejecución termina con `Trap::OutOfFuel`. Un bucle infinito no se cuelga.
- **Verificador**: profundidades de pila estáticas, consistentes en los puntos de unión, argumentos y destinos de salto en rango, y ningún programa que se caiga al final. El código inalcanzable se admite y no tiene profundidad.
- **Intérprete** (`interp`): implementación de referencia. El JIT tiene que coincidir con él en el valor o en la trampa.
- **Encoder** (`asm`): un puñado de instrucciones x86-64 (`rax`, `rcx`, `rdx`, `rsp`, `rbp`, `rsi`, `rdi`; solo prefijo `REX.W`). Etiquetas y parches `rel32`.
- **JIT** (`jit`): como el verificador fija la profundidad antes de cada instrucción, la ranura `i` de la pila de operandos es `[rbp - 8*(i+1)]`. No hace falta `push`/`pop` en los puntos de unión. Convención System V: `fn(args: *const i64, state: *mut State) -> i64`.
- **Memoria** (`ExecMem`): `mmap` anónimo `PROT_READ|PROT_WRITE`, copia del código, relleno con `int3` (`0xCC`), `mprotect` a `PROT_READ|PROT_EXEC`, `munmap` al destruir. Todo eso es `unsafe` y está concentrado en `ExecMem` y en la llamada a la función generada.

## Uso

```rust
use cortex9_x64_jit::bytecode::{verify, Insn};
use cortex9_x64_jit::interp;
use cortex9_x64_jit::jit::JitFn;

let code = [Insn::Arg(0), Insn::Arg(1), Insn::Add, Insn::Ret];
let verified = verify(&code).expect("el programa verifica");
let jit = JitFn::compile(&verified).expect("compila");
let args = [2, 3, 0, 0];
assert_eq!(jit.call(&args, 0), Ok(5));
assert_eq!(jit.call(&args, 0), interp::run(&verified, &args, 0));
```

```sh
cargo test
cargo test --release
cargo run --release --example bench
```

El test `objdump_agrees_with_encoder` se omite en silencio si `objdump` no está instalado.

## Qué está probado

| Test | Qué cubre |
|---|---|
| `bytecode::tests` | Semántica de bordes (`Div`/`Rem`/desplazamientos/`Lt`/desbordamiento), el verificador (bucles, underflow, caída al final, argumento inválido, desajuste de profundidad, programa vacío, código inalcanzable) y que `decode` no paniquea y deja los saltos en rango. |
| `asm::tests` | Bytes esperados de cada forma que usa el encoder, y la resolución de saltos hacia adelante y hacia atrás. |
| `tests/diff.rs` | El JIT y el intérprete coinciden: bucle `sum 1..=100` (y la trampa por combustible), las doce operaciones binarias y los dos unarios sobre valores de borde, barajado y una pila de 255 ranuras, un `Jmp` infinito detenido por combustible, 2 000 programas generados en debug / 20 000 en release (con saltos y bucles contados) vía proptest, y bytes aleatorios que pasan el verificador después de `decode`. Hay semillas de regresión en `tests/diff.proptest-regressions`. |
| `tests/wx.rs` (Linux) | Tras compilar, `/proc/self/maps` muestra `r-x` en la región del código, y ningún mapeo del proceso es a la vez `w` y `x`. |
| `tests/objdump.rs` (Linux, si hay `objdump`) | `objdump -D -b binary -m i386:x86-64 -M intel` coincide con las mnemónicas previstas del encoder. |

En esta máquina, `cargo test` y `cargo test --release` pasaron **15 tests** (5 unitarios + 7 diferenciales + 1 objdump + 2 W^X). No hay tests de documentación.

## Qué se midió

Una sola vez, en esta máquina, un hilo, perfil `release`. No es un harness estadístico: se informa la **mediana** de 7 corridas.

- Comando: `cargo run --release --example bench`
- `rustc --version`: `rustc 1.100.0 (0ef1bc386 2026-11-09)`
- CPU según `lscpu`: `Intel(R) Xeon(R) Processor`, 8 vCPU, 2400 MHz, hipervisor KVM, Linux 6.12.113 `x86_64`

| carga | tamaño del código | intérprete (mediana) | JIT (mediana) |
|---|---|---|---|
| `sum 1..=100` (bucle con `Jz`/`Jmp`, combustible 1000) | 265 bytes | 1739.02 ns/llamada | 199.82 ns/llamada |
| `arg0 * (arg1 + 1) - 12` (línea recta) | 173 bytes | 24.61 ns/llamada | 2.97 ns/llamada |

Compilar el bucle `sum` (incluye `mmap`/`mprotect`): mediana **6878.79 ns** por compilación (7 corridas de 2000; las muestras fueron 8349.30, 11240.77, 6712.95, 6700.33, 6490.66, 6878.79, 8142.69 ns).

Las llamadas al JIT pagan una llamada indirecta y, en el bucle, el chequeo de combustible en cada salto. No se midió contra un equivalente escrito a mano en Rust ni contra otro JIT.

## Qué no está medido ni verificado

- El CI de GitHub Actions **no es verificable en local**: el flujo existe en `.github/workflows/ci.yml` (fmt, clippy `-D warnings` y `cargo test` en `ubuntu-latest`), pero no se ejecutó en GitHub desde este árbol.
- `rust-version = "1.75"` está declarado en `Cargo.toml`; en esta máquina solo se compiló y se probaron los tests con rustc 1.100.0. No se corrió una build con 1.75.
- No hay tests en macOS ni en otras Unix no Linux. El módulo `jit` se compilaría en cualquier Unix `x86_64`, pero no se ejecutó fuera de Linux. El test W^X lee `/proc/self/maps` y solo corre en Linux.
- No hay soporte de ejecución JIT en Windows ni en arquitecturas que no sean x86-64: el módulo `jit` ni siquiera se compila ahí.
- `decode` está pensado para un fuzzer, pero este repositorio no incluye un harness de cargo-fuzz. No se midió fuzzing.
- No hay tests de concurrencia ni de que `JitFn`/`ExecMem` sean `Send` o `Sync` (contienen punteros crudos).
- No se midió el JIT contra un compilador nativo, contra `tiny-jit-x64`, ni en una máquina distinta. No se midió el tiempo de `cargo test` como métrica de rendimiento.
- No se verificó el encoder con GNU `as` en esta preparación: los bytes esperados están en los tests unitarios, y `objdump` (Binutils 2.44) coincidió al correr `cargo test`.
- No hay asignación de registros, ni plegado de constantes, ni un lenguaje fuente con parser.

## Límites

- Solo x86-64 en Unix para emitir y ejecutar. El encoder no usa `r8`–`r15` ni SIB; `rsp` no puede ser base de un direccionamiento.
- La proyección ejecutable es `unsafe` (`mmap`, `mprotect`, `munmap`, `transmute` a `extern "sysv64" fn`). Un programa que no pasó el verificador no debe entregarse al JIT: el JIT asume las profundidades del `Verified`.
- W^X por cambio de protección de la misma proyección (no hay un doble mapeo RW/RX). Después del `mprotect` a RX no se puede ampliar esa región. Si `mprotect` falla, se desmapea y se devuelve error.
- Los saltos del encoder son `rel32`. Un programa que no verifica (pila inconsistente, destino fuera de rango) se rechaza antes de emitir.
- Proceso único, en memoria: el código no se serializa ni se comparte entre procesos.
- MSRV declarada 1.75, no comprobada aquí (véase arriba).

## Diferencia con tiny-jit-x64

La comparación se basa en el árbol local de [tiny-jit-x64](https://github.com/federiconbaez/tiny-jit-x64) (commit `afd1b2874234ad36442d21e5bafd88c0eca7ff91`, remoto `https://github.com/federiconbaez/tiny-jit-x64.git`) y en su README. El repositorio público existe; no se volvió a medir ese proyecto en esta máquina.

Los dos emiten x86-64 a mano, ejecutan desde una página W^X (`mmap` RW → copia → `mprotect` RX, relleno `int3`) y contrastan el JIT contra un intérprete. A partir de ahí no son el mismo programa:

| | tiny-jit-x64 | cortex9-x64-jit |
|---|---|---|
| Lenguaje de implementación | C++20 | Rust |
| Entrada | expresiones enteras con parser (`x*(y+1) - 3*4`) | bytecode de pila ya formado; no hay parser ni lenguaje fuente |
| Operandos | tres variables (`x`,`y`,`z` → `rdi`,`rsi`,`rdx`) | cuatro argumentos `i64` y una pila de operandos |
| Operaciones | `+`, `-`, `*`, menos unario y paréntesis. Sin división | aritmética (incluida `Div`/`Rem` con bordes definidos), bits, desplazamientos, comparaciones, barajado de pila y saltos con combustible |
| Optimización | plegado de constantes y de identidades (`a+0`, `a*1`, …) | no hay plegado; las constantes ya vienen en el bytecode |
| Modelo del JIT | máquina de pila sobre `rax`/`rcx` con `push`/`pop` | ranuras fijas en el frame, porque el verificador conoce la profundidad en cada PC |
| Control | un árbol de expresión, sin saltos en el lenguaje | `Jmp`/`Jz` y un verificador de uniones |
| Pruebas que se pudieron leer en su árbol | bytes dorados, diferencial de expresiones, errores de parseo, W^X por `/proc/self/maps`, CI con g++/clang++ y sanitizers; en arm64 solo emite | las de la sección «Qué está probado»; CI solo en `ubuntu-latest` (x86-64), sin sanitizers |

No se comparan números de rendimiento entre los dos: las cargas no son equivalentes y el bench de tiny-jit-x64 no se corrió en esta preparación.

## Licencia

Apache-2.0
