//! `pdf_worker`: the isolated PDF engine process (ADR 0008).
//!
//! MVP-01 only prints its version; MuPDF (MVP-03) and the IPC loop (MVP-04) come later.

fn version_banner() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

fn main() {
    println!("{}", version_banner());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_contains_name_and_version() {
        assert_eq!(version_banner(), "pdf_worker 0.1.0");
    }
}
