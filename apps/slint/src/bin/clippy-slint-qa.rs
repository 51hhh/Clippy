use clippy_platform::identity::SLINT_QA_IDENTITY;

fn main() -> Result<(), slint::PlatformError> {
    clippy_slint::run(&SLINT_QA_IDENTITY)
}
