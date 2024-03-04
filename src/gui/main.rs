use torrent_engine::{download_torrent, logger};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::Logger::init(format!(
        "/tmp/log{}.txt",
        chrono::Local::now().format("%d-%m-%Y_%H-%M-%S")
    ))?;
    download_torrent("file.torrent", "path/to/save").await?;
    Ok(())
}
