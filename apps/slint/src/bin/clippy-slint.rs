use clippy_platform::identity::PRODUCTION_IDENTITY;

fn main() -> Result<(), slint::PlatformError> {
    clippy_slint::run(&PRODUCTION_IDENTITY)
}
