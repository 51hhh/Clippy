use clippy_platform::identity::SLINT_QA_IDENTITY;

fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--probe-capture")
    {
        let started = std::time::Instant::now();
        let outputs = clippy_platform::capture::freeze()?;
        println!("freeze_ms={:.1}", started.elapsed().as_secs_f64() * 1000.0);
        for output in outputs {
            println!(
                "{}: logical={}x{}@{},{} pixels={}x{}",
                output.name,
                output.geometry.width,
                output.geometry.height,
                output.geometry.x,
                output.geometry.y,
                output.pixels.width,
                output.pixels.height,
            );
        }
        return Ok(());
    }
    clippy_slint::run(&SLINT_QA_IDENTITY)
}
