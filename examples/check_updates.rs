fn main() -> anyhow::Result<()> {
    let (release, published) = scribe_seek::updater::check_latest(env!("CARGO_PKG_VERSION"))?;
    if let Some(release) = release {
        println!(
            "Update available: {} ({} bytes)",
            release.version, release.size
        );
    } else if published {
        println!("Up to date");
    } else {
        println!("No GitHub Releases have been published yet");
    }
    Ok(())
}
