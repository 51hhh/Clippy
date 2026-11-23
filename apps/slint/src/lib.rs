slint::include_modules!();

use clippy_platform::identity::DeploymentIdentity;

pub const UI_BACKEND: &str = "Slint";
pub const FRAMEWORK_VERSION: &str = "1.19.0";

pub fn run(identity: &'static DeploymentIdentity) -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    window.set_product_name(identity.product_name.into());
    window.set_build_label(build_label(identity).into());
    window.run()
}

pub fn build_label(identity: &DeploymentIdentity) -> String {
    format!(
        "v{} · {UI_BACKEND} {FRAMEWORK_VERSION} · {}",
        env!("CARGO_PKG_VERSION"),
        identity.channel.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_platform::identity::{PRODUCTION_IDENTITY, SLINT_QA_IDENTITY};

    #[test]
    fn framework_identity_is_explicit() {
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.2.0");
        assert_eq!(UI_BACKEND, "Slint");
        assert_eq!(FRAMEWORK_VERSION, "1.19.0");
        assert_eq!(clippy_core::PRODUCT_NAME, "Clippy");
    }

    #[test]
    fn build_label_exposes_the_selected_distribution_channel() {
        assert!(build_label(&PRODUCTION_IDENTITY).ends_with("production"));
        assert!(build_label(&SLINT_QA_IDENTITY).ends_with("qa"));
    }
}
