use clippy_platform::identity::SLINT_QA_IDENTITY;

fn main() -> Result<(), clippy_slint::AppError> {
    clippy_slint::run(&SLINT_QA_IDENTITY)
}
