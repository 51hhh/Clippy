use clippy_platform::identity::PRODUCTION_IDENTITY;

fn main() -> Result<(), clippy_slint::AppError> {
    clippy_slint::run(&PRODUCTION_IDENTITY)
}
