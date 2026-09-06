//! Kernel build script: emits the linker script argument and generates the
//! 256 interrupt-vector entry stubs (isr.S) so the IDT can be filled from a table.
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg=-T{manifest}/link.ld");
    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rerun-if-changed=build.rs");

    // Vectors for which the CPU pushes an error code.
    let with_err = [8u32, 10, 11, 12, 13, 14, 17, 21, 29, 30];
    let mut s = String::new();
    s.push_str(".section .text.isr, \"ax\"\n");
    for v in 0..256u32 {
        s.push_str(&format!(".global vec_{v}\nvec_{v}:\n"));
        if !with_err.contains(&v) {
            s.push_str("    push 0\n");
        }
        s.push_str(&format!("    push {v}\n    jmp isr_common\n"));
    }
    s.push_str(".section .rodata\n.global isr_table\n.balign 8\nisr_table:\n");
    for v in 0..256u32 {
        s.push_str(&format!("    .quad vec_{v}\n"));
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("isr.S");
    fs::write(out, s).unwrap();
}
