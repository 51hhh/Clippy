slint::include_modules!();

const UI_BACKEND: &str = "Slint";
const FRAMEWORK_VERSION: &str = "1.19.0";

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    window.set_build_label(
        format!(
            "v{} · {UI_BACKEND} {FRAMEWORK_VERSION}",
            env!("CARGO_PKG_VERSION")
        )
        .into(),
    );
    window.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framework_identity_is_explicit() {
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.2.0");
        assert_eq!(UI_BACKEND, "Slint");
        assert_eq!(FRAMEWORK_VERSION, "1.19.0");
    }
}
