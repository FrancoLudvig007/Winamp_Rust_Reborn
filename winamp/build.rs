//! Copia `winamp.ico` (raiz do projeto) para OUT_DIR, de onde `main.rs` o embute no executável.
//! Se o arquivo não existir, o build continua (sem ícone) e emite um aviso.
use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=winamp.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR ausente")).join("winamp.ico");
    if fs::copy("winamp.ico", &out).is_err() {
        println!("cargo:warning=winamp.ico não encontrado na raiz do projeto; compilando sem ícone embutido");
        fs::write(&out, []).expect("falha ao criar ícone vazio");
    }
}