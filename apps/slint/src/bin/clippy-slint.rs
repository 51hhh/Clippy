use clippy_platform::identity::PRODUCTION_IDENTITY;

fn main() -> anyhow::Result<()> {
    clippy_slint::run(&PRODUCTION_IDENTITY)
}
