#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    wftpg::communication::dbus::run_daemon().await?;
    Ok(())
}
