//! Writes `src/ipc/generated/contract.ts`. Run through `pnpm ipc:generate`.

fn main() -> std::io::Result<()> {
    let path = ipc_contract::typescript::output_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, ipc_contract::typescript::bindings())?;
    println!("wrote {}", path.display());
    Ok(())
}
